//! Quake BSP map (BSP29 / BSP2 / 2PSB) parser.
//!
//! Donor provenance: `readQ1Bsp` in `src/formats/q1-map/index.ts`, record
//! readers in `src/formats/q1-map/records.ts`, the texture lump in
//! `src/formats/q1-map/textures.ts`, and entity text in
//! `src/formats/q1-map/entities.ts`.
//!
//! Lump payloads (visibility, lighting, texture levels) are borrowed
//! from the input; decoded records are owned. Lighting selection follows
//! `selectLighting` in `src/formats/q1-map/index.ts`: an external `.lit`
//! file wins over BSPX RGB samples, Quake64 packed samples, and the
//! monochrome lump (`readQ1Lit`, `BspLighting`). Q1 BSPX geometry and the
//! map queries live in [`crate::bspx`]; Quake64 map geometry stays
//! rejected with an explicit error, and only its packed lighting samples
//! are understood.

use std::borrow::Cow;

use qa_core::binary::{BinaryError, BinaryReader};

use crate::common::Bounds;
use crate::wad::MipTexture;

/// BSP29 version (`Q1_BSP_VERSION`).
pub const BSP_VERSION_29: u32 = 29;
/// BSP2 version (`Q1_BSP2_VERSION`).
pub const BSP_VERSION_BSP2: u32 = 0x3250_5342;
/// 2PSB version (`Q1_2PSB_VERSION`).
pub const BSP_VERSION_2PSB: u32 = 0x4253_5032;
/// Quake64 version (explicitly unsupported).
pub const BSP_VERSION_QUAKE64: u32 = 0x5136_3420;

/// BSP format (`Q1BspFormat`, minus Quake64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BspFormat {
    /// Classic 16-bit limits.
    Bsp29,
    /// 32-bit limits, float node bounds.
    Bsp2,
    /// 32-bit limits, short node bounds.
    Psb2,
}

/// Lump directory entry (`Q1Lump`, without copied bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lump {
    /// Lump name.
    pub name: &'static str,
    /// File offset.
    pub offset: i32,
    /// Length in bytes.
    pub length: i32,
}

/// BSP lump names in directory order.
pub const LUMP_NAMES: [&str; 15] = [
    "entities",
    "planes",
    "textures",
    "vertices",
    "visibility",
    "nodes",
    "textureInfo",
    "faces",
    "lighting",
    "clipnodes",
    "leaves",
    "leafFaces",
    "edges",
    "surfaceEdges",
    "models",
];

/// Index range (`IndexRange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexRange {
    /// First index.
    pub first: u32,
    /// Count.
    pub count: u32,
}

/// BSP plane (`BspPlane`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Normal.
    pub normal: [f32; 3],
    /// Distance.
    pub distance: f32,
    /// Plane type.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: u8,
}

/// BSP tree child (`BspChild`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeChild {
    /// Node index.
    Node(u32),
    /// Leaf index.
    Leaf(u32),
}

/// BSP node (`BspNode`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    /// Plane index.
    pub plane: u32,
    /// Children.
    pub children: [NodeChild; 2],
    /// Bounds.
    pub bounds: Bounds,
    /// Faces.
    pub faces: IndexRange,
}

/// BSP leaf (`Q1Leaf`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leaf {
    /// Contents.
    pub contents: i32,
    /// Visibility offset.
    pub visibility_offset: Option<u32>,
    /// Bounds.
    pub bounds: Bounds,
    /// Faces.
    pub faces: IndexRange,
    /// Ambient sound levels.
    pub ambient_sound: [u8; 4],
}

/// BSP edge (`BspEdge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// Vertex indices.
    pub vertices: [u32; 2],
}

/// BSP face (`BspFace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    /// Plane index.
    pub plane: u32,
    /// Back side.
    pub back: bool,
    /// First surface edge.
    pub edge_first: i32,
    /// Edge count.
    pub edge_count: u32,
    /// Texture info index.
    pub texture_info: u32,
    /// Light styles.
    pub styles: [u8; 4],
    /// Lighting offset.
    pub lighting_offset: Option<u32>,
}

/// Texture projection (`Q1TextureInfo`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureInfo {
    /// S projection.
    pub s: [f32; 4],
    /// T projection.
    pub t: [f32; 4],
    /// Texture index.
    pub texture: i32,
    /// Flags.
    pub flags: i32,
}

/// Clip child (`Q1ClipChild`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipChild {
    /// Contents value.
    Contents(i32),
    /// Clip node index.
    ClipNode(u32),
}

/// Clip node (`Q1ClipNode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipNode {
    /// Plane index.
    pub plane: i32,
    /// Children.
    pub children: [ClipChild; 2],
}

/// World model (`Q1WorldModel`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldModel {
    /// Bounds.
    pub bounds: Bounds,
    /// Origin.
    pub origin: [f32; 3],
    /// Head nodes per hull.
    pub headnodes: [i32; 4],
    /// Visible leaves.
    pub visible_leaves: i32,
    /// First face.
    pub face_first: i32,
    /// Face count.
    pub face_count: i32,
}

/// Quake entity (`Q1Entity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1Entity {
    /// Ordered properties (duplicates retained).
    pub properties: Vec<(String, String)>,
}

