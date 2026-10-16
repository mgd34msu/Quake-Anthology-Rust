use super::*;
use qa_core::math::{multiply_quaternion, normalize_quaternion};
use qa_core::text::{atof_prefix_bytes, atoi_prefix};
use std::mem::size_of;

#[derive(Clone, Copy, Debug)]
pub struct Md5HierarchyJoint<'a> {
    pub name: &'a [u8],
    pub parent: Option<u32>,
    pub flags: u8,
    pub start_component: u32,
    pub scale_positions: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Md5AnimationFrame {
    pub bounds: Bounds,
    pub joints: IndexSpan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Md5ScaleDiagnostic<'a> {
    InvalidData,
    UnknownJoint(&'a [u8]),
    InvalidFrame(&'a [u8]),
    InvalidScale(&'a [u8]),
}

#[derive(Debug)]
pub struct Md5Animation<'a> {
    pub command_line: &'a [u8],
    pub frame_rate: u32,
    pub hierarchy: Vec<Md5HierarchyJoint<'a>>,
    /// Authored joint-local poses, before animated components and scale edits.
    pub base_frame: Vec<JointPose>,
    pub frames: Vec<Md5AnimationFrame>,
    /// Frame-major model-space poses; each frame borrows its numeric span.
    pub poses: Vec<JointPose>,
    pub scale_diagnostics: Vec<Md5ScaleDiagnostic<'a>>,
}

impl<'a> Md5Animation<'a> {
    pub fn parse(bytes: &'a [u8], scales: Option<&'a [u8]>) -> Result<Self, FormatError> {
        let mut t = Tokens::new(bytes);
        let command_line = header(&mut t)?;
        let frame_count = count(&mut t, b"numFrames", i32::MAX)?;
        let joint_count = count(&mut t, b"numJoints", 256)?;
        if frame_count == 0 || joint_count == 0 {
            return Err(FormatError::InvalidRange);
        }
        t.require(b"frameRate")?;
        let frame_rate = t.integer(1, i32::MAX)? as u32;
        let component_count = count(&mut t, b"numAnimatedComponents", joint_count as i32 * 6)?;
        let pose_count = frame_count
            .checked_mul(joint_count)
            .ok_or(FormatError::InvalidRange)?;
        let decoded_bytes = [
            (pose_count, size_of::<JointPose>()),
            (
                frame_count,
                size_of::<Md5AnimationFrame>() + size_of::<bool>(),
            ),
            (
                joint_count,
                size_of::<JointPose>() + size_of::<Md5HierarchyJoint<'_>>(),
            ),
            (component_count, size_of::<f32>()),
        ]
        .into_iter()
        .try_fold(0usize, |total, (count, size)| {
            total.checked_add(count.checked_mul(size)?)
        })
        .filter(|&size| size <= MAX_DECODED_BYTES)
        .ok_or(FormatError::InvalidRange)?;

        let mut animation = Self {
            command_line,
            frame_rate,
            hierarchy: Vec::new(),
            base_frame: Vec::new(),
            frames: Vec::new(),
            poses: Vec::new(),
            scale_diagnostics: Vec::new(),
        };
        reserve(&mut animation.hierarchy, joint_count)?;
        reserve(&mut animation.base_frame, joint_count)?;
        reserve(&mut animation.frames, frame_count)?;
        reserve(&mut animation.poses, pose_count)?;
        animation.poses.resize(pose_count, JointPose::default());

        t.require(b"hierarchy")?;
        t.require(b"{")?;
        for index in 0..joint_count {
            let name = t.value()?;
            let parent = t.integer(-1, index as i32 - 1)?;
            let flags = t.integer(0, 63)? as u8;
            let start_component = t.integer(0, component_count as i32)? as u32;
            if start_component as usize + flags.count_ones() as usize > component_count {
                return Err(FormatError::InvalidRange);
            }
            animation.hierarchy.push(Md5HierarchyJoint {
                name,
                parent: (parent >= 0).then_some(parent as u32),
                flags,
                start_component,
                scale_positions: false,
            });
        }
        t.require(b"}")?;
        t.require(b"bounds")?;
        t.require(b"{")?;
        for frame in 0..frame_count {
            let bounds = Bounds {
                mins: vector(&mut t)?,
                maxs: vector(&mut t)?,
            };
            if !bounds.is_valid() {
                return Err(FormatError::InvalidValue);
            }
            animation.frames.push(Md5AnimationFrame {
                bounds,
                joints: IndexSpan {
                    first: (frame * joint_count) as u32,
                    count: joint_count as u32,
                },
            });
        }
        t.require(b"}")?;
        t.require(b"baseframe")?;
        t.require(b"{")?;
        for _ in 0..joint_count {
            let position = vector(&mut t)?;
            let orientation =
                quaternion_from_xyz(vector(&mut t)?).ok_or(FormatError::InvalidValue)?;
            animation.base_frame.push(JointPose {
                position,
                orientation,
                ..JointPose::default()
            });
        }
        t.require(b"}")?;
        if let Some(scales) = scales {
            animation.load_scales(scales, MAX_DECODED_BYTES - decoded_bytes)?;
        }

        let mut components = Vec::new();
        reserve(&mut components, component_count)?;
        components.resize(component_count, 0.0);
        let mut seen = Vec::new();
        reserve(&mut seen, frame_count)?;
        seen.resize(frame_count, false);
        for _ in 0..frame_count {
            let frame = record(&mut t, b"frame", &mut seen)?;
            t.require(b"{")?;
            for component in &mut components {
                *component = t.scalar()?;
            }
            t.require(b"}")?;
            let poses = &mut animation.poses[animation.frames[frame].joints.range()];
            for index in 0..joint_count {
                let joint = animation.hierarchy[index];
                let base = animation.base_frame[index];
                let mut values = [
                    base.position.0[0],
                    base.position.0[1],
                    base.position.0[2],
                    base.orientation[0],
                    base.orientation[1],
                    base.orientation[2],
                ];
                let mut component = joint.start_component as usize;
                for (flag, value) in values.iter_mut().enumerate() {
                    if joint.flags & (1 << flag) != 0 {
                        *value = components[component];
                        component += 1;
                    }
                }
                let mut position = Vec3([values[0], values[1], values[2]]);
                let mut orientation = quaternion_from_xyz(Vec3([values[3], values[4], values[5]]))
                    .ok_or(FormatError::InvalidValue)?;
                let scale = poses[index].scale;
                if joint.scale_positions {
                    // MD5 scales are uniform. Keep native scalar multiplication.
                    position = position * scale.0[0];
                }
                if let Some(parent) = joint.parent {
                    let parent = poses[parent as usize];
                    position = parent.position + rotate_quaternion(parent.orientation, position);
                    orientation = multiply_quaternion(parent.orientation, orientation);
                    for value in orientation {
                        finite(value)?;
                    }
                    orientation = normalize_quaternion(orientation);
                }
                finite_vec(position)?;
                for value in orientation {
                    finite(value)?;
                }
                poses[index] = JointPose {
                    position,
                    orientation,
                    scale,
                };
            }
        }
        if t.next()?.is_some() {
            return Err(FormatError::InvalidValue);
        }
        Ok(animation)
    }

    pub fn frame(&self, frame: u32) -> Option<&[JointPose]> {
        self.poses
            .get(self.frames.get(frame as usize)?.joints.range())
    }

    pub fn validate_mesh(&self, mesh: &Model<'_>) -> Result<(), FormatError> {
        if mesh.format != ModelFormat::Md5Mesh || mesh.bones.len() != self.hierarchy.len() {
            return Err(FormatError::InvalidValue);
        }
        for (joint, bone) in self.hierarchy.iter().zip(&mesh.bones) {
            if joint.name != bone.name || joint.parent != bone.parent {
                return Err(FormatError::InvalidValue);
            }
        }
        Ok(())
    }

    fn diagnostic(&mut self, diagnostic: Md5ScaleDiagnostic<'a>) -> Result<(), FormatError> {
        self.scale_diagnostics
            .try_reserve(1)
            .map_err(|_| FormatError::InvalidRange)?;
        self.scale_diagnostics.push(diagnostic);
        Ok(())
    }

    fn load_scales(&mut self, bytes: &'a [u8], budget: usize) -> Result<(), FormatError> {
        // Native jsmn validates the entire JSON document before applying entries.
        // Type errors encountered during the subsequent walk preserve its prefix.
        let nodes = match json_nodes(bytes, budget) {
            Ok(nodes) => nodes,
            Err(_) => return self.diagnostic(Md5ScaleDiagnostic::InvalidData),
        };
        let Some(root) = nodes.first() else {
            return Ok(());
        };
        if root.kind != JsonKind::Object {
            return self.diagnostic(Md5ScaleDiagnostic::InvalidData);
        }
        let mut pair = 1;
        while pair < root.end {
            let name = nodes[pair];
            let values = nodes[pair + 1];
            if values.kind != JsonKind::Object {
                return self.diagnostic(Md5ScaleDiagnostic::InvalidData);
            }
            let joint = self
                .hierarchy
                .iter()
                .position(|joint| joint.name == name.value);
            if joint.is_none() {
                self.diagnostic(Md5ScaleDiagnostic::UnknownJoint(name.value))?;
            }
            let mut entry = pair + 2;
            while entry < values.end {
                let key = nodes[entry].value;
                let value = nodes[entry + 1];
                if value.kind != JsonKind::Primitive {
                    return self.diagnostic(Md5ScaleDiagnostic::InvalidData);
                }
                if let Some(joint) = joint {
                    if key == b"scale_positions" {
                        self.hierarchy[joint].scale_positions = value.value.first() == Some(&b't');
                    } else {
                        let frame = atoi_prefix(key) as u32;
                        if let Some(frame) = self.frames.get(frame as usize) {
                            let scale = atof_prefix_bytes(value.value).0 as f32;
                            if scale.is_finite() {
                                self.poses[frame.joints.first as usize + joint].scale =
                                    Vec3([scale; 3]);
                            } else {
                                self.diagnostic(Md5ScaleDiagnostic::InvalidScale(value.value))?;
                            }
                        } else {
                            self.diagnostic(Md5ScaleDiagnostic::InvalidFrame(key))?;
                        }
                    }
                }
                entry = value.end;
            }
            pair = values.end;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonKind {
    Object,
    Array,
    String,
    Primitive,
}

#[derive(Clone, Copy)]
struct JsonNode<'a> {
    value: &'a [u8],
    kind: JsonKind,
    end: usize,
}

#[derive(Clone, Copy)]
enum JsonState {
    ObjectFirst,
    ObjectKey,
    ObjectColon,
    ObjectValue,
    ObjectComma,
    ArrayFirst,
    ArrayValue,
    ArrayComma,
}

fn json_string(bytes: &[u8]) -> Result<(), FormatError> {
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte < 32 {
            return Err(FormatError::InvalidValue);
        }
        if byte == b'\\' {
            at += 1;
            match bytes.get(at) {
                Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => (),
                Some(b'u') => {
                    let digits = bytes.get(at + 1..at + 5).ok_or(FormatError::Truncated)?;
                    if !digits.iter().all(u8::is_ascii_hexdigit) {
                        return Err(FormatError::InvalidValue);
                    }
                    at += 4;
                }
                _ => return Err(FormatError::InvalidValue),
            }
        }
        at += 1;
    }
    Ok(())
}

fn json_push<'a>(
    t: &Tokens<'a>,
    value: &'a [u8],
    nodes: &mut Vec<JsonNode<'a>>,
    stack: &mut Vec<(usize, JsonState)>,
    budget: usize,
) -> Result<(), FormatError> {
    let (kind, state) = if t.was_quoted() {
        json_string(value)?;
        (JsonKind::String, None)
    } else {
        match value {
            b"{" => (JsonKind::Object, Some(JsonState::ObjectFirst)),
            b"[" => (JsonKind::Array, Some(JsonState::ArrayFirst)),
            b"}" | b"]" | b":" | b"," => return Err(FormatError::InvalidValue),
            _ => {
                if value.is_empty()
                    || value
                        .iter()
                        .any(|&byte| !(32..127).contains(&byte) || matches!(byte, b'"' | b'\\'))
                {
                    return Err(FormatError::InvalidValue);
                }
                (JsonKind::Primitive, None)
            }
        }
    };
    let index = nodes.len();
    let storage = (index + 1)
        .checked_mul(size_of::<JsonNode<'_>>() + size_of::<Md5ScaleDiagnostic<'_>>())
        .and_then(|bytes| {
            bytes.checked_add(
                (stack.len() + usize::from(state.is_some())) * size_of::<(usize, JsonState)>(),
            )
        })
        .ok_or(FormatError::InvalidRange)?;
    if storage > budget {
        return Err(FormatError::InvalidRange);
    }
    nodes
        .try_reserve(1)
        .map_err(|_| FormatError::InvalidRange)?;
    nodes.push(JsonNode {
        value,
        kind,
        end: index + 1,
    });
    if let Some(state) = state {
        stack
            .try_reserve(1)
            .map_err(|_| FormatError::InvalidRange)?;
        stack.push((index, state));
    }
    Ok(())
}

fn json_nodes(bytes: &[u8], budget: usize) -> Result<Vec<JsonNode<'_>>, FormatError> {
    let mut t = Tokens::json(bytes);
    let mut nodes = Vec::new();
    let mut stack = Vec::new();
    let Some(value) = t.next()? else {
        return Ok(nodes);
    };
    json_push(&t, value, &mut nodes, &mut stack, budget)?;
    while let Some(&(index, state)) = stack.last() {
        let top = stack.len() - 1;
        let value = t.value()?;
        let close = match state {
            JsonState::ObjectFirst if !t.was_quoted() && value == b"}" => true,
            JsonState::ArrayFirst if !t.was_quoted() && value == b"]" => true,
            JsonState::ObjectComma | JsonState::ArrayComma => {
                let end = if matches!(state, JsonState::ObjectComma) {
                    b"}"
                } else {
                    b"]"
                };
                if !t.was_quoted() && value == end {
                    true
                } else {
                    if t.was_quoted() || value != b"," {
                        return Err(FormatError::InvalidValue);
                    }
                    stack[top].1 = if matches!(state, JsonState::ObjectComma) {
                        JsonState::ObjectKey
                    } else {
                        JsonState::ArrayValue
                    };
                    continue;
                }
            }
            JsonState::ObjectFirst | JsonState::ObjectKey => {
                if !t.was_quoted() {
                    return Err(FormatError::InvalidValue);
                }
                stack[top].1 = JsonState::ObjectColon;
                json_push(&t, value, &mut nodes, &mut stack, budget)?;
                continue;
            }
            JsonState::ObjectColon => {
                if t.was_quoted() || value != b":" {
                    return Err(FormatError::InvalidValue);
                }
                stack[top].1 = JsonState::ObjectValue;
                continue;
            }
            JsonState::ObjectValue | JsonState::ArrayFirst | JsonState::ArrayValue => {
                stack[top].1 = if matches!(state, JsonState::ObjectValue) {
                    JsonState::ObjectComma
                } else {
                    JsonState::ArrayComma
                };
                json_push(&t, value, &mut nodes, &mut stack, budget)?;
                continue;
            }
        };
        if close {
            nodes[index].end = nodes.len();
            stack.pop();
        }
    }
    if t.next()?.is_some() {
        return Err(FormatError::InvalidValue);
    }
    Ok(nodes)
}
