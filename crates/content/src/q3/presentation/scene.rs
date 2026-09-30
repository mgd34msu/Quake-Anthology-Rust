//! Quake III presentation: scene.
//!
//! Donor provenance: `src/content/q3/presentation/scene.ts`.

use qa_core::math::{vec4, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::ref_entity::*;
use crate::q3::presentation::refdef::*;

// ---------------------------------------------------------------------------
// scene.ts
// ---------------------------------------------------------------------------

/// Scene admission origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneAdmissionOrigin {
    /// Native.
    Native,
    /// Mixed.
    Mixed,
}

/// Scene admission identity (`SceneAdmissionIdentity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q3SceneAdmissionId {
    /// Origin.
    pub origin: SceneAdmissionOrigin,
    /// Unique token.
    pub token: u64,
}

pub(crate) static ADMISSION_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Scene admission snapshot (`Q3SceneAdmission`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneAdmission {
    /// Identity.
    pub id: Q3SceneAdmissionId,
    /// Entities.
    pub entities: Vec<Q3AdmittedRefEntity>,
    /// Polygons.
    pub polygons: Vec<Q3AdmittedPoly>,
}

/// Snapshot a scene admission (`snapshotQ3SceneAdmission`).
#[must_use]
pub fn snapshot_q3_scene_admission(
    origin: SceneAdmissionOrigin,
    entities: Vec<Q3AdmittedRefEntity>,
    polygons: Vec<Q3AdmittedPoly>,
) -> Q3SceneAdmission {
    let token = ADMISSION_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Q3SceneAdmission {
        id: Q3SceneAdmissionId { origin, token },
        entities,
        polygons,
    }
}

/// Fog selection (`Q3FogSelection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3FogSelection {
    /// Index.
    pub index: i32,
    /// Volume.
    pub volume: FogVolume,
}

/// Admitted polygon with fog (`Q3AdmittedPoly`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AdmittedPoly {
    /// Polygon.
    pub poly: RefPoly,
    /// Fog.
    pub fog: Option<Q3FogSelection>,
}

/// Admit a polygon with fog selection (`admitQ3Poly`).
pub fn admit_q3_poly(poly: &RefPoly, fogs: &[Q3FogSelection]) -> PresentResult<Q3AdmittedPoly> {
    let copied = copy_ref_poly(poly);
    if fogs.is_empty() {
        return Ok(Q3AdmittedPoly {
            poly: copied,
            fog: None,
        });
    }
    let first = copied
        .vertices
        .first()
        .ok_or_else(|| PresentError::range("Source polygon fog requires its first admitted vertex"))?;
    let mut min = first.position;
    let mut max = first.position;
    for vertex in &copied.vertices {
        min.x = min.x.min(vertex.position.x);
        min.y = min.y.min(vertex.position.y);
        min.z = min.z.min(vertex.position.z);
        max.x = max.x.max(vertex.position.x);
        max.y = max.y.max(vertex.position.y);
        max.z = max.z.max(vertex.position.z);
    }
    let fog = fogs
        .iter()
        .find(|selection| {
            let bounds = selection.volume.bounds;
            max.x >= bounds.min.x
                && max.y >= bounds.min.y
                && max.z >= bounds.min.z
                && min.x <= bounds.max.x
                && min.y <= bounds.max.y
                && min.z <= bounds.max.z
        })
        .copied();
    Ok(Q3AdmittedPoly { poly: copied, fog })
}

/// Procedural fog selection (`q3ProceduralFog`).
#[must_use]
pub fn q3_procedural_fog(origin: Vec3, radius: f32, fogs: &[Q3FogSelection]) -> Option<Q3FogSelection> {
    fogs.iter()
        .find(|selection| {
            let bounds = selection.volume.bounds;
            origin.x - radius < bounds.max.x
                && origin.x + radius > bounds.min.x
                && origin.y - radius < bounds.max.y
                && origin.y + radius > bounds.min.y
                && origin.z - radius < bounds.max.z
                && origin.z + radius > bounds.min.z
        })
        .copied()
}

/// Geometry admission reference (`Q3GeometryAdmission`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GeometryAdmission {
    /// Reference entity.
    RefEntity {
        /// Index.
        index: usize,
    },
    /// Polygon.
    Polygon {
        /// Index.
        index: usize,
    },
}

/// Presented special entity (beam or default model).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedSpecialEntity {
    /// Entity index.
    pub entity_index: usize,
    /// Source.
    pub source: SpecialEntitySource,
}

