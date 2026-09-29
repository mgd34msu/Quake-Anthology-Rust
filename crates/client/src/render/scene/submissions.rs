//! Scene submission grouping and final operation ordering.
//!
//! Donor provenance: `src/render/scene/submissions.ts`. Surfaces submit
//! sortable groups (compiled materials, source-ranked Q3 draws, legacy
//! sequence phases); [`finish_scene_operations`] flattens each pending run
//! into ordered render operations. Source ranking packs through
//! [`crate::render::pack_draw_sort`] and sorts with the source quicksort in
//! `super::source_sort`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::render::types::{DrawBatch, RenderOperation};
use crate::render::RenderError;
use crate::render::{ENTITY_WORLD, MAX_DRAW_SORT_DLIGHT, MAX_DRAW_SORT_FOG, MAX_DRAW_SORT_SHADER};

use super::material_registrations::{RegisteredSceneMaterial, ShaderRegistration};
use super::source_sort::{pack_source_draw_sort, sort_draw_surfs, SortRange};

static NEXT_ORDER_ID: AtomicU64 = AtomicU64::new(1);

/// Draw-sort rank snapshot plus the refentity allocator for one source view.
#[derive(Debug)]
struct OrderInner {
    id: u64,
    ranks: Vec<ShaderRegistration>,
    entities: std::sync::atomic::AtomicU32,
}

/// Source scene order: shader-rank snapshot with a shared entity allocator.
#[derive(Debug, Clone)]
pub struct SourceSceneOrder {
    inner: Arc<OrderInner>,
}

impl SourceSceneOrder {
    /// Rank lookup shared by every surface submitted in this view.
    #[must_use]
    pub fn ranks(&self) -> &[ShaderRegistration] {
        &self.inner.ranks
    }

    /// Reserve `count` refentity slots; returns the first index.
    pub fn reserve_entities(&self, count: u32) -> Result<u32, RenderError> {
        let mut current = self.inner.entities.load(Ordering::SeqCst);
        loop {
            let end = current
                .checked_add(count)
                .ok_or_else(|| RenderError::Backend("Source view exceeds the refentity draw-sort field".to_string()))?;
            if end > ENTITY_WORLD {
                return Err(RenderError::Backend(
                    "Source view exceeds the refentity draw-sort field".to_string(),
                ));
            }
            match self
                .inner
                .entities
                .compare_exchange(current, end, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Ok(current),
                Err(actual) => current = actual,
            }
        }
    }
}

impl PartialEq for SourceSceneOrder {
    fn eq(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
    }
}

impl Eq for SourceSceneOrder {}

/// Build a source order over a rank snapshot (index = shader rank).
#[must_use]
pub fn create_source_scene_order(ranks: Vec<ShaderRegistration>) -> SourceSceneOrder {
    SourceSceneOrder {
        inner: Arc::new(OrderInner {
            id: NEXT_ORDER_ID.fetch_add(1, Ordering::SeqCst),
            ranks,
            entities: std::sync::atomic::AtomicU32::new(0),
        }),
    }
}

/// Reserve `count` refentity slots in a source view.
pub fn reserve_source_entity_range(view: &SourceSceneOrder, count: u32) -> Result<u32, RenderError> {
    view.reserve_entities(count)
}

/// Entity half of a source draw-sort word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEntityOrder {
    /// World entity (rank 1022).
    World,
    /// Reference entity slot.
    RefEntity {
        /// Entity index below 1022.
        index: u32,
    },
}

/// Full source draw-sort position for one surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSurfaceOrder {
    /// Owning view.
    pub view: SourceSceneOrder,
    /// Entity half.
    pub entity: SourceEntityOrder,
    /// Surface index.
    pub surface: u32,
    /// Fog index (0..=31).
    pub fog: u32,
    /// Dlight flag (0..=3).
    pub dlight: u32,
}

/// Legacy submission phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequencePhase {
    /// Sky (sort 2).
    Sky,
    /// Opaque (sort 3).
    Opaque,
    /// Translucent (sort 9).
    Translucent,
}

