//! `CM_LoadMap` records from id Software's `code/qcommon/cm_load.c` and
//! `cm_local.h`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/map-resource.ts`.
//!
//! Records decode eagerly at load: no Rust guest reads raw hunk cells, so
//! the donor's live `DataView` getters become plain vectors plus the
//! mutation cells the trace paths share (`checkCount`, area floods,
//! portal counts, box-hull planes). Two deliberate timing shifts follow
//! from eager decoding: corrupt-map cross-index failures surface at load
//! instead of first access, and string records decode at load instead of
//! first read. Valid maps behave identically.
//!
//! Byte input to [`CollisionMapResource::load`] must already be normalized
//! to the IBSP46 loading contract: normalization lives in `qa-content`
//! (`normalize_q3_bsp`), which `qa-world` cannot depend on. The content
//! layer applies it before delegating here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use qa_core::binary::BinaryError;
use qa_core::math::{vec3, Bounds, Vec3};

use super::allocation::{HunkAccountingProfile, HunkAllocation, HunkKind, HunkPreference, SOURCE_HUNK_RELEASE32};
use super::patch::{generate_patch_collide, CollisionDebugSurface, PatchAllocSite, PatchAllocator, PatchCollide};
use crate::error::WorldError;

/// Quake III body contents for the temporary box brush.
pub const BODY_CONTENTS: i32 = 0x0200_0000;

/// Index span into a parallel lump array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexRange {
    /// First index.
    pub first: usize,
    /// Index count.
    pub count: usize,
}

/// BSP child reference in decoded input geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3BspChild {
    /// Interior node.
    Node(usize),
    /// Leaf.
    Leaf(usize),
}

/// Decoded input node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CollisionNodeInput {
    /// Plane index.
    pub plane: usize,
    /// Children.
    pub children: [Q3BspChild; 2],
}

/// Decoded input leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CollisionLeafInput {
    /// PVS cluster, or -1.
    pub cluster: i32,
    /// Area number.
    pub area: i32,
    /// Brush span.
    pub brushes: IndexRange,
    /// Surface span.
    pub surfaces: IndexRange,
}

/// Decoded input model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3CollisionModelInput {
    /// Model bounds.
    pub bounds: Bounds,
    /// Brush span.
    pub brushes: IndexRange,
    /// Surface span.
    pub surfaces: IndexRange,
}

/// Decoded input brush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CollisionBrushInput {
    /// Shader index.
    pub shader: usize,
    /// Side span.
    pub sides: IndexRange,
}

/// Decoded input brush side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CollisionBrushSideInput {
    /// Plane index.
    pub plane: usize,
    /// Shader index.
    pub shader: usize,
}

/// Decoded input surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SurfaceKind {
    /// Planar surface.
    Planar,
    /// Triangle soup.
    Triangles,
    /// Flare.
    Flare,
    /// Quadratic patch with control dimensions.
    Patch {
        /// Control width.
        width: i32,
        /// Control height.
        height: i32,
    },
}

/// Decoded input surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CollisionSurfaceInput {
    /// Surface kind.
    pub kind: Q3SurfaceKind,
    /// Shader index.
    pub shader: usize,
    /// Vertex span.
    pub vertices: IndexRange,
}

/// Decoded input visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CollisionVisibilityInput {
    /// Cluster count.
    pub cluster_count: i32,
    /// Bytes per cluster row.
    pub bytes_per_cluster: i32,
    /// Packed rows.
    pub bits: Vec<u8>,
}

/// Decoded Q3 world geometry consumed by collision: the `Q3WorldGeometry`
/// subset `decodedCollisionMap` reads. Render-only lumps (lightmaps,
/// light grid, fogs, draw indexes) never reach collision.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CollisionGeometry {
    /// Entity string.
    pub entities: String,
    /// Shaders.
    pub shaders: Vec<CollisionShader>,
    /// Planes (type and signbits derive at decode).
    pub planes: Vec<qa_core::math::Plane>,
    /// Nodes.
    pub nodes: Vec<Q3CollisionNodeInput>,
    /// Leaves.
    pub leaves: Vec<Q3CollisionLeafInput>,
    /// Leaf brush indexes.
    pub leaf_brushes: Vec<i32>,
    /// Leaf surface indexes.
    pub leaf_surfaces: Vec<i32>,
    /// Models.
    pub models: Vec<Q3CollisionModelInput>,
    /// Brushes.
    pub brushes: Vec<Q3CollisionBrushInput>,
    /// Brush sides.
    pub brush_sides: Vec<Q3CollisionBrushSideInput>,
    /// Surface vertex positions.
    pub vertices: Vec<Vec3>,
    /// Surfaces.
    pub surfaces: Vec<Q3CollisionSurfaceInput>,
    /// Visibility, when the map ships any.
    pub visibility: Option<Q3CollisionVisibilityInput>,
}

/// Collision shader record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionShader {
    /// Shader name.
    pub name: String,
    /// Surface flags.
    pub surface_flags: i32,
    /// Content flags.
    pub content_flags: i32,
}

/// Collision plane with source type and sign bits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionPlane {
    /// Unit normal.
    pub normal: Vec3,
    /// Distance from the origin.
    pub distance: f32,
    /// Axial type (0/1/2) or 3; box planes use 0..=5.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: i32,
}

/// Collision BSP node. Negative children name leaves as `-1 - leaf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionNode {
    /// Plane index.
    pub plane: usize,
    /// Children.
    pub children: [i32; 2],
}

/// Collision leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionLeaf {
    /// PVS cluster, or -1.
    pub cluster: i32,
    /// Area number.
    pub area: i32,
    /// First leaf-brush index.
    pub first_brush: usize,
    /// Brush count.
    pub brush_count: usize,
    /// First leaf-surface index.
    pub first_surface: usize,
    /// Surface count.
    pub surface_count: usize,
}

/// Collision model with inline index lists.
#[derive(Debug, Clone, PartialEq)]
pub struct CollisionModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// Inline brush indexes (model 0 uses leaves instead).
    pub brushes: Vec<i32>,
    /// Inline surface indexes.
    pub surfaces: Vec<i32>,
}

/// Collision brush.
#[derive(Debug, Clone, PartialEq)]
pub struct CollisionBrush {
    /// Shader index.
    pub shader: usize,
    /// Contents flags.
    pub contents: i32,
    /// First side index.
    pub first_side: usize,
    /// Side count.
    pub side_count: usize,
    /// Last visiting check count.
    pub check_count: Cell<i32>,
}

/// Collision brush side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionBrushSide {
    /// Plane index.
    pub plane: usize,
    /// Shader index.
    pub shader: usize,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Collision area flood state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionArea {
    /// Flood number.
    pub flood: Cell<i32>,
    /// Flood validity stamp.
    pub flood_valid: Cell<i32>,
}

/// Unlike the clip map, the source's static box model survives map clears:
/// the loader retains this handle and every box hull shares its bounds.
#[derive(Debug, Clone)]
pub struct CollisionBoxModel {
    bounds: Rc<Cell<Bounds>>,
}

impl CollisionBoxModel {
    /// Zeroed retained bounds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bounds: Rc::new(Cell::new(Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            })),
        }
    }

    /// Current retained bounds.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        self.bounds.get()
    }
}

impl Default for CollisionBoxModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Box-hull storage: the temporary hull's brush records plus the shared
/// retained bounds.
#[derive(Debug, Clone)]
pub struct CollisionBoxHull {
    /// Retained bounds shared with [`CollisionBoxModel`].
    pub bounds: Rc<Cell<Bounds>>,
    /// Box brush bounds, written by `setBounds`.
    pub brush_bounds: Cell<Bounds>,
    /// Box brush check count.
    pub brush_check_count: Cell<i32>,
    /// First box side in the map's side array (error fidelity).
    pub first_side: usize,
    /// Box side records (plane indexes into [`CollisionBoxHull::planes`]).
    pub sides: [CollisionBrushSide; 6],
    /// The twelve box planes; distances move with `setBounds`.
    pub planes: Cell<[CollisionPlane; 12]>,
}

