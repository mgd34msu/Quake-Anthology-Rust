//! Opt-in owned-context proof; never opens a display during ordinary gates.
use qa_core::primitives::Vec3;
use qa_platform::Window;
use qa_render::{
    Assets, FrontEnd, Limits, Refdef, SceneEntity, Vertex, Viewport,
    assets::{Cull, MaterialSettings, Sampler, Sky, Stage, StageTexture, Wrap, upload},
    gl::{GlBackend, GlContext},
    scene::SceneModel,
    sky::CloudSphere,
};
use std::{
    ffi::{CStr, c_void},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

type Gen = unsafe extern "system" fn(i32, *mut u32);
type Delete = unsafe extern "system" fn(i32, *const u32);
type Image = unsafe extern "system" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
struct Procedures {
    gen_buffers: Gen,
    delete_buffers: Delete,
    gen_vaos: Gen,
    delete_vaos: Delete,
    gen_textures: Gen,
    delete_textures: Delete,
    image: Image,
}
static PROCEDURES: OnceLock<Procedures> = OnceLock::new();
static TRACK: AtomicBool = AtomicBool::new(false);
static FAIL_IMAGE: AtomicBool = AtomicBool::new(false);
static OBJECTS: Mutex<[[usize; 2]; 3]> = Mutex::new([[0; 2]; 3]);

unsafe fn record(kind: usize, deleted: usize, count: i32, names: *const u32) {
    if TRACK.load(Ordering::Relaxed) && count > 0 {
        // SAFETY: These wrappers receive the backend's complete owned GL name
        // spans and inspect them synchronously before their Rust storage drops.
        let count = unsafe { std::slice::from_raw_parts(names, count as usize) }
            .iter()
            .filter(|&&name| name != 0)
            .count();
        OBJECTS.lock().unwrap()[kind][deleted] += count;
    }
}
macro_rules! objects {
    ($generate:ident, $delete:ident, $gen_field:ident, $delete_field:ident, $kind:expr) => {
        unsafe extern "system" fn $generate(count: i32, names: *mut u32) {
            unsafe {
                (PROCEDURES.get().unwrap().$gen_field)(count, names);
                record($kind, 0, count, names);
            }
        }
        unsafe extern "system" fn $delete(count: i32, names: *const u32) {
            unsafe {
                record($kind, 1, count, names);
                (PROCEDURES.get().unwrap().$delete_field)(count, names);
            }
        }
    };
}
objects!(gen_buffers, delete_buffers, gen_buffers, delete_buffers, 0);
objects!(gen_vaos, delete_vaos, gen_vaos, delete_vaos, 1);
objects!(
    gen_textures,
    delete_textures,
    gen_textures,
    delete_textures,
    2
);
unsafe extern "system" fn texture_image(
    target: u32,
    level: i32,
    internal: i32,
    width: i32,
    height: i32,
    border: i32,
    format: u32,
    kind: u32,
    pixels: *const c_void,
) {
    let level = if FAIL_IMAGE.swap(false, Ordering::Relaxed) {
        -1
    } else {
        level
    };
    // SAFETY: The real current-context procedure receives the unchanged upload
    // span; the one injected negative level produces GL_INVALID_VALUE.
    unsafe {
        (PROCEDURES.get().unwrap().image)(
            target, level, internal, width, height, border, format, kind, pixels,
        )
    };
}

fn procedure(window: &Window, name: &CStr) -> *const c_void {
    let pointer = window.gl_proc(name);
    assert!(!pointer.is_null());
    pointer
}
fn bindings(window: &Window) -> (i32, i32, i32, i32) {
    // SAFETY: These exact procedures belong to this owned current GL context.
    let (get, active): (
        unsafe extern "system" fn(u32, *mut i32),
        unsafe extern "system" fn(u32),
    ) = unsafe {
        (
            std::mem::transmute(procedure(window, c"glGetIntegerv")),
            std::mem::transmute(procedure(window, c"glActiveTexture")),
        )
    };
    let (mut unit, mut unpack, mut image, mut tables) = (0, 0, 0, 0);
    unsafe {
        get(0x84e0, &mut unit);
        get(0x0cf5, &mut unpack);
        active(0x84c0);
        get(0x8069, &mut image);
        active(0x84c1);
        get(0x8c2c, &mut tables);
        active(unit as u32);
    }
    (unit, unpack, image, tables)
}

fn quad(
    assets: &mut Assets,
    image: qa_render::ImageId,
    name: &str,
    sky: bool,
) -> qa_render::ModelId {
    let material = assets
        .register_material(
            name,
            &[Stage {
                texture: StageTexture::Image(image),
                ..Default::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                sky: sky.then_some(Sky::Cube {
                    outer_box: Some([image; 6]),
                    inner_box: None,
                    clouds: CloudSphere::native(256.),
                    rotation: None,
                    params: Default::default(),
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let vertices = [
        Vec3([4., 4., 4.]),
        Vec3([4., -4., 4.]),
        Vec3([4., -4., -4.]),
        Vec3([4., 4., -4.]),
    ]
    .map(|position| Vertex {
        position,
        ..Default::default()
    });
    assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], material)
        .unwrap()
}
fn draw(gl: &mut GlBackend, assets: &Assets, model: qa_render::ModelId) -> [u8; 3] {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Mesh(model),
        ..Default::default()
    }));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 64,
                height: 48,
                ..Default::default()
            },
            fov: [90.; 2],
            near: 1.,
            far: 64.,
            ..Default::default()
        },
        &[],
        assets
    ));
    assert_eq!(gl.render(&frame.finish(), assets).rejected, 0);
    let mut pixels = vec![0; 64 * 48 * 4];
    assert!(gl.read_pixels(&mut pixels));
    assert!(gl.take_error().is_none());
    let center = (24 * 64 + 32) * 4;
    pixels[center..center + 3].try_into().unwrap()
}

