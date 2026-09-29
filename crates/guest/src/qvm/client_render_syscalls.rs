//! Renderer resource, scene, and 2D traps for cgame and ui modules.
//!
//! Provenance: `src/compat/qvm/client-render-syscalls.ts` (renderer
//! resource and picture traps from Quake III Arena `cl_ui.c`/`cl_cgame.c`).
//! The [`RefEntity`], [`Refdef`], [`PolyVertex`], and [`Orientation`] record
//! layouts are local mirrors of `render-record.ts` (owned by another
//! worker); [`RenderResourceHost`] and [`DrawHost`] mirror the
//! `Q3RendererResources`/`Draw2D` surfaces the donor consumes. Guest model,
//! skin, and shader handles pass through unresolved for the host, matching
//! the donor's handle-resolution order at the scene-publication boundary.

use qa_core::math::Vec3;

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use crate::error::GuestError;

/// Byte length of `refEntity_t`.
pub const QVM_REF_ENTITY_BYTES: usize = 140;
/// Byte length of `refdef_t`.
pub const QVM_REFDEF_BYTES: usize = 368;
/// Byte length of `polyVert_t`.
pub const QVM_POLY_VERTEX_BYTES: usize = 24;
/// Byte length of `orientation_t`.
pub const QVM_ORIENTATION_BYTES: usize = 48;

/// Ref-entity kind by `reType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefEntityKind {
    /// Model entity.
    Model,
    /// Poly entity.
    Poly,
    /// Sprite entity.
    Sprite,
    /// Beam entity.
    Beam,
    /// Rail-core entity.
    RailCore,
    /// Rail-rings entity.
    RailRings,
    /// Lightning entity.
    Lightning,
    /// Portal-surface entity.
    PortalSurface,
}

/// Decoded `refEntity_t` with unresolved guest handles.
#[derive(Debug, Clone, PartialEq)]
pub struct RefEntity {
    /// Entity kind.
    pub kind: RefEntityKind,
    /// Render flags.
    pub render_flags: i32,
    /// Model handle.
    pub model: i32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Axis.
    pub axis: [Vec3; 3],
    /// Non-normalized axes flag.
    pub non_normalized_axes: bool,
    /// Origin.
    pub origin: Vec3,
    /// Frame.
    pub frame: i32,
    /// Old origin.
    pub old_origin: Vec3,
    /// Old frame.
    pub old_frame: i32,
    /// Back-lerp fraction.
    pub back_lerp: f32,
    /// Skin number.
    pub skin_num: i32,
    /// Custom skin handle.
    pub custom_skin: i32,
    /// Custom shader handle.
    pub custom_shader: i32,
    /// Shader RGBA bytes.
    pub shader_rgba: [u8; 4],
    /// Shader texture coordinates.
    pub shader_tex_coord: [f32; 2],
    /// Shader time.
    pub shader_time: f32,
    /// Radius.
    pub radius: f32,
    /// Rotation.
    pub rotation: f32,
}

/// Decoded polygon vertex.
#[derive(Debug, Clone, PartialEq)]
pub struct PolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinates.
    pub tex_coord: [f32; 2],
    /// Color bytes.
    pub color: [u8; 4],
}

/// Decoded `refdef_t`.
#[derive(Debug, Clone, PartialEq)]
pub struct Refdef {
    /// Viewport x.
    pub x: i32,
    /// Viewport y.
    pub y: i32,
    /// Viewport width.
    pub width: i32,
    /// Viewport height.
    pub height: i32,
    /// Horizontal field of view.
    pub fov_x: f32,
    /// Vertical field of view.
    pub fov_y: f32,
    /// View origin.
    pub view_origin: Vec3,
    /// View axis.
    pub view_axis: [Vec3; 3],
    /// Time.
    pub time: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Area mask.
    pub area_mask: [u8; 32],
    /// Render text rows (full 32-byte rows, NUL-inclusive).
    pub text: [[u8; 32]; 8],
}

/// Lerped tag orientation.
#[derive(Debug, Clone, PartialEq)]
pub struct Orientation {
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axes: [Vec3; 3],
}

/// Float 2D rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatRect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Texture-coordinate rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    /// Left.
    pub s: f32,
    /// Top.
    pub t: f32,
    /// Right.
    pub s2: f32,
    /// Bottom.
    pub t2: f32,
}