/// Borrowed box side view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxSideView {
    /// Side plane.
    pub plane: CollisionPlane,
    /// Side surface flags.
    pub surface_flags: i32,
}

impl CollisionBoxHull {
    /// Read a box side's live plane and surface flags.
    pub fn read_side(&self, index: usize) -> Result<BoxSideView, WorldError> {
        let side = self.sides.get(index).ok_or_else(|| {
            WorldError::BadCollisionRecord(format!(
                "CM source record {} outside allocation",
                self.first_side + index
            ))
        })?;
        let planes = self.planes.get();
        let plane = planes.get(side.plane).copied().ok_or_else(|| {
            WorldError::BadCollisionRecord(format!("CM source record {} outside allocation", side.plane))
        })?;
        Ok(BoxSideView {
            plane,
            surface_flags: side.surface_flags,
        })
    }

    /// Move the box hull to `mins`/`maxs`. The capsule call changes the
    /// retained bounds but leaves the brush planes intact.
    pub fn set_bounds(&self, mins: Vec3, maxs: Vec3, capsule: bool) {
        self.bounds.set(Bounds {
            min: vec3(mins.x, mins.y, mins.z),
            max: vec3(maxs.x, maxs.y, maxs.z),
        });
        if capsule {
            return;
        }
        let ordered = [
            maxs.x, -maxs.x, mins.x, -mins.x, maxs.y, -maxs.y, mins.y, -mins.y, maxs.z, -maxs.z, mins.z, -mins.z,
        ];
        let mut planes = self.planes.get();
        for (index, distance) in ordered.into_iter().enumerate() {
            planes[index].distance = distance;
        }
        self.planes.set(planes);
        self.brush_bounds.set(Bounds {
            min: vec3(mins.x, mins.y, mins.z),
            max: vec3(maxs.x, maxs.y, maxs.z),
        });
    }

    /// Box brush contents (`BODY`).
    #[must_use]
    pub fn brush_contents(&self) -> i32 {
        BODY_CONTENTS
    }

    /// Box brush side count.
    #[must_use]
    pub fn brush_side_count(&self) -> usize {
        6
    }
}

/// How brush bounds resolve.
#[derive(Debug, Clone, PartialEq)]
enum BrushBoundsMode {
    /// Source loads store bounds per brush.
    Stored(Vec<Bounds>),
    /// Decoded maps derive bounds from the first six sides on access.
    Derived,
}

/// `CM` consumers borrow these records. The asset parser remains a
/// separate diagnostic reader.
#[derive(Debug)]
pub struct CollisionMapData {
    /// Visit stamp, bumped per query.
    pub check_count: Cell<i32>,
    /// Entity string.
    pub entities: String,
    /// Shaders.
    pub shaders: Vec<CollisionShader>,
    /// Planes.
    pub planes: Vec<CollisionPlane>,
    /// Nodes.
    pub nodes: Vec<CollisionNode>,
    /// Leaves.
    pub leaves: Vec<CollisionLeaf>,
    /// Leaf brush index table.
    pub leaf_brushes: Vec<i32>,
    /// Leaf surface index table.
    pub leaf_surfaces: Vec<i32>,
    /// Brushes.
    pub brushes: Vec<CollisionBrush>,
    /// Brush sides.
    pub brush_sides: Vec<CollisionBrushSide>,
    /// Models.
    pub models: Vec<CollisionModel>,
    /// Patch records by surface index (`None` for non-patch surfaces).
    pub patches: Vec<Option<PatchCollide>>,
    /// Areas.
    pub areas: Vec<CollisionArea>,
    /// Portal reference counts.
    pub portals: Vec<Cell<i32>>,
    /// Cluster count.
    pub cluster_count: i32,
    /// Visibility bytes.
    pub visibility: Vec<u8>,
    /// Visibility row width, when the map ships visibility.
    pub visibility_row_bytes: Option<i32>,
    /// Box hull, once initialized.
    pub box_hull: Option<CollisionBoxHull>,
    brush_bounds: BrushBoundsMode,
}

pub(crate) fn map_at<'a, T>(values: &'a [T], index: usize, what: &str) -> Result<&'a T, WorldError> {
    values
        .get(index)
        .ok_or_else(|| WorldError::BadCollisionRecord(format!("{what} {index} outside allocation")))
}

pub(crate) fn map_index(length: usize, index: usize) -> Result<usize, WorldError> {
    if index < length {
        Ok(index)
    } else {
        Err(WorldError::BadCollisionRecord(format!(
            "CM source index {index} outside allocation of {length} records"
        )))
    }
}

impl CollisionMapData {
    /// Read a plane record.
    pub fn plane(&self, index: usize) -> Result<&CollisionPlane, WorldError> {
        map_at(&self.planes, index, "CM source record")
    }

    /// Read a node record.
    pub fn node(&self, index: usize) -> Result<&CollisionNode, WorldError> {
        map_at(&self.nodes, index, "CM source record")
    }

    /// Read a leaf record.
    pub fn leaf(&self, index: usize) -> Result<&CollisionLeaf, WorldError> {
        map_at(&self.leaves, index, "CM source record")
    }

    /// Read a brush record.
    pub fn brush(&self, index: usize) -> Result<&CollisionBrush, WorldError> {
        map_at(&self.brushes, index, "CM source record")
    }

    /// Read a brush-side record.
    pub fn brush_side(&self, index: usize) -> Result<&CollisionBrushSide, WorldError> {
        map_at(&self.brush_sides, index, "CM source record")
    }

    /// Read a model record.
    pub fn model(&self, index: usize) -> Result<&CollisionModel, WorldError> {
        map_at(&self.models, index, "CM source record")
    }

    /// Read a shader record.
    pub fn shader(&self, index: usize) -> Result<&CollisionShader, WorldError> {
        map_at(&self.shaders, index, "CM source record")
    }

    /// Read a leaf-brush table entry.
    pub fn leaf_brush_at(&self, index: usize) -> Result<i32, WorldError> {
        map_index(self.leaf_brushes.len(), index)?;
        Ok(self.leaf_brushes[index])
    }

    /// Read a leaf-surface table entry.
    pub fn leaf_surface_at(&self, index: usize) -> Result<i32, WorldError> {
        map_index(self.leaf_surfaces.len(), index)?;
        Ok(self.leaf_surfaces[index])
    }

    /// Read a model's inline brush index.
    pub fn model_brush_at(&self, model: usize, item: usize) -> Result<i32, WorldError> {
        let model = self.model(model)?;
        map_index(model.brushes.len(), item)?;
        Ok(model.brushes[item])
    }

    /// Read a model's inline surface index.
    pub fn model_surface_at(&self, model: usize, item: usize) -> Result<i32, WorldError> {
        let model = self.model(model)?;
        map_index(model.surfaces.len(), item)?;
        Ok(model.surfaces[item])
    }

    /// Read a patch record by surface index (`None` for non-patches).
    pub fn patch(&self, index: usize) -> Result<Option<&PatchCollide>, WorldError> {
        map_at(&self.patches, index, "CM source record")?;
        Ok(self.patches[index].as_ref())
    }

    /// Read a brush's bounds: stored for source loads, derived from the
    /// first six sides for decoded maps.
    pub fn brush_bounds(&self, index: usize) -> Result<Bounds, WorldError> {
        let brush = self.brush(index)?;
        match &self.brush_bounds {
            BrushBoundsMode::Stored(bounds) => map_at(bounds, index, "CM source record").copied(),
            BrushBoundsMode::Derived => {
                let distance = |side: usize| -> Result<f32, WorldError> {
                    let side = self.brush_side(brush.first_side + side)?;
                    Ok(self.plane(side.plane)?.distance)
                };
                Ok(Bounds {
                    min: vec3(-distance(0)?, -distance(2)?, -distance(4)?),
                    max: vec3(distance(1)?, distance(3)?, distance(5)?),
                })
            }
        }
    }

    /// Read a portal reference count.
    pub fn portal_at(&self, index: usize) -> Result<i32, WorldError> {
        map_index(self.portals.len(), index)?;
        Ok(self.portals[index].get())
    }

