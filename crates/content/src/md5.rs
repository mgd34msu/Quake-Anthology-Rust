//! Doom 3 / rerelease skeletal models (MD5 v10 mesh and animation).
//!
//! Donor provenance: `src/formats/q3-model/md5.ts` (mesh, animation,
//! scales, and skinning from the Q1/Q2 rerelease ports and q2repro
//! `refresh/models.c`, `mesh.c`), with tokens from [`crate::model_text`]
//! and quaternions from [`crate::quaternion`]. Scale files are JSON; the
//! small strict parser below preserves the donor fallback to unit scales
//! with diagnostics.
//!
//! Single-`fround` expressions compute in `f64` and cast once at the
//! store, per the `qa-core` math discipline; per-operation roundings reuse
//! [`qa_core::math`].

use std::collections::{HashMap, HashSet};

use qa_core::binary::BinaryError;
use qa_core::math::{add3, sub3, vec3, Bounds, Vec2, Vec3, Vec4};

use crate::bsp::IndexRange;
use crate::model_text::{at, indexed_records, ModelTokens};
use crate::quaternion::{
    conjugate_quaternion, md5_quaternion, multiply_quaternion, normalize_quaternion, quaternion_rotation_rows,
    rotate_quaternion, rotate_quaternion_rows, slerp_quaternion,
};

/// MD5 joint (`Md5Joint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Md5Joint {
    /// Name.
    pub name: String,
    /// Parent index, or -1.
    pub parent: i32,
    /// Whether frame scales apply to positions.
    pub scale_positions: bool,
}

/// Skeleton joint pose (`SkeletonJointPose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkeletonJointPose {
    /// Position.
    pub position: Vec3,
    /// Orientation.
    pub orientation: Vec4,
    /// Scale.
    pub scale: f32,
}

/// MD5 vertex weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md5Weight {
    /// Joint index.
    pub joint: u32,
    /// Bias.
    pub bias: f32,
    /// Position.
    pub position: Vec3,
}

/// MD5 vertex.
#[derive(Debug, Clone, PartialEq)]
pub struct Md5Vertex {
    /// Texture coordinates.
    pub tex_coord: Vec2,
    /// Normal.
    pub normal: Vec3,
    /// Weight range.
    pub weights: IndexRange,
}

/// MD5 mesh (`Md5Mesh`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md5Mesh {
    /// Shader name.
    pub shader: String,
    /// Vertices.
    pub vertices: Vec<Md5Vertex>,
    /// Indices.
    pub indices: Vec<u32>,
    /// Weights.
    pub weights: Vec<Md5Weight>,
}

/// Parsed MD5 mesh file (`Md5MeshFile`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md5MeshFile {
    /// Source name.
    pub source: String,
    /// Command line.
    pub command_line: String,
    /// Joints.
    pub joints: Vec<Md5Joint>,
    /// Model-space bind pose.
    pub bind_pose: Vec<SkeletonJointPose>,
    /// Meshes.
    pub meshes: Vec<Md5Mesh>,
}

/// MD5 hierarchy joint (`Md5HierarchyJoint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Md5HierarchyJoint {
    /// Name.
    pub name: String,
    /// Parent index, or -1.
    pub parent: i32,
    /// Animated component flags.
    pub flags: u32,
    /// First animated component.
    pub start_index: usize,
    /// Whether frame scales apply to positions.
    pub scale_positions: bool,
}

/// MD5 animation frame (`Md5AnimationFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md5AnimationFrame {
    /// Bounds.
    pub bounds: Bounds,
    /// Animated components.
    pub components: Vec<f32>,
    /// Local joints.
    pub local_joints: Vec<SkeletonJointPose>,
    /// Model-space joints.
    pub joints: Vec<SkeletonJointPose>,
}

/// Parsed MD5 animation (`Md5Animation`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md5Animation {
    /// Source name.
    pub source: String,
    /// Command line.
    pub command_line: String,
    /// Frame rate.
    pub frame_rate: i32,
    /// Hierarchy.
    pub hierarchy: Vec<Md5HierarchyJoint>,
    /// Base frame.
    pub base_frame: Vec<SkeletonJointPose>,
    /// Frames.
    pub frames: Vec<Md5AnimationFrame>,
    /// Scale source, when provided.
    pub scale_source: Option<String>,
    /// Loader diagnostics.
    pub diagnostics: Vec<String>,
}

/// Q1 replacement animation timing (`ReplacementAnimationTiming`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q1AnimationTiming {
    /// Entity frames drive the animation.
    EntityFrame,
    /// Elapsed time drives the animation.
    ElapsedTime {
        /// Frame rate.
        frame_rate: f32,
    },
}