/// Parsed Quake map (`Q1Map`, core geometry without BSPX).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Map<'a> {
    /// Format.
    pub format: BspFormat,
    /// Source name.
    pub source: String,
    /// Raw version.
    pub version: u32,
    /// Input bytes (borrowed lump payloads point here).
    pub data: &'a [u8],
    /// Lump directory.
    pub lumps: Vec<Lump>,
    /// Entity text.
    pub entities: String,
    /// Parsed entities.
    pub entity_list: Vec<Q1Entity>,
    /// Planes.
    pub planes: Vec<Plane>,
    /// Vertices.
    pub vertices: Vec<[f32; 3]>,
    /// Textures (`None` entries are omitted).
    pub textures: Vec<Option<MipTexture<'a>>>,
    /// Texture lump offsets (`None` entries are omitted).
    pub texture_offsets: Vec<Option<u32>>,
    /// Mip offsets per texture.
    pub mip_offsets: Vec<Option<[u32; 4]>>,
    /// Texture info.
    pub texture_info: Vec<TextureInfo>,
    /// Faces.
    pub faces: Vec<Face>,
    /// Models.
    pub models: Vec<WorldModel>,
    /// Nodes.
    pub nodes: Vec<Node>,
    /// Leaves.
    pub leaves: Vec<Leaf>,
    /// Edges.
    pub edges: Vec<Edge>,
    /// Clip nodes.
    pub clipnodes: Vec<ClipNode>,
    /// Surface edges.
    pub surface_edges: Vec<i32>,
    /// Leaf faces.
    pub leaf_faces: Vec<u32>,
    /// Visibility data.
    pub visibility: &'a [u8],
    /// Monochrome lighting (`monochromeLighting` in `src/formats/q1-map/types.ts`).
    pub monochrome_lighting: &'a [u8],
    /// Selected lighting (`lighting` on `Q1WorldGeometry` in
    /// `src/contracts/scene.ts`): the `.lit` override wins, else monochrome.
    pub lighting: BspLighting<'a>,
}

fn optional_offset(value: i32) -> Option<u32> {
    if value == -1 {
        None
    } else {
        Some(value as u32)
    }
}

fn vec3(reader: &mut BinaryReader<'_>) -> Result<[f32; 3], BinaryError> {
    Ok([reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?])
}

fn vec4(reader: &mut BinaryReader<'_>) -> Result<[f32; 4], BinaryError> {
    Ok([
        reader.finite_f32()?,
        reader.finite_f32()?,
        reader.finite_f32()?,
        reader.finite_f32()?,
    ])
}

fn short_vec3(reader: &mut BinaryReader<'_>) -> Result<[f32; 3], BinaryError> {
    Ok([
        f32::from(reader.i16()?),
        f32::from(reader.i16()?),
        f32::from(reader.i16()?),
    ])
}

fn bounds(reader: &mut BinaryReader<'_>, floats: bool) -> Result<Bounds, BinaryError> {
    if floats {
        Ok(Bounds {
            min: vec3(reader)?,
            max: vec3(reader)?,
        })
    } else {
        Ok(Bounds {
            min: short_vec3(reader)?,
            max: short_vec3(reader)?,
        })
    }
}

fn check_index(value: i64, count: usize, source: &str) -> Result<(), BinaryError> {
    if value < 0 || value >= count as i64 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("index {value} outside {count} records"),
        });
    }
    Ok(())
}

fn check_range(first: i64, count: i64, len: usize, source: &str) -> Result<(), BinaryError> {
    if first < 0 || count < 0 || first > len as i64 - count {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("range {first}+{count} outside {len} records"),
        });
    }
    Ok(())
}

fn read_planes(reader: &mut BinaryReader<'_>) -> Result<Vec<Plane>, BinaryError> {
    let mut planes = Vec::new();
    while reader.remaining() > 0 {
        let normal = vec3(reader)?;
        let distance = reader.finite_f32()?;
        let plane_type = reader.i32()?;
        let signbits =
            (u8::from(normal[0] < 0.0)) | (u8::from(normal[1] < 0.0) << 1) | (u8::from(normal[2] < 0.0) << 2);
        planes.push(Plane {
            normal,
            distance,
            plane_type,
            signbits,
        });
    }
    Ok(planes)
}

fn node_child(reader: &mut BinaryReader<'_>, format: BspFormat, node_count: u32) -> Result<NodeChild, BinaryError> {
    if format == BspFormat::Bsp29 {
        let value = u32::from(reader.u16()?);
        if value < node_count {
            return Ok(NodeChild::Node(value));
        }
        return Ok(NodeChild::Leaf(65535 - value));
    }
    let value = reader.i32()?;
    if value >= 0 {
        return Ok(NodeChild::Node(value as u32));
    }
    Ok(NodeChild::Leaf((-1 - value) as u32))
}

fn read_nodes(reader: &mut BinaryReader<'_>, source: &str, format: BspFormat) -> Result<Vec<Node>, BinaryError> {
    let stride = match format {
        BspFormat::Bsp29 => 24,
        BspFormat::Psb2 => 32,
        BspFormat::Bsp2 => 44,
    };
    if !reader.length().is_multiple_of(stride) {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("lump length {} is not a multiple of {stride}", reader.length()),
        });
    }
    let node_count = (reader.length() / stride) as u32;
    let mut nodes = Vec::with_capacity(node_count as usize);
    while reader.remaining() > 0 {
        let plane = reader.i32()? as u32;
        let children = [
            node_child(reader, format, node_count)?,
            node_child(reader, format, node_count)?,
        ];
        let bounds = bounds(reader, format == BspFormat::Bsp2)?;
        let faces = if format == BspFormat::Bsp29 {
            IndexRange {
                first: u32::from(reader.u16()?),
                count: u32::from(reader.u16()?),
            }
        } else {
            IndexRange {
                first: reader.u32()?,
                count: reader.u32()?,
            }
        };
        nodes.push(Node {
            plane,
            children,
            bounds,
            faces,
        });
    }
    Ok(nodes)
}

fn clip_child(reader: &mut BinaryReader<'_>, format: BspFormat, count: i64) -> Result<ClipChild, BinaryError> {
    let mut value = if format == BspFormat::Bsp29 {
        i64::from(reader.u16()?)
    } else {
        i64::from(reader.i32()?)
    };
    if format == BspFormat::Bsp29 && value >= count {
        value -= 65536;
    }
    if value < 0 {
        return Ok(ClipChild::Contents(value as i32));
    }
    Ok(ClipChild::ClipNode(value as u32))
}

fn read_clipnodes(reader: &mut BinaryReader<'_>, format: BspFormat) -> Result<Vec<ClipNode>, BinaryError> {
    let stride = if format == BspFormat::Bsp29 { 8 } else { 12 };
    let count = reader.length() as i64 / stride as i64;
    let mut nodes = Vec::with_capacity(count as usize);
    while reader.remaining() > 0 {
        let plane = reader.i32()?;
        nodes.push(ClipNode {
            plane,
            children: [clip_child(reader, format, count)?, clip_child(reader, format, count)?],
        });
    }
    Ok(nodes)
}