    /// Write a portal reference count.
    pub fn portal_set(&self, index: usize, value: i32) -> Result<(), WorldError> {
        map_index(self.portals.len(), index)?;
        self.portals[index].set(value);
        Ok(())
    }

    /// Move the box hull; maps without an initialized hull cannot answer.
    pub fn set_box_bounds(&self, mins: Vec3, maxs: Vec3, capsule: bool) -> Result<(), WorldError> {
        let Some(hull) = &self.box_hull else {
            return Err(WorldError::BadCollisionRecord(
                "CM box hull requires loaded collision allocations".to_string(),
            ));
        };
        hull.set_bounds(mins, maxs, capsule);
        Ok(())
    }
}

fn range_check(raw: &[u8], source: &str, offset: usize, length: usize) -> Result<(), WorldError> {
    if offset.saturating_add(length) <= raw.len() {
        Ok(())
    } else {
        Err(WorldError::BadCollisionRecord(
            BinaryError::custom(
                source,
                offset,
                format!("CM source read of {length} bytes outside {}-byte file", raw.len()),
            )
            .to_string(),
        ))
    }
}

fn read_int(raw: &[u8], source: &str, offset: usize) -> Result<i32, WorldError> {
    range_check(raw, source, offset, 4)?;
    Ok(i32::from_le_bytes([
        raw[offset],
        raw[offset + 1],
        raw[offset + 2],
        raw[offset + 3],
    ]))
}

fn read_float(raw: &[u8], source: &str, offset: usize) -> Result<f32, WorldError> {
    range_check(raw, source, offset, 4)?;
    Ok(f32::from_le_bytes([
        raw[offset],
        raw[offset + 1],
        raw[offset + 2],
        raw[offset + 3],
    ]))
}

struct Lump {
    offset: usize,
    length: usize,
}

fn lump_count(length: usize, stride: usize, message: &str) -> Result<usize, WorldError> {
    if !length.is_multiple_of(stride) {
        return Err(WorldError::BadCollisionRecord(message.to_string()));
    }
    Ok(length / stride)
}

fn plane_type(normal: Vec3) -> i32 {
    if normal.x == 1.0 {
        0
    } else if normal.y == 1.0 {
        1
    } else if normal.z == 1.0 {
        2
    } else {
        3
    }
}

fn plane_signbits(normal: Vec3) -> i32 {
    i32::from(normal.x < 0.0) | (i32::from(normal.y < 0.0) << 1) | (i32::from(normal.z < 0.0) << 2)
}

fn cstring(bytes: &[u8], offset: usize, length: usize) -> Result<String, WorldError> {
    let mut result = String::new();
    for index in 0..length {
        let byte = bytes
            .get(offset + index)
            .copied()
            .ok_or_else(|| WorldError::BadCollisionRecord("CM string outside allocation".to_string()))?;
        if byte == 0 {
            return Ok(result);
        }
        result.push(byte as char);
    }
    Err(WorldError::BadCollisionRecord(
        "CM string has no terminator inside its allocation".to_string(),
    ))
}

fn positive_index(value: i32) -> Result<usize, WorldError> {
    usize::try_from(value)
        .map_err(|_| WorldError::BadCollisionRecord(format!("CM source record {value} outside allocation")))
}

/// A source load mutates its reached allocations in place and keeps them
/// after a source abort.
pub struct CollisionMapResource {
    source: String,
    memory: HunkAccountingProfile,
    debug: Option<Rc<CollisionDebugSurface>>,
    box_model: CollisionBoxModel,
    data: CollisionMapData,
    num_planes: usize,
    num_sides: usize,
    num_brushes: usize,
    num_leaf_brushes: usize,
    diagnostic_offset: u64,
    loaded: bool,
}

impl std::fmt::Debug for CollisionMapResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollisionMapResource")
            .field("source", &self.source)
            .field("loaded", &self.loaded)
            .finish_non_exhaustive()
    }
}

impl CollisionMapResource {
    /// Borrow the source name, memory profile, debug surface, and retained
    /// box model.
    pub fn new(
        source: &str,
        memory: HunkAccountingProfile,
        debug: Option<Rc<CollisionDebugSurface>>,
        box_model: CollisionBoxModel,
    ) -> Self {
        Self {
            source: source.to_string(),
            memory,
            debug,
            box_model,
            data: CollisionMapData {
                check_count: Cell::new(0),
                entities: String::new(),
                shaders: Vec::new(),
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: Vec::new(),
                leaf_brushes: Vec::new(),
                leaf_surfaces: Vec::new(),
                brushes: Vec::new(),
                brush_sides: Vec::new(),
                models: Vec::new(),
                patches: Vec::new(),
                areas: Vec::new(),
                portals: Vec::new(),
                cluster_count: 0,
                visibility: Vec::new(),
                visibility_row_bytes: None,
                box_hull: None,
                brush_bounds: BrushBoundsMode::Stored(Vec::new()),
            },
            num_planes: 0,
            num_sides: 0,
            num_brushes: 0,
            num_leaf_brushes: 0,
            diagnostic_offset: 32,
            loaded: false,
        }
    }

    fn allocate(&mut self, site: &str, bytes: i64, resource: Option<&str>) -> Result<HunkAllocation, WorldError> {
        if let HunkAccountingProfile::SourceHunk { accounting } = &self.memory {
            let bytes = usize::try_from(bytes)
                .map_err(|_| WorldError::BadCollisionRecord("Invalid CM allocation size".to_string()))?;
            let resource = match resource {
                Some(resource) => resource.to_string(),
                None => self.source.clone(),
            };
            return Ok(accounting.reserve(site, &resource, bytes, HunkPreference::High));
        }
        if !(0..=0x7fff_ffff).contains(&bytes) {
            return Err(WorldError::BadCollisionRecord("Invalid CM allocation size".to_string()));
        }
        let bytes = bytes as usize;
        let offset = self.diagnostic_offset;
        self.diagnostic_offset += bytes.div_ceil(32) as u64 * 32;
        Ok(HunkAllocation {
            kind: HunkKind::Permanent,
            byte_offset: u32::try_from(offset).map_err(|_| {
                WorldError::BadCollisionRecord("CM source pointer exceeds the 32-bit address space".to_string())
            })?,
            byte_length: bytes,
            bytes: vec![0; bytes],
        })
    }