/// MD5 skin selection (`Md5Model["skinSelection"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum SkinSelection {
    /// Mesh shaders select skins.
    MeshShaders,
    /// Q1 MDL replacement skins.
    Q1MdlReplacement {
        /// Per-mesh skin groups.
        mesh_skin_groups: Vec<Vec<crate::common::TimedFrames<String>>>,
        /// Model flags.
        flags: i32,
        /// Animation timing.
        timing: Q1AnimationTiming,
    },
    /// Q2 MD2 replacement skins.
    Q2Md2Replacement {
        /// Replacement skins.
        skins: Vec<String>,
        /// Source frame count.
        source_frame_count: usize,
        /// Scale source.
        scale_source: Option<String>,
        /// Loader diagnostics.
        diagnostics: Vec<String>,
    },
}

/// Decoded MD5 model (`DecodedMd5Model`).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedMd5Model {
    /// Mesh source name.
    pub mesh_source: String,
    /// Mesh command line.
    pub mesh_command_line: String,
    /// Mesh joints.
    pub mesh_joints: Vec<Md5Joint>,
    /// Model-space bind pose.
    pub bind_pose: Vec<SkeletonJointPose>,
    /// Animation.
    pub animation: Md5Animation,
    /// Joints (animation hierarchy).
    pub joints: Vec<Md5HierarchyJoint>,
    /// Meshes.
    pub meshes: Vec<Md5Mesh>,
    /// Frame rate.
    pub frame_rate: i32,
    /// Frames (animation frames).
    pub frames: Vec<Md5AnimationFrame>,
    /// Skin selection.
    pub skin_selection: SkinSelection,
}

/// Optional MD5 scale file (`Md5ScaleSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Md5ScaleSource {
    /// Source name.
    pub source: String,
    /// File text.
    pub text: String,
}

/// Skinned MD5 vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md5SkinnedVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
}

fn md5_header(tokens: &mut ModelTokens) -> Result<String, BinaryError> {
    tokens.expect("MD5Version")?;
    tokens.expect("10")?;
    tokens.expect("commandline")?;
    tokens.token()
}

fn read_mesh(tokens: &mut ModelTokens, joints: &[SkeletonJointPose]) -> Result<Md5Mesh, BinaryError> {
    tokens.expect("mesh")?;
    tokens.expect("{")?;
    tokens.expect("shader")?;
    let shader = tokens.token()?;
    tokens.expect("numverts")?;
    let vertex_count = tokens.integer(0, 65535)? as usize;
    let mut vertices = indexed_records(tokens, vertex_count, "vert", |tokens| {
        tokens.expect("(")?;
        let tex_coord = qa_core::math::vec2(tokens.float()?, tokens.float()?);
        tokens.expect(")")?;
        Ok(Md5Vertex {
            tex_coord,
            normal: vec3(0.0, 0.0, 0.0),
            weights: IndexRange {
                first: tokens.integer(0, i32::MAX)? as u32,
                count: tokens.integer(0, i32::MAX)? as u32,
            },
        })
    })?;
    tokens.expect("numtris")?;
    let triangle_count = tokens.integer(0, 65535)? as usize;
    let triangles = indexed_records(tokens, triangle_count, "tri", |tokens| {
        Ok([
            tokens.integer(0, vertex_count as i32 - 1)? as u32,
            tokens.integer(0, vertex_count as i32 - 1)? as u32,
            tokens.integer(0, vertex_count as i32 - 1)? as u32,
        ])
    })?;
    tokens.expect("numweights")?;
    let weight_count = tokens.integer(0, 1_048_576)? as usize;
    let weights = indexed_records(tokens, weight_count, "weight", |tokens| {
        let joint = tokens.integer(0, joints.len() as i32 - 1)? as u32;
        let bias = tokens.float()?;
        if !(0.0..=1.0).contains(&bias) {
            return tokens.fail(format!("weight bias {bias} outside 0..1"));
        }
        Ok(Md5Weight {
            joint,
            bias,
            position: tokens.vector()?,
        })
    })?;
    tokens.expect("}")?;
    for vertex in &vertices {
        if vertex.weights.count as usize > weight_count
            || vertex.weights.first as usize > weight_count - vertex.weights.count as usize
        {
            return tokens.fail("vertex weight range exceeds mesh weights".to_string());
        }
    }
    let mesh = Md5Mesh {
        shader,
        vertices: std::mem::take(&mut vertices),
        indices: triangles.into_iter().flatten().collect(),
        weights,
    };
    let normals = compute_normals(&mesh, joints);
    Ok(Md5Mesh {
        vertices: normals,
        ..mesh
    })
}