/// Dynamic light.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Additive blend.
    pub additive: bool,
}

/// Host renderer-resource surface used by the traps.
pub trait RenderResourceHost {
    /// Register a model, returning its handle.
    fn register_model(&mut self, name: &str) -> i32;
    /// Register a skin, returning its handle.
    fn register_skin(&mut self, name: &str) -> i32;
    /// Register a shader, returning its handle.
    fn register_shader(&mut self, name: &str) -> i32;
    /// Register a no-mip shader, returning its handle.
    fn register_shader_nomip(&mut self, name: &str) -> i32;
    /// Model bounds by handle.
    fn model_bounds(&mut self, handle: i32) -> (Vec3, Vec3);
    /// Lerped tag by model handle and tag name.
    ///
    /// The name resolver reads guest memory lazily: like `R_LerpTag`, hosts
    /// without tag storage report no tag without consuming the name.
    fn lerp_tag(
        &mut self,
        handle: i32,
        name: &dyn Fn() -> Result<String, GuestError>,
        start: i32,
        end: i32,
        fraction: f32,
    ) -> Result<Option<Orientation>, GuestError>;
    /// Remap a shader.
    fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str);
    /// Load the world map.
    fn load_world(&mut self, name: &str);
    /// Next entity token plus whether tokens continue.
    fn entity_token(&mut self) -> (String, bool);
    /// PVS visibility between two points.
    fn in_pvs(&mut self, first: Vec3, second: Vec3) -> bool;
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Add a reference entity to the scene.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Add a polygon to the scene.
    fn add_poly(&mut self, shader: i32, vertices: Vec<PolyVertex>);
    /// Add a light to the scene.
    fn add_light(&mut self, light: GuestLight);
    /// Render the scene.
    fn render_scene(&mut self, refdef: Refdef);
}

/// Host 2D-draw surface used by the traps.
pub trait DrawHost {
    /// Set the draw color (`None` clears it).
    fn set_color(&mut self, color: Option<[f32; 4]>);
    /// Stretch a shader picture.
    fn stretch_pixels(&mut self, rect: FloatRect, uv: UvRect, shader: i32);
}

/// Read a `refEntity_t` from guest memory.
pub fn read_ref_entity(memory: &SyscallMemory, word: i32) -> Result<RefEntity, GuestError> {
    let range = memory.span(word, QVM_REF_ENTITY_BYTES, 0)?;
    let kind = match memory.read_i32(range.start)? {
        0 => RefEntityKind::Model,
        1 => RefEntityKind::Poly,
        2 => RefEntityKind::Sprite,
        3 => RefEntityKind::Beam,
        4 => RefEntityKind::RailCore,
        5 => RefEntityKind::RailRings,
        6 => RefEntityKind::Lightning,
        7 => RefEntityKind::PortalSurface,
        other => return Err(GuestError::abi(format!("QVM refEntity_t unsupported entity type {other}"))),
    };
    let color = |offset: usize| -> Result<[u8; 4], GuestError> {
        Ok([memory.get(range.start + offset)?, memory.get(range.start + offset + 1)?, memory.get(range.start + offset + 2)?, memory.get(range.start + offset + 3)?])
    };
    Ok(RefEntity {
        kind,
        render_flags: memory.read_i32(range.start + 4)?,
        model: memory.read_i32(range.start + 8)?,
        lighting_origin: memory.read_vec3(range.start + 12)?,
        shadow_plane: memory.read_f32(range.start + 24)?,
        axis: [
            memory.read_vec3(range.start + 28)?,
            memory.read_vec3(range.start + 40)?,
            memory.read_vec3(range.start + 52)?,
        ],
        non_normalized_axes: memory.read_i32(range.start + 64)? != 0,
        origin: memory.read_vec3(range.start + 68)?,
        frame: memory.read_i32(range.start + 80)?,
        old_origin: memory.read_vec3(range.start + 84)?,
        old_frame: memory.read_i32(range.start + 96)?,
        back_lerp: memory.read_f32(range.start + 100)?,
        skin_num: memory.read_i32(range.start + 104)?,
        custom_skin: memory.read_i32(range.start + 108)?,
        custom_shader: memory.read_i32(range.start + 112)?,
        shader_rgba: color(116)?,
        shader_tex_coord: [memory.read_f32(range.start + 120)?, memory.read_f32(range.start + 124)?],
        shader_time: memory.read_f32(range.start + 128)?,
        radius: memory.read_f32(range.start + 132)?,
        rotation: memory.read_f32(range.start + 136)?,
    })
}

