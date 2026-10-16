use qa_app::{
    host::ClientView,
    renderer::{Kind, view_refdef},
};
use qa_core::primitives::Vec3;
use qa_render::{
    BlendPhase, CpuPresentation, PaletteId, PaletteTransform, Viewport,
    material::world_load::PresentationDefaults,
    surface_cache::IndexedLighting,
    world::{WorldId, WorldPresentationId},
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
        world_presentation: WorldPresentationId(3),
        origin: Vec3([1.0, 2.0, 3.0]),
        angles: Vec3([5.0, 45.0, 0.0]),
        fov_x: 90.0,
        screen_blend: [0.0; 4],
        damage_blend: [0.0; 4],
        flags: Default::default(),
        time_ms: 123,
        model_time: Some(qa_core::primitives::ThinkTime::Seconds(0.125_000_000_1)),
        presentation: presentation(PaletteId(0), IndexedLighting::Gray),
        presentation_rules: qa_core::primitives::RuleSetId::Quake,
        lighting_scales: [0.6, 1.],
        curve_error: 250.0,
        model_lod: Default::default(),
        policy: Default::default(),
        near: 4.0,
    };
    let mut second = first;
    second.world_presentation = WorldPresentationId(4);
    assert_eq!(first.world, second.world);
    assert_ne!(first.world_presentation, second.world_presentation);
    second.presentation = presentation(PaletteId(1), IndexedLighting::NativeRgb);
    second.presentation_rules = qa_core::primitives::RuleSetId::Quake3;
    second.model_time = Some(qa_core::primitives::ThinkTime::Seconds(0.375_000_000_1));
    second.presentation.lightstyles[0].indexed_scale = 384;
    second.presentation.identity_light = 0.5;
    second.curve_error = 17.25;
    second.model_lod.scale = 7.25;
    second.model_lod.bias = -1;
    second.policy.ignore_pvs = true;
    second.policy.ignore_areas = true;
    second.policy.draw_entities = false;
    second.near = 2.0;
    let a = view_refdef(first, viewport, Kind::Cpu);
    let b = view_refdef(second, viewport, Kind::Cpu);
    assert_eq!(a.cpu_presentation, first.presentation.cpu);
    assert_eq!(b.cpu_presentation, second.presentation.cpu);
    assert_ne!(a.cpu_presentation, b.cpu_presentation);
    assert_eq!(b.lightstyles[0].indexed_scale, 384);
    assert_eq!(b.identity_light, 0.5);
    assert_eq!(a.curve_error, first.curve_error);
    assert_eq!(a.model_lod, first.model_lod);
    assert_eq!(b.curve_error, second.curve_error);
    assert_eq!(b.model_lod, second.model_lod);
    assert_eq!(a.policy, first.policy);
    assert_eq!(b.policy, second.policy);
    assert_eq!(a.near, first.near);
    assert_eq!(b.near, second.near);
    assert_eq!(a.axes, b.axes);
    assert_eq!(a.origin, b.origin);
    assert_eq!(a.fov, b.fov);
    assert_eq!(a.time_ms, b.time_ms);
    assert_eq!(a.model_time, first.model_time);
    assert_eq!(b.model_time, second.model_time);
    assert_ne!(a.model_time, b.model_time);
    assert_eq!(a.frame_timing, qa_render::scene::FrameTiming::Cumulative);
    assert_eq!(b.frame_timing, qa_render::scene::FrameTiming::Cumulative);
    assert_eq!(
        view_refdef(first, viewport, Kind::Gl).frame_timing,
        qa_render::scene::FrameTiming::UniformFirst
    );
    assert_eq!(a.blend_phase, BlendPhase::FinalPalette);
    assert_eq!(a.palette_transform, first.presentation.palette_transform);
    let gl = view_refdef(second, viewport, Kind::Gl);
    assert_eq!(gl.blend_phase, BlendPhase::AfterView);
    assert_eq!(gl.palette_transform, None);
    assert_eq!(gl.lightstyles[0], second.presentation.lightstyles[0]);
    assert_eq!(gl.curve_error, second.curve_error);
    assert_eq!(gl.model_lod, second.model_lod);
    assert_eq!(gl.policy, second.policy);
    assert_eq!(gl.near, second.near);
    assert_eq!(gl.model_time, second.model_time);
    assert_eq!(gl.frame_timing, qa_render::scene::FrameTiming::Cumulative);
}