#[test]
#[ignore = "requires an owned private GL display"]
fn appended_gl_assets_commit_once_and_failed_upload_releases_only_candidate_objects() {
    assert!(std::env::var_os("QA_PRIVATE_GL").is_some());
    let window = Window::open_gl(64, 48).unwrap();
    // SAFETY: SDL resolves these signatures for this owned current context; all
    // wrappers delegate to those exact procedures and outlive the backend.
    let procedures = unsafe {
        Procedures {
            gen_buffers: std::mem::transmute(procedure(&window, c"glGenBuffers")),
            delete_buffers: std::mem::transmute(procedure(&window, c"glDeleteBuffers")),
            gen_vaos: std::mem::transmute(procedure(&window, c"glGenVertexArrays")),
            delete_vaos: std::mem::transmute(procedure(&window, c"glDeleteVertexArrays")),
            gen_textures: std::mem::transmute(procedure(&window, c"glGenTextures")),
            delete_textures: std::mem::transmute(procedure(&window, c"glDeleteTextures")),
            image: std::mem::transmute(procedure(&window, c"glTexImage2D")),
        }
    };
    assert!(PROCEDURES.set(procedures).is_ok());
    let mut assets = Assets::load().unwrap();
    let red = assets.register_image(1, 1, &[220, 0, 0, 255]).unwrap();
    let old_model = quad(&mut assets, red, "append/old", false);
    let context = unsafe {
        GlContext::load(
            |name| match name.to_bytes() {
                b"glGenBuffers" => gen_buffers as *const c_void,
                b"glDeleteBuffers" => delete_buffers as *const c_void,
                b"glGenVertexArrays" => gen_vaos as *const c_void,
                b"glDeleteVertexArrays" => delete_vaos as *const c_void,
                b"glGenTextures" => gen_textures as *const c_void,
                b"glDeleteTextures" => delete_textures as *const c_void,
                b"glTexImage2D" => texture_image as *const c_void,
                _ => window.gl_proc(name),
            },
            window.display_configuration().unwrap(),
        )
        .unwrap()
    };
    let mut gl = unsafe { GlBackend::load(context, &assets, 64, 48, 16_384).unwrap() };
    let before = gl.admitted_assets().unwrap();
    assert_eq!(draw(&mut gl, &assets, old_model), [220, 0, 0]);
    let blue = assets
        .register_image(2, 2, &[0, 0, 240, 255].repeat(4))
        .unwrap();
    assets
        .prepare_image(
            blue,
            upload::UploadParams {
                mipmaps: upload::MipmapBuild::Box,
                ..Default::default()
            },
        )
        .unwrap();
    assets
        .set_image_native_sampler(
            blue,
            Sampler {
                wrap: Wrap::Clamp,
                ..Default::default()
            },
        )
        .unwrap();
    let appended = quad(&mut assets, blue, "append/new", false);
    // A nondefault upload alignment and active evaluator-table unit must survive
    // failure; the next draw then selects unit zero as the executor expects.
    let (active, pixel): (
        unsafe extern "system" fn(u32),
        unsafe extern "system" fn(u32, i32),
    ) = unsafe {
        (
            std::mem::transmute(procedure(&window, c"glActiveTexture")),
            std::mem::transmute(procedure(&window, c"glPixelStorei")),
        )
    };
    unsafe {
        active(0x84c1);
        pixel(0x0cf5, 8);
    }
    let before_bindings = bindings(&window);
    FAIL_IMAGE.store(true, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    assert!(gl.prepare_assets(&assets).is_err());
    TRACK.store(false, Ordering::Relaxed);
    assert_eq!(bindings(&window), before_bindings);
    unsafe {
        active(0x84c0);
    }
    assert_eq!(*OBJECTS.lock().unwrap(), [[2, 2], [2, 2], [1, 1]]);
    assert_eq!(gl.admitted_assets(), Some(before));
    assert_eq!(draw(&mut gl, &assets, old_model), [220, 0, 0]);
    gl.prepare_assets(&assets).unwrap();
    assert_eq!(gl.admitted_assets(), Some(assets.counts()));
    assert_eq!(draw(&mut gl, &assets, old_model), [220, 0, 0]);
    assert_eq!(draw(&mut gl, &assets, appended), [0, 0, 240]);
    TRACK.store(true, Ordering::Relaxed);
    let unchanged = *OBJECTS.lock().unwrap();
    gl.prepare_assets(&assets).unwrap();
    assert_eq!(*OBJECTS.lock().unwrap(), unchanged);
    TRACK.store(false, Ordering::Relaxed);
    let sky = quad(&mut assets, blue, "append/clouds", true);
    gl.prepare_assets(&assets).unwrap();
    assert_eq!(draw(&mut gl, &assets, sky), [0, 0, 240]);
    assert_eq!(gl.admitted_assets(), Some(assets.counts()));
    assets
        .set_image_native_sampler(
            red,
            Sampler {
                wrap: Wrap::Clamp,
                ..Default::default()
            },
        )
        .unwrap();
    let admitted = gl.admitted_assets();
    assert!(gl.prepare_assets(&assets).is_err());
    assert_eq!(gl.admitted_assets(), admitted);
    assert_eq!(draw(&mut gl, &assets, old_model), [220, 0, 0]);
}