/// Read a `refdef_t` from guest memory.
pub fn read_refdef(memory: &SyscallMemory, word: i32) -> Result<Refdef, GuestError> {
    let range = memory.span(word, QVM_REFDEF_BYTES, 0)?;
    let mut area_mask = [0u8; 32];
    area_mask.copy_from_slice(memory.read_bytes(range.start + 80, 32)?);
    let mut text = [[0u8; 32]; 8];
    for (row, slot) in text.iter_mut().enumerate() {
        let bytes = memory.read_bytes(range.start + 112 + row * 32, 32)?;
        if !bytes.contains(&0) {
            return Err(GuestError::abi(format!("QVM refdef_t render text row {row} has no NUL within 32 bytes")));
        }
        slot.copy_from_slice(bytes);
    }
    Ok(Refdef {
        x: memory.read_i32(range.start)?,
        y: memory.read_i32(range.start + 4)?,
        width: memory.read_i32(range.start + 8)?,
        height: memory.read_i32(range.start + 12)?,
        fov_x: memory.read_f32(range.start + 16)?,
        fov_y: memory.read_f32(range.start + 20)?,
        view_origin: memory.read_vec3(range.start + 24)?,
        view_axis: [
            memory.read_vec3(range.start + 36)?,
            memory.read_vec3(range.start + 48)?,
            memory.read_vec3(range.start + 60)?,
        ],
        time: memory.read_i32(range.start + 72)?,
        render_flags: memory.read_i32(range.start + 76)?,
        area_mask,
        text,
    })
}

/// Read polygon vertices from guest memory.
pub fn read_poly_vertices(memory: &SyscallMemory, word: i32, count: i32) -> Result<Vec<PolyVertex>, GuestError> {
    if count < 0 {
        return Err(GuestError::abi(format!("QVM polyVert_t invalid vertex count {count}")));
    }
    let base = memory.pointer(word).ok_or_else(|| GuestError::invalid("QVM poly vertices require a nonnull pointer"))?;
    let mut vertices = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let offset = base + index * QVM_POLY_VERTEX_BYTES;
        vertices.push(PolyVertex {
            position: memory.read_vec3(offset)?,
            tex_coord: [memory.read_f32(offset + 12)?, memory.read_f32(offset + 16)?],
            color: [memory.get(offset + 20)?, memory.get(offset + 21)?, memory.get(offset + 22)?, memory.get(offset + 23)?],
        });
    }
    Ok(vertices)
}

/// Write an `orientation_t` to guest memory.
pub fn write_orientation(memory: &mut SyscallMemory, word: i32, value: &Orientation) -> Result<(), GuestError> {
    let range = memory.span(word, QVM_ORIENTATION_BYTES, 0)?;
    memory.write_vec3(range.start, &value.origin)?;
    memory.write_vec3(range.start + 12, &value.axes[0])?;
    memory.write_vec3(range.start + 24, &value.axes[1])?;
    memory.write_vec3(range.start + 36, &value.axes[2])?;
    Ok(())
}

fn identity_orientation() -> Orientation {
    Orientation {
        origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        axes: [
            Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            Vec3 { x: 0.0, y: 0.0, z: 1.0 },
        ],
    }
}

struct ResourceTraps {
    model: i32,
    skin: i32,
    shader: Option<i32>,
    shader_nomip: i32,
    color: i32,
    picture: i32,
    bounds: i32,
    tag: i32,
    remap: i32,
}

const UI_TRAPS: ResourceTraps =
    ResourceTraps { model: 18, skin: 19, shader: None, shader_nomip: 20, color: 26, picture: 27, bounds: 56, tag: 29, remap: 80 };
const CGAME_TRAPS: ResourceTraps =
    ResourceTraps { model: 37, skin: 38, shader: Some(39), shader_nomip: 57, color: 45, picture: 46, bounds: 47, tag: 48, remap: 79 };

