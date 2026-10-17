use super::MeshLayout;
use crate::assets::Vertex;
use std::mem::size_of;

#[test]
fn appended_meshes_keep_native_model_order_and_old_static_offsets() {
    let old = MeshLayout::prepare([(4, 6), (8, 9)]).unwrap();
    let extended = MeshLayout::prepare([(4, 6), (8, 9), (12, 18)]).unwrap();
    for (old, new) in old.meshes.iter().zip(extended.meshes.iter()) {
        assert_eq!(
            (old.first_index, old.count, old.base_vertex),
            (new.first_index, new.count, new.base_vertex),
        );
    }
    let appended = &extended.meshes[2];
    assert_eq!(
        (appended.first_index, appended.count, appended.base_vertex),
        (15, 18, 12)
    );
    assert_eq!(extended.vertex_bytes, (24 * size_of::<Vertex>()) as isize);
    assert_eq!(extended.index_bytes, (33 * size_of::<u32>()) as isize);
}

#[test]
fn mesh_layout_rejects_gl_scalar_overflow_before_flattening_or_upload() {
    assert!(MeshLayout::prepare([(0, i32::MAX as usize + 1)]).is_err());
    assert!(MeshLayout::prepare([(i32::MAX as usize + 1, 0), (0, 0)]).is_err());
    assert!(MeshLayout::prepare([(usize::MAX, 0)]).is_err());
    assert!(MeshLayout::prepare([(0, 0)]).is_ok());
}

use super::{PreparedAssets, UploadBindings};
use crate::{Assets, ImageId, assets::AssetAdmission};
use std::{cell::RefCell, ffi::c_void};

#[derive(Default)]
struct Device {
    next: u32,
    active: u32,
    texture: u32,
    unpack: i32,
    generated: Vec<u32>,
    deleted: Vec<u32>,
    uploaded: Vec<(u32, i32, i32, i32)>,
    fail_upload: Option<usize>,
    error: u32,
}
thread_local! {
    static DEVICE: RefCell<Device> = RefCell::new(Device::default());
}
unsafe extern "system" fn get_integer(name: u32, output: *mut i32) {
    let value = DEVICE.with(|device| {
        let device = device.borrow();
        match name {
            0x84e0 => device.active as i32,
            0x8069 => device.texture as i32,
            0x0cf5 => device.unpack,
            _ => panic!("unexpected upload query"),
        }
    });
    // SAFETY: UploadBindings supplies a live scalar for this synchronous query.
    unsafe { *output = value };
}
unsafe extern "system" fn active_texture(value: u32) {
    DEVICE.with(|device| device.borrow_mut().active = value);
}
unsafe extern "system" fn bind_texture(target: u32, value: u32) {
    assert_eq!(target, super::TEXTURE_2D);
    DEVICE.with(|device| device.borrow_mut().texture = value);
}
unsafe extern "system" fn pixel_store(name: u32, value: i32) {
    assert_eq!(name, 0x0cf5);
    DEVICE.with(|device| device.borrow_mut().unpack = value);
}
unsafe extern "system" fn gen_textures(count: i32, output: *mut u32) {
    DEVICE.with(|device| {
        let mut device = device.borrow_mut();
        for index in 0..count as usize {
            device.next += 1;
            let name = device.next;
            // SAFETY: PreparedAssets supplies count writable texture-name slots.
            unsafe { *output.add(index) = name };
            device.generated.push(name);
        }
    });
}
unsafe extern "system" fn delete_textures(count: i32, input: *const u32) {
    // SAFETY: The candidate supplies count live texture names for this call.
    let names = unsafe { std::slice::from_raw_parts(input, count as usize) };
    DEVICE.with(|device| {
        let mut device = device.borrow_mut();
        device
            .deleted
            .extend(names.iter().copied().filter(|&name| name != 0));
    });
}
unsafe extern "system" fn texture_image(
    _: u32,
    level: i32,
    _: i32,
    width: i32,
    height: i32,
    _: i32,
    _: u32,
    _: u32,
    _: *const c_void,
) {
    DEVICE.with(|device| {
        let mut device = device.borrow_mut();
        let name = device.texture;
        device.uploaded.push((name, level, width, height));
        if device.fail_upload == Some(device.uploaded.len()) {
            device.error = 0x0502;
        }
    });
}
unsafe extern "system" fn get_error() -> u32 {
    DEVICE.with(|device| std::mem::take(&mut device.borrow_mut().error))
}
fn device(fail_upload: Option<usize>) -> super::Gl {
    DEVICE.with(|device| {
        *device.borrow_mut() = Device {
            next: 100,
            active: 0x84c1,
            texture: 22,
            unpack: 8,
            fail_upload,
            ..Default::default()
        }
    });
    super::Gl {
        get_integer,
        active_texture,
        bind_texture,
        pixel_store,
        gen_textures,
        delete_textures,
        texture_image,
        get_error,
        ..super::Gl::for_tests()
    }
}
fn image_candidate() -> (Assets, AssetAdmission, AssetAdmission) {
    let mut assets = Assets::load().unwrap();
    let first = assets.register_image(1, 1, &[1, 2, 3, 255]).unwrap();
    assets.register_image(1, 1, &[4, 5, 6, 255]).unwrap();
    let previous = AssetAdmission::prepare(None, &assets).unwrap().unwrap();
    assets
        .prepare_image_with_rgba(first, &[7, 8, 9, 255], Default::default())
        .unwrap();
    assets
        .register_image(2, 1, &[10, 11, 12, 255, 13, 14, 15, 255])
        .unwrap();
    let current = AssetAdmission::prepare(Some(&previous), &assets)
        .unwrap()
        .unwrap();
    (assets, previous, current)
}
fn params() -> Box<[super::TextureParams]> {
    vec![
        super::TextureParams {
            inverse_intensity: 1.0,
            sampler: None
        };
        3
    ]
    .into_boxed_slice()
}