fn read_leaves(reader: &mut BinaryReader<'_>, format: BspFormat) -> Result<Vec<Leaf>, BinaryError> {
    let mut leaves = Vec::new();
    while reader.remaining() > 0 {
        let contents = reader.i32()?;
        let visibility_offset = optional_offset(reader.i32()?);
        let bounds = bounds(reader, format == BspFormat::Bsp2)?;
        let faces = if format == BspFormat::Bsp29 {
            IndexRange {
                first: u32::from(reader.u16()?),
                count: u32::from(reader.u16()?),
            }
        } else {
            IndexRange {
                first: reader.u32()?,
                count: reader.u32()?,
            }
        };
        leaves.push(Leaf {
            contents,
            visibility_offset,
            bounds,
            faces,
            ambient_sound: [reader.u8()?, reader.u8()?, reader.u8()?, reader.u8()?],
        });
    }
    Ok(leaves)
}

fn read_edges(reader: &mut BinaryReader<'_>, format: BspFormat) -> Result<Vec<Edge>, BinaryError> {
    let mut edges = Vec::new();
    while reader.remaining() > 0 {
        if format == BspFormat::Bsp29 {
            edges.push(Edge {
                vertices: [u32::from(reader.u16()?), u32::from(reader.u16()?)],
            });
        } else {
            edges.push(Edge {
                vertices: [reader.u32()?, reader.u32()?],
            });
        }
    }
    Ok(edges)
}

fn read_faces(reader: &mut BinaryReader<'_>, source: &str, format: BspFormat) -> Result<Vec<Face>, BinaryError> {
    let mut faces = Vec::new();
    while reader.remaining() > 0 {
        let plane = if format == BspFormat::Bsp29 {
            u32::from(reader.u16()?)
        } else {
            reader.u32()?
        };
        let side = if format == BspFormat::Bsp29 {
            u32::from(reader.u16()?)
        } else {
            reader.u32()?
        };
        if side != 0 && side != 1 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: reader.offset(),
                message: format!("invalid face side {side}"),
            });
        }
        let edge_first = reader.i32()?;
        let edge_count = if format == BspFormat::Bsp29 {
            u32::from(reader.u16()?)
        } else {
            reader.u32()?
        };
        let texture_info = if format == BspFormat::Bsp29 {
            u32::from(reader.u16()?)
        } else {
            reader.u32()?
        };
        let styles = [reader.u8()?, reader.u8()?, reader.u8()?, reader.u8()?];
        faces.push(Face {
            plane,
            back: side == 1,
            edge_first,
            edge_count,
            texture_info,
            styles,
            lighting_offset: optional_offset(reader.i32()?),
        });
    }
    Ok(faces)
}

fn read_texture_info(reader: &mut BinaryReader<'_>) -> Result<Vec<TextureInfo>, BinaryError> {
    let mut infos = Vec::new();
    while reader.remaining() > 0 {
        infos.push(TextureInfo {
            s: vec4(reader)?,
            t: vec4(reader)?,
            texture: reader.i32()?,
            flags: reader.i32()?,
        });
    }
    Ok(infos)
}

fn read_models(reader: &mut BinaryReader<'_>) -> Result<Vec<WorldModel>, BinaryError> {
    let mut models = Vec::new();
    while reader.remaining() > 0 {
        models.push(WorldModel {
            bounds: bounds(reader, true)?,
            origin: vec3(reader)?,
            headnodes: [reader.i32()?, reader.i32()?, reader.i32()?, reader.i32()?],
            visible_leaves: reader.i32()?,
            face_first: reader.i32()?,
            face_count: reader.i32()?,
        });
    }
    Ok(models)
}

struct TextureLump<'a> {
    textures: Vec<Option<MipTexture<'a>>>,
    offsets: Vec<Option<u32>>,
    mip_offsets: Vec<Option<[u32; 4]>>,
}