    /// Load normalized IBSP46 bytes into collision records.
    pub fn load(&mut self, raw: &[u8]) -> Result<(), WorldError> {
        let source = self.source.clone();
        let range =
            |offset: usize, length: usize| -> Result<(), WorldError> { range_check(raw, &source, offset, length) };
        let int = |offset: usize| -> Result<i32, WorldError> { read_int(raw, &source, offset) };
        let float = |offset: usize| -> Result<f32, WorldError> { read_float(raw, &source, offset) };
        range(0, 144)?;
        let version = int(4)?;
        let mut lumps = Vec::with_capacity(17);
        for index in 0..17 {
            let offset = int(8 + index * 8)?;
            let length = int(12 + index * 8)?;
            lumps.push(Lump {
                offset: usize::try_from(offset).unwrap_or(usize::MAX),
                length: usize::try_from(length).unwrap_or(usize::MAX),
            });
        }
        if version != 46 {
            return Err(WorldError::BadCollisionRecord(format!(
                "CM_LoadMap: {} has wrong version number ({version} should be 46)",
                self.source
            )));
        }
        let lump = |index: usize| -> Result<&Lump, WorldError> { map_at(&lumps, index, "CM source record") };
        let sizes = SOURCE_HUNK_RELEASE32;

        let shader_lump = lump(1)?;
        let shader_count = lump_count(shader_lump.length, 72, "CMod_LoadShaders: funny lump size")?;
        if shader_count < 1 {
            return Err(WorldError::BadCollisionRecord("Map with no shaders".to_string()));
        }
        self.allocate("CMod_LoadShaders", shader_count as i64 * sizes.disk_shader as i64, None)?;
        let mut shaders = Vec::with_capacity(shader_count);
        for index in 0..shader_count {
            let base = shader_lump.offset + index * 72;
            shaders.push(CollisionShader {
                name: cstring(raw, base, 64)?,
                surface_flags: int(base + 64)?,
                content_flags: int(base + 68)?,
            });
        }
        self.data.shaders = shaders;

        let leaf_lump = lump(4)?;
        let leaf_count = lump_count(leaf_lump.length, 48, "MOD_LoadBmodel: funny lump size")?;
        if leaf_count < 1 {
            return Err(WorldError::BadCollisionRecord("Map with no leafs".to_string()));
        }
        self.allocate(
            "CMod_LoadLeafs:leafs",
            (leaf_count as i64 + 2) * sizes.leaf as i64,
            None,
        )?;
        let mut leaves = Vec::with_capacity(leaf_count);
        let mut area_count = 0i32;
        for index in 0..leaf_count {
            let input = leaf_lump.offset + index * 48;
            let leaf = CollisionLeaf {
                cluster: int(input)?,
                area: int(input + 4)?,
                first_brush: positive_index(int(input + 40)?)?,
                brush_count: positive_index(int(input + 44)?)?,
                first_surface: positive_index(int(input + 32)?)?,
                surface_count: positive_index(int(input + 36)?)?,
            };
            if leaf.cluster >= self.data.cluster_count {
                self.data.cluster_count = leaf.cluster.wrapping_add(1);
            }
            if leaf.area >= area_count {
                area_count = leaf.area.wrapping_add(1);
            }
            leaves.push(leaf);
        }
        self.data.leaves = leaves;
        let area_count = usize::try_from(area_count).unwrap_or(0);
        self.allocate("CMod_LoadLeafs:areas", area_count as i64 * sizes.area as i64, None)?;
        self.data.areas = (0..area_count)
            .map(|_| CollisionArea {
                flood: Cell::new(0),
                flood_valid: Cell::new(0),
            })
            .collect();
        self.allocate(
            "CMod_LoadLeafs:areaPortals",
            area_count as i64 * area_count as i64 * 4,
            None,
        )?;
        self.data.portals = (0..area_count * area_count).map(|_| Cell::new(0)).collect();

        let leaf_brush_lump = lump(6)?;
        self.num_leaf_brushes = lump_count(leaf_brush_lump.length, 4, "MOD_LoadBmodel: funny lump size")?;
        self.allocate("CMod_LoadLeafBrushes", (self.num_leaf_brushes as i64 + 1) * 4, None)?;
        let mut leaf_brushes = vec![0i32; self.num_leaf_brushes + 1];
        for (index, slot) in leaf_brushes.iter_mut().enumerate().take(self.num_leaf_brushes) {
            *slot = int(leaf_brush_lump.offset + index * 4)?;
        }
        self.data.leaf_brushes = leaf_brushes;

        let leaf_surface_lump = lump(5)?;
        let leaf_surface_count = lump_count(leaf_surface_lump.length, 4, "MOD_LoadBmodel: funny lump size")?;
        self.allocate("CMod_LoadLeafSurfaces", leaf_surface_count as i64 * 4, None)?;
        let mut leaf_surfaces = vec![0i32; leaf_surface_count];
        for (index, slot) in leaf_surfaces.iter_mut().enumerate() {
            *slot = int(leaf_surface_lump.offset + index * 4)?;
        }
        self.data.leaf_surfaces = leaf_surfaces;

        let plane_lump = lump(2)?;
        self.num_planes = lump_count(plane_lump.length, 16, "MOD_LoadBmodel: funny lump size")?;
        if self.num_planes < 1 {
            return Err(WorldError::BadCollisionRecord("Map with no planes".to_string()));
        }
        self.allocate(
            "CMod_LoadPlanes",
            (self.num_planes as i64 + 12) * sizes.plane as i64,
            None,
        )?;
        let mut planes = vec![
            CollisionPlane {
                normal: vec3(0.0, 0.0, 0.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            };
            self.num_planes + 12
        ];
        for (index, slot) in planes.iter_mut().enumerate().take(self.num_planes) {
            let input = plane_lump.offset + index * 16;
            let normal = vec3(float(input)?, float(input + 4)?, float(input + 8)?);
            *slot = CollisionPlane {
                normal,
                distance: float(input + 12)?,
                plane_type: plane_type(normal),
                signbits: plane_signbits(normal),
            };
        }
        self.data.planes = planes;

        let side_lump = lump(9)?;
        self.num_sides = lump_count(side_lump.length, 8, "MOD_LoadBmodel: funny lump size")?;
        self.allocate(
            "CMod_LoadBrushSides",
            (self.num_sides as i64 + 6) * sizes.brush_side as i64,
            None,
        )?;
        let mut sides = vec![
            CollisionBrushSide {
                plane: 0,
                shader: 0,
                surface_flags: 0,
            };
            self.num_sides + 6
        ];
        for (index, slot) in sides.iter_mut().enumerate().take(self.num_sides) {
            let input = side_lump.offset + index * 8;
            let plane = int(input)?;
            let shader = int(input + 4)?;
            if shader < 0 || shader as usize >= shader_count {
                return Err(WorldError::BadCollisionRecord(format!(
                    "CMod_LoadBrushSides: bad shaderNum: {shader}"
                )));
            }
            let plane = positive_index(plane)?;
            if plane >= self.num_planes + 12 {
                return Err(WorldError::BadCollisionRecord(format!(
                    "CM source record {plane} outside allocation"
                )));
            }
            *slot = CollisionBrushSide {
                plane,
                shader: shader as usize,
                surface_flags: self.data.shaders[shader as usize].surface_flags,
            };
        }
        self.data.brush_sides = sides;

        let brush_lump = lump(8)?;
        self.num_brushes = lump_count(brush_lump.length, 12, "MOD_LoadBmodel: funny lump size")?;
        self.allocate(
            "CMod_LoadBrushes",
            (self.num_brushes as i64 + 1) * sizes.brush as i64,
            None,
        )?;
        let mut brushes = Vec::with_capacity(self.num_brushes + 1);
        let mut brush_bounds = Vec::with_capacity(self.num_brushes + 1);
        for index in 0..self.num_brushes {
            let input = brush_lump.offset + index * 12;
            let first_side = positive_index(int(input)?)?;
            let side_count = positive_index(int(input + 4)?)?;
            let shader = int(input + 8)?;
            if shader < 0 || shader as usize >= shader_count {
                return Err(WorldError::BadCollisionRecord(format!(
                    "CMod_LoadBrushes: bad shaderNum: {shader}"
                )));
            }
            let contents = self.data.shaders[shader as usize].content_flags;
            let side_distance = |side: usize| -> Result<f32, WorldError> {
                let side = self.data.brush_side(first_side + side)?;
                Ok(self.data.plane(side.plane)?.distance)
            };
            brush_bounds.push(Bounds {
                min: vec3(-side_distance(0)?, -side_distance(2)?, -side_distance(4)?),
                max: vec3(side_distance(1)?, side_distance(3)?, side_distance(5)?),
            });
            brushes.push(CollisionBrush {
                shader: shader as usize,
                contents,
                first_side,
                side_count,
                check_count: Cell::new(0),
            });
        }
        brushes.push(CollisionBrush {
            shader: 0,
            contents: 0,
            first_side: 0,
            side_count: 0,
            check_count: Cell::new(0),
        });
        brush_bounds.push(Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        });
        self.data.brushes = brushes;
        self.data.brush_bounds = BrushBoundsMode::Stored(brush_bounds);

        let model_lump = lump(7)?;
        let model_count = lump_count(model_lump.length, 40, "CMod_LoadSubmodels: funny lump size")?;
        if model_count < 1 {
            return Err(WorldError::BadCollisionRecord("Map with no models".to_string()));
        }
        if model_count > 256 {
            return Err(WorldError::BadCollisionRecord("MAX_SUBMODELS exceeded".to_string()));
        }
        self.allocate(
            "CMod_LoadSubmodels",
            model_count as i64 * sizes.collision_model as i64,
            None,
        )?;
        let mut models = Vec::with_capacity(model_count);
        for index in 0..model_count {
            let input = model_lump.offset + index * 40;
            let bounds = Bounds {
                min: vec3(float(input)? - 1.0, float(input + 4)? - 1.0, float(input + 8)? - 1.0),
                max: vec3(
                    float(input + 12)? + 1.0,
                    float(input + 16)? + 1.0,
                    float(input + 20)? + 1.0,
                ),
            };
            if index == 0 {
                models.push(CollisionModel {
                    bounds,
                    brushes: Vec::new(),
                    surfaces: Vec::new(),
                });
                continue;
            }
            let brush_count = int(input + 36)?;
            let brush_count_usize = positive_index(brush_count)?;
            self.allocate("CMod_LoadSubmodels:brushes", brush_count as i64 * 4, None)?;
            let first_brush = int(input + 32)?;
            let brushes: Vec<i32> = (0..brush_count_usize)
                .map(|item| first_brush.wrapping_add(item as i32))
                .collect();
            let surface_count = int(input + 28)?;
            let surface_count_usize = positive_index(surface_count)?;
            self.allocate("CMod_LoadSubmodels:surfaces", surface_count as i64 * 4, None)?;
            let first_surface = int(input + 24)?;
            let surfaces: Vec<i32> = (0..surface_count_usize)
                .map(|item| first_surface.wrapping_add(item as i32))
                .collect();
            models.push(CollisionModel {
                bounds,
                brushes,
                surfaces,
            });
        }
        self.data.models = models;

        let node_lump = lump(3)?;
        let node_count = lump_count(node_lump.length, 36, "MOD_LoadBmodel: funny lump size")?;
        if node_count < 1 {
            return Err(WorldError::BadCollisionRecord("Map has no nodes".to_string()));
        }
        self.allocate("CMod_LoadNodes", node_count as i64 * sizes.collision_node as i64, None)?;
        let mut nodes = Vec::with_capacity(node_count);
        for index in 0..node_count {
            let input = node_lump.offset + index * 36;
            let plane = positive_index(int(input)?)?;
            if plane >= self.num_planes + 12 {
                return Err(WorldError::BadCollisionRecord(format!(
                    "CM source record {plane} outside allocation"
                )));
            }
            nodes.push(CollisionNode {
                plane,
                children: [int(input + 4)?, int(input + 8)?],
            });
        }
        self.data.nodes = nodes;

        let entities = lump(0)?;
        self.allocate("CMod_LoadEntityString", entities.length as i64, None)?;
        range(entities.offset, entities.length)?;
        self.data.entities = cstring(raw, entities.offset, entities.length)?;

        let visibility = lump(16)?;
        if visibility.length == 0 {
            let bytes = ((self.data.cluster_count + 31) & !31).max(0) as usize;
            self.allocate("CMod_LoadVisibility:novis", bytes as i64, None)?;
            self.data.visibility = vec![255u8; bytes];
        } else {
            self.data.visibility_row_bytes = Some(0);
            self.allocate("CMod_LoadVisibility", visibility.length as i64, None)?;
            range(visibility.offset, visibility.length)?;
            self.data.cluster_count = int(visibility.offset)?;
            self.data.visibility_row_bytes = Some(int(visibility.offset + 4)?);
            let mut stored = vec![0u8; visibility.length];
            stored[..visibility.length - 8]
                .copy_from_slice(&raw[visibility.offset + 8..visibility.offset + visibility.length]);
            self.data.visibility = stored;
        }

        let surface_lump = lump(13)?;
        let surface_count = lump_count(surface_lump.length, 104, "MOD_LoadBmodel: funny lump size")?;
        let vertex_lump = lump(10)?;
        lump_count(vertex_lump.length, 44, "MOD_LoadBmodel: funny lump size")?;
        self.allocate(
            "CMod_LoadPatches:surfaces",
            surface_count as i64 * sizes.pointer as i64,
            None,
        )?;
        let mut patches: Vec<Option<PatchCollide>> = Vec::with_capacity(surface_count);
        for index in 0..surface_count {
            let input = surface_lump.offset + index * 104;
            if int(input + 8)? != 2 {
                patches.push(None);
                continue;
            }
            let width = int(input + 96)?;
            let height = int(input + 100)?;
            if width.wrapping_mul(height) > 1024 {
                return Err(WorldError::BadCollisionRecord("ParseMesh: MAX_PATCH_VERTS".to_string()));
            }
            let first_vertex = int(input + 12)?;
            let points_count = usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0);
            let mut points = Vec::with_capacity(points_count);
            for item in 0..points_count {
                let vertex = first_vertex as i64 + item as i64;
                let vertex = usize::try_from(vertex).unwrap_or(usize::MAX);
                let offset = vertex_lump.offset.saturating_add(vertex.saturating_mul(44));
                points.push(vec3(float(offset)?, float(offset + 4)?, float(offset + 8)?));
            }
            let shader = positive_index(int(input)?)?;
            self.data.shader(shader)?;
            let resource = format!("{}#{index}", self.source);
            self.allocate("CMod_LoadPatches:patch", sizes.collision_patch as i64, Some(&resource))?;
            let recorder = RecordingPatchAllocator::new();
            let collide = generate_patch_collide(width, height, &points, self.debug.as_deref(), Some(&recorder))?;
            if !recorder.generated() {
                return Err(WorldError::BadCollisionRecord(
                    "CM patch generator did not allocate its result".to_string(),
                ));
            }
            for (site, bytes) in recorder.sites() {
                self.allocate(site.name(), bytes as i64, Some(&resource))?;
            }
            patches.push(Some(collide));
        }
        self.data.patches = patches;
        self.loaded = true;
        Ok(())
    }

    /// Wire the box hull into the loaded records. `CM_LoadMap` reaches this
    /// only after the file read succeeds.
    pub fn initialize_box_hull(&mut self) -> Result<(), WorldError> {
        if !self.loaded {
            return Err(WorldError::BadCollisionRecord(
                "CM box hull requires loaded collision allocations".to_string(),
            ));
        }
        self.data.brushes[self.num_brushes].side_count = 6;
        self.data.brushes[self.num_brushes].first_side = self.num_sides;
        self.data.brushes[self.num_brushes].contents = BODY_CONTENTS;
        self.data.leaf_brushes[self.num_leaf_brushes] = self.num_brushes as i32;
        let mut planes = [CollisionPlane {
            normal: vec3(0.0, 0.0, 0.0),
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        }; 12];
        for index in 0..6 {
            self.data.brush_sides[self.num_sides + index].plane = index * 2 + (index & 1);
            self.data.brush_sides[self.num_sides + index].surface_flags = 0;
            for opposite in 0..2 {
                let axis = index >> 1;
                let sign = if opposite == 0 { 1.0 } else { -1.0 };
                planes[index * 2 + opposite] = CollisionPlane {
                    normal: vec3(
                        if axis == 0 { sign } else { 0.0 },
                        if axis == 1 { sign } else { 0.0 },
                        if axis == 2 { sign } else { 0.0 },
                    ),
                    distance: 0.0,
                    plane_type: axis as i32 + opposite as i32 * 3,
                    signbits: if opposite == 0 { 0 } else { 1 << axis },
                };
            }
        }
        let mut box_sides = [CollisionBrushSide {
            plane: 0,
            shader: 0,
            surface_flags: 0,
        }; 6];
        for (index, side) in box_sides.iter_mut().enumerate() {
            *side = self.data.brush_sides[self.num_sides + index];
        }
        self.data.box_hull = Some(CollisionBoxHull {
            bounds: self.box_model.bounds.clone(),
            brush_bounds: Cell::new(Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            }),
            brush_check_count: Cell::new(0),
            first_side: self.num_sides,
            sides: box_sides,
            planes: Cell::new(planes),
        });
        Ok(())
    }

    /// Release the decoded records.
    pub fn into_data(self) -> Result<CollisionMapData, WorldError> {
        if !self.loaded {
            return Err(WorldError::BadCollisionRecord(
                "CM entity string has not been loaded".to_string(),
            ));
        }
        Ok(self.data)
    }
}