/// Sortable group order.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneGroupOrder {
    /// Generic compiled material.
    Compiled {
        /// Material.
        material: RegisteredSceneMaterial,
    },
    /// Source-ranked Q3 draw.
    Source {
        /// Material.
        material: RegisteredSceneMaterial,
        /// Draw-sort position.
        source: SourceSurfaceOrder,
    },
    /// Legacy sequence phase.
    Sequence {
        /// Phase.
        phase: SequencePhase,
    },
}

/// One sortable scene group.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneGroup {
    /// Group order.
    pub order: SceneGroupOrder,
    /// Group operations.
    pub operations: Vec<RenderOperation>,
}

/// A scene group holding draw operations.
pub type SceneModelGroup = SceneGroup;

/// Either a passthrough operation or a sortable group.
// Groups own their batches inline; submission vectors are short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum SceneOperation {
    /// Passthrough render operation.
    Operation(RenderOperation),
    /// Sortable group.
    Group(SceneGroup),
}

/// Group compiled-material batches.
#[must_use]
pub fn compiled_draw_group(material: RegisteredSceneMaterial, batches: Vec<DrawBatch>) -> SceneModelGroup {
    SceneGroup {
        order: SceneGroupOrder::Compiled { material },
        operations: vec![RenderOperation::Draw(batches)],
    }
}

/// Group source-ranked batches after validating the draw-sort fields.
pub fn source_draw_group(
    material: RegisteredSceneMaterial,
    source: SourceSurfaceOrder,
    batches: Vec<DrawBatch>,
) -> Result<SceneModelGroup, RenderError> {
    if let SourceEntityOrder::RefEntity { index } = source.entity {
        if index >= ENTITY_WORLD {
            return Err(RenderError::Backend(
                "Source refentity index exceeds the draw-sort field".to_string(),
            ));
        }
    }
    if source.fog > MAX_DRAW_SORT_FOG {
        return Err(RenderError::Backend(
            "Source fog index exceeds the draw-sort field".to_string(),
        ));
    }
    if source.dlight > MAX_DRAW_SORT_DLIGHT {
        return Err(RenderError::Backend(
            "Source light flag exceeds the draw-sort field".to_string(),
        ));
    }
    if !source.view.ranks().contains(&material.registration) {
        return Err(RenderError::Backend(
            "Source material belongs to another scene".to_string(),
        ));
    }
    Ok(SceneGroup {
        order: SceneGroupOrder::Source { material, source },
        operations: if batches.is_empty() {
            Vec::new()
        } else {
            vec![RenderOperation::Draw(batches)]
        },
    })
}

/// Group legacy sequence-phase batches.
#[must_use]
pub fn sequence_draw_group(phase: SequencePhase, batches: Vec<DrawBatch>) -> SceneModelGroup {
    SceneGroup {
        order: SceneGroupOrder::Sequence { phase },
        operations: vec![RenderOperation::Draw(batches)],
    }
}