fn read_textures<'a>(
    data: &'a [u8],
    source: &str,
    lump_offset: usize,
    lump_length: usize,
) -> Result<TextureLump<'a>, BinaryError> {
    if lump_length == 0 {
        return Ok(TextureLump {
            textures: Vec::new(),
            offsets: Vec::new(),
            mip_offsets: Vec::new(),
        });
    }
    let lump = data
        .get(lump_offset..lump_offset + lump_length)
        .ok_or_else(|| BinaryError {
            input: source.to_string(),
            offset: lump_offset,
            message: "texture lump exceeds input".to_string(),
        })?;
    let mut reader = BinaryReader::new(lump, source);
    let texture_count = reader.i32()?;
    if texture_count < 0 || texture_count as usize > reader.remaining() / 4 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: lump_offset,
            message: format!("invalid miptex count {texture_count}"),
        });
    }
    let mut offsets = Vec::with_capacity(texture_count as usize);
    let table_end = 4i64 + i64::from(texture_count) * 4;
    for _ in 0..texture_count {
        let offset = reader.i32()?;
        let entry_offset = lump_offset + reader.offset() - 4;
        if offset != -1 && i64::from(offset) < table_end {
            return Err(BinaryError {
                input: source.to_string(),
                offset: entry_offset,
                message: "miptex overlaps offset table".to_string(),
            });
        }
        offsets.push(if offset == -1 { None } else { Some(offset as u32) });
    }
    let mut textures = Vec::with_capacity(offsets.len());
    let mut mip_offsets = Vec::with_capacity(offsets.len());
    for offset in &offsets {
        let Some(offset) = *offset else {
            textures.push(None);
            mip_offsets.push(None);
            continue;
        };
        let header_at = lump_offset + offset as usize;
        let header = BinaryReader::new(data, source)
            .section(header_at, 40)
            .map_err(|_| BinaryError {
                input: source.to_string(),
                offset: header_at,
                message: "miptex header exceeds input".to_string(),
            })?;
        let mut header = header;
        let name = header.fixed_byte_string(16)?;
        let width = header.u32()?;
        let height = header.u32()?;
        let mips = [header.u32()?, header.u32()?, header.u32()?, header.u32()?];
        mip_offsets.push(Some(mips));
        if width == 0 || height == 0 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: header_at,
                message: "zero-sized mip texture".to_string(),
            });
        }
        if mips == [0, 0, 0, 0] {
            textures.push(Some(MipTexture::External { name, width, height }));
            continue;
        }
        let mut levels: [&[u8]; 4] = [&[], &[], &[], &[]];
        for (index, level) in levels.iter_mut().enumerate() {
            let scale = 1u32 << index;
            if mips[index] < 40 {
                return Err(BinaryError {
                    input: source.to_string(),
                    offset: header_at + mips[index] as usize,
                    message: "mip pixels overlap texture header".to_string(),
                });
            }
            let length = (width / scale) as usize * (height / scale) as usize;
            let at = header_at + mips[index] as usize;
            *level = data.get(at..at + length).ok_or_else(|| BinaryError {
                input: source.to_string(),
                offset: at,
                message: "mip pixels exceed input".to_string(),
            })?;
        }
        textures.push(Some(MipTexture::Embedded {
            name,
            width,
            height,
            levels,
        }));
    }
    Ok(TextureLump {
        textures,
        offsets,
        mip_offsets,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntityToken {
    Open,
    Close,
    Text(String),
}

struct EntityTokenizer {
    chars: Vec<char>,
    position: usize,
}

impl EntityTokenizer {
    fn token(&mut self) -> Result<Option<EntityToken>, usize> {
        while self.position < self.chars.len() {
            let character = self.chars[self.position];
            if character == '\0' {
                return Ok(None);
            }
            if character <= ' ' {
                self.position += 1;
                continue;
            }
            if character == '/' && self.position + 1 < self.chars.len() && self.chars[self.position + 1] == '/' {
                while self.position < self.chars.len() && self.chars[self.position] != '\n' {
                    self.position += 1;
                }
                continue;
            }
            break;
        }
        if self.position >= self.chars.len() {
            return Ok(None);
        }
        let first = self.chars[self.position];
        self.position += 1;
        if first == '{' {
            return Ok(Some(EntityToken::Open));
        }
        if first == '}' {
            return Ok(Some(EntityToken::Close));
        }
        if first == '"' {
            let start = self.position;
            while self.position < self.chars.len()
                && self.chars[self.position] != '"'
                && self.chars[self.position] != '\0'
            {
                self.position += 1;
            }
            if self.position >= self.chars.len() || self.chars[self.position] == '\0' {
                return Err(start);
            }
            let value: String = self.chars[start..self.position].iter().collect();
            self.position += 1;
            return Ok(Some(EntityToken::Text(value)));
        }
        let start = self.position - 1;
        while self.position < self.chars.len()
            && self.chars[self.position] > ' '
            && self.chars[self.position] != '{'
            && self.chars[self.position] != '}'
        {
            self.position += 1;
        }
        Ok(Some(EntityToken::Text(
            self.chars[start..self.position].iter().collect(),
        )))
    }
}

/// Parse Quake entity text (`parseQ1Entities`).
pub fn parse_q1_entities(text: &str, source: &str) -> Result<Vec<Q1Entity>, BinaryError> {
    let mut tokenizer = EntityTokenizer {
        chars: text.chars().collect(),
        position: 0,
    };
    let mut entities = Vec::new();
    let tokenize = |tokenizer: &mut EntityTokenizer| {
        tokenizer.token().map_err(|start| BinaryError {
            input: source.to_string(),
            offset: start,
            message: "unterminated quote".to_string(),
        })
    };
    while let Some(first) = tokenize(&mut tokenizer)? {
        if first != EntityToken::Open {
            return Err(BinaryError {
                input: source.to_string(),
                offset: tokenizer.position,
                message: "expected opening brace".to_string(),
            });
        }
        let mut properties = Vec::new();
        loop {
            let key = tokenize(&mut tokenizer)?;
            if key == Some(EntityToken::Close) {
                break;
            }
            let Some(EntityToken::Text(key)) = key else {
                return Err(BinaryError {
                    input: source.to_string(),
                    offset: tokenizer.position,
                    message: "expected key or closing brace".to_string(),
                });
            };
            let value = tokenize(&mut tokenizer)?;
            let Some(EntityToken::Text(value)) = value else {
                return Err(BinaryError {
                    input: source.to_string(),
                    offset: tokenizer.position,
                    message: format!("expected value for {key}"),
                });
            };
            properties.push((key, value));
        }
        entities.push(Q1Entity { properties });
    }
    Ok(entities)
}

/// RGB lighting source (`BspLighting` source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightingSource {
    /// Unpacked Quake64 samples from the lighting lump.
    Bsp,
    /// External `.lit` override.
    Lit,
    /// BSPX RGB samples.
    Bspx,
}

/// Selected map lighting (`BspLighting`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BspLighting<'a> {
    /// Monochrome samples borrowed from the lighting lump.
    Luminance8 {
        /// One byte per sample.
        samples: &'a [u8],
    },
    /// RGB samples with their source.
    Rgb8 {
        /// Three bytes per sample.
        samples: Cow<'a, [u8]>,
        /// Where the samples came from.
        source: LightingSource,
    },
}

/// Read an external Quake `.lit` file (`readQ1Lit`).
///
/// `sample_count` is the lighting sample count the file must match; `None`
/// (used when the map has no lighting samples) only requires whole RGB
/// triples.
pub fn read_q1_lit(data: &[u8], sample_count: Option<usize>, source: &str) -> Result<Vec<u8>, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    reader.expect_magic("QLIT")?;
    let version = reader.u32()?;
    if version != 1 {
        return Err(BinaryError::custom(source, 4, format!("unsupported version {version}")));
    }
    if !reader.remaining().is_multiple_of(3) || sample_count.is_some_and(|count| reader.remaining() != count * 3) {
        return Err(BinaryError::custom(
            source,
            8,
            "RGB sample count differs from BSP lighting",
        ));
    }
    reader.bytes(reader.remaining())
}

