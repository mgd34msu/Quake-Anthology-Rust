use qa_core::primitives::Vec3;
use qa_render::{
    Assets,
    assets::upload::{MAX_DIMENSION, UploadError, UploadParams, prepare_rgba},
    material::world_load::SkyEnvironment,
    surface_cache::{IndexedTexture, LightGrid, SurfaceSource},
};

#[test]
fn resource_dimensions_keep_the_same_inclusive_limit_and_payload_checks() {
    let mut assets = Assets::load().unwrap();
    for (width, height) in [(MAX_DIMENSION, 1), (1, MAX_DIMENSION)] {
        let pixels = vec![17; width as usize * height as usize];
        let rgba = vec![23; pixels.len() * 4];
        let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
            vec![17; (width >> mip).max(1) as usize * (height >> mip).max(1) as usize]
        });
        let id = assets.register_image(width, height, &rgba).unwrap();
        assert_eq!(assets.image(id).unwrap().rgba.as_ref(), rgba);
        assert_eq!(
            prepare_rgba(width, height, &rgba, UploadParams::default())
                .unwrap()
                .levels[0]
                .rgba
                .as_ref(),
            rgba
        );
        assert!(IndexedTexture::load_base(width, height, &pixels, None).is_ok());
        assert!(
            IndexedTexture::load_masked(width, height, mips.each_ref().map(Vec::as_slice), None)
                .is_ok()
        );
        assert!(LightGrid::gray(width, height, [0, 255, 255, 255], &pixels).is_ok());
        assert!(SurfaceSource::load_rgba([0; 2], [width, height], 1).is_ok());
    }
    for (width, height) in [(0, 1), (1, 0), (MAX_DIMENSION + 1, 1), (1, u32::MAX)] {
        assert_eq!(
            assets.register_image(width, height, &[]),
            Err("invalid RGBA image dimensions")
        );
        assert_eq!(
            prepare_rgba(width, height, &[], UploadParams::default()),
            Err(UploadError::Dimensions)
        );
        assert!(IndexedTexture::load_base(width, height, &[], None).is_err());
        assert!(IndexedTexture::load_masked(width, height, [&[]; 4], None).is_err());
        assert!(LightGrid::gray(width, height, [0, 255, 255, 255], &[]).is_err());
        assert!(SurfaceSource::load_rgba([0; 2], [width, height], 1).is_err());
    }
    assert_eq!(
        prepare_rgba(1, 1, &[], UploadParams::default()),
        Err(UploadError::PixelCount)
    );
    assert!(assets.register_image(1, 1, &[]).is_err());
    assert!(IndexedTexture::load_base(1, 1, &[], None).is_err());
    assert!(IndexedTexture::load_masked(1, 1, [&[17], &[17], &[17], &[]], None).is_err());
    assert!(LightGrid::gray(1, 1, [0, 255, 255, 255], &[]).is_err());
    assert!(SurfaceSource::load_rgba([0; 2], [1; 2], 0).is_err());
    assert!(SurfaceSource::load([0; 2], [MAX_DIMENSION, 16], None, false, None).is_ok());
    assert!(SurfaceSource::load([0; 2], [MAX_DIMENSION + 16, 16], None, false, None).is_err());
    assert!(SurfaceSource::load([0; 2], [17, 16], None, false, None).is_err());
}

#[test]
fn sky_names_keep_byte_capacity_utf8_and_native_validation() {
    let name = format!("{}x", "é".repeat(31));
    assert_eq!(name.len(), 63);
    let environment = SkyEnvironment::new(name.as_bytes(), -3.5, Vec3([0.0, 0.0, 1.0])).unwrap();
    let copied = environment.clone();
    assert_eq!(copied.name().unwrap(), name);
    assert_eq!(copied.degrees_per_second.to_bits(), (-3.5f32).to_bits());
    assert_eq!(copied.axis, Vec3([0.0, 0.0, 1.0]));
    let default = SkyEnvironment::default();
    assert_eq!(default.name().unwrap(), "unit1_");
    assert_eq!(default.degrees_per_second, 0.0);
    assert_eq!(default.axis, Vec3::default());

    let too_long = [b'a'; 64];
    for name in [
        b"".as_slice(),
        &too_long,
        b"env/sky",
        b"env\\sky",
        b"sky\0",
        &[0xff],
    ] {
        assert!(SkyEnvironment::new(name, 0.0, Vec3::default()).is_err());
    }
    assert!(SkyEnvironment::new(b"sky", f32::NAN, Vec3::default()).is_err());
    assert!(SkyEnvironment::new(b"sky", 0.0, Vec3([0.0, f32::INFINITY, 0.0])).is_err());
}