/// Flatten draw batches out of model groups.
#[must_use]
pub fn scene_model_batches(groups: &[SceneModelGroup]) -> Vec<DrawBatch> {
    groups
        .iter()
        .flat_map(|group| group.operations.iter())
        .filter_map(|operation| match operation {
            RenderOperation::Draw(batches) => Some(batches.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn priority(order: &SceneGroupOrder) -> i32 {
    match order {
        SceneGroupOrder::Compiled { material } | SceneGroupOrder::Source { material, .. } => material.finished.sort,
        SceneGroupOrder::Sequence { phase } => match phase {
            SequencePhase::Sky => 2,
            SequencePhase::Opaque => 3,
            SequencePhase::Translucent => 9,
        },
    }
}

struct PendingEntry {
    group: SceneGroup,
    ordinal: usize,
}

struct SourceEntry {
    group: SceneGroup,
    ordinal: usize,
    sort: u32,
}

struct SourceRange<'a> {
    entries: &'a mut Vec<SourceEntry>,
}

impl SortRange for SourceRange<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get_sort(&self, index: usize) -> u32 {
        self.entries[index].sort
    }

    fn set_sort(&mut self, index: usize, sort: u32) {
        self.entries[index].sort = sort;
    }

    fn swap(&mut self, first: usize, second: usize) {
        self.entries.swap(first, second);
    }
}

/// Flatten sortable groups into ordered render operations. Pending groups
/// flush before each passthrough operation; source groups sort by packed
/// draw-sort word, then merge stably with compiled and sequence groups by
/// priority.
pub fn finish_scene_operations(input: Vec<SceneOperation>) -> Result<Vec<RenderOperation>, RenderError> {
    let mut result = Vec::new();
    let mut pending: Vec<PendingEntry> = Vec::new();
    let flush = |pending: &mut Vec<PendingEntry>, result: &mut Vec<RenderOperation>| -> Result<(), RenderError> {
        let mut generic: Vec<&PendingEntry> = pending
            .iter()
            .filter(|entry| matches!(entry.group.order, SceneGroupOrder::Compiled { .. }))
            .collect();
        generic.sort_by(|a, b| {
            priority(&a.group.order)
                .cmp(&priority(&b.group.order))
                .then_with(|| a.ordinal.cmp(&b.ordinal))
        });
        let mut source: Vec<SourceEntry> = pending
            .iter()
            .filter(|entry| matches!(entry.group.order, SceneGroupOrder::Source { .. }))
            .map(|entry| SourceEntry {
                group: entry.group.clone(),
                ordinal: entry.ordinal,
                sort: 0,
            })
            .collect();
        if let Some(first) = source.first() {
            let SceneGroupOrder::Source { source: order, .. } = &first.group.order else {
                unreachable!();
            };
            let view = order.view.clone();
            let ranks: std::collections::HashMap<ShaderRegistration, usize> = view
                .ranks()
                .iter()
                .enumerate()
                .map(|(index, registration)| (*registration, index))
                .collect();
            for entry in &mut source {
                let SceneGroupOrder::Source {
                    material,
                    source: order,
                } = &entry.group.order
                else {
                    unreachable!();
                };
                if order.view != view {
                    return Err(RenderError::Backend(
                        "Different source views share a sortable range".to_string(),
                    ));
                }
                let rank = ranks
                    .get(&material.registration)
                    .copied()
                    .ok_or_else(|| RenderError::Backend("Source material has no published shader rank".to_string()))?;
                if rank as u32 > MAX_DRAW_SORT_SHADER {
                    return Err(RenderError::Backend(
                        "Source shader rank exceeds the draw-sort field".to_string(),
                    ));
                }
                let entity = match order.entity {
                    SourceEntityOrder::World => ENTITY_WORLD,
                    SourceEntityOrder::RefEntity { index } => index,
                };
                entry.sort = pack_source_draw_sort(rank as u32, entity, order.fog, order.dlight)?;
            }
            sort_draw_surfs(&mut SourceRange { entries: &mut source })?;
        }
        let mut compiled: Vec<PendingEntry> = Vec::new();
        let mut source_index = 0;
        let mut generic_index = 0;
        while source_index < source.len() || generic_index < generic.len() {
            let native = source.get(source_index);
            let other = generic.get(generic_index);
            let take_native = match (native, other) {
                (Some(native), Some(other)) => {
                    priority(&native.group.order) < priority(&other.group.order)
                        || priority(&native.group.order) == priority(&other.group.order)
                            && native.ordinal < other.ordinal
                }
                (Some(_), None) => true,
                _ => false,
            };
            if take_native {
                let entry = &source[source_index];
                compiled.push(PendingEntry {
                    group: entry.group.clone(),
                    ordinal: entry.ordinal,
                });
                source_index += 1;
            } else if let Some(other) = other {
                compiled.push(PendingEntry {
                    group: other.group.clone(),
                    ordinal: other.ordinal,
                });
                generic_index += 1;
            }
        }
        let sequence: Vec<&PendingEntry> = pending
            .iter()
            .filter(|entry| matches!(entry.group.order, SceneGroupOrder::Sequence { .. }))
            .collect();
        let mut material_index = 0;
        let mut sequence_index = 0;
        while material_index < compiled.len() || sequence_index < sequence.len() {
            let material = compiled.get(material_index);
            let legacy = sequence.get(sequence_index);
            let take_material = match (material, legacy) {
                (Some(material), Some(legacy)) => {
                    priority(&material.group.order) < priority(&legacy.group.order)
                        || priority(&material.group.order) == priority(&legacy.group.order)
                            && material.ordinal < legacy.ordinal
                }
                (Some(_), None) => true,
                _ => false,
            };
            if take_material {
                result.extend(compiled[material_index].group.operations.clone());
                material_index += 1;
            } else if let Some(legacy) = legacy {
                result.extend(legacy.group.operations.clone());
                sequence_index += 1;
            }
        }
        pending.clear();
        Ok(())
    };
    for operation in input {
        match operation {
            SceneOperation::Group(group) => {
                let ordinal = pending.len();
                pending.push(PendingEntry { group, ordinal });
            }
            SceneOperation::Operation(operation) => {
                flush(&mut pending, &mut result)?;
                result.push(operation);
            }
        }
    }
    flush(&mut pending, &mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::compile::{
        default_shader_profile, shader_render_material, RegisteredExplicitShader, RegisteredStage, RegistrationOutcome,
    };
    use crate::materials::finish::{finish_implicit_shader, FinishImplicitShaderInput, ImplicitShaderKind};
    use crate::materials::material::ShaderDefinition;
    use crate::materials::state::CullFace;
    use crate::render::types::{
        AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace as ContractCull, DepthTest,
        RenderState, RenderVertex, TextureBinding,
    };
    use qa_core::math::{vec2, vec4};

    fn registered(name: &str, sort: i32) -> RegisteredSceneMaterial {
        let definition = ShaderDefinition {
            name: name.to_string(),
            stages: Vec::new(),
            surface_parms: Vec::new(),
            cull: CullFace::Back,
            sort: None,
            sky: None,
            fog: None,
            sun: None,
            deforms: Vec::new(),
            polygon_offset: false,
            no_mipmaps: false,
            no_picmip: false,
            entity_mergable: false,
            portal_range: 0.0,
            clamp_time: 0.0,
            warnings: Vec::new(),
            compiler_directives: Vec::new(),
        };
        let mut finished = finish_implicit_shader(&FinishImplicitShaderInput {
            name: name.to_string(),
            base_image: RegisteredStage::Missing,
            profile: default_shader_profile(),
            kind: ImplicitShaderKind::Default,
        })
        .expect("implicit finish");
        finished.sort = sort;
        let mut table = super::super::material_registrations::MaterialRegistrationTable::default();
        table.admit(crate::materials::compile::CompiledMaterial {
            registered: RegisteredExplicitShader {
                definition: definition.clone(),
                stages: Vec::new(),
                sky: None,
                outcome: RegistrationOutcome::Defined,
            },
            finished,
            material: shader_render_material(&definition).expect("view"),
        })
    }

    fn batch() -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0],
            texture: TextureBinding::RetainCurrentTexture,
            state: RenderState::opaque(ContractCull::Back),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![RenderVertex {
                position: vec4(0.0, 0.0, 0.0, 1.0),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }]),
        }
    }

    #[test]
    fn entity_reservation_caps_at_world_slot() {
        let view = create_source_scene_order(Vec::new());
        assert_eq!(reserve_source_entity_range(&view, 4).unwrap(), 0);
        assert_eq!(view.reserve_entities(2).unwrap(), 4);
        assert!(view.reserve_entities(ENTITY_WORLD).is_err());
    }

    #[test]
    fn source_group_validates_fields_and_scene() {
        let material = registered("unit/submission-a", 3);
        let view = create_source_scene_order(vec![material.registration]);
        let order = SourceSurfaceOrder {
            view: view.clone(),
            entity: SourceEntityOrder::RefEntity { index: 5 },
            surface: 7,
            fog: 2,
            dlight: 1,
        };
        assert!(source_draw_group(material.clone(), order.clone(), vec![batch()]).is_ok());
        let bad_entity = SourceSurfaceOrder {
            entity: SourceEntityOrder::RefEntity { index: ENTITY_WORLD },
            ..order.clone()
        };
        assert!(source_draw_group(material.clone(), bad_entity, vec![batch()]).is_err());
        let bad_fog = SourceSurfaceOrder {
            fog: 32,
            ..order.clone()
        };
        assert!(source_draw_group(material.clone(), bad_fog, vec![batch()]).is_err());
        let bad_light = SourceSurfaceOrder {
            dlight: 4,
            ..order.clone()
        };
        assert!(source_draw_group(material.clone(), bad_light, vec![batch()]).is_err());
        let foreign = registered("unit/submission-foreign", 3);
        assert!(source_draw_group(foreign, order, vec![batch()]).is_err());
    }

    #[test]
    fn finish_orders_compiled_before_translucent_sequence() {
        let material = registered("unit/submission-order", 3);
        let groups = vec![
            SceneOperation::Group(sequence_draw_group(SequencePhase::Translucent, vec![batch()])),
            SceneOperation::Group(compiled_draw_group(material, vec![batch()])),
            SceneOperation::Group(sequence_draw_group(SequencePhase::Sky, vec![batch()])),
        ];
        let operations = finish_scene_operations(groups).expect("finish");
        assert_eq!(operations.len(), 3);
    }

    #[test]
    fn finish_sorts_source_groups_by_packed_word() {
        let low = registered("unit/submission-low", 3);
        let high = registered("unit/submission-high", 3);
        let view = create_source_scene_order(vec![low.registration, high.registration]);
        let order_for = |_material: &RegisteredSceneMaterial, entity| SourceSurfaceOrder {
            view: view.clone(),
            entity,
            surface: 0,
            fog: 0,
            dlight: 0,
        };
        let high_group = source_draw_group(
            high.clone(),
            order_for(&high, SourceEntityOrder::RefEntity { index: 1 }),
            vec![batch()],
        )
        .unwrap();
        let low_group = source_draw_group(
            low.clone(),
            order_for(&low, SourceEntityOrder::RefEntity { index: 9 }),
            vec![batch()],
        )
        .unwrap();
        let operations = finish_scene_operations(vec![
            SceneOperation::Group(high_group),
            SceneOperation::Group(low_group),
        ])
        .expect("finish");
        assert_eq!(operations.len(), 2);
        assert!(matches!(operations[0], RenderOperation::Draw(_)));
    }

    #[test]
    fn finish_flushes_before_passthrough() {
        let material = registered("unit/submission-flush", 9);
        let plain = RenderOperation::DepthRange([0.0, 1.0]);
        let operations = finish_scene_operations(vec![
            SceneOperation::Group(compiled_draw_group(material, vec![batch()])),
            SceneOperation::Operation(plain.clone()),
        ])
        .expect("finish");
        assert_eq!(operations.len(), 2);
        assert!(matches!(operations[0], RenderOperation::Draw(_)));
        assert_eq!(operations[1], plain);
    }

    #[test]
    fn batch_flattening_skips_non_draws() {
        let group = SceneGroup {
            order: SceneGroupOrder::Sequence {
                phase: SequencePhase::Opaque,
            },
            operations: vec![
                RenderOperation::Draw(vec![batch()]),
                RenderOperation::DepthRange([0.0, 1.0]),
            ],
        };
        assert_eq!(scene_model_batches(&[group]).len(), 1);
        let _ = (AlphaTest::None, BlendFactor::One, DepthTest::LessEqual);
    }
}