/// Expand Quake64 packed lighting samples to RGB (`selectLighting`).
fn expand_packed_lighting(packed: &[u8]) -> Vec<u8> {
    let mut samples = Vec::with_capacity(packed.len() / 2 * 3);
    for pair in packed.as_chunks::<2>().0 {
        let first = pair[0];
        let second = pair[1];
        samples.push(first & 0xf8);
        samples.push(((first & 7) << 5) | ((second & 0xc0) >> 5));
        samples.push((second & 0x3f) << 2);
    }
    samples
}

/// Select map lighting (`selectLighting`).
///
/// Priority is `.lit` override, BSPX RGB samples, Quake64 packed samples,
/// then the monochrome lump. `packed` carries the raw Quake64 lighting
/// lump; `monochrome` is empty for Quake64 maps.
pub fn select_lighting<'a>(
    monochrome: &'a [u8],
    rgb: Option<&'a [u8]>,
    lit: Option<&[u8]>,
    packed: Option<&'a [u8]>,
) -> Result<BspLighting<'a>, BinaryError> {
    if packed.is_some_and(|samples| !samples.len().is_multiple_of(2)) {
        return Err(BinaryError::custom(
            "Quake64 lighting",
            0,
            "incomplete packed RGB sample",
        ));
    }
    let sample_count = packed.map_or(monochrome.len(), |samples| samples.len() / 2);
    if let Some(lit) = lit {
        let expected = if sample_count > 0 { Some(sample_count) } else { None };
        let samples = read_q1_lit(lit, expected, "Quake .lit")?;
        return Ok(BspLighting::Rgb8 {
            samples: Cow::Owned(samples),
            source: LightingSource::Lit,
        });
    }
    if let Some(rgb) = rgb {
        if !rgb.len().is_multiple_of(3) || (sample_count > 0 && rgb.len() != sample_count * 3) {
            return Err(BinaryError::custom(
                "BSPX RGBLIGHTING",
                0,
                "RGB sample count differs from BSP lighting",
            ));
        }
        return Ok(BspLighting::Rgb8 {
            samples: Cow::Borrowed(rgb),
            source: LightingSource::Bspx,
        });
    }
    if let Some(packed) = packed {
        return Ok(BspLighting::Rgb8 {
            samples: Cow::Owned(expand_packed_lighting(packed)),
            source: LightingSource::Bsp,
        });
    }
    Ok(BspLighting::Luminance8 { samples: monochrome })
}

impl<'a> Q1Map<'a> {
    /// Select this map's lighting, applying an external `.lit` override.
    pub fn selected_lighting(&self, lit: Option<&[u8]>) -> Result<BspLighting<'a>, BinaryError> {
        select_lighting(self.monochrome_lighting, None, lit, None)
    }
}

/// Look up an entity property, last write wins (`q1EntityValue`).
#[must_use]
pub fn q1_entity_value<'a>(entity: &'a Q1Entity, key: &str) -> Option<&'a str> {
    let mut value = None;
    for property in &entity.properties {
        if property.0 == key {
            value = Some(property.1.as_str());
        }
    }
    value
}

/// BSP sidecar inputs (`Q1MapOptions` in `src/formats/q1-map/types.ts`).
///
/// The caller resolves mount priority; `None` selects the embedded lump,
/// matching an absent options field in `readQ1Bsp`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Q1BspOptions<'a> {
    /// Already resolved `.ent` replacement (`options.entities`).
    pub entities: Option<&'a [u8]>,
    /// Already resolved QLIT version 1 file (`options.lit`).
    pub lit: Option<&'a [u8]>,
}

