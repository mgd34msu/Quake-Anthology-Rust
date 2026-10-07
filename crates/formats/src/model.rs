//! Load boundaries produce the same frame-major mesh arrays for every game.
//! Identity belongs to the VFS/precache table, never the bytes decoded here.
mod alias;
mod md3;
mod md5;
mod normals;
mod read;

use crate::FormatError;
use qa_core::primitives::{Bounds, Vec3};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFormat {
    Mdl,
    Md2,
    Md3,
    Mdc,
    Spr,
    Sp2,
    Md5Mesh,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Triangle {
    pub vertex: [u32; 3],
    pub texcoord: [u32; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct Shader<'a> {
    pub name: &'a [u8],
    pub native_index: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct Weight {
    pub bone: u32,
    pub bias: f32,
    pub offset: Vec3,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct WeightRange {
    pub first: u32,
    pub count: u32,
}

#[derive(Debug, Default)]
pub struct Mesh<'a> {
    /// Native surface name. Material bindings normalise names once at load.
    pub name: &'a [u8],
    pub flags: i32,
    pub vertices_per_frame: usize,
    /// Frames are contiguous; UVs and triangles are shared by every frame.
    pub vertices: Vec<Vertex>,
    pub texcoords: Vec<[f32; 2]>,
    pub triangles: Vec<Triangle>,
    pub shaders: Vec<Shader<'a>>,
    pub weights: Vec<Weight>,
    pub vertex_weights: Vec<WeightRange>,
    /// Bone-local normals for skeletal deformation; vertices hold bind output.
    pub bind_normals: Vec<Vec3>,
}

#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub name: &'a [u8],
    pub bounds: Bounds,
    pub origin: Vec3,
    pub radius: f32,
    pub scale: Vec3,
    pub translation: Vec3,
    /// Original alias bytes retained for native lighting/interpolation rules.
    pub packed_vertices: &'a [u8],
}
impl Default for Frame<'_> {
    fn default() -> Self {
        Self {
            name: &[],
            bounds: Bounds::default(),
            origin: Vec3::default(),
            radius: 0.0,
            scale: Vec3([1.0; 3]),
            translation: Vec3::default(),
            packed_vertices: &[],
        }
    }
}
#[derive(Debug)]
pub struct Group {
    pub members: Range<usize>,
    /// Native cumulative endpoints, indexing Model::intervals.
    pub intervals: Range<usize>,
    pub bounds: Bounds,
}
#[derive(Clone, Copy, Debug)]
pub enum Image<'a> {
    Indexed {
        width: u32,
        height: u32,
        pixels: &'a [u8],
    },
    External(&'a [u8]),
}
#[derive(Clone, Copy, Debug)]
pub struct Sprite<'a> {
    pub width: u32,
    pub height: u32,
    pub origin: [i32; 2],
    pub image: Image<'a>,
}
#[derive(Clone, Copy, Debug)]
pub struct Tag<'a> {
    pub name: &'a [u8],
    pub origin: Vec3,
    pub axes: [Vec3; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct Bone<'a> {
    pub name: &'a [u8],
    pub parent: Option<u32>,
    /// MD5 mesh joints store model-space bind poses.
    pub position: Vec3,
    pub orientation: [f32; 4],
}

#[derive(Debug)]
pub struct Model<'a> {
    pub format: ModelFormat,
    pub name: &'a [u8],
    pub flags: i32,
    /// Native metadata; only ST_RAND (1) selects random synchronisation.
    pub sync: i32,
    pub orientation: u32,
    pub radius: f32,
    pub beam_length: f32,
    pub eye_position: Vec3,
    pub native_size: f32,
    pub declared_skins: u32,
    pub bounds: Bounds,
    pub meshes: Vec<Mesh<'a>>,
    pub frames: Vec<Frame<'a>>,
    pub frame_groups: Vec<Group>,
    pub skin_groups: Vec<Group>,
    pub intervals: Vec<f32>,
    pub skins: Vec<Image<'a>>,
    pub sprites: Vec<Sprite<'a>>,
    pub tags_per_frame: usize,
    pub tags: Vec<Tag<'a>>,
    pub bones: Vec<Bone<'a>>,
    pub command_line: &'a [u8],
    pub gl_commands: &'a [u8],
}
impl<'a> Model<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let format = match bytes.get(..4) {
            Some(b"IDPO") => ModelFormat::Mdl,
            Some(b"IDP2") => ModelFormat::Md2,
            Some(b"IDP3") => ModelFormat::Md3,
            Some(b"IDPC") => ModelFormat::Mdc,
            Some(b"IDSP") => ModelFormat::Spr,
            Some(b"IDS2") => ModelFormat::Sp2,
            _ if crate::text::Tokens::new(bytes).next()? == Some(b"MD5Version") => {
                ModelFormat::Md5Mesh
            }
            _ => return Err(FormatError::Unsupported),
        };
        let model = Self {
            format,
            name: &[],
            flags: 0,
            sync: 0,
            orientation: 0,
            radius: 0.0,
            beam_length: 0.0,
            eye_position: Vec3::default(),
            native_size: 0.0,
            declared_skins: 0,
            bounds: read::empty_bounds(),
            meshes: Vec::new(),
            frames: Vec::new(),
            frame_groups: Vec::new(),
            skin_groups: Vec::new(),
            intervals: Vec::new(),
            skins: Vec::new(),
            sprites: Vec::new(),
            tags_per_frame: 0,
            tags: Vec::new(),
            bones: Vec::new(),
            command_line: &[],
            gl_commands: &[],
        };
        match format {
            ModelFormat::Mdl => alias::mdl(bytes, model),
            ModelFormat::Md2 => alias::md2(bytes, model),
            ModelFormat::Spr | ModelFormat::Sp2 => alias::sprite(bytes, model),
            ModelFormat::Md3 | ModelFormat::Mdc => md3::load(bytes, model),
            ModelFormat::Md5Mesh => md5::load(bytes, model),
        }
    }
}
