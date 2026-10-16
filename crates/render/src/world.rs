//! BSP boundary conversion. Drawing uses only the normalized world primitives.
use crate::{
    assets::{ImageId, MaterialId, ModelId, ShaderBinding, ShaderBindingId, ShaderRegistration},
    scene::{Refdef, Span},
};
use qa_core::primitives::{Plane, RuleSetId};
use qa_formats::bsp::{Lump, Map};
use qa_world::visibility::{
    PvsRows, SurfaceSpan, VisLeaf, VisNode, VisibilityError, VisibilityWorld,
};
use qa_world::visibility::{ViewVisibility, VisibilityQueryError};

pub mod geometry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldPresentationId(pub u32);

pub struct WorldPresentation {
    pub world: WorldId,
    pub(crate) bindings: Box<[SurfaceBinding]>,
    registrations: Box<[ShaderRegistration]>,
    native_page0: Option<ImageId>,
    native_default: Option<ShaderBindingId>,
    initial_sun_direction: qa_core::primitives::Vec3,
    initial_sun_light: qa_core::primitives::Vec3,
    pub(crate) profile: Option<(
        crate::material::world_load::WorldPresentationProfile,
        crate::material::world_load::PresentationDefaults,
    )>,
}
impl WorldPresentation {
    pub(crate) fn bind(
        world: WorldId,
        materials: &[SurfaceMaterial],
        mesh_ranges: impl Iterator<Item = Span>,
        shaders: impl Iterator<Item = ShaderBindingId>,
        registrations: Box<[ShaderRegistration]>,
        native_page0: Option<ImageId>,
        native_default: Option<ShaderBindingId>,
        name_capacity: usize,
        shader_capacity: usize,
    ) -> Result<Self, &'static str> {
        let mut context =
            crate::shader::context::ShaderContext::load(name_capacity, shader_capacity)?;
        let mut append = context.append();
        append.reset_sun_direction();
        for &registration in &registrations {
            append.admit(registration)?;
        }
        append.commit();
        let initial_sun_direction = context.sun_direction();
        let initial_sun_light = context.sun_light();
        Ok(Self {
            world,
            bindings: materials
                .iter()
                .zip(mesh_ranges)
                .zip(shaders)
                .map(|((material, mesh_indices), shader)| SurfaceBinding {
                    shader,
                    texture_scale: material.texture_scale,
                    mesh_indices,
                })
                .collect(),
            profile: None,
            registrations,
            native_page0,
            native_default,
            initial_sun_direction,
            initial_sun_light,
        })
    }
    pub fn sun_direction(&self) -> qa_core::primitives::Vec3 {
        self.initial_sun_direction
    }
    pub fn sun_light(&self) -> qa_core::primitives::Vec3 {
        self.initial_sun_light
    }
    pub fn shader_registrations(&self) -> &[ShaderRegistration] {
        &self.registrations
    }
    pub fn native_default(&self) -> Option<ShaderBindingId> {
        self.native_default
    }
    pub fn native_page0(&self) -> Option<ImageId> {
        self.native_page0
    }
    pub fn world(&self) -> WorldId {
        self.world
    }
    pub fn bindings(&self) -> &[SurfaceBinding] {
        &self.bindings
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceBinding {
    pub shader: ShaderBindingId,
    /// Range in the world's packed static triangle mesh.
    pub mesh_indices: Span,
    /// Source geometry retains its original texture projection scale.
    pub texture_scale: [f32; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct SurfaceMaterial {
    pub shader: ShaderBinding,
    pub texture_scale: [f32; 2],
}
impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            shader: ShaderBinding::material(MaterialId(0), geometry::LightSource::TwoDimensional),
            texture_scale: [1.0; 2],
        }
    }
}

/// A source surface resolves after its styled canonical binding is interned.
#[derive(Clone, Copy, Debug)]
pub enum PreparedShader {
    Surface(u32),
    Binding(ShaderBinding),
}
/// One cold presentation input; native requests retain their authored order.
#[derive(Clone, Copy)]
pub struct WorldPresentationInput<'a> {
    pub surfaces: &'a [SurfaceMaterial],
    pub registrations: &'a [ShaderRegistration<PreparedShader>],
    pub native_page0: Option<ImageId>,
    pub native_default: Option<ShaderBindingId>,
}
impl<'a> WorldPresentationInput<'a> {
    pub fn new(surfaces: &'a [SurfaceMaterial]) -> Self {
        Self {
            surfaces,
            registrations: &[],
            native_page0: None,
            native_default: None,
        }
    }
}