/// Read a Quake BSP map (`readQ1Bsp`, core geometry).
pub fn read_q1_bsp<'a>(data: &'a [u8], source: &str, options: Q1BspOptions<'_>) -> Result<Q1Map<'a>, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let version = reader.u32()?;
    let format = match version {
        BSP_VERSION_29 => BspFormat::Bsp29,
        BSP_VERSION_BSP2 => BspFormat::Bsp2,
        BSP_VERSION_2PSB => BspFormat::Psb2,
        BSP_VERSION_QUAKE64 => {
            return Err(BinaryError {
                input: source.to_string(),
                offset: 0,
                message: "Quake64 BSP is not supported".to_string(),
            });
        }
        _ => {
            return Err(BinaryError {
                input: source.to_string(),
                offset: 0,
                message: format!("unsupported Quake BSP version {version}"),
            });
        }
    };
    let mut directory = reader.section(4, 120)?;
    let mut lumps = Vec::with_capacity(LUMP_NAMES.len());
    for name in LUMP_NAMES {
        let offset = directory.i32()?;
        let length = directory.i32()?;
        if length > 0 && offset < 124 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: offset.max(0) as usize,
                message: format!("{name} overlaps BSP header"),
            });
        }
        if offset < 0 || length < 0 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: 0,
                message: format!("negative {name} lump range"),
            });
        }
        if offset as usize + length as usize > data.len() {
            return Err(BinaryError {
                input: source.to_string(),
                offset: offset as usize,
                message: format!("{name} lump exceeds input"),
            });
        }
        lumps.push(Lump { name, offset, length });
    }
    let lump = |name: &str| -> (usize, usize) {
        let entry = lumps.iter().find(|lump| lump.name == name).unwrap();
        (entry.offset as usize, entry.length as usize)
    };
    let section = |name: &str| -> Result<BinaryReader<'_>, BinaryError> {
        let (offset, length) = lump(name);
        reader.section(offset, length)
    };
    let (entities_offset, entities_length) = lump("entities");
    let (entities, entity_list) = match options.entities {
        Some(override_bytes) => {
            let text =
                BinaryReader::new(override_bytes, &format!("{source}:.ent")).fixed_byte_string(override_bytes.len())?;
            let list = parse_q1_entities(&text, source)?;
            (text, list)
        }
        None => {
            let text = BinaryReader::new(data, source)
                .section(entities_offset, entities_length)?
                .fixed_byte_string(entities_length)?;
            let list = parse_q1_entities(&text, &format!("{source}:entities"))?;
            (text, list)
        }
    };
    let planes = read_planes(&mut section("planes")?)?;
    let mut vertex_reader = section("vertices")?;
    let mut vertices = Vec::new();
    while vertex_reader.remaining() > 0 {
        vertices.push(vec3(&mut vertex_reader)?);
    }
    let (textures_offset, textures_length) = lump("textures");
    let texture_lump = read_textures(data, source, textures_offset, textures_length)?;
    let faces = read_faces(&mut section("faces")?, source, format)?;
    let models = read_models(&mut section("models")?)?;
    let texture_info = read_texture_info(&mut section("textureInfo")?)?;
    let nodes = read_nodes(&mut section("nodes")?, source, format)?;
    let leaves = read_leaves(&mut section("leaves")?, format)?;
    let edges = read_edges(&mut section("edges")?, format)?;
    let clipnodes = read_clipnodes(&mut section("clipnodes")?, format)?;
    let mut surface_reader = section("surfaceEdges")?;
    let mut surface_edges = Vec::new();
    while surface_reader.remaining() > 0 {
        surface_edges.push(surface_reader.i32()?);
    }
    let mut leaf_face_reader = section("leafFaces")?;
    let mut leaf_faces = Vec::new();
    while leaf_face_reader.remaining() > 0 {
        if format == BspFormat::Bsp29 {
            leaf_faces.push(u32::from(leaf_face_reader.u16()?));
        } else {
            leaf_faces.push(leaf_face_reader.u32()?);
        }
    }
    let (visibility_offset, visibility_length) = lump("visibility");
    let visibility = reader.view(visibility_offset, visibility_length)?;
    let (lighting_offset, lighting_length) = lump("lighting");
    let monochrome_lighting = reader.view(lighting_offset, lighting_length)?;
    // `selectLighting` with no BSPX RGB (core reader without BSPX) and no
    // packed samples (Quake64 is rejected above).
    let lighting = select_lighting(monochrome_lighting, None, options.lit, None)?;
    let map = Q1Map {
        format,
        source: source.to_string(),
        version,
        data,
        lumps,
        entities,
        entity_list,
        planes,
        vertices,
        textures: texture_lump.textures,
        texture_offsets: texture_lump.offsets,
        mip_offsets: texture_lump.mip_offsets,
        texture_info,
        faces,
        models,
        nodes,
        leaves,
        edges,
        clipnodes,
        surface_edges,
        leaf_faces,
        visibility,
        monochrome_lighting,
        lighting,
    };
    validate_references(&map)?;
    Ok(map)
}

