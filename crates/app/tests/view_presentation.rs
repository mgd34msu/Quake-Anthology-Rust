use qa_app::{
    host::ClientView,
    renderer::{Kind, view_refdef},
};
use qa_core::primitives::Vec3;
use qa_render::{
    BlendPhase, CpuPresentation, PaletteId, PaletteTransform, Viewport,
    material::world_load::PresentationDefaults, surface_cache::IndexedLighting, world::WorldId,
};

fn presentation(palette: PaletteId, lighting: IndexedLighting) -> PresentationDefaults {
    PresentationDefaults {
        cpu: CpuPresentation::Indexed {
            palette,
            lighting,
            ambient: 0,
            fullbright: false,
        },
        blend_phase: BlendPhase::FinalPalette,
        palette_transform: Some(PaletteTransform::default()),
        ..PresentationDefaults::default()
    }
}

#[test]
fn seats_share_geometry_without_sharing_refdef_presentation() {
    let viewport = Viewport {
        width: 320,
        height: 200,
        ..Viewport::default()
    };
    let first = ClientView {
        world: WorldId(2),
        origin: Vec3([1.0, 2.0, 3.0]),
        angles: Vec3([5.0, 45.0, 0.0]),
        fov_x: 90.0,
        time_ms: 123,
        presentation: presentation(PaletteId(0), IndexedLighting::Gray),
    };
    let mut second = first;
    second.presentation = presentation(PaletteId(1), IndexedLighting::NativeRgb);
    second.presentation.lightstyles[0].indexed_scale = 384;
    second.presentation.identity_light = 0.5;
    let a = view_refdef(first, viewport, Kind::Cpu);
    let b = view_refdef(second, viewport, Kind::Cpu);
    assert_eq!(a.cpu_presentation, first.presentation.cpu);
    assert_eq!(b.cpu_presentation, second.presentation.cpu);
    assert_ne!(a.cpu_presentation, b.cpu_presentation);
    assert_eq!(b.lightstyles[0].indexed_scale, 384);
    assert_eq!(b.identity_light, 0.5);
    assert_eq!(a.axes, b.axes);
    assert_eq!(a.origin, b.origin);
    assert_eq!(a.fov, b.fov);
    assert_eq!(a.time_ms, b.time_ms);
    assert_eq!(a.blend_phase, BlendPhase::FinalPalette);
    assert_eq!(a.palette_transform, first.presentation.palette_transform);
    let gl = view_refdef(second, viewport, Kind::Gl);
    assert_eq!(gl.blend_phase, BlendPhase::AfterView);
    assert_eq!(gl.palette_transform, None);
    assert_eq!(gl.lightstyles[0], second.presentation.lightstyles[0]);
}