/// Parse an MD5 mesh file (`parseMd5Mesh`).
pub fn parse_md5_mesh(text: &str, source: &str) -> Result<Md5MeshFile, BinaryError> {
    let mut tokens = ModelTokens::new(text, source);
    let command_line = md5_header(&mut tokens)?;
    tokens.expect("numJoints")?;
    let joint_count = tokens.integer(1, 256)? as usize;
    tokens.expect("numMeshes")?;
    let mesh_count = tokens.integer(1, 32)? as usize;
    tokens.expect("joints")?;
    tokens.expect("{")?;
    let mut joints = Vec::with_capacity(joint_count);
    let mut bind_pose = Vec::with_capacity(joint_count);
    for _ in 0..joint_count {
        // Mesh parent order is metadata; model-space bind joints need no hierarchy traversal.
        joints.push(Md5Joint {
            name: tokens.token()?,
            parent: tokens.integer(-1, joint_count as i32 - 1)?,
            scale_positions: false,
        });
        bind_pose.push(SkeletonJointPose {
            position: tokens.vector()?,
            orientation: md5_quaternion(tokens.vector()?),
            scale: 1.0,
        });
    }
    tokens.expect("}")?;
    let mut meshes = Vec::with_capacity(mesh_count);
    for _ in 0..mesh_count {
        meshes.push(read_mesh(&mut tokens, &bind_pose)?);
    }
    tokens.end()?;
    Ok(Md5MeshFile {
        source: source.to_string(),
        command_line,
        joints,
        bind_pose,
        meshes,
    })
}

fn vector_normal(value: Vec3) -> Vec3 {
    let (x, y, z) = (f64::from(value.x), f64::from(value.y), f64::from(value.z));
    let length = (x * x + y * y + z * z).sqrt();
    if length == 0.0 {
        return value;
    }
    vec3((x / length) as f32, (y / length) as f32, (z / length) as f32)
}