fn validate_references(map: &Q1Map<'_>) -> Result<(), BinaryError> {
    let source = |label: &str| format!("{}:{label}", map.source);
    for edge in &map.edges {
        for vertex in edge.vertices {
            check_index(i64::from(vertex), map.vertices.len(), &source("edge vertex"))?;
        }
    }
    for edge in &map.surface_edges {
        check_index(i64::from(*edge).abs(), map.edges.len(), &source("surface edge"))?;
    }
    for face in &map.leaf_faces {
        check_index(i64::from(*face), map.faces.len(), &source("leaf face"))?;
    }
    for info in &map.texture_info {
        if !map.textures.is_empty() {
            check_index(i64::from(info.texture), map.textures.len(), &source("miptex"))?;
        }
    }
    // Donor `validateReferences`: face offsets validate against the wider of
    // the monochrome lump and the selected lighting (no HDR samples here).
    let selected_count = match &map.lighting {
        BspLighting::Luminance8 { samples } => samples.len(),
        BspLighting::Rgb8 { samples, .. } => samples.len() / 3,
    };
    let lighting_count = map.monochrome_lighting.len().max(selected_count);
    for face in &map.faces {
        check_index(i64::from(face.plane), map.planes.len(), &source("face plane"))?;
        check_index(
            i64::from(face.texture_info),
            map.texture_info.len(),
            &source("face texture info"),
        )?;
        check_range(
            i64::from(face.edge_first),
            i64::from(face.edge_count),
            map.surface_edges.len(),
            &source("face edges"),
        )?;
        if let Some(offset) = face.lighting_offset {
            if lighting_count > 0 {
                check_index(i64::from(offset), lighting_count, &source("face lighting"))?;
            }
        }
    }
    for leaf in &map.leaves {
        check_range(
            i64::from(leaf.faces.first),
            i64::from(leaf.faces.count),
            map.leaf_faces.len(),
            &source("leaf faces"),
        )?;
        if let Some(offset) = leaf.visibility_offset {
            if !map.visibility.is_empty() {
                check_index(i64::from(offset), map.visibility.len(), &source("leaf visibility"))?;
            }
        }
    }
    for node in &map.nodes {
        check_index(i64::from(node.plane), map.planes.len(), &source("node plane"))?;
        check_range(
            i64::from(node.faces.first),
            i64::from(node.faces.count),
            map.faces.len(),
            &source("node faces"),
        )?;
        for child in node.children {
            let (index, len) = match child {
                NodeChild::Node(index) => (index, map.nodes.len()),
                NodeChild::Leaf(index) => (index, map.leaves.len()),
            };
            check_index(i64::from(index), len, &source("node child"))?;
        }
    }
    for node in &map.clipnodes {
        check_index(i64::from(node.plane), map.planes.len(), &source("clipnode plane"))?;
        for child in node.children {
            if let ClipChild::ClipNode(index) = child {
                check_index(i64::from(index), map.clipnodes.len(), &source("clipnode child"))?;
            }
        }
    }
    for model in &map.models {
        check_range(
            i64::from(model.face_first),
            i64::from(model.face_count),
            map.faces.len(),
            &source("model faces"),
        )?;
        check_range(
            0,
            i64::from(model.visible_leaves),
            map.leaves.len().saturating_sub(1),
            &source("model visible leaves"),
        )?;
        for (hull, headnode) in model.headnodes.iter().enumerate() {
            if hull == 0 {
                if *headnode >= 0 {
                    check_index(i64::from(*headnode), map.nodes.len(), &source("model headnode"))?;
                } else {
                    check_index(i64::from(-1 - *headnode), map.leaves.len(), &source("model head leaf"))?;
                }
            } else if hull < 3 && *headnode >= 0 {
                check_index(
                    i64::from(*headnode),
                    map.clipnodes.len(),
                    &source("model clip headnode"),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    fn fixture() -> Vec<u8> {
        let entities = b"{\n\"classname\" \"worldspawn\"\n}\n\0";
        let mut planes = BinaryWriter::new(20);
        planes.f32(0.0).unwrap();
        planes.f32(0.0).unwrap();
        planes.f32(1.0).unwrap();
        planes.f32(0.0).unwrap();
        planes.i32(2).unwrap();
        let planes = planes.finish();
        let mut vertices = BinaryWriter::new(24);
        for _ in 0..6 {
            vertices.f32(0.0).unwrap();
        }
        let vertices = vertices.finish();
        let mut texinfo = BinaryWriter::new(40);
        for _ in 0..8 {
            texinfo.f32(0.0).unwrap();
        }
        texinfo.i32(0).unwrap();
        texinfo.i32(0).unwrap();
        let texinfo = texinfo.finish();
        let mut faces = BinaryWriter::new(20);
        faces.u16(0).unwrap();
        faces.u16(0).unwrap();
        faces.i32(0).unwrap();
        faces.u16(1).unwrap();
        faces.u16(0).unwrap();
        faces.bytes(&[0, 0, 0, 0]).unwrap();
        faces.i32(-1).unwrap();
        let faces = faces.finish();
        let mut leaves = BinaryWriter::new(28);
        leaves.i32(-1).unwrap();
        leaves.i32(-1).unwrap();
        for _ in 0..6 {
            leaves.i16(0).unwrap();
        }
        leaves.u16(0).unwrap();
        leaves.u16(0).unwrap();
        leaves.bytes(&[0, 0, 0, 0]).unwrap();
        let leaves = leaves.finish();
        let mut edges = BinaryWriter::new(4);
        edges.u16(0).unwrap();
        edges.u16(1).unwrap();
        let edges = edges.finish();
        let mut surfedges = BinaryWriter::new(4);
        surfedges.i32(0).unwrap();
        let surfedges = surfedges.finish();
        let mut models = BinaryWriter::new(64);
        for _ in 0..9 {
            models.f32(0.0).unwrap();
        }
        for _ in 0..4 {
            models.i32(-1).unwrap();
        }
        models.i32(0).unwrap();
        models.i32(0).unwrap();
        models.i32(1).unwrap();
        let models = models.finish();

        let lumps: [(&[u8], usize); 15] = [
            (entities, entities.len()),
            (&planes, planes.len()),
            (&[], 0),
            (&vertices, vertices.len()),
            (&[], 0),
            (&[], 0),
            (&texinfo, texinfo.len()),
            (&faces, faces.len()),
            (&[], 0),
            (&[], 0),
            (&leaves, leaves.len()),
            (&[], 0),
            (&edges, edges.len()),
            (&surfedges, surfedges.len()),
            (&models, models.len()),
        ];
        let mut writer = BinaryWriter::new(2048);
        writer.u32(BSP_VERSION_29).unwrap();
        let mut offset = 124;
        let mut directory = Vec::new();
        for (_, length) in &lumps {
            directory.push((offset as i32, *length as i32));
            offset += *length;
        }
        for (entry_offset, length) in &directory {
            writer.i32(*entry_offset).unwrap();
            writer.i32(*length).unwrap();
        }
        for (bytes, _) in &lumps {
            writer.bytes(bytes).unwrap();
        }
        writer.finish()
    }

    #[test]
    fn bsp_round_trip() {
        let bytes = fixture();
        let map = read_q1_bsp(&bytes, "<test>", Q1BspOptions::default()).unwrap();
        assert_eq!(map.format, BspFormat::Bsp29);
        assert_eq!(map.lumps.len(), 15);
        assert_eq!(map.entity_list.len(), 1);
        assert_eq!(q1_entity_value(&map.entity_list[0], "classname"), Some("worldspawn"));
        assert_eq!(map.planes.len(), 1);
        assert_eq!(map.planes[0].signbits, 0);
        assert_eq!(map.vertices.len(), 2);
        assert_eq!(map.faces.len(), 1);
        assert_eq!(map.models.len(), 1);
        assert_eq!(map.edges.len(), 1);
        assert_eq!(map.surface_edges, vec![0]);
        assert!(map.textures.is_empty());
        assert!(map.monochrome_lighting.is_empty());
    }

    #[test]
    fn bsp_entities_cover_syntax() {
        let entities = parse_q1_entities(
            "// comment\n{\n\"a\" \"1\"\n\"a\" \"2\"\n}\n{\nkey value\n}\n",
            "<test>",
        )
        .unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(q1_entity_value(&entities[0], "a"), Some("2"));
        assert!(parse_q1_entities("{ \"a\" }", "<test>").is_err());
        assert!(parse_q1_entities("{ \"a\" \"unterminated", "<test>").is_err());
        assert!(parse_q1_entities("} {", "<test>").is_err());
    }

    #[test]
    fn bsp_rejects_bad_input() {
        let good = fixture();
        let mut bad_version = good.clone();
        bad_version[0] = 30;
        assert!(read_q1_bsp(&bad_version, "<test>", Q1BspOptions::default()).is_err());
        let mut quake64 = good.clone();
        quake64[0..4].copy_from_slice(&BSP_VERSION_QUAKE64.to_le_bytes());
        assert!(read_q1_bsp(&quake64, "<test>", Q1BspOptions::default()).is_err());
        // Face plane index past the plane table.
        let mut bad_face = good.clone();
        // Header + entities + planes + vertices + texture info (skipping empties).
        let faces_offset = 124 + 30 + 20 + 24 + 40;
        bad_face[faces_offset] = 9;
        assert!(read_q1_bsp(&bad_face, "<test>", Q1BspOptions::default()).is_err());
    }

    fn lit_bytes(samples: &[u8]) -> Vec<u8> {
        let mut bytes = b"QLIT".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(samples);
        bytes
    }

    #[test]
    fn lit_round_trip() {
        let samples = [10, 20, 30, 40, 50, 60];
        assert_eq!(read_q1_lit(&lit_bytes(&samples), Some(2), "<test>").unwrap(), samples);
        // Empty maps skip the sample-count check but still require triples.
        assert_eq!(read_q1_lit(&lit_bytes(&samples), None, "<test>").unwrap(), samples);
        assert!(read_q1_lit(&lit_bytes(&samples), Some(3), "<test>").is_err());
        assert!(read_q1_lit(&lit_bytes(&[1, 2]), Some(0), "<test>").is_err());
        assert!(read_q1_lit(b"XXXX\x01\x00\x00\x00", Some(0), "<test>").is_err());
        let mut bad_version = lit_bytes(&samples);
        bad_version[4] = 2;
        let error = read_q1_lit(&bad_version, Some(2), "<test>").unwrap_err();
        assert_eq!(error.message, "unsupported version 2");
        assert_eq!(error.offset, 4);
    }

    #[test]
    fn lighting_selection_prefers_lit_over_rgb_over_packed_over_mono() {
        let mono = [7u8, 8];
        let rgb = [1, 2, 3, 4, 5, 6];
        let packed = [0b1111_1001u8, 0b0110_0100u8, 0x00, 0x00];
        let lit = lit_bytes(&[9, 9, 9, 8, 8, 8]);
        // Packed expansion: 0xf9 & f8 = f8; ((1) << 5) | (0x40 >> 5) = 34; (0x24) << 2 = 0x90.
        let expanded = [0xf8u8, 34, 0x90, 0, 0, 0];
        assert!(matches!(
            select_lighting(&mono, Some(&rgb), Some(&lit), Some(&packed)).unwrap(),
            BspLighting::Rgb8 {
                source: LightingSource::Lit,
                ..
            }
        ));
        match select_lighting(&mono, Some(&rgb), None, Some(&packed)).unwrap() {
            BspLighting::Rgb8 { samples, source } => {
                assert_eq!(source, LightingSource::Bspx);
                assert_eq!(samples.as_ref(), &rgb);
            }
            other => panic!("expected bspx rgb, got {other:?}"),
        }
        match select_lighting(&[], None, None, Some(&packed)).unwrap() {
            BspLighting::Rgb8 { samples, source } => {
                assert_eq!(source, LightingSource::Bsp);
                assert_eq!(samples.as_ref(), &expanded);
            }
            other => panic!("expected packed rgb, got {other:?}"),
        }
        match select_lighting(&mono, None, None, None).unwrap() {
            BspLighting::Luminance8 { samples } => assert_eq!(samples, &mono),
            other => panic!("expected monochrome, got {other:?}"),
        }
        // A .lit override must still match the packed sample count.
        assert!(select_lighting(&[], None, Some(&lit_bytes(&[1, 2, 3])), Some(&packed)).is_err());
        // Odd packed input and mismatched RGB fail like the donor.
        assert!(select_lighting(&mono, None, None, Some(&[1, 2, 3])).is_err());
        assert!(select_lighting(&mono, Some(&[1, 2, 3, 4]), None, None).is_err());
        assert!(select_lighting(&mono, Some(&[1, 2, 3]), None, None).is_err());
    }

    #[test]
    fn map_applies_lit_override() {
        let bytes = fixture();
        let map = read_q1_bsp(&bytes, "<test>", Q1BspOptions::default()).unwrap();
        assert!(matches!(
            map.selected_lighting(None).unwrap(),
            BspLighting::Luminance8 { .. }
        ));
        // The empty fixture lighting skips the count check.
        let lit = lit_bytes(&[1, 2, 3]);
        match map.selected_lighting(Some(&lit)).unwrap() {
            BspLighting::Rgb8 { samples, source } => {
                assert_eq!(source, LightingSource::Lit);
                assert_eq!(samples.as_ref(), &[1, 2, 3]);
            }
            other => panic!("expected lit rgb, got {other:?}"),
        }
    }

    #[test]
    fn bsp_options_thread_ent_and_lit_sidecars() {
        let bytes = fixture();
        // Missing sidecars fall back to embedded entities and monochrome lighting.
        let map = read_q1_bsp(&bytes, "<test>", Q1BspOptions::default()).unwrap();
        assert!(matches!(map.lighting, BspLighting::Luminance8 { .. }));
        assert_eq!(map.entity_list.len(), 1);
        assert_eq!(q1_entity_value(&map.entity_list[0], "classname"), Some("worldspawn"));
        // A .lit sidecar flips the selected lighting to RGB.
        let lit = lit_bytes(&[1, 2, 3]);
        let options = Q1BspOptions {
            lit: Some(&lit),
            ..Default::default()
        };
        let map = read_q1_bsp(&bytes, "<test>", options).unwrap();
        match map.lighting {
            BspLighting::Rgb8 { samples, source } => {
                assert_eq!(source, LightingSource::Lit);
                assert_eq!(samples.as_ref(), &[1, 2, 3]);
            }
            other => panic!("expected lit rgb, got {other:?}"),
        }
        // A .ent sidecar replaces the embedded entities.
        let entities = b"{\n\"classname\" \"info_player_start\"\n}\n";
        let options = Q1BspOptions {
            entities: Some(entities),
            ..Default::default()
        };
        let map = read_q1_bsp(&bytes, "<test>", options).unwrap();
        assert_eq!(map.entity_list.len(), 1);
        assert_eq!(
            q1_entity_value(&map.entity_list[0], "classname"),
            Some("info_player_start")
        );
        // Malformed sidecars fail like the donor.
        let options = Q1BspOptions {
            lit: Some(&lit_bytes(&[1, 2])),
            ..Default::default()
        };
        assert!(read_q1_bsp(&bytes, "<test>", options).is_err());
        let options = Q1BspOptions {
            entities: Some(b"{ bad"),
            ..Default::default()
        };
        assert!(read_q1_bsp(&bytes, "<test>", options).is_err());
    }
}