fn resource_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    resources: &mut dyn RenderResourceHost,
    draw: &mut dyn DrawHost,
) -> Result<Option<i32>, GuestError> {
    if call.role == QvmRole::Qagame {
        return Ok(None);
    }
    let ui = call.role == QvmRole::Ui;
    let ids = if ui { &UI_TRAPS } else { &CGAME_TRAPS };
    let trap = call.code;
    if trap == ids.model {
        let name_word = call.int(1)?;
        let name = if name_word == 0 { String::new() } else { memory.read_string(name_word)? };
        return Ok(Some(resources.register_model(&name)));
    }
    if trap == ids.skin {
        let name_word = call.int(1)?;
        let name = if name_word == 0 { String::new() } else { memory.read_string(name_word)? };
        return Ok(Some(resources.register_skin(&name)));
    }
    if Some(trap) == ids.shader || trap == ids.shader_nomip {
        let name = memory.read_string(call.int(1)?)?;
        if Some(trap) == ids.shader {
            return Ok(Some(resources.register_shader(&name)));
        }
        return Ok(Some(resources.register_shader_nomip(&name)));
    }
    if trap == ids.color {
        let word = call.int(1)?;
        if word == 0 {
            draw.set_color(None);
        } else {
            let base = memory.pointer(word).ok_or_else(|| GuestError::invalid("QVM color requires a nonnull pointer"))?;
            draw.set_color(Some([
                memory.read_f32(base)?,
                memory.read_f32(base + 4)?,
                memory.read_f32(base + 8)?,
                memory.read_f32(base + 12)?,
            ]));
        }
        return Ok(Some(0));
    }
    if trap == ids.picture {
        let rect = FloatRect { x: call.float(1)?, y: call.float(2)?, width: call.float(3)?, height: call.float(4)? };
        let uv = UvRect { s: call.float(5)?, t: call.float(6)?, s2: call.float(7)?, t2: call.float(8)? };
        draw.stretch_pixels(rect, uv, call.int(9)?);
        return Ok(Some(0));
    }
    if trap == ids.bounds {
        let (min, max) = resources.model_bounds(call.int(1)?);
        let min_word = call.int(2)?;
        let max_word = call.int(3)?;
        let base = memory.pointer(min_word).ok_or_else(|| GuestError::invalid("QVM model bounds require a nonnull pointer"))?;
        memory.write_vec3(base, &min)?;
        let base = memory.pointer(max_word).ok_or_else(|| GuestError::invalid("QVM model bounds require a nonnull pointer"))?;
        memory.write_vec3(base, &max)?;
        return Ok(Some(0));
    }
    if trap == ids.tag {
        let destination = call.int(1)?;
        let handle = call.int(2)?;
        let start = call.int(3)?;
        let end = call.int(4)?;
        let fraction = call.float(5)?;
        let name_word = call.int(6)?;
        // Hosts without tag storage report no tag without consuming the name.
        let tag = resources.lerp_tag(handle, &|| memory.read_string(name_word), start, end, fraction)?;
        match tag {
            None => {
                write_orientation(memory, destination, &identity_orientation())?;
                Ok(Some(0))
            }
            Some(orientation) => {
                write_orientation(memory, destination, &orientation)?;
                Ok(Some(i32::from(!ui)))
            }
        }
    } else if trap == ids.remap {
        let original = memory.read_string(call.int(1)?)?;
        let replacement = memory.read_string(call.int(2)?)?;
        let offset_word = call.int(3)?;
        let offset = if offset_word == 0 { String::new() } else { memory.read_string(offset_word)? };
        resources.remap_shader(&original, &replacement, &offset);
        Ok(Some(0))
    } else {
        Ok(None)
    }
}

