use qa_compat::{
    renderer::{Q3_GL_CONFIG_BYTES, write_q3},
    services::CallError,
};
use qa_core::presentation::{DisplayConfiguration, RendererConfiguration};

#[test]
fn native_layout_is_exact_and_terminated() {
    let mut configuration = RendererConfiguration {
        display: DisplayConfiguration {
            width: 1234,
            height: 567,
            frequency: 143,
            fullscreen: true,
            color_bits: 30,
            depth_bits: 24,
            stencil_bits: 8,
            stereo: true,
        },
        max_texture_size: 16384,
        max_active_textures: 32,
        hardware_gamma: true,
        compressed_textures: true,
        texture_env_add: true,
        render_worker: true,
        ..Default::default()
    };
    configuration.renderer.set_bytes(&[0xf1; 1023]).unwrap();
    configuration.vendor.set("vendor").unwrap();
    configuration.version.set("version").unwrap();
    configuration.extensions.set_bytes(&[b'x'; 8191]).unwrap();
    assert_eq!(Q3_GL_CONFIG_BYTES, 11332);
    let mut output = [0x55; Q3_GL_CONFIG_BYTES];
    write_q3(&configuration, &mut output).unwrap();
    assert_eq!(&output[..1023], &[0xf1; 1023]);
    assert_eq!(output[1023], 0);
    assert_eq!(&output[1024..1031], b"vendor\0");
    assert!(output[1031..2048].iter().all(|&b| b == 0));
    assert_eq!(&output[2048..2056], b"version\0");
    assert_eq!(&output[3072..11263], &[b'x'; 8191]);
    assert_eq!(output[11263], 0);
    let cells: Vec<_> = output[11264..]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(
        cells,
        [
            16384,
            32,
            30,
            24,
            8,
            0,
            0,
            1,
            1,
            1,
            1234,
            567,
            (1234.0f32 / 567.0).to_bits(),
            143,
            1,
            1,
            1
        ]
    );
}

#[test]
fn rejected_projection_leaves_destination_unchanged() {
    let mut configuration = RendererConfiguration::default();
    let mut output = [0x77; Q3_GL_CONFIG_BYTES];
    assert_eq!(
        write_q3(&configuration, &mut output),
        Err(CallError::Geometry)
    );
    assert!(output.iter().all(|&b| b == 0x77));
    configuration.display.width = 640;
    configuration.display.height = 400;
    assert_eq!(
        write_q3(&configuration, &mut output[..11331]),
        Err(CallError::Memory)
    );
    assert!(output.iter().all(|&b| b == 0x77));
}
