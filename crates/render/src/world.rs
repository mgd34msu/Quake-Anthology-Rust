//! BSP boundary conversion. Drawing uses only the normalized world primitives.
use crate::{
    assets::{ImageId, MaterialId, ModelId},
    scene::{Refdef, Span},
};
use qa_core::primitives::{Plane, Vec3};
use qa_formats::bsp::{Lump, Map};
use qa_world::visibility::{
    PvsRows, SurfaceSpan, VisLeaf, VisNode, VisibilityError, VisibilityWorld,
};
use qa_world::visibility::{ViewVisibility, VisibilityQueryError};

pub mod geometry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldId(pub u32);

#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceBinding {
    pub material: MaterialId,
    /// Range in the world's packed static triangle mesh, independent of its
    /// retained polygon-boundary index storage.
    pub mesh_indices: Span,
    pub lightmap: ImageId,
    /// Legacy projections retain texel coordinates for the native cache.
    /// Generic stages resolve them to normalized coordinates at draw setup.
    pub texture_scale: [f32; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct SurfaceMaterial {
    pub material: MaterialId,
    pub lightmap: ImageId,
    pub texture_scale: [f32; 2],
}
impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            material: MaterialId(0),
            lightmap: ImageId(0),
            texture_scale: [1.0; 2],
        }
    }
}

pub struct World {
    pub(crate) geometry: geometry::WorldGeometry,
    pub(crate) visibility: VisibilityWorld,
    pub(crate) mesh: ModelId,
    pub(crate) bindings: Box<[SurfaceBinding]>,
}
impl World {
    pub fn geometry(&self) -> &geometry::WorldGeometry {
        &self.geometry
    }
    pub fn visibility(&self) -> &VisibilityWorld {
        &self.visibility
    }
    pub fn mesh(&self) -> ModelId {
        self.mesh
    }
    pub fn bindings(&self) -> &[SurfaceBinding] {
        &self.bindings
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
}
impl WorldView {
    pub fn load(world: &World) -> Self {
        Self {
            visibility: ViewVisibility::new(&world.visibility),
            surfaces: vec![VisibleSurface::default(); world.visibility.surface_count()]
                .into_boxed_slice(),
            count: 0,
        }
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
            .point_in_leaf(refdef.origin)
            .and_then(|leaf| world.visibility.leaf(leaf))
            .and_then(|leaf| leaf.selector);
        self.visibility.query(
            &world.visibility,
            refdef.origin,
            primary,
            secondary,
            hidden_areas,
            &frustum(refdef)?,
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
    if view
        .fov
        .iter()
        .any(|&f| !f.is_finite() || f <= 0.0 || f >= 179.0)
        || !view.near.is_finite()
        || view.near <= 0.0
        || !view.far.is_finite()
        || view.far <= view.near
    {
        return Err(VisibilityQueryError::Frustum);
    }
    let tangent = view.fov.map(|f| (f.to_radians() * 0.5).tan());
    let normals = [
        Vec3(std::array::from_fn(|i| {
            view.axes[0].0[i] * tangent[0] + view.axes[1].0[i]
        })),
        Vec3(std::array::from_fn(|i| {
            view.axes[0].0[i] * tangent[0] - view.axes[1].0[i]
        })),
        Vec3(std::array::from_fn(|i| {
            view.axes[0].0[i] * tangent[1] + view.axes[2].0[i]
        })),
        Vec3(std::array::from_fn(|i| {
            view.axes[0].0[i] * tangent[1] - view.axes[2].0[i]
        })),
        view.axes[0],
        Vec3(view.axes[0].0.map(|x| -x)),
    ];
    Ok(std::array::from_fn(|i| Plane {
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
    let family = map.bsp.format.family();
    let bytes = map.bsp.bytes(Lump::Visibility);
    let count = if family == 1 {
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
    if count > map.leaves.len() && family == 1 {
        return Err(WorldLoadError::VisibilityHeader);
    }
    let pvs = if bytes.is_empty() {
        PvsRows::all_visible(count)
    } else if family == 1 {
        let offsets = (1..=count)
            .map(|index| {
                map.leaves
                    .get(index)
                    .map(|leaf| u32::try_from(leaf.visibility_offset).ok())
                    .ok_or(WorldLoadError::VisibilityHeader)
            })
            .collect::<Result<Vec<_>, _>>()?;
        PvsRows::load(offsets, bytes.to_vec())?
    } else if family == 2 {
        let offsets = (0..count)
            .map(|cluster| {
                let offset = word(bytes, 4 + cluster * 8)? as i32;
                Ok(u32::try_from(offset).ok())
            })
            .collect::<Result<Vec<_>, WorldLoadError>>()?;
        PvsRows::load(offsets, bytes.to_vec())?
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
            encode_row(
                &bytes[8 + cluster * stride..8 + cluster * stride + length],
                &mut encoded,
            );
        }
        PvsRows::load(offsets, encoded)?
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
            selector: if family == 1 {
                (index > 0 && index <= count).then_some(index.saturating_sub(1) as u32)
            } else {
                u32::try_from(leaf.cluster).ok()
            },
            area: if family == 1 {
                None
            } else {
                u32::try_from(leaf.area).ok()
            },
            solid: if family == 1 {
                leaf.contents == -2
            } else if family == 2 {
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
        if family == 3 {
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

fn encode_row(row: &[u8], encoded: &mut Vec<u8>) {
    let mut at = 0;
    while at < row.len() {
        let byte = row[at];
        at += 1;
        if byte != 0 {
            encoded.push(byte);
            continue;
        }
        let mut run = 1u8;
        while at < row.len() && row[at] == 0 && run < 255 {
            run += 1;
            at += 1;
        }
        encoded.extend_from_slice(&[0, run]);
    }
}