/// Special entity source.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SpecialEntitySource {
    /// Beam.
    Beam(RefBeamEntity),
    /// Model.
    Model(RefModelEntity),
}

/// Presented portal.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedPortal {
    /// Entity index.
    pub entity_index: usize,
    /// Source.
    pub source: RefPortalEntity,
}

/// Presented model (`PresentedModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedModel {
    /// Entity index.
    pub entity_index: usize,
    /// Scene entity.
    pub entity: PresentSceneEntity,
    /// Model source options.
    pub options: ModelSourceOptions,
    /// Source entity.
    pub source: RefModelEntity,
}

/// Prepare an authored model for the shared renderer (`prepareQ3Model`).
#[must_use]
pub fn prepare_q3_model(
    entity: &RefModelEntity,
    actor: Option<ActorId>,
    entity_index: usize,
) -> Option<PresentedModel> {
    let model = match &entity.model {
        SceneModel::Default(_) => return None,
        SceneModel::Inline(inline) => PresentEntityModel::BrushModel {
            world: inline.geometry.clone(),
            model: inline.index,
        },
        SceneModel::Loaded(loaded) => PresentEntityModel::Decoded(loaded.model.clone()),
    };
    let resource = match &entity.model {
        SceneModel::Loaded(loaded) => loaded.resource.clone(),
        SceneModel::Inline(inline) => inline.resource.clone(),
        SceneModel::Default(_) => return None,
    };
    Some(PresentedModel {
        entity_index,
        entity: PresentSceneEntity {
            actor,
            resource,
            model,
            origin: entity.origin,
            axis: entity.axis,
            previous_origin: entity.old_origin,
            frame: entity.frame,
            previous_frame: entity.old_frame,
            back_lerp: entity.back_lerp,
            skin: entity.skin_num,
            color: vec4(
                entity.shading.shader_rgba.x / 255.0,
                entity.shading.shader_rgba.y / 255.0,
                entity.shading.shader_rgba.z / 255.0,
                entity.shading.shader_rgba.w / 255.0,
            ),
            shader_time: entity.shading.shader_time,
            render_flags: entity.shading.render_flags,
            lighting_origin: entity.lighting_origin,
            shadow_plane: entity.shadow_plane,
        },
        source: entity.clone(),
        options: ModelSourceOptions {
            custom_shader: entity.shading.custom_shader.as_ref().map(|shader| shader.name.clone()),
            custom_skin: entity.custom_skin.as_ref().map(|skin| skin.surfaces.clone()),
            non_normalized_axes: entity.non_normalized_axes,
        },
    })
}

/// Presented effect geometry (`PresentedGeometry`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedGeometry {
    /// Admission.
    pub admission: Q3GeometryAdmission,
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Source.
    pub source: PresentedGeometrySource,
}

/// Effect geometry source.
#[derive(Debug, Clone, PartialEq)]
pub enum PresentedGeometrySource {
    /// Sprite.
    Sprite(RefSpriteEntity),
    /// Rail core.
    RailCore(RefRailCoreEntity),
    /// Rail rings.
    RailRings(RefRailRingsEntity),
    /// Lightning.
    Lightning(RefLightningEntity),
    /// Polygon.
    Poly(Q3AdmittedPoly),
}

/// Scene content (`Q3SceneContent`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneContent {
    /// Admission.
    pub admission: Q3SceneAdmission,
    /// Models.
    pub models: Vec<PresentedModel>,
    /// Effects.
    pub effects: Vec<PresentedGeometry>,
    /// Special entities.
    pub special_entities: Vec<PresentedSpecialEntity>,
    /// Portals.
    pub portals: Vec<PresentedPortal>,
    /// Lights.
    pub lights: Vec<PresentSceneLight>,
}

/// Presented scene (`Q3PresentedScene`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PresentedScene {
    /// Content.
    pub content: Q3SceneContent,
    /// Seat.
    pub seat: SeatId,
    /// Viewport.
    pub viewport: Rect,
    /// Camera.
    pub camera: SceneCamera,
    /// Source refdef.
    pub source: Refdef,
}

/// Scene target (`Q3SceneTarget`).
pub trait Q3SceneTarget {
    /// Seat.
    fn seat(&self) -> SeatId;
    /// Viewport.
    fn viewport(&self) -> Rect;
    /// Far clip.
    fn far_clip(&self) -> f32;
    /// Near clip.
    fn near_clip(&self) -> f32;
    /// Rail settings.
    fn rail(&self) -> RailSettings;
    /// Fog selections.
    fn fog_selections(&self) -> Vec<Q3FogSelection>;
    /// Print.
    fn print(&mut self, text: &str);
    /// Actor for an entity.
    fn actor(&self, entity: &RefModelEntity) -> Option<ActorId>;
    /// Publish a scene.
    fn publish(&mut self, scene: Q3PresentedScene);
}