/// Dispatch a client-render trap. Returns `Ok(None)` when unhandled.
pub fn client_render_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    resources: &mut dyn RenderResourceHost,
    draw: &mut dyn DrawHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role == QvmRole::Qagame {
        return Ok(None);
    }
    if let Some(result) = resource_syscall(call, memory, resources, draw)? {
        return Ok(Some(result));
    }
    let ui = call.role == QvmRole::Ui;
    let trap = call.code;
    if !ui && trap == 36 {
        let name = memory.read_string(call.int(1)?)?;
        resources.load_world(&name);
        return Ok(Some(0));
    }
    if !ui && trap == 86 {
        let (token, continued) = resources.entity_token();
        memory.write_string(call.int(1)?, &token, call.int(2)? as usize)?;
        return Ok(Some(i32::from(continued)));
    }
    if !ui && trap == 88 {
        let first = memory.read_vec3_ptr(call.int(1)?)?;
        let second = memory.read_vec3_ptr(call.int(2)?)?;
        return Ok(Some(i32::from(resources.in_pvs(first, second))));
    }
    if trap == if ui { 21 } else { 40 } {
        resources.clear_scene();
        return Ok(Some(0));
    }
    if trap == if ui { 22 } else { 41 } {
        resources.add_ref_entity(read_ref_entity(memory, call.int(1)?)?);
        return Ok(Some(0));
    }
    if trap == if ui { 23 } else { 42 } || !ui && trap == 87 {
        let shader = call.int(1)?;
        let count = call.int(2)?;
        let word = call.int(3)?;
        let polys = if trap == 87 { call.int(4)? } else { 1 };
        if shader == 0 || count <= 0 || polys <= 0 {
            return Ok(Some(0));
        }
        let stride = count as usize * QVM_POLY_VERTEX_BYTES;
        memory.span(word, stride * polys as usize, 0)?;
        let base = memory.pointer(word).expect("checked span");
        for index in 0..polys as usize {
            let mut vertices = Vec::with_capacity(count as usize);
            for vertex in 0..count as usize {
                let offset = base + index * stride + vertex * QVM_POLY_VERTEX_BYTES;
                vertices.push(PolyVertex {
                    position: memory.read_vec3(offset)?,
                    tex_coord: [memory.read_f32(offset + 12)?, memory.read_f32(offset + 16)?],
                    color: [memory.get(offset + 20)?, memory.get(offset + 21)?, memory.get(offset + 22)?, memory.get(offset + 23)?],
                });
            }
            resources.add_poly(shader, vertices);
        }
        return Ok(Some(0));
    }
    if trap == if ui { 24 } else { 43 } || !ui && trap == 85 {
        let radius = call.float(2)?;
        if radius <= 0.0 {
            return Ok(Some(0));
        }
        let origin = memory.read_vec3_ptr(call.int(1)?)?;
        resources.add_light(GuestLight {
            origin,
            radius,
            color: Vec3 { x: call.float(3)?, y: call.float(4)?, z: call.float(5)? },
            additive: trap == 85,
        });
        return Ok(Some(0));
    }
    if trap == if ui { 25 } else { 44 } {
        resources.render_scene(read_refdef(memory, call.int(1)?)?);
        return Ok(Some(0));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeResources {
        log: Vec<String>,
    }

    impl RenderResourceHost for FakeResources {
        fn register_model(&mut self, name: &str) -> i32 {
            self.log.push(format!("model {name}"));
            11
        }
        fn register_skin(&mut self, name: &str) -> i32 {
            self.log.push(format!("skin {name}"));
            12
        }
        fn register_shader(&mut self, name: &str) -> i32 {
            self.log.push(format!("shader {name}"));
            13
        }
        fn register_shader_nomip(&mut self, name: &str) -> i32 {
            self.log.push(format!("nomip {name}"));
            14
        }
        fn model_bounds(&mut self, handle: i32) -> (Vec3, Vec3) {
            self.log.push(format!("bounds {handle}"));
            (Vec3 { x: -1.0, y: -1.0, z: -1.0 }, Vec3 { x: 1.0, y: 1.0, z: 1.0 })
        }
        fn lerp_tag(
            &mut self,
            handle: i32,
            name: &dyn Fn() -> Result<String, GuestError>,
            _start: i32,
            _end: i32,
            _fraction: f32,
        ) -> Result<Option<Orientation>, GuestError> {
            if handle != 11 {
                return Ok(None);
            }
            let name = name()?;
            self.log.push(format!("tag {handle} {name}"));
            Ok(Some(identity_orientation()))
        }
        fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str) {
            self.log.push(format!("remap {original} {replacement} {offset}"));
        }
        fn load_world(&mut self, name: &str) {
            self.log.push(format!("world {name}"));
        }
        fn entity_token(&mut self) -> (String, bool) {
            ("{".to_string(), true)
        }
        fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }
        fn clear_scene(&mut self) {
            self.log.push("clear".to_string());
        }
        fn add_ref_entity(&mut self, entity: RefEntity) {
            self.log.push(format!("entity {:?} {}", entity.kind, entity.model));
        }
        fn add_poly(&mut self, shader: i32, vertices: Vec<PolyVertex>) {
            self.log.push(format!("poly {shader} {}", vertices.len()));
        }
        fn add_light(&mut self, light: GuestLight) {
            self.log.push(format!("light {} {}", light.radius, light.additive));
        }
        fn render_scene(&mut self, refdef: Refdef) {
            self.log.push(format!("scene {}x{}", refdef.width, refdef.height));
        }
    }

    struct FakeDraw {
        log: Vec<String>,
    }

    impl DrawHost for FakeDraw {
        fn set_color(&mut self, color: Option<[f32; 4]>) {
            self.log.push(format!("color {color:?}"));
        }
        fn stretch_pixels(&mut self, rect: FloatRect, _uv: UvRect, shader: i32) {
            self.log.push(format!("pic {} {} {shader}", rect.width, rect.height));
        }
    }

    fn cg(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, code, args, AbiProfile::Modern)
    }

    fn ui(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Ui, code, args, AbiProfile::Modern)
    }

    fn harness() -> (SyscallMemory, FakeResources, FakeDraw) {
        (SyscallMemory::new(65536).unwrap(), FakeResources { log: Vec::new() }, FakeDraw { log: Vec::new() })
    }

    #[test]
    fn resource_registration() {
        let (mut memory, mut resources, mut draw) = harness();
        memory.write_string(512, "models/a.md3", 13).unwrap();
        assert_eq!(client_render_syscall(&cg(37, &[512]), &mut memory, &mut resources, &mut draw).unwrap(), Some(11));
        assert_eq!(client_render_syscall(&cg(38, &[0]), &mut memory, &mut resources, &mut draw).unwrap(), Some(12));
        assert_eq!(client_render_syscall(&cg(39, &[512]), &mut memory, &mut resources, &mut draw).unwrap(), Some(13));
        assert_eq!(client_render_syscall(&ui(20, &[512]), &mut memory, &mut resources, &mut draw).unwrap(), Some(14));
        assert_eq!(client_render_syscall(&ui(39, &[512]), &mut memory, &mut resources, &mut draw).unwrap(), None);
        assert_eq!(
            resources.log,
            vec![
                "model models/a.md3".to_string(),
                "skin ".to_string(),
                "shader models/a.md3".to_string(),
                "nomip models/a.md3".to_string(),
            ]
        );
    }

    #[test]
    fn color_and_picture() {
        let (mut memory, mut resources, mut draw) = harness();
        memory.write_f32(512, 1.0).unwrap();
        memory.write_f32(516, 0.5).unwrap();
        memory.write_f32(520, 0.25).unwrap();
        memory.write_f32(524, 1.0).unwrap();
        assert_eq!(client_render_syscall(&cg(45, &[512]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&ui(26, &[0]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        let pic = [0f32, 0.0, 64.0, 64.0, 0.0, 0.0, 1.0, 1.0].map(f32::to_bits).map(|bits| bits as i32);
        let mut args = pic.to_vec();
        args.push(13);
        assert_eq!(client_render_syscall(&cg(46, &args), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(draw.log[0], "color Some([1.0, 0.5, 0.25, 1.0])".to_string());
        assert_eq!(draw.log[1], "color None".to_string());
        assert_eq!(draw.log[2], "pic 64 64 13".to_string());
    }

    #[test]
    fn bounds_tag_and_remap() {
        let (mut memory, mut resources, mut draw) = harness();
        assert_eq!(client_render_syscall(&cg(47, &[11, 512, 1024]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(memory.read_vec3(512).unwrap(), Vec3 { x: -1.0, y: -1.0, z: -1.0 });
        memory.write_string(2048, "tag_head", 9).unwrap();
        let frac = 0.5f32.to_bits() as i32;
        assert_eq!(
            client_render_syscall(&cg(48, &[4096, 11, 0, 1, frac, 2048]), &mut memory, &mut resources, &mut draw).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(4096 + 12).unwrap(), Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        assert_eq!(client_render_syscall(&ui(29, &[4096, 99, 0, 1, frac, 2048]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        memory.write_string(512, "old", 4).unwrap();
        memory.write_string(1024, "new", 4).unwrap();
        assert_eq!(
            client_render_syscall(&cg(79, &[512, 1024, 0]), &mut memory, &mut resources, &mut draw).unwrap(),
            Some(0)
        );
        assert_eq!(resources.log[3], "remap old new ".to_string());
    }

    #[test]
    fn scene_traps() {
        let (mut memory, mut resources, mut draw) = harness();
        memory.write_string(512, "maps/q3dm1.bsp", 15).unwrap();
        assert_eq!(client_render_syscall(&cg(36, &[512]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&cg(86, &[1024, 64]), &mut memory, &mut resources, &mut draw).unwrap(), Some(1));
        assert_eq!(memory.read_string(1024).unwrap(), "{");
        memory.write_vec3(2048, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(3072, &Vec3 { x: 1.0, y: 1.0, z: 1.0 }).unwrap();
        assert_eq!(client_render_syscall(&cg(88, &[2048, 3072]), &mut memory, &mut resources, &mut draw).unwrap(), Some(1));
        assert_eq!(client_render_syscall(&cg(40, &[]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&ui(21, &[]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        memory.write_i32(8192, 0).unwrap();
        memory.write_i32(8200, 11).unwrap();
        assert_eq!(client_render_syscall(&cg(41, &[8192]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        memory.write_i32(8192, 9).unwrap();
        assert!(client_render_syscall(&cg(41, &[8192]), &mut memory, &mut resources, &mut draw).is_err());
        assert_eq!(resources.log[4], "entity Model 11".to_string());
    }

    #[test]
    fn polys_lights_and_scene() {
        let (mut memory, mut resources, mut draw) = harness();
        for vertex in 0..3 {
            memory.write_vec3(8192 + vertex * 24, &Vec3 { x: vertex as f32, y: 0.0, z: 0.0 }).unwrap();
        }
        assert_eq!(client_render_syscall(&cg(42, &[13, 3, 8192]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&cg(42, &[0, 3, 8192]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&cg(87, &[13, 3, 8192, 0]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        memory.write_vec3(1024, &Vec3 { x: 1.0, y: 2.0, z: 3.0 }).unwrap();
        let radius = 300.0f32.to_bits() as i32;
        let one = 1.0f32.to_bits() as i32;
        assert_eq!(client_render_syscall(&cg(43, &[1024, radius, one, one, one]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&cg(85, &[1024, radius, one, one, one]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(client_render_syscall(&cg(43, &[1024, 0, one, one, one]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        memory.write_i32(16384 + 8, 640).unwrap();
        memory.write_i32(16384 + 12, 480).unwrap();
        assert_eq!(client_render_syscall(&cg(44, &[16384]), &mut memory, &mut resources, &mut draw).unwrap(), Some(0));
        assert_eq!(
            resources.log,
            vec![
                "poly 13 3".to_string(),
                "light 300 false".to_string(),
                "light 300 true".to_string(),
                "scene 640x480".to_string(),
            ]
        );
    }

    #[test]
    fn refdef_rejects_missing_nul() {
        let (mut memory, mut resources, mut draw) = harness();
        memory.fill(16384, QVM_REFDEF_BYTES, 0x41).unwrap();
        assert!(client_render_syscall(&cg(44, &[16384]), &mut memory, &mut resources, &mut draw).is_err());
    }

    #[test]
    fn routing() {
        let (mut memory, mut resources, mut draw) = harness();
        let game = HostCall::engine(QvmRole::Qagame, 40, &[], AbiProfile::Modern);
        assert_eq!(client_render_syscall(&game, &mut memory, &mut resources, &mut draw).unwrap(), None);
        assert_eq!(client_render_syscall(&cg(999, &[]), &mut memory, &mut resources, &mut draw).unwrap(), None);
        assert_eq!(client_render_syscall(&ui(36, &[0]), &mut memory, &mut resources, &mut draw).unwrap(), None);
    }
}