pub struct World {
    pub(crate) geometry: geometry::WorldGeometry,
    pub(crate) visibility: std::sync::Arc<VisibilityWorld>,
    pub(crate) mesh: ModelId,
    pub(crate) presentations: Vec<WorldPresentationId>,
    pub(crate) material_source: Option<crate::material::world_load::WorldMaterialSource>,
    pub(crate) light_grid: Option<crate::lighting::LightGrid>,
    pub(crate) point_lighting: Option<crate::lighting::PointLighting>,
}
impl World {
    pub fn light_grid(&self) -> Option<&crate::lighting::LightGrid> {
        self.light_grid.as_ref()
    }
    pub fn geometry(&self) -> &geometry::WorldGeometry {
        &self.geometry
    }
    pub fn visibility(&self) -> &VisibilityWorld {
        &self.visibility
    }
    /// Cold sharing keeps module queries on the same immutable loaded tree.
    pub fn shared_visibility(&self) -> std::sync::Arc<VisibilityWorld> {
        self.visibility.clone()
    }
    pub fn mesh(&self) -> ModelId {
        self.mesh
    }
    pub fn presentations(&self) -> &[WorldPresentationId] {
        &self.presentations
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibleSurface {
    pub surface: u32,
    pub depth_key: u32,
}

/// One scratch owner per view. Neither backend performs its own PVS walk.
pub struct WorldView {
    visibility: ViewVisibility,
    surfaces: Box<[VisibleSurface]>,
    count: usize,
    light_scratch: qa_world::visibility::SurfaceRayScratch,
    pub(crate) marks: crate::marks::MarkScratch,
}
impl WorldView {
    pub fn load(world: &World) -> Self {
        Self {
            visibility: ViewVisibility::new(&world.visibility),
            surfaces: vec![VisibleSurface::default(); world.visibility.surface_count()]
                .into_boxed_slice(),
            count: 0,
            light_scratch: qa_world::visibility::SurfaceRayScratch::load(&world.visibility),
            marks: crate::marks::MarkScratch::load(world),
        }
    }
    pub fn point_lighting(
        &mut self,
        world: &World,
        point: qa_core::primitives::Vec3,
        styles: &[crate::LightStyle; 256],
        modulate: f32,
    ) -> Option<crate::stage::EntityLighting> {
        world.point_lighting.as_ref()?.sample(
            &world.geometry,
            &world.visibility,
            &mut self.light_scratch,
            point,
            styles,
            modulate,
        )
    }
    pub fn query(
        &mut self,
        world: &World,
        refdef: Refdef,
        secondary: Option<u32>,
        hidden_areas: &[u8],
    ) -> Result<&[VisibleSurface], VisibilityQueryError> {
        self.count = 0;
        let primary = world
            .visibility
            .point_in_leaf(
                refdef.pvs_origin.unwrap_or(refdef.origin),
                qa_world::visibility::PointLeafRules::RENDER,
            )
            .and_then(|leaf| world.visibility.leaf(leaf))
            .and_then(|leaf| leaf.selector);
        let base = frustum(refdef)?;
        let mut planes = [base[0]; 7];
        planes[..6].copy_from_slice(&base);
        let count = if let Some(plane) = refdef.clip_plane {
            planes[6] = plane;
            7
        } else {
            6
        };
        self.visibility.query(
            &world.visibility,
            refdef.origin,
            primary,
            secondary,
            hidden_areas,
            &planes[..count],
        )?;
        for (&surface, &depth_key) in self
            .visibility
            .visible_surfaces()
            .iter()
            .zip(self.visibility.depth_keys())
        {
            if world.geometry.surfaces[surface as usize].no_draw {
                continue;
            }
            self.surfaces[self.count] = VisibleSurface { surface, depth_key };
            self.count += 1;
        }
        Ok(&self.surfaces[..self.count])
    }
}

pub fn frustum(view: Refdef) -> Result<[Plane; 6], VisibilityQueryError> {
    // Visibility has no framebuffer; validate the view's own bounded extents.
    let tangent = view
        .validate(u32::MAX, u32::MAX)
        .ok_or(VisibilityQueryError::Frustum)?;
    let normals = [
        view.axes[0] * tangent[0] + view.axes[1],
        view.axes[0] * tangent[0] - view.axes[1],
        view.axes[0] * tangent[1] + view.axes[2],
        view.axes[0] * tangent[1] - view.axes[2],
        view.axes[0],
        -view.axes[0],
    ];
    Ok(std::array::from_fn(|i| Plane {
        encoding: None,
        normal: normals[i],
        distance: normals[i].dot(view.origin)
            + if i == 4 {
                view.near
            } else if i == 5 {
                -view.far
            } else {
                0.0
            },
        axis: None,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldLoadError {
    MissingWorld,
    VisibilityHeader,
    Visibility(VisibilityError),
}

impl From<VisibilityError> for WorldLoadError {
    fn from(error: VisibilityError) -> Self {
        Self::Visibility(error)
    }
}

/// Q1 leaf bits, Q2 compressed cluster rows and Q3 dense cluster rows become
/// one RLE table at load. There is no format dispatch in the visibility query.
pub fn load_visibility(map: &Map<'_>) -> Result<VisibilityWorld, WorldLoadError> {
    let model = map.models.first().ok_or(WorldLoadError::MissingWorld)?;
    let rules = map.bsp.format.rule_set();
    let bytes = map.bsp.bytes(Lump::Visibility);
    let count = if matches!(rules, RuleSetId::Quake) {
        usize::try_from(model.visible_leaves).map_err(|_| WorldLoadError::VisibilityHeader)?
    } else if bytes.is_empty() {
        map.leaves
            .iter()
            .filter_map(|leaf| usize::try_from(leaf.cluster).ok())
            .max()
            .map_or(0, |cluster| cluster.saturating_add(1))
    } else {
        word(bytes, 0)? as usize
    };
    if count > map.leaves.len() && matches!(rules, RuleSetId::Quake) {
        return Err(WorldLoadError::VisibilityHeader);
    }
    let pvs = if bytes.is_empty() {
        PvsRows::all_visible(count)
    } else if matches!(rules, RuleSetId::Quake) {
        let offsets = (1..=count)
            .map(|index| {
                map.leaves
                    .get(index)
                    .map(|leaf| u32::try_from(leaf.visibility_offset).ok())
                    .ok_or(WorldLoadError::VisibilityHeader)
            })
            .collect::<Result<Vec<_>, _>>()?;
        PvsRows::load(offsets, bytes.to_vec(), None)?
    } else if matches!(rules, RuleSetId::Quake2) {
        let (offsets, hearing): (Vec<_>, Vec<_>) = (0..count)
            .map(|cluster| {
                let offset = word(bytes, 4 + cluster * 8)? as i32;
                let hearing = word(bytes, 8 + cluster * 8)? as i32;
                Ok((u32::try_from(offset).ok(), u32::try_from(hearing).ok()))
            })
            .collect::<Result<Vec<_>, WorldLoadError>>()?
            .into_iter()
            .unzip();
        PvsRows::load(offsets, bytes.to_vec(), Some(hearing))?
    } else {
        let stride = word(bytes, 4)? as usize;
        let length = count.div_ceil(8);
        if stride < length
            || count
                .checked_mul(stride)
                .is_none_or(|n| n > bytes.len().saturating_sub(8))
        {
            return Err(WorldLoadError::VisibilityHeader);
        }
        let mut encoded = Vec::new();
        let mut offsets = Vec::with_capacity(count);
        for cluster in 0..count {
            offsets.push(Some(
                u32::try_from(encoded.len()).map_err(|_| WorldLoadError::VisibilityHeader)?,
            ));
            qa_core::rle::encode_pvs(
                &bytes[8 + cluster * stride..8 + cluster * stride + length],
                &mut encoded,
            );
        }
        PvsRows::load(offsets, encoded, None)?
    };
    let nodes = map
        .nodes
        .iter()
        .map(|node| VisNode {
            plane: node.plane,
            children: node.children,
            bounds: node.bounds,
            surfaces: SurfaceSpan {
                first: node.faces.first,
                count: node.faces.count,
            },
        })
        .collect();
    let leaves = map
        .leaves
        .iter()
        .enumerate()
        .map(|(index, leaf)| VisLeaf {
            selector: if matches!(rules, RuleSetId::Quake) {
                (index > 0 && index <= count).then_some(index.saturating_sub(1) as u32)
            } else {
                u32::try_from(leaf.cluster).ok()
            },
            area: if matches!(rules, RuleSetId::Quake) {
                None
            } else {
                u32::try_from(leaf.area).ok()
            },
            solid: if matches!(rules, RuleSetId::Quake) {
                leaf.contents == -2
            } else if matches!(rules, RuleSetId::Quake2) {
                leaf.contents & 1 != 0
            } else {
                false
            },
            bounds: leaf.bounds,
            surfaces: SurfaceSpan {
                first: leaf.faces.first,
                count: leaf.faces.count,
            },
        })
        .collect();
    Ok(VisibilityWorld::load(
        map.planes.clone(),
        nodes,
        leaves,
        map.leaf_faces.clone(),
        if matches!(rules, RuleSetId::Quake3) {
            map.surfaces.len()
        } else {
            map.faces.len()
        },
        model.headnodes[0],
        pvs,
    )?)
}

fn word(bytes: &[u8], at: usize) -> Result<u32, WorldLoadError> {
    let row = bytes
        .get(at..at + 4)
        .ok_or(WorldLoadError::VisibilityHeader)?;
    Ok(u32::from_le_bytes([row[0], row[1], row[2], row[3]]))
}