fn compute_normals(mesh: &Md5Mesh, bind_pose: &[SkeletonJointPose]) -> Vec<Md5Vertex> {
    let positions: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|vertex| {
            let mut position = vec3(0.0, 0.0, 0.0);
            for offset in 0..vertex.weights.count {
                let weight = at(&mesh.weights, vertex.weights.first as usize + offset as usize, "weight");
                let joint = at(bind_pose, weight.joint as usize, "joint");
                let rotated = rotate_quaternion(joint.orientation, weight.position);
                let world = add3(joint.position, rotated);
                let bias = f64::from(weight.bias);
                position = vec3(
                    (f64::from(position.x) + f64::from(world.x) * bias) as f32,
                    (f64::from(position.y) + f64::from(world.y) * bias) as f32,
                    (f64::from(position.z) + f64::from(world.z) * bias) as f32,
                );
            }
            position
        })
        .collect();
    let key = |point: Vec3| format!("{}|{}|{}", point.x, point.y, point.z);
    let mut normals: HashMap<String, Vec3> = HashMap::new();
    for base in (0..mesh.indices.len()).step_by(3) {
        let triangle = [mesh.indices[base], mesh.indices[base + 1], mesh.indices[base + 2]];
        let a = at(&positions, triangle[0] as usize, "position");
        let b = at(&positions, triangle[1] as usize, "position");
        let c = at(&positions, triangle[2] as usize, "position");
        let d1 = vector_normal(sub3(*c, *a));
        let d2 = vector_normal(sub3(*b, *a));
        let normal = vector_normal(vec3(
            (f64::from(d1.y) * f64::from(d2.z) - f64::from(d1.z) * f64::from(d2.y)) as f32,
            (f64::from(d1.z) * f64::from(d2.x) - f64::from(d1.x) * f64::from(d2.z)) as f32,
            (f64::from(d1.x) * f64::from(d2.y) - f64::from(d1.y) * f64::from(d2.x)) as f32,
        ));
        let dot =
            f64::from(d1.x) * f64::from(d2.x) + f64::from(d1.y) * f64::from(d2.y) + f64::from(d1.z) * f64::from(d2.z);
        let angle = dot.clamp(-1.0, 1.0).acos();
        let weighted = vec3(
            (f64::from(normal.x) * angle) as f32,
            (f64::from(normal.y) * angle) as f32,
            (f64::from(normal.z) * angle) as f32,
        );
        for point in [a, b, c] {
            let entry = normals.entry(key(*point)).or_insert(vec3(0.0, 0.0, 0.0));
            *entry = add3(*entry, weighted);
        }
    }
    for normal in normals.values_mut() {
        *normal = vector_normal(*normal);
    }
    mesh.vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            let world_normal = normals
                .get(&key(positions[index]))
                .copied()
                .unwrap_or(vec3(0.0, 0.0, 0.0));
            let mut normal = vec3(0.0, 0.0, 0.0);
            for offset in 0..vertex.weights.count {
                let weight = at(&mesh.weights, vertex.weights.first as usize + offset as usize, "weight");
                let joint = at(bind_pose, weight.joint as usize, "joint");
                let local = rotate_quaternion(conjugate_quaternion(joint.orientation), world_normal);
                let bias = f64::from(weight.bias);
                normal = vec3(
                    (f64::from(normal.x) + bias * f64::from(local.x)) as f32,
                    (f64::from(normal.y) + bias * f64::from(local.y)) as f32,
                    (f64::from(normal.z) + bias * f64::from(local.z)) as f32,
                );
            }
            Md5Vertex {
                normal,
                ..vertex.clone()
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

struct JsonParser {
    chars: Vec<char>,
    offset: usize,
}

impl JsonParser {
    fn parse(text: &str) -> Option<JsonValue> {
        let mut parser = Self {
            chars: text.chars().collect(),
            offset: 0,
        };
        let value = parser.value()?;
        parser.whitespace();
        if parser.offset != parser.chars.len() {
            None
        } else {
            Some(value)
        }
    }

    fn whitespace(&mut self) {
        while self.offset < self.chars.len() && matches!(self.chars[self.offset], ' ' | '\t' | '\n' | '\r') {
            self.offset += 1;
        }
    }

    fn literal(&mut self, text: &str) -> bool {
        let pattern: Vec<char> = text.chars().collect();
        if self.chars[self.offset..].starts_with(&pattern) {
            self.offset += pattern.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<JsonValue> {
        self.whitespace();
        let character = *self.chars.get(self.offset)?;
        match character {
            'n' => self.literal("null").then_some(JsonValue::Null),
            't' => self.literal("true").then_some(JsonValue::Bool(true)),
            'f' => self.literal("false").then_some(JsonValue::Bool(false)),
            '"' => self.string().map(JsonValue::String),
            '[' => self.array(),
            '{' => self.object(),
            _ => self.number().map(JsonValue::Number),
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.chars.get(self.offset) != Some(&'"') {
            return None;
        }
        self.offset += 1;
        let mut result = String::new();
        while self.offset < self.chars.len() {
            let character = self.chars[self.offset];
            self.offset += 1;
            match character {
                '"' => return Some(result),
                '\\' => {
                    let escape = *self.chars.get(self.offset)?;
                    self.offset += 1;
                    match escape {
                        '"' | '\\' | '/' => result.push(escape),
                        'b' => result.push('\u{0008}'),
                        'f' => result.push('\u{000c}'),
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        'u' => {
                            let digits: String = self.chars.get(self.offset..self.offset + 4)?.iter().collect();
                            let code = u32::from_str_radix(&digits, 16).ok()?;
                            result.push(char::from_u32(code)?);
                            self.offset += 4;
                        }
                        _ => return None,
                    }
                }
                _ => {
                    if (character as u32) < 0x20 {
                        return None;
                    }
                    result.push(character);
                }
            }
        }
        None
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.offset;
        if self.chars.get(self.offset) == Some(&'-') {
            self.offset += 1;
        }
        let digits = |parser: &mut Self| {
            let begin = parser.offset;
            while parser.offset < parser.chars.len() && parser.chars[parser.offset].is_ascii_digit() {
                parser.offset += 1;
            }
            parser.offset - begin
        };
        if self.chars.get(self.offset) == Some(&'0') {
            self.offset += 1;
        } else if digits(self) == 0 {
            return None;
        }
        if self.chars.get(self.offset) == Some(&'.') {
            self.offset += 1;
            if digits(self) == 0 {
                return None;
            }
        }
        if matches!(self.chars.get(self.offset), Some('e' | 'E')) {
            self.offset += 1;
            if matches!(self.chars.get(self.offset), Some('+' | '-')) {
                self.offset += 1;
            }
            if digits(self) == 0 {
                return None;
            }
        }
        if start == self.offset {
            return None;
        }
        self.chars[start..self.offset].iter().collect::<String>().parse().ok()
    }

    fn array(&mut self) -> Option<JsonValue> {
        self.offset += 1;
        let mut items = Vec::new();
        self.whitespace();
        if self.chars.get(self.offset) == Some(&']') {
            self.offset += 1;
            return Some(JsonValue::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            match self.chars.get(self.offset) {
                Some(',') => {
                    self.offset += 1;
                }
                Some(']') => {
                    self.offset += 1;
                    return Some(JsonValue::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn object(&mut self) -> Option<JsonValue> {
        self.offset += 1;
        let mut entries = Vec::new();
        self.whitespace();
        if self.chars.get(self.offset) == Some(&'}') {
            self.offset += 1;
            return Some(JsonValue::Object(entries));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            if self.chars.get(self.offset) != Some(&':') {
                return None;
            }
            self.offset += 1;
            entries.push((key, self.value()?));
            self.whitespace();
            match self.chars.get(self.offset) {
                Some(',') => {
                    self.offset += 1;
                }
                Some('}') => {
                    self.offset += 1;
                    return Some(JsonValue::Object(entries));
                }
                _ => return None,
            }
        }
    }
}

fn js_number(text: &str) -> f64 {
    let trimmed =
        text.trim_matches(|character: char| matches!(character, '\t' | '\n' | '\u{000b}' | '\u{000c}' | '\r' | ' '));
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse().unwrap_or(f64::NAN)
}

type ParsedScales = (Vec<Md5HierarchyJoint>, HashMap<(usize, usize), f32>, Vec<String>);

/// Optional scale failures are diagnostics, matching q2repro's fallback to
/// unit scales (`parseScales`).
fn parse_scales(source: Option<&Md5ScaleSource>, hierarchy: &[Md5HierarchyJoint], frame_count: usize) -> ParsedScales {
    let mut scales: HashMap<(usize, usize), f32> = HashMap::new();
    let mut scale_positions: HashSet<usize> = HashSet::new();
    let mut diagnostics = Vec::new();
    if let Some(source) = source {
        let root = JsonParser::parse(&source.text);
        let entries = match root {
            Some(JsonValue::Object(entries)) => Some(entries),
            _ => None,
        };
        match entries {
            None => diagnostics.push(format!("{}: Invalid JSON scale object", source.source)),
            Some(entries) => {
                for (name, entry) in &entries {
                    let JsonValue::Object(values) = entry else {
                        diagnostics.push(format!("{}: Invalid scale entry for {name}", source.source));
                        break;
                    };
                    let joint = hierarchy.iter().position(|joint| joint.name == *name);
                    let Some(joint) = joint else {
                        diagnostics.push(format!("{}: No such joint {name}", source.source));
                        continue;
                    };
                    for (key, value) in values {
                        if key == "scale_positions" {
                            if *value == JsonValue::Bool(true) {
                                scale_positions.insert(joint);
                            }
                        } else {
                            let frame = js_number(key);
                            let valid = !key.is_empty()
                                && frame.fract() == 0.0
                                && frame >= 0.0
                                && frame < frame_count as f64
                                && matches!(value, JsonValue::Number(_));
                            let rounded = match value {
                                JsonValue::Number(number) => *number as f32,
                                _ => f32::NAN,
                            };
                            if valid && rounded.is_finite() {
                                scales.insert((frame as usize, joint), rounded);
                            } else {
                                diagnostics.push(format!("{}: Invalid frame scale {name}/{key}", source.source));
                            }
                        }
                    }
                }
            }
        }
    }
    let hierarchy = hierarchy
        .iter()
        .enumerate()
        .map(|(index, joint)| Md5HierarchyJoint {
            name: joint.name.clone(),
            parent: joint.parent,
            flags: joint.flags,
            start_index: joint.start_index,
            scale_positions: scale_positions.contains(&index),
        })
        .collect();
    (hierarchy, scales, diagnostics)
}

/// Parse an MD5 animation file (`parseMd5Anim`).
pub fn parse_md5_anim(
    text: &str,
    source: &str,
    scale_source: Option<&Md5ScaleSource>,
) -> Result<Md5Animation, BinaryError> {
    let mut tokens = ModelTokens::new(text, source);
    let command_line = md5_header(&mut tokens)?;
    tokens.expect("numFrames")?;
    let frame_count = tokens.integer(1, 65535)? as usize;
    tokens.expect("numJoints")?;
    let joint_count = tokens.integer(1, 256)? as usize;
    tokens.expect("frameRate")?;
    let frame_rate = tokens.integer(1, 1000)?;
    tokens.expect("numAnimatedComponents")?;
    let component_count = tokens.integer(0, joint_count as i32 * 6)? as usize;
    tokens.expect("hierarchy")?;
    tokens.expect("{")?;
    let mut raw_hierarchy = Vec::with_capacity(joint_count);
    for index in 0..joint_count {
        let name = tokens.token()?;
        let parent = tokens.integer(-1, index as i32 - 1)?;
        let flags = tokens.integer(0, 63)? as u32;
        let start_index = tokens.integer(0, component_count as i32)? as usize;
        let mut animated = 0;
        for bit in 0..6 {
            if flags & (1 << bit) != 0 {
                animated += 1;
            }
        }
        if start_index + animated > component_count {
            return tokens.fail("animated joint components exceed frame".to_string());
        }
        raw_hierarchy.push(Md5HierarchyJoint {
            name,
            parent,
            flags,
            start_index,
            scale_positions: false,
        });
    }
    tokens.expect("}")?;
    tokens.expect("bounds")?;
    tokens.expect("{")?;
    let mut bounds = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        bounds.push(Bounds {
            min: tokens.vector()?,
            max: tokens.vector()?,
        });
    }
    tokens.expect("}")?;
    tokens.expect("baseframe")?;
    tokens.expect("{")?;
    let mut base_frame = Vec::with_capacity(joint_count);
    for _ in 0..joint_count {
        base_frame.push(SkeletonJointPose {
            position: tokens.vector()?,
            orientation: md5_quaternion(tokens.vector()?),
            scale: 1.0,
        });
    }
    tokens.expect("}")?;
    let (hierarchy, scales, diagnostics) = parse_scales(scale_source, &raw_hierarchy, frame_count);
    let raw_frames = indexed_records(&mut tokens, frame_count, "frame", |tokens| {
        tokens.expect("{")?;
        let mut frame = Vec::with_capacity(component_count);
        for _ in 0..component_count {
            frame.push(tokens.float()?);
        }
        tokens.expect("}")?;
        Ok(frame)
    })?;
    tokens.end()?;
    let mut frames = Vec::with_capacity(frame_count);
    for (frame, components) in raw_frames.iter().enumerate() {
        let mut local_joints = Vec::with_capacity(joint_count);
        let mut joints = Vec::with_capacity(joint_count);
        for index in 0..joint_count {
            let info = at(&hierarchy, index, "hierarchy joint");
            let base = at(&base_frame, index, "base joint");
            let mut component = info.start_index;
            let mut animated = |bit: u32, fallback: f32| {
                if info.flags & bit == 0 {
                    fallback
                } else {
                    let value = components[component];
                    component += 1;
                    value
                }
            };
            let position = vec3(
                animated(1, base.position.x),
                animated(2, base.position.y),
                animated(4, base.position.z),
            );
            let orientation = md5_quaternion(vec3(
                animated(8, base.orientation.x),
                animated(16, base.orientation.y),
                animated(32, base.orientation.z),
            ));
            let scale = scales.get(&(frame, index)).copied().unwrap_or(1.0);
            local_joints.push(SkeletonJointPose {
                position,
                orientation,
                scale,
            });
            let scaled_position = if info.scale_positions {
                scale_position(position, scale)
            } else {
                position
            };
            if info.parent < 0 {
                joints.push(SkeletonJointPose {
                    position: scaled_position,
                    orientation,
                    scale,
                });
            } else {
                let parent = at(&joints, info.parent as usize, "parent joint");
                joints.push(SkeletonJointPose {
                    position: add3(parent.position, rotate_quaternion(parent.orientation, scaled_position)),
                    orientation: normalize_quaternion(multiply_quaternion(parent.orientation, orientation)),
                    scale,
                });
            }
        }
        frames.push(Md5AnimationFrame {
            bounds: bounds[frame],
            components: components.clone(),
            local_joints,
            joints,
        });
    }
    Ok(Md5Animation {
        source: source.to_string(),
        command_line,
        frame_rate,
        hierarchy,
        base_frame,
        frames,
        scale_source: scale_source.map(|source| source.source.clone()),
        diagnostics,
    })
}

fn scale_position(position: Vec3, scale: f32) -> Vec3 {
    vec3(position.x * scale, position.y * scale, position.z * scale)
}

/// Join a mesh file with its animation (`createMd5Model`).
///
/// # Panics
///
/// Panics when the joint counts differ.
#[must_use]
pub fn create_md5_model(mesh: Md5MeshFile, animation: Md5Animation, skin_selection: SkinSelection) -> DecodedMd5Model {
    if mesh.joints.len() != animation.hierarchy.len() {
        panic!("{}: mesh and animation joint counts differ", mesh.source);
    }
    DecodedMd5Model {
        mesh_source: mesh.source.clone(),
        mesh_command_line: mesh.command_line.clone(),
        mesh_joints: mesh.joints.clone(),
        bind_pose: mesh.bind_pose.clone(),
        frame_rate: animation.frame_rate,
        joints: animation.hierarchy.clone(),
        frames: animation.frames.clone(),
        meshes: mesh.meshes.clone(),
        animation,
        skin_selection,
    }
}

/// Interpolate joint poses (`sampleMd5Pose`).
///
/// Positions and orientations blend; Q2 uses the new frame's scale
/// directly.
///
/// # Panics
///
/// Panics on invalid frame selections.
#[must_use]
pub fn sample_md5_pose(
    frames: &[Md5AnimationFrame],
    frame: i32,
    previous_frame: i32,
    back_lerp: f32,
) -> Vec<SkeletonJointPose> {
    if frame < 0 || previous_frame < 0 || !back_lerp.is_finite() {
        panic!("Invalid MD5 frame selection");
    }
    let current = &at(frames, frame as usize % frames.len(), "MD5 frame").joints;
    if back_lerp == 0.0 || frame == previous_frame {
        return current.to_vec();
    }
    let previous = &at(frames, previous_frame as usize % frames.len(), "MD5 previous frame").joints;
    let back = f64::from(back_lerp);
    let front = 1.0 - back;
    current
        .iter()
        .enumerate()
        .map(|(index, joint)| {
            let old = at(previous, index, "previous joint");
            SkeletonJointPose {
                position: vec3(
                    (f64::from(old.position.x) * back + f64::from(joint.position.x) * front) as f32,
                    (f64::from(old.position.y) * back + f64::from(joint.position.y) * front) as f32,
                    (f64::from(old.position.z) * back + f64::from(joint.position.z) * front) as f32,
                ),
                orientation: slerp_quaternion(old.orientation, joint.orientation, back_lerp),
                scale: joint.scale,
            }
        })
        .collect()
}

/// Skin one MD5 mesh (`skinMd5Mesh`).
#[must_use]
pub fn skin_md5_mesh(mesh: &Md5Mesh, joints: &[SkeletonJointPose]) -> Vec<Md5SkinnedVertex> {
    let mut rotations: Vec<Option<crate::quaternion::QuaternionRotationRows>> = Vec::new();
    mesh.vertices
        .iter()
        .map(|vertex| {
            let mut position = vec3(0.0, 0.0, 0.0);
            let mut normal = vec3(0.0, 0.0, 0.0);
            for offset in 0..vertex.weights.count {
                let weight = at(&mesh.weights, vertex.weights.first as usize + offset as usize, "weight");
                let joint = at(joints, weight.joint as usize, "joint");
                while rotations.len() <= weight.joint as usize {
                    rotations.push(None);
                }
                let rows = *rotations[weight.joint as usize]
                    .get_or_insert_with(|| quaternion_rotation_rows(joint.orientation));
                let rotated = rotate_quaternion_rows(&rows, weight.position);
                let scale = f64::from(joint.scale);
                let point = vec3(
                    (f64::from(joint.position.x) + scale * f64::from(rotated.x)) as f32,
                    (f64::from(joint.position.y) + scale * f64::from(rotated.y)) as f32,
                    (f64::from(joint.position.z) + scale * f64::from(rotated.z)) as f32,
                );
                let direction = rotate_quaternion_rows(&rows, vertex.normal);
                let bias = f64::from(weight.bias);
                position = vec3(
                    (f64::from(position.x) + bias * f64::from(point.x)) as f32,
                    (f64::from(position.y) + bias * f64::from(point.y)) as f32,
                    (f64::from(position.z) + bias * f64::from(point.z)) as f32,
                );
                normal = vec3(
                    (f64::from(normal.x) + bias * f64::from(direction.x)) as f32,
                    (f64::from(normal.y) + bias * f64::from(direction.y)) as f32,
                    (f64::from(normal.z) + bias * f64::from(direction.z)) as f32,
                );
            }
            Md5SkinnedVertex { position, normal }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESH: &str = r#"MD5Version 10
commandline "test"
numJoints 1
numMeshes 1
joints {
  "origin" -1 ( 0 0 0 ) ( 0 0 0 )
}
mesh {
  shader "shader0"
  numverts 3
  vert 0 ( 0 0 ) 0 1
  vert 1 ( 1 0 ) 1 1
  vert 2 ( 0 1 ) 2 1
  numtris 1
  tri 0 0 1 2
  numweights 3
  weight 0 0 1.0 ( 0 0 0 )
  weight 1 0 1.0 ( 1 0 0 )
  weight 2 0 1.0 ( 0 1 0 )
}
"#;

    const ANIM: &str = r#"MD5Version 10
commandline "test"
numFrames 2
numJoints 1
frameRate 24
numAnimatedComponents 1
hierarchy {
  "origin" -1 1 0
}
bounds {
  ( -1 -1 -1 ) ( 1 1 1 )
  ( -1 -1 -1 ) ( 1 1 1 )
}
baseframe {
  ( 0 0 0 ) ( 0 0 0 )
}
frame 0 {
  0.0
}
frame 1 {
  2.0
}
"#;

    #[test]
    fn md5_round_trip() {
        let mesh = parse_md5_mesh(MESH, "<test>").unwrap();
        assert_eq!(mesh.command_line, "test");
        assert_eq!(mesh.joints.len(), 1);
        assert_eq!(mesh.bind_pose[0].position, vec3(0.0, 0.0, 0.0));
        assert_eq!(mesh.meshes[0].shader, "shader0");
        assert_eq!(mesh.meshes[0].indices, vec![0, 1, 2]);
        // The triangle faces -Z under the donor winding.
        for vertex in &mesh.meshes[0].vertices {
            assert_eq!(vertex.normal, vec3(0.0, 0.0, -1.0));
        }

        let animation = parse_md5_anim(ANIM, "<test>", None).unwrap();
        assert_eq!(animation.frame_rate, 24);
        assert_eq!(animation.frames.len(), 2);
        assert_eq!(animation.frames[0].joints[0].position, vec3(0.0, 0.0, 0.0));
        assert_eq!(animation.frames[1].joints[0].position, vec3(2.0, 0.0, 0.0));
        assert!(animation.diagnostics.is_empty());

        let model = create_md5_model(mesh, animation, SkinSelection::MeshShaders);
        assert_eq!(model.frames.len(), 2);
        let pose = sample_md5_pose(&model.frames, 1, 0, 0.5);
        assert_eq!(pose[0].position, vec3(1.0, 0.0, 0.0));
        let skinned = skin_md5_mesh(&model.meshes[0], &model.bind_pose);
        assert_eq!(skinned[1].position, vec3(1.0, 0.0, 0.0));
        assert_eq!(skinned[2].position, vec3(0.0, 1.0, 0.0));
    }

    #[test]
    fn md5_scales() {
        let scales = Md5ScaleSource {
            source: "<scales>".to_string(),
            text: r#"{"origin": {"scale_positions": true, "1": 2.0}}"#.to_string(),
        };
        let animation = parse_md5_anim(ANIM, "<test>", Some(&scales)).unwrap();
        assert!(animation.diagnostics.is_empty());
        assert!(animation.hierarchy[0].scale_positions);
        assert_eq!(animation.frames[1].joints[0].scale, 2.0);
        assert_eq!(animation.frames[1].joints[0].position, vec3(4.0, 0.0, 0.0));
        assert_eq!(animation.scale_source.as_deref(), Some("<scales>"));

        let bad = Md5ScaleSource {
            source: "<scales>".to_string(),
            text: "nope".to_string(),
        };
        let animation = parse_md5_anim(ANIM, "<test>", Some(&bad)).unwrap();
        assert_eq!(
            animation.diagnostics,
            vec!["<scales>: Invalid JSON scale object".to_string()]
        );
        let bad = Md5ScaleSource {
            source: "<scales>".to_string(),
            text: r#"{"origin": 5, "nope": {"0": 1.0}, "origin": {"9": 1.0}}"#.to_string(),
        };
        let animation = parse_md5_anim(ANIM, "<test>", Some(&bad)).unwrap();
        // A non-object entry stops the whole scale pass.
        assert_eq!(
            animation.diagnostics,
            vec!["<scales>: Invalid scale entry for origin".to_string()]
        );
    }

    #[test]
    fn md5_rejects_bad_input() {
        let error = parse_md5_mesh("MD5Version 9", "<test>").unwrap_err();
        assert!(error.message.contains("expected"), "{}", error.message);
        let error = parse_md5_mesh(&MESH[..MESH.len() - 2], "<test>").unwrap_err();
        assert!(error.message.contains("unexpected end"), "{}", error.message);
        let error = parse_md5_mesh(&MESH.replace("tri 0 0 1 2", "tri 0 0 1 9"), "<test>").unwrap_err();
        assert!(error.message.contains("outside 0..2"), "{}", error.message);
        let error = parse_md5_mesh(&MESH.replace("0 1.0 ( 0 0 0 )", "0 1.5 ( 0 0 0 )"), "<test>").unwrap_err();
        assert!(error.message.contains("weight bias"), "{}", error.message);
        let error = parse_md5_mesh(&format!("{MESH}trailing"), "<test>").unwrap_err();
        assert!(error.message.contains("unexpected trailing token"), "{}", error.message);
        let error = parse_md5_anim(&ANIM.replace("frameRate 24", "frameRate 0"), "<test>", None).unwrap_err();
        assert!(error.message.contains("outside 1..1000"), "{}", error.message);
    }
}