/// Records patch generation allocation sites so the loader can reserve
/// them in order after generation returns.
struct RecordingPatchAllocator {
    sites: RefCell<Vec<(PatchAllocSite, usize)>>,
}

impl RecordingPatchAllocator {
    fn new() -> Self {
        Self {
            sites: RefCell::new(Vec::new()),
        }
    }

    fn generated(&self) -> bool {
        self.sites
            .borrow()
            .iter()
            .any(|(site, _)| *site == PatchAllocSite::Generate)
    }

    fn sites(&self) -> Vec<(PatchAllocSite, usize)> {
        self.sites.borrow().clone()
    }
}

impl PatchAllocator for RecordingPatchAllocator {
    fn allocate(&self, site: PatchAllocSite, bytes: usize) -> HunkAllocation {
        self.sites.borrow_mut().push((site, bytes));
        HunkAllocation {
            kind: HunkKind::Permanent,
            byte_offset: 0,
            byte_length: bytes,
            bytes: vec![0; bytes],
        }
    }
}

/// Decoded world geometry shares the same collision records as source
/// `CM_LoadMap`.
pub fn decoded_collision_map(
    map: &Q3CollisionGeometry,
    debug: Option<&CollisionDebugSurface>,
) -> Result<CollisionMapData, WorldError> {
    let planes: Vec<CollisionPlane> = map
        .planes
        .iter()
        .map(|plane| CollisionPlane {
            normal: plane.normal,
            distance: plane.distance,
            plane_type: plane_type(plane.normal),
            signbits: plane_signbits(plane.normal),
        })
        .collect();
    let mut patches: Vec<Option<PatchCollide>> = Vec::with_capacity(map.surfaces.len());
    for surface in &map.surfaces {
        let Q3SurfaceKind::Patch { width, height } = surface.kind else {
            patches.push(None);
            continue;
        };
        map_at(&map.shaders, surface.shader, "CM source record")?;
        let start = surface.vertices.first.min(map.vertices.len());
        let end = surface
            .vertices
            .first
            .saturating_add(surface.vertices.count)
            .min(map.vertices.len());
        let collide = generate_patch_collide(width, height, &map.vertices[start..end], debug, None)?;
        patches.push(Some(collide));
    }
    let mut area_count = 0i32;
    let mut cluster_count = 0i32;
    for leaf in &map.leaves {
        area_count = area_count.max(leaf.area.wrapping_add(1));
        cluster_count = cluster_count.max(leaf.cluster.wrapping_add(1));
    }
    let area_count = usize::try_from(area_count).unwrap_or(0);
    let areas = (0..area_count)
        .map(|_| CollisionArea {
            flood: Cell::new(0),
            flood_valid: Cell::new(0),
        })
        .collect();
    let portals = (0..area_count * area_count).map(|_| Cell::new(0)).collect();
    let (visibility, cluster_count, visibility_row_bytes) = match &map.visibility {
        None => {
            let bytes = ((cluster_count + 31) & !31).max(0) as usize;
            (vec![255u8; bytes], cluster_count, None)
        }
        Some(visibility) => {
            let mut bytes = vec![0u8; visibility.bits.len() + 8];
            bytes[..visibility.bits.len()].copy_from_slice(&visibility.bits);
            (bytes, visibility.cluster_count, Some(visibility.bytes_per_cluster))
        }
    };
    let nodes = map
        .nodes
        .iter()
        .map(|node| {
            let child = |child: &Q3BspChild| -> i32 {
                match child {
                    Q3BspChild::Node(index) => *index as i32,
                    Q3BspChild::Leaf(index) => -1 - (*index as i32),
                }
            };
            CollisionNode {
                plane: node.plane,
                children: [child(&node.children[0]), child(&node.children[1])],
            }
        })
        .collect();
    let leaves = map
        .leaves
        .iter()
        .map(|leaf| CollisionLeaf {
            cluster: leaf.cluster,
            area: leaf.area,
            first_brush: leaf.brushes.first,
            brush_count: leaf.brushes.count,
            first_surface: leaf.surfaces.first,
            surface_count: leaf.surfaces.count,
        })
        .collect();
    let mut brushes = Vec::with_capacity(map.brushes.len());
    for brush in &map.brushes {
        let shader = map_at(&map.shaders, brush.shader, "CM source record")?;
        brushes.push(CollisionBrush {
            shader: brush.shader,
            contents: shader.content_flags,
            first_side: brush.sides.first,
            side_count: brush.sides.count,
            check_count: Cell::new(0),
        });
    }
    let mut brush_sides = Vec::with_capacity(map.brush_sides.len());
    for side in &map.brush_sides {
        let shader = map_at(&map.shaders, side.shader, "CM source record")?;
        brush_sides.push(CollisionBrushSide {
            plane: side.plane,
            shader: side.shader,
            surface_flags: shader.surface_flags,
        });
    }
    let models = map
        .models
        .iter()
        .map(|model| {
            let brushes = (0..model.brushes.count)
                .map(|item| (model.brushes.first as i32).wrapping_add(item as i32))
                .collect();
            let surfaces = (0..model.surfaces.count)
                .map(|item| (model.surfaces.first as i32).wrapping_add(item as i32))
                .collect();
            CollisionModel {
                bounds: Bounds {
                    min: vec3(
                        model.bounds.min.x - 1.0,
                        model.bounds.min.y - 1.0,
                        model.bounds.min.z - 1.0,
                    ),
                    max: vec3(
                        model.bounds.max.x + 1.0,
                        model.bounds.max.y + 1.0,
                        model.bounds.max.z + 1.0,
                    ),
                },
                brushes,
                surfaces,
            }
        })
        .collect();
    Ok(CollisionMapData {
        check_count: Cell::new(0),
        entities: map.entities.clone(),
        shaders: map.shaders.clone(),
        planes,
        nodes,
        leaves,
        leaf_brushes: map.leaf_brushes.clone(),
        leaf_surfaces: map.leaf_surfaces.clone(),
        brushes,
        brush_sides,
        models,
        patches,
        areas,
        portals,
        cluster_count,
        visibility,
        visibility_row_bytes,
        box_hull: None,
        brush_bounds: BrushBoundsMode::Derived,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::Plane;

    fn push_i32(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    /// Minimal IBSP46 exercising every loader path: two shaders, seven
    /// planes, one node, two leaves, one six-sided brush, one model, one
    /// 3x3 patch surface, and two visibility clusters.
    fn fixture_blob() -> Vec<u8> {
        let mut lumps: Vec<Vec<u8>> = Vec::new();
        let entities = b"{ \"classname\" \"worldspawn\" }\n".to_vec();
        lumps.push([entities, vec![0]].concat());
        let mut shaders = Vec::new();
        for (name, surface, content) in [("test_solid", 1, 1), ("test_patch", 2, 0)] {
            let mut slot = vec![0u8; 64];
            slot[..name.len()].copy_from_slice(name.as_bytes());
            shaders.extend_from_slice(&slot);
            push_i32(&mut shaders, surface);
            push_i32(&mut shaders, content);
        }
        lumps.push(shaders);
        let mut planes = Vec::new();
        for (normal, distance) in [
            ([1.0, 0.0, 0.0], 64.0),
            ([-1.0, 0.0, 0.0], 64.0),
            ([0.0, 1.0, 0.0], 64.0),
            ([0.0, -1.0, 0.0], 64.0),
            ([0.0, 0.0, 1.0], 64.0),
            ([0.0, 0.0, -1.0], 64.0),
            ([0.0, 0.0, 1.0], 0.0),
        ] {
            for component in normal {
                push_f32(&mut planes, component);
            }
            push_f32(&mut planes, distance);
        }
        lumps.push(planes);
        let mut nodes = Vec::new();
        push_i32(&mut nodes, 6);
        push_i32(&mut nodes, -1);
        push_i32(&mut nodes, -2);
        nodes.extend_from_slice(&[0u8; 24]);
        lumps.push(nodes);
        let mut leaves = Vec::new();
        for (cluster, area, first_surface, surface_count, first_brush, brush_count) in
            [(0, 0, 0, 1, 0, 1), (1, 0, 1, 0, 1, 0)]
        {
            push_i32(&mut leaves, cluster);
            push_i32(&mut leaves, area);
            leaves.extend_from_slice(&[0u8; 24]);
            push_i32(&mut leaves, first_surface);
            push_i32(&mut leaves, surface_count);
            push_i32(&mut leaves, first_brush);
            push_i32(&mut leaves, brush_count);
        }
        lumps.push(leaves);
        lumps.push(0i32.to_le_bytes().to_vec());
        lumps.push(0i32.to_le_bytes().to_vec());
        let mut models = Vec::new();
        for value in [-64.0, -64.0, -64.0, 64.0, 64.0, 64.0] {
            push_f32(&mut models, value);
        }
        for value in [0, 0, 0, 0] {
            push_i32(&mut models, value);
        }
        lumps.push(models);
        let mut brushes = Vec::new();
        push_i32(&mut brushes, 0);
        push_i32(&mut brushes, 6);
        push_i32(&mut brushes, 0);
        lumps.push(brushes);
        let mut sides = Vec::new();
        for plane in 0..6 {
            push_i32(&mut sides, plane);
            push_i32(&mut sides, 0);
        }
        lumps.push(sides);
        let mut verts = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                push_f32(&mut verts, (x - 1) as f32 * 64.0);
                push_f32(&mut verts, (y - 1) as f32 * 64.0);
                push_f32(&mut verts, 0.0);
                verts.extend_from_slice(&[0u8; 32]);
            }
        }
        lumps.push(verts);
        lumps.push(Vec::new());
        lumps.push(Vec::new());
        let mut surface = vec![0u8; 104];
        surface[0..4].copy_from_slice(&1i32.to_le_bytes());
        surface[8..12].copy_from_slice(&2i32.to_le_bytes());
        surface[12..16].copy_from_slice(&0i32.to_le_bytes());
        surface[16..20].copy_from_slice(&9i32.to_le_bytes());
        surface[96..100].copy_from_slice(&3i32.to_le_bytes());
        surface[100..104].copy_from_slice(&3i32.to_le_bytes());
        lumps.push(surface);
        lumps.push(Vec::new());
        lumps.push(Vec::new());
        let mut visibility = Vec::new();
        push_i32(&mut visibility, 2);
        push_i32(&mut visibility, 1);
        visibility.extend_from_slice(&[3, 3]);
        lumps.push(visibility);
        assert_eq!(lumps.len(), 17);
        let mut blob = vec![0u8; 144];
        blob[0..4].copy_from_slice(&0x5053_4249i32.to_le_bytes());
        blob[4..8].copy_from_slice(&46i32.to_le_bytes());
        let mut cursor = 144;
        for (index, lump) in lumps.iter().enumerate() {
            blob[8 + index * 8..12 + index * 8].copy_from_slice(&(cursor as i32).to_le_bytes());
            blob[12 + index * 8..16 + index * 8].copy_from_slice(&(lump.len() as i32).to_le_bytes());
            cursor += lump.len();
        }
        for lump in &lumps {
            blob.extend_from_slice(lump);
        }
        blob
    }

    fn loaded_resource() -> CollisionMapData {
        let blob = fixture_blob();
        assert_eq!(blob.len(), 1180);
        let mut resource = CollisionMapResource::new(
            "fixture",
            HunkAccountingProfile::Unaccounted,
            None,
            CollisionBoxModel::new(),
        );
        resource.load(&blob).expect("load");
        resource.initialize_box_hull().expect("box hull");
        resource.into_data().expect("data")
    }

    #[test]
    fn source_load_matches_donor_records() {
        let map = loaded_resource();
        assert_eq!(map.entities, "{ \"classname\" \"worldspawn\" }\n");
        assert_eq!(map.shaders.len(), 2);
        assert_eq!(map.shaders[0].name, "test_solid");
        assert_eq!(map.shaders[0].surface_flags, 1);
        assert_eq!(map.shaders[0].content_flags, 1);
        assert_eq!(map.shaders[1].name, "test_patch");
        assert_eq!(map.shaders[1].surface_flags, 2);
        assert_eq!(map.shaders[1].content_flags, 0);
        let expected_planes = [
            (vec3(1.0, 0.0, 0.0), 64.0, 0, 0),
            (vec3(-1.0, 0.0, 0.0), 64.0, 3, 1),
            (vec3(0.0, 1.0, 0.0), 64.0, 1, 0),
            (vec3(0.0, -1.0, 0.0), 64.0, 3, 2),
            (vec3(0.0, 0.0, 1.0), 64.0, 2, 0),
            (vec3(0.0, 0.0, -1.0), 64.0, 3, 4),
            (vec3(0.0, 0.0, 1.0), 0.0, 2, 0),
        ];
        assert_eq!(map.planes.len(), 19);
        for (index, (normal, distance, plane_type, signbits)) in expected_planes.into_iter().enumerate() {
            let plane = &map.planes[index];
            assert_eq!(plane.normal, normal, "plane {index} normal");
            assert_eq!(plane.distance, distance, "plane {index} distance");
            assert_eq!(plane.plane_type, plane_type, "plane {index} type");
            assert_eq!(plane.signbits, signbits, "plane {index} signbits");
        }
        assert_eq!(map.nodes.len(), 1);
        assert_eq!(map.nodes[0].plane, 6);
        assert_eq!(map.nodes[0].children, [-1, -2]);
        assert_eq!(map.leaves.len(), 2);
        assert_eq!(map.leaves[0].cluster, 0);
        assert_eq!(map.leaves[0].brush_count, 1);
        assert_eq!(map.leaves[0].surface_count, 1);
        assert_eq!(map.leaves[1].cluster, 1);
        assert_eq!(map.leaf_brushes, [0, 1]);
        assert_eq!(map.leaf_surfaces, [0]);
        assert_eq!(map.brushes.len(), 2);
        assert_eq!(map.brushes[0].contents, 1);
        assert_eq!(map.brushes[0].side_count, 6);
        assert_eq!(map.brush_bounds(0).expect("bounds").min, vec3(-64.0, -64.0, -64.0));
        assert_eq!(map.brush_bounds(0).expect("bounds").max, vec3(64.0, 64.0, 64.0));
        assert_eq!(map.brush_sides[0].plane, 0);
        assert_eq!(map.brush_sides[0].surface_flags, 1);
        assert_eq!(map.models.len(), 1);
        assert_eq!(map.models[0].bounds.min, vec3(-65.0, -65.0, -65.0));
        assert_eq!(map.models[0].bounds.max, vec3(65.0, 65.0, 65.0));
        let patch = map.patch(0).expect("patch").expect("patch surface");
        assert_eq!(patch.planes.len(), 5);
        assert_eq!(patch.facets.len(), 1);
        assert_eq!(patch.bounds.min, vec3(-65.0, -65.0, -1.0));
        assert_eq!(patch.bounds.max, vec3(65.0, 65.0, 1.0));
        assert_eq!(map.cluster_count, 2);
        assert_eq!(map.visibility_row_bytes, Some(1));
        assert_eq!(map.visibility, [3, 3, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(map.areas.len(), 1);
        assert_eq!(map.portal_at(0).expect("portal"), 0);
    }

    #[test]
    fn box_hull_matches_donor() {
        let map = loaded_resource();
        let hull = map.box_hull.as_ref().expect("box hull");
        assert_eq!(hull.brush_contents(), BODY_CONTENTS);
        assert_eq!(hull.brush_side_count(), 6);
        map.set_box_bounds(vec3(-16.0, -16.0, -16.0), vec3(16.0, 16.0, 16.0), false)
            .expect("set bounds");
        assert_eq!(hull.bounds.get().min, vec3(-16.0, -16.0, -16.0));
        assert_eq!(hull.brush_bounds.get().max, vec3(16.0, 16.0, 16.0));
        let expected = [
            (vec3(1.0, 0.0, 0.0), 0, 0),
            (vec3(-1.0, 0.0, 0.0), 3, 1),
            (vec3(0.0, 1.0, 0.0), 1, 0),
            (vec3(0.0, -1.0, 0.0), 4, 2),
            (vec3(0.0, 0.0, 1.0), 2, 0),
            (vec3(0.0, 0.0, -1.0), 5, 4),
        ];
        for (index, (normal, plane_type, signbits)) in expected.into_iter().enumerate() {
            let side = hull.read_side(index).expect("side");
            assert_eq!(side.plane.normal, normal, "side {index} normal");
            assert_eq!(side.plane.distance, 16.0, "side {index} distance");
            assert_eq!(side.plane.plane_type, plane_type, "side {index} type");
            assert_eq!(side.plane.signbits, signbits, "side {index} signbits");
            assert_eq!(side.surface_flags, 0);
        }
        map.set_box_bounds(vec3(-8.0, -8.0, -8.0), vec3(8.0, 8.0, 8.0), true)
            .expect("capsule bounds");
        assert_eq!(hull.bounds.get().min, vec3(-8.0, -8.0, -8.0));
        assert_eq!(
            hull.read_side(0).expect("side").plane.distance,
            16.0,
            "capsule keeps brush planes"
        );
        let error = hull.read_side(6).expect_err("side 6 is outside");
        assert_eq!(
            error.to_string(),
            format!("CM source record {} outside allocation", hull.first_side + 6)
        );
    }

    #[test]
    fn load_rejects_bad_maps() {
        let mut blob = fixture_blob();
        blob[4..8].copy_from_slice(&47i32.to_le_bytes());
        let mut resource = CollisionMapResource::new(
            "fixture",
            HunkAccountingProfile::Unaccounted,
            None,
            CollisionBoxModel::new(),
        );
        let error = resource.load(&blob).expect_err("bad version");
        assert_eq!(
            error.to_string(),
            "CM_LoadMap: fixture has wrong version number (47 should be 46)"
        );
        let mut blob = fixture_blob();
        blob[12 + 64..16 + 64].copy_from_slice(&13i32.to_le_bytes());
        let mut resource = CollisionMapResource::new(
            "fixture",
            HunkAccountingProfile::Unaccounted,
            None,
            CollisionBoxModel::new(),
        );
        let error = resource.load(&blob).expect_err("funny lump");
        assert_eq!(error.to_string(), "MOD_LoadBmodel: funny lump size");
        let mut resource = CollisionMapResource::new(
            "fixture",
            HunkAccountingProfile::Unaccounted,
            None,
            CollisionBoxModel::new(),
        );
        let error = resource.load(&[0u8; 64]).expect_err("short file");
        assert!(error.to_string().contains("CM source read of 144 bytes"), "{error}");
        let mut resource = CollisionMapResource::new(
            "fixture",
            HunkAccountingProfile::Unaccounted,
            None,
            CollisionBoxModel::new(),
        );
        let error = resource.initialize_box_hull().expect_err("hull needs a load");
        assert_eq!(error.to_string(), "CM box hull requires loaded collision allocations");
        let error = resource.into_data().expect_err("data needs a load");
        assert_eq!(error.to_string(), "CM entity string has not been loaded");
    }

    #[test]
    fn decoded_map_shares_source_records() {
        let geometry = Q3CollisionGeometry {
            entities: "world".to_string(),
            shaders: vec![
                CollisionShader {
                    name: "solid".to_string(),
                    surface_flags: 4,
                    content_flags: 8,
                },
                CollisionShader {
                    name: "patch".to_string(),
                    surface_flags: 2,
                    content_flags: 0,
                },
            ],
            planes: vec![
                Plane {
                    normal: vec3(1.0, 0.0, 0.0),
                    distance: 64.0,
                },
                Plane {
                    normal: vec3(-1.0, 0.0, 0.0),
                    distance: 64.0,
                },
                Plane {
                    normal: vec3(0.0, 1.0, 0.0),
                    distance: 64.0,
                },
                Plane {
                    normal: vec3(0.0, -1.0, 0.0),
                    distance: 64.0,
                },
                Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 64.0,
                },
                Plane {
                    normal: vec3(0.0, 0.0, -1.0),
                    distance: 64.0,
                },
            ],
            nodes: vec![Q3CollisionNodeInput {
                plane: 4,
                children: [Q3BspChild::Leaf(0), Q3BspChild::Leaf(1)],
            }],
            leaves: vec![
                Q3CollisionLeafInput {
                    cluster: 0,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 1 },
                    surfaces: IndexRange { first: 0, count: 1 },
                },
                Q3CollisionLeafInput {
                    cluster: -1,
                    area: 1,
                    brushes: IndexRange { first: 1, count: 0 },
                    surfaces: IndexRange { first: 1, count: 0 },
                },
            ],
            leaf_brushes: vec![0],
            leaf_surfaces: vec![0],
            models: vec![Q3CollisionModelInput {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                brushes: IndexRange { first: 0, count: 0 },
                surfaces: IndexRange { first: 0, count: 0 },
            }],
            brushes: vec![Q3CollisionBrushInput {
                shader: 0,
                sides: IndexRange { first: 0, count: 6 },
            }],
            brush_sides: vec![
                Q3CollisionBrushSideInput { plane: 0, shader: 0 },
                Q3CollisionBrushSideInput { plane: 1, shader: 0 },
                Q3CollisionBrushSideInput { plane: 2, shader: 0 },
                Q3CollisionBrushSideInput { plane: 3, shader: 0 },
                Q3CollisionBrushSideInput { plane: 4, shader: 0 },
                Q3CollisionBrushSideInput { plane: 5, shader: 0 },
            ],
            vertices: (0..9)
                .map(|n| vec3((n % 3) as f32 * 64.0 - 64.0, (n / 3) as f32 * 64.0 - 64.0, 0.0))
                .collect(),
            surfaces: vec![Q3CollisionSurfaceInput {
                kind: Q3SurfaceKind::Patch { width: 3, height: 3 },
                shader: 1,
                vertices: IndexRange { first: 0, count: 9 },
            }],
            visibility: None,
        };
        let map = decoded_collision_map(&geometry, None).expect("decode");
        assert_eq!(map.entities, "world");
        assert_eq!(map.planes[1].plane_type, 3);
        assert_eq!(map.planes[1].signbits, 1);
        assert_eq!(map.nodes[0].children, [-1, -2]);
        assert_eq!(map.brushes[0].contents, 8);
        assert_eq!(map.brush_sides[0].surface_flags, 4);
        assert_eq!(
            map.brush_bounds(0).expect("derived bounds").min,
            vec3(-64.0, -64.0, -64.0)
        );
        assert_eq!(map.models[0].bounds.max, vec3(65.0, 65.0, 65.0));
        let patch = map.patch(0).expect("patch").expect("patch surface");
        assert_eq!(patch.facets.len(), 1);
        assert_eq!(map.cluster_count, 1);
        assert!(map.visibility_row_bytes.is_none());
        assert!(map.visibility.iter().all(|byte| *byte == 255));
        assert_eq!(map.areas.len(), 2);
        assert!(map.box_hull.is_none());
        let error = map.patch(1).expect_err("surface 1 is missing");
        assert_eq!(error.to_string(), "CM source record 1 outside allocation");
        let error = map
            .set_box_bounds(vec3(0.0, 0.0, 0.0), vec3(1.0, 1.0, 1.0), false)
            .expect_err("decoded maps have no hull");
        assert_eq!(error.to_string(), "CM box hull requires loaded collision allocations");
    }

    #[test]
    fn record_accessors_validate() {
        let map = loaded_resource();
        let error = map.plane(99).expect_err("plane 99 is missing");
        assert_eq!(error.to_string(), "CM source record 99 outside allocation");
        let error = map.leaf_brush_at(99).expect_err("entry 99 is missing");
        assert_eq!(error.to_string(), "CM source index 99 outside allocation of 2 records");
        let error = map.model_brush_at(0, 0).expect_err("model 0 has no brushes");
        assert_eq!(error.to_string(), "CM source index 0 outside allocation of 0 records");
        map.portal_set(0, 3).expect("portal write");
        assert_eq!(map.portal_at(0).expect("portal"), 3);
        let error = map.portal_at(1).expect_err("portal 1 is missing");
        assert_eq!(error.to_string(), "CM source index 1 outside allocation of 1 records");
    }
}
