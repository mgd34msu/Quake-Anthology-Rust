//! Borrowed lump directory and flat records shared by collision and rendering.
//! Layouts: original qfiles.h/bspfile.h; v44 directly follows the C port reader.
pub use crate::image::MipTexture;
use crate::{FormatError, span, word};
pub use qa_core::primitives::IndexSpan;
use qa_core::primitives::{Bounds, ClipNode, Plane, RuleSetId, Vec3};

mod decode;
mod validate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BspFormat {
    Quake,
    HalfLife,
    Bsp2,
    Psb2,
    Quake64,
    Quake2,
    Qbsp,
    Quake3Test,
    Quake3,
    QuakeLive,
}
impl BspFormat {
    /// Natural file semantics; capability roles remain independently selected.
    pub fn rule_set(self) -> RuleSetId {
        match self {
            Self::Quake | Self::HalfLife | Self::Bsp2 | Self::Psb2 | Self::Quake64 => {
                RuleSetId::Quake
            }
            Self::Quake2 | Self::Qbsp => RuleSetId::Quake2,
            _ => RuleSetId::Quake3,
        }
    }
    fn wide(self) -> bool {
        matches!(self, Self::Bsp2 | Self::Psb2 | Self::Qbsp)
    }
    fn narrow_q1(self) -> bool {
        matches!(self.rule_set(), RuleSetId::Quake) && !self.wide()
    }
    fn modern_q3(self) -> bool {
        matches!(self, Self::Quake3 | Self::QuakeLive)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lump {
    Entities,
    Planes,
    Textures,
    Vertices,
    Visibility,
    Nodes,
    TexInfo,
    Faces,
    Lighting,
    ClipNodes,
    Leaves,
    LeafFaces,
    Edges,
    SurfEdges,
    Models,
    LeafBrushes,
    Brushes,
    BrushSides,
    Pop,
    Areas,
    AreaPortals,
    Shaders,
    Indices,
    Fogs,
    Surfaces,
    LightGrid,
    Advertisements,
}
use Lump::*;
const Q1: &[(Lump, usize)] = &[
    (Entities, 0),
    (Planes, 20),
    (Textures, 0),
    (Vertices, 12),
    (Visibility, 0),
    (Nodes, 24),
    (TexInfo, 40),
    (Faces, 20),
    (Lighting, 0),
    (ClipNodes, 8),
    (Leaves, 28),
    (LeafFaces, 2),
    (Edges, 4),
    (SurfEdges, 4),
    (Models, 64),
];
const Q2: &[(Lump, usize)] = &[
    (Entities, 0),
    (Planes, 20),
    (Vertices, 12),
    (Visibility, 0),
    (Nodes, 28),
    (TexInfo, 76),
    (Faces, 20),
    (Lighting, 0),
    (Leaves, 28),
    (LeafFaces, 2),
    (LeafBrushes, 2),
    (Edges, 4),
    (SurfEdges, 4),
    (Models, 48),
    (Brushes, 12),
    (BrushSides, 4),
    (Pop, 0),
    (Areas, 8),
    (AreaPortals, 8),
];
const Q3: &[(Lump, usize)] = &[
    (Entities, 0),
    (Shaders, 72),
    (Planes, 16),
    (Nodes, 36),
    (Leaves, 48),
    (LeafFaces, 4),
    (LeafBrushes, 4),
    (Models, 40),
    (Brushes, 12),
    (BrushSides, 8),
    (Vertices, 44),
    (Indices, 4),
    (Fogs, 72),
    (Surfaces, 104),
    (Lighting, 49152),
    (LightGrid, 8),
    (Visibility, 0),
    (Advertisements, 128),
];
const Q3_TEST: &[(Lump, usize)] = &[
    (Entities, 0),
    (Planes, 20),
    (Nodes, 36),
    (Leaves, 48),
    (LeafFaces, 4),
    (LeafBrushes, 4),
    (Models, 48),
    (Brushes, 12),
    (BrushSides, 8),
    (Lighting, 49152),
    (Visibility, 0),
    (Vertices, 44),
    (Surfaces, 164),
    (Fogs, 68),
    (Indices, 4),
];

/// Native wire checks retained from one accepted map source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeMapChecksums {
    pub whole_file: u32,
    pub quakeworld_lumps: Option<u32>,
}

pub struct Bsp<'a> {
    pub format: BspFormat,
    lumps: [&'a [u8]; 19],
    strides: [usize; 19],
    layout: &'static [(Lump, usize)],
    source: &'a [u8],
    end: usize,
    /// Q2 retail headers sometimes overstate the terminated entity tail length.
    pub entity_tail_clamped: bool,
    /// Native Q2 does not consume POP. Preserve only its available bytes.
    pub pop_tail_clamped: bool,
}
impl<'a> Bsp<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let first = word(bytes, 0)?;
        let (format, base, layout) = match first {
            29 => (BspFormat::Quake, 4, Q1),
            30 => (BspFormat::HalfLife, 4, Q1),
            0x32505342 => (BspFormat::Bsp2, 4, Q1),
            0x42535032 => (BspFormat::Psb2, 4, Q1),
            0x51363420 => (BspFormat::Quake64, 4, Q1),
            0x50534249 | 0x50534251 => match (first, word(bytes, 4)?) {
                (0x50534249, 38) => (BspFormat::Quake2, 8, Q2),
                (0x50534251, 38) => (BspFormat::Qbsp, 8, Q2),
                (0x50534249, 44) => (BspFormat::Quake3Test, 8, Q3_TEST),
                (0x50534249, 46) => (BspFormat::Quake3, 8, &Q3[..17]),
                (0x50534249, 47) => (BspFormat::QuakeLive, 8, Q3),
                _ => return Err(FormatError::Unsupported),
            },
            _ => return Err(FormatError::Unsupported),
        };
        let header = base + layout.len() * 8;
        if bytes.len() < header {
            return Err(FormatError::Truncated);
        }
        let mut result = Self {
            format,
            lumps: [&[]; 19],
            strides: [0; 19],
            layout,
            source: bytes,
            end: header,
            entity_tail_clamped: false,
            pop_tail_clamped: false,
        };
        for (index, &(kind, original_stride)) in layout.iter().enumerate() {
            let offset = word(bytes, base + index * 8)?;
            let mut length = word(bytes, base + index * 8 + 4)?;
            if matches!(format.rule_set(), RuleSetId::Quake2)
                && matches!(kind, Entities | Pop)
                && (offset as usize) < bytes.len()
                && length as usize > bytes.len() - offset as usize
            {
                length = (bytes.len() - offset as usize) as u32;
                if kind == Entities {
                    result.entity_tail_clamped = true;
                } else {
                    result.pop_tail_clamped = true;
                }
            }
            if length > 0 && (offset as usize) < header {
                return Err(FormatError::InvalidRange);
            }
            let data = span(bytes, offset, length)?;
            let stride = match (format, kind) {
                (BspFormat::Bsp2, Nodes | Leaves) => 44,
                (BspFormat::Psb2, Nodes | Leaves) => 32,
                (BspFormat::Bsp2 | BspFormat::Psb2, ClipNodes) => 12,
                (BspFormat::Bsp2 | BspFormat::Psb2 | BspFormat::Qbsp, Faces) => 28,
                (BspFormat::Bsp2 | BspFormat::Psb2 | BspFormat::Qbsp, LeafFaces) => 4,
                (BspFormat::Bsp2 | BspFormat::Psb2 | BspFormat::Qbsp, Edges) => 8,
                (BspFormat::Qbsp, Nodes) => 44,
                (BspFormat::Qbsp, Leaves) => 52,
                (BspFormat::Qbsp, LeafBrushes) => 4,
                (BspFormat::Qbsp, BrushSides) => 8,
                _ => original_stride,
            };
            if stride > 0 && !data.len().is_multiple_of(stride) {
                return Err(FormatError::InvalidRecordSize);
            }
            result.lumps[index] = data;
            result.strides[index] = stride;
            if length > 0 {
                result.end = result.end.max(offset as usize + length as usize);
            }
        }
        Ok(result)
    }
    pub fn lump(&self, index: usize) -> Option<&'a [u8]> {
        self.layout.get(index).map(|_| self.lumps[index])
    }
    pub fn native_checksums(&self) -> NativeMapChecksums {
        NativeMapChecksums {
            whole_file: qa_core::checksum::block_checksum(self.source),
            quakeworld_lumps: self.quakeworld_checksum2(),
        }
    }
    /// Original QW prespawn checksum. This is a wire check, not a resource key.
    pub fn quakeworld_checksum2(&self) -> Option<u32> {
        if self.format != BspFormat::Quake {
            return None;
        }
        Some(
            self.layout
                .iter()
                .enumerate()
                .filter(|(_, (kind, _))| !matches!(kind, Entities | Visibility | Nodes | Leaves))
                .fold(0, |checksum, (index, _)| {
                    checksum ^ qa_core::checksum::block_checksum(self.lumps[index])
                }),
        )
    }
    pub fn texture_info(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<TextureInfo<'a>, FormatError>> + use<'a> {
        let rules = self.format.rule_set();
        self.bytes(TexInfo)
            .chunks_exact(self.stride(TexInfo))
            .map(move |bytes| decode::texture_info(&mut crate::read::Reader::new(bytes), rules))
    }
    pub fn record_count(&self, index: usize) -> Option<usize> {
        self.layout.get(index).and_then(|_| {
            (self.strides[index] != 0).then(|| self.lumps[index].len() / self.strides[index])
        })
    }
    pub fn bytes(&self, kind: Lump) -> &'a [u8] {
        self.layout
            .iter()
            .position(|&(key, _)| key == kind)
            .map_or(&[], |at| self.lumps[at])
    }
    fn stride(&self, kind: Lump) -> usize {
        self.layout
            .iter()
            .position(|&(key, _)| key == kind)
            .map_or(1, |at| self.strides[at])
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: Vec3,
    pub texcoord: [f32; 2],
    pub lightmap_coord: [f32; 2],
    pub normal: Vec3,
    pub color: [u8; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub plane: u32,
    /// Negative children encode leaf = -1 - child.
    pub children: [i32; 2],
    pub bounds: Bounds,
    pub faces: IndexSpan,
}
#[derive(Clone, Copy, Debug)]
pub struct Leaf {
    pub contents: i32,
    pub cluster: i64,
    pub area: i64,
    pub visibility_offset: i32,
    pub bounds: Bounds,
    pub faces: IndexSpan,
    pub brushes: IndexSpan,
    pub ambient: [u8; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub plane: u32,
    pub flags: u32,
    pub edges: IndexSpan,
    pub texture_info: u32,
    pub styles: [u8; 4],
    pub lighting_offset: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct TextureInfo<'a> {
    pub projection: [[f32; 4]; 2],
    pub flags: i32,
    pub texture: i32,
    pub value: i32,
    pub next: i32,
    pub name: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub bounds: Bounds,
    pub origin: Vec3,
    pub headnodes: [i32; 4],
    pub visible_leaves: i32,
    pub faces: IndexSpan,
    pub brushes: IndexSpan,
    pub membership_from_tree: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub sides: IndexSpan,
    pub contents: i32,
    pub shader: Option<u32>,
}
#[derive(Clone, Copy, Debug)]
pub struct BrushSide {
    pub plane: u32,
    pub texture_info: Option<u32>,
    pub shader: Option<u32>,
    pub flags: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct Shader<'a> {
    pub name: &'a [u8],
    pub surface_flags: i32,
    pub content_flags: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct Fog<'a> {
    pub name: &'a [u8],
    pub brush: i32,
    pub visible_side: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Planar,
    Patch,
    Triangles,
    Flare,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightmapSource<'a> {
    Vertex,
    White,
    None,
    TwoDimensional,
    External(u32),
    Embedded(&'a [u8]),
}
#[derive(Clone, Copy, Debug)]
pub struct Surface<'a> {
    pub kind: SurfaceKind,
    pub shader: Option<u32>,
    pub shader_name: &'a [u8],
    pub brush_side: i32,
    pub fog: i32,
    pub vertices: IndexSpan,
    pub indices: IndexSpan,
    pub lightmap: i32,
    pub lightmap_rect: [i32; 4],
    pub lightmap_origin: Vec3,
    pub lightmap_vectors: [Vec3; 3],
    pub patch: [i32; 2],
    pub triangle_fan: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct AreaPortal {
    pub portal: u32,
    pub other_area: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Extension<'a> {
    pub name: &'a [u8],
    pub bytes: &'a [u8],
}

/// BSPX lightmap sample coordinates are independent of the base texture axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecoupledLightmap {
    pub width: u32,
    pub height: u32,
    /// A checked byte offset into Lighting; unlit or malformed spans are None.
    pub offset: Option<u32>,
    pub projection: [[f32; 4]; 2],
}

pub struct Map<'a> {
    pub bsp: Bsp<'a>,
    pub planes: Vec<Plane>,
    pub vertices: Vec<Vertex>,
    pub nodes: Vec<Node>,
    pub leaves: Vec<Leaf>,
    pub edges: Vec<[u32; 2]>,
    pub surface_edges: Vec<i32>,
    pub faces: Vec<Face>,
    pub leaf_faces: Vec<u32>,
    pub leaf_brushes: Vec<u32>,
    pub clipnodes: Vec<ClipNode>,
    pub texture_info: Vec<TextureInfo<'a>>,
    pub textures: Vec<Option<MipTexture<'a>>>,
    pub models: Vec<Model>,
    pub brushes: Vec<Brush>,
    pub brush_sides: Vec<BrushSide>,
    /// Present BSPX BRUSHLIST makes every model brush span authoritative.
    pub authored_brushes: bool,
    pub shaders: Vec<Shader<'a>>,
    pub fogs: Vec<Fog<'a>>,
    pub surfaces: Vec<Surface<'a>>,
    pub indices: Vec<i32>,
    pub areas: Vec<IndexSpan>,
    pub area_portals: Vec<AreaPortal>,
    pub extensions: Vec<Extension<'a>>,
    /// Face-indexed BSPX records. None retains the ordinary face lighting.
    pub decoupled_lightmaps: Vec<Option<DecoupledLightmap>>,
}
#[derive(Clone, Copy, Debug)]
pub struct LightGridPoint {
    pub ambient: [u8; 3],
    pub directed: [u8; 3],
    pub longitude: u8,
    pub latitude: u8,
}
impl<'a> Map<'a> {
    /// Bsp::parse checks the native eight-byte lump stride before this decoder.
    pub fn light_grid(&self) -> impl ExactSizeIterator<Item = LightGridPoint> + '_ {
        self.bsp
            .bytes(Lump::LightGrid)
            .as_chunks::<8>()
            .0
            .iter()
            .map(|point| LightGridPoint {
                ambient: [point[0], point[1], point[2]],
                directed: [point[3], point[4], point[5]],
                longitude: point[6],
                latitude: point[7],
            })
    }
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let bsp = Bsp::parse(bytes)?;
        let mut result = decode::map(bsp)?;
        validate::map(&result)?;
        // Q3 contents live on the brush shader, not its integer shader index.
        for brush in &mut result.brushes {
            if let Some(shader) = brush.shader {
                brush.contents = result.shaders[shader as usize].content_flags;
            }
        }
        Ok(result)
    }
    pub fn entity_text(&self) -> &'a [u8] {
        let text = self.bsp.bytes(Entities);
        qa_core::text::until_nul(text)
    }
    /// The renderer receives a bounded sample span or a load-time material
    /// request, never a raw file index into an absent lighting table.
    pub fn surface_lightmap(&self, surface: &Surface<'_>) -> LightmapSource<'a> {
        if matches!(surface.kind, SurfaceKind::Triangles | SurfaceKind::Flare)
            || surface.lightmap == -3
        {
            return LightmapSource::Vertex;
        }
        if surface.lightmap < 0 {
            return match surface.lightmap {
                -4 => LightmapSource::TwoDimensional,
                -2 => LightmapSource::White,
                _ => LightmapSource::None,
            };
        }
        let samples = self.bsp.bytes(Lighting);
        let index = surface.lightmap as usize;
        if index < samples.len() / 49152 {
            LightmapSource::Embedded(&samples[index * 49152..(index + 1) * 49152])
        } else {
            LightmapSource::External(surface.lightmap as u32)
        }
    }
}