#[test]
fn rejected_replacement_upload_deletes_only_candidate_names_and_restores_bindings() {
    let gl = device(Some(1));
    let (assets, previous, current) = image_candidate();
    let changes = current.changed_images(Some(&previous)).collect();
    let names = [11, 22, 33];
    {
        let _bindings = UploadBindings::new(&gl);
        assert!(PreparedAssets::prepare(&gl, &assets, &names, &params(), 0, changes).is_err());
    }
    DEVICE.with(|device| {
        let device = device.borrow();
        assert_eq!(device.generated, [101]);
        assert_eq!(device.deleted, [101]);
        assert_eq!(device.uploaded, [(101, 0, 1, 1)]);
        assert_eq!(
            (device.active, device.texture, device.unpack),
            (0x84c1, 22, 8)
        );
    });
    assert_eq!(names, [11, 22, 33]);
}

#[test]
fn committed_replacement_reuses_unchanged_names_and_retires_only_replaced_old_names() {
    let gl = device(None);
    let (assets, previous, current) = image_candidate();
    let changes = current.changed_images(Some(&previous)).collect();
    let mut names: Box<[_]> = [11, 22, 33].into();
    let mut old_params = params();
    {
        let mut bindings = UploadBindings::new(&gl);
        let mut candidate =
            PreparedAssets::prepare(&gl, &assets, &names, &old_params, 0, changes).unwrap();
        assert_eq!(&*candidate.changed_images, [ImageId(1), ImageId(3)]);
        assert_eq!(&*candidate.textures, [11, 101, 33, 102]);
        bindings.replace_texture(names[1], candidate.textures[1]);
        std::mem::swap(&mut names, &mut candidate.textures);
        std::mem::swap(&mut old_params, &mut candidate.texture_params);
    }
    DEVICE.with(|device| {
        let device = device.borrow();
        assert_eq!(device.generated, [101, 102]);
        assert_eq!(device.deleted, [22]);
        assert_eq!(device.uploaded, [(101, 0, 1, 1), (102, 0, 2, 1)]);
        assert_eq!(
            (device.active, device.texture, device.unpack),
            (0x84c1, 101, 8)
        );
    });
    assert_eq!(&*names, [11, 101, 33, 102]);
    assert!(
        AssetAdmission::prepare(Some(&current), &assets)
            .unwrap()
            .is_none()
    );
    DEVICE.with(|device| assert_eq!(device.borrow().generated, [101, 102]));
}