/// Per-seat scene recorder (`Q3SceneRecorder`).
pub struct Q3SceneRecorder {
    /// Target.
    pub target: Box<dyn Q3SceneTarget>,
    /// Entities.
    entities: Vec<Q3AdmittedRefEntity>,
    /// Polygons.
    polygons: Vec<Q3AdmittedPoly>,
    /// Lights.
    lights: Vec<PresentSceneLight>,
}

impl Q3SceneRecorder {
    /// New recorder.
    pub fn new(target: Box<dyn Q3SceneTarget>) -> Self {
        Self {
            target,
            entities: Vec::new(),
            polygons: Vec::new(),
            lights: Vec::new(),
        }
    }

    /// Clear the scene.
    pub fn clear_scene(&mut self) {
        self.entities.clear();
        self.polygons.clear();
        self.lights.clear();
    }

    /// Add a reference entity.
    pub fn add_ref_entity(&mut self, entity: &Q3AdmittedRefEntity) {
        self.entities.push(copy_admitted_ref_entity(entity));
    }

    /// Add a polygon.
    pub fn add_poly(&mut self, poly: &RefPoly) -> PresentResult<()> {
        if poly.shader.is_none() {
            self.target.print("^3WARNING: RE_AddPolyToScene: NULL poly shader\n");
            return Ok(());
        }
        let fogs = self.target.fog_selections();
        self.polygons.push(admit_q3_poly(poly, &fogs)?);
        Ok(())
    }

    /// Add a light.
    pub fn add_light(&mut self, light: &DynamicLight) {
        self.lights.push(PresentSceneLight {
            origin: light.origin,
            radius: light.radius,
            color: light.color,
            additive: light.additive,
        });
    }

    /// Capture scene content.
    pub fn capture(&self) -> Q3SceneContent {
        let admission = snapshot_q3_scene_admission(
            SceneAdmissionOrigin::Native,
            self.entities.clone(),
            self.polygons.clone(),
        );
        let mut models = Vec::new();
        let mut effects = Vec::new();
        let mut portals = Vec::new();
        let mut special_entities = Vec::new();
        for (entity_index, entity) in admission.entities.iter().enumerate() {
            match entity {
                Q3AdmittedRefEntity::Poly(_) => continue,
                Q3AdmittedRefEntity::Entity(RefEntity::Portal(source)) => {
                    portals.push(PresentedPortal {
                        entity_index,
                        source: source.clone(),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Beam(source)) => {
                    special_entities.push(PresentedSpecialEntity {
                        entity_index,
                        source: SpecialEntitySource::Beam(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Model(source)) => {
                    if source.model.is_default() {
                        special_entities.push(PresentedSpecialEntity {
                            entity_index,
                            source: SpecialEntitySource::Model(source.clone()),
                        });
                        continue;
                    }
                    if let Some(prepared) = prepare_q3_model(source, self.target.actor(source), entity_index) {
                        models.push(prepared);
                    }
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Sprite(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::Sprite(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::RailCore(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::RailCore(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::RailRings(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::RailRings(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Lightning(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::Lightning(source.clone()),
                    });
                }
            }
        }
        for (index, poly) in admission.polygons.iter().enumerate() {
            effects.push(PresentedGeometry {
                admission: Q3GeometryAdmission::Polygon { index },
                shader: poly.poly.shader.clone(),
                source: PresentedGeometrySource::Poly(poly.clone()),
            });
        }
        Q3SceneContent {
            admission,
            models,
            effects,
            portals,
            special_entities,
            lights: self.lights.clone(),
        }
    }

    /// Render a scene.
    pub fn render_scene(&mut self, input: &Refdef) {
        let source = copy_refdef(input);
        let content = self.capture();
        let viewport = Rect {
            x: self.target.viewport().x + source.x,
            y: self.target.viewport().y + source.y,
            width: source.width,
            height: source.height,
        };
        let camera = SceneCamera {
            viewport,
            origin: source.view_origin,
            axis: source.view_axis,
            projection: perspective_projection(
                source.fov_x,
                source.fov_y,
                self.target.far_clip(),
                self.target.near_clip(),
            ),
        };
        self.target.publish(Q3PresentedScene {
            content,
            seat: self.target.seat(),
            viewport,
            camera,
            source,
        });
    }
}
