use super::*;
use qa_core::primitives::{RuleSetId, ThinkTime};
use qa_render::{
    ModelId, ModelPart,
    scene::{
        ModelCapture, ModelChoice, ModelFramePose, ModelReplacementSettings, ModelSelector,
        SkeletalFrameMap, item_frames,
    },
    shader::reference::ShaderView,
};

fn resources() -> (Assets, ModelId, ModelId, [ShaderBindingId; 12]) {
    let (mut assets, native, shaders) = super::mdl_groups::skin_models();
    let replacement = super::skeletal::mesh(&mut assets, super::skeletal::skeleton(1));
    (assets, native, replacement, shaders)
}

fn part(model: ModelId, shader: ShaderBindingId) -> ModelPart {
    ModelPart {
        model,
        shader: shader.into(),
    }
}

fn pair(
    frame: &mut qa_render::Frame,
    pose: SceneEntity,
    native: ModelPart,
    replacement: ModelPart,
    rules: RuleSetId,
    mapping: SkeletalFrameMap,
) -> bool {
    frame.add_model_choices(pose, |capture| {
        let Some(replacement) = capture.map_frames(mapping, ModelChoice::part(replacement)) else {
            return false;
        };
        let Some(root) = capture.select(
            ModelSelector::Replacement { rules },
            [ModelChoice::part(native), replacement],
        ) else {
            return false;
        };
        capture.submit(root)
    })
}

fn composed<'capture>(
    capture: &mut ModelCapture<'capture>,
    native: ModelId,
    replacement: ModelId,
    shaders: &[ShaderBindingId; 12],
) -> Option<ModelChoice<'capture>> {
    let timing_source = native;
    let native = capture.select(
        ModelSelector::Lod { radius: Some(2.) },
        [shaders[0], shaders[4]].map(|shader| ModelChoice::part(part(native, shader))),
    )?;
    let replacement = capture.select(
        ModelSelector::SkinGroup {
            timing_source,
            group: 1,
        },
        [shaders[1], shaders[2]].map(|shader| ModelChoice::part(part(replacement, shader))),
    )?;
    let replacement = capture.map_frames(SkeletalFrameMap::Wrap, replacement)?;
    capture.select(
        ModelSelector::Replacement {
            rules: RuleSetId::Quake,
        },
        [native, replacement],
    )
}

#[test]
fn one_view_keeps_independent_replacement_roles_and_copied_distance_choices() {
    let (assets, native, replacement, shaders) = resources();
    let beyond = f32::from_bits(8.0f32.to_bits() + 1);
    let requests = [
        (-8., true, false, 8., 8., false, [replacement, native]),
        (-8., false, true, 8., 8., false, [native, replacement]),
        (-beyond, true, true, 8., 8., false, [native, native]),
        (-80., true, false, 8., 8., true, [replacement, native]),
        (-80., true, true, 0., -1., false, [replacement, replacement]),
        (-8., true, true, f32::NAN, f32::NAN, false, [native, native]),
        (
            -8.,
            true,
            true,
            f32::NAN,
            f32::NAN,
            true,
            [replacement, replacement],
        ),
    ];
    let mut pixels = None;
    for reverse in [false, true] {
        let mut frontend = FrontEnd::load(Limits::default().for_assets(&assets).unwrap()).unwrap();
        let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
        for at in 0..requests.len() {
            let index = if reverse { requests.len() - 1 - at } else { at };
            let (camera, q1, q2, q1_distance, q2_distance, no_lod, _) = requests[index];
            for (rules, original, enhanced) in [
                (RuleSetId::Quake, shaders[0], shaders[1]),
                (RuleSetId::Quake2, shaders[2], shaders[3]),
            ] {
                assert!(pair(
                    &mut frame,
                    SceneEntity {
                        no_model_lod: no_lod,
                        ..Default::default()
                    },
                    part(native, original),
                    part(replacement, enhanced),
                    rules,
                    SkeletalFrameMap::Wrap
                ));
            }
            let mut view = super::skeletal::view(index as u32 * 32);
            view.origin = Vec3([camera, 0., 0.]);
            view.model_replacements[RuleSetId::Quake as usize] = ModelReplacementSettings {
                enabled: q1,
                distance: q1_distance,
            };
            view.model_replacements[RuleSetId::Quake2 as usize] = ModelReplacementSettings {
                enabled: q2,
                distance: q2_distance,
            };
            assert!(frame.render_scene(view, &[], &assets, ShaderView::direct()));
        }
        let packet = frame.finish();
        assert_eq!(packet.rejected, 0);
        for command in packet.commands() {
            let Command::View(view) = command else {
                continue;
            };
            let index = view.refdef.viewport.x as usize / 32;
            let expected = requests[index].6;
            let draws = packet.draws(view.scene.draws);
            assert_eq!(
                draws
                    .iter()
                    .map(|draw| draw.mesh.unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
            for (seat, draw) in draws.iter().enumerate() {
                let expected_shader =
                    shaders[seat * 2 + usize::from(expected[seat] == replacement)];
                assert_eq!(draw.shader.unwrap().source(&()), Some(expected_shader));
                let source = packet.entity(draw.index);
                assert!(matches!(source.model, SceneModel::Choice { .. }));
                assert_eq!(
                    (source.frame, source.old_frame, source.back_lerp),
                    (0, 0, 0.)
                );
            }
        }
        let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(224, 32, &assets).unwrap();
        let stats = cpu.render(&packet, &assets);
        assert_eq!((stats.views, stats.rejected), (7, 0));
        assert!(cpu.inverse_depth().iter().any(|&depth| depth > 0.));
        if let Some(expected) = &pixels {
            assert_eq!(cpu.pixels(), expected);
        } else {
            pixels = Some(cpu.pixels().to_vec());
        }
    }
}

#[test]
fn lod_and_timed_skin_choices_compose_with_replacement_in_one_arena() {
    let (assets, native, replacement, shaders) = resources();
    let mut frontend = FrontEnd::load(Limits::default().for_assets(&assets).unwrap()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    for (index, enabled) in [false, true].into_iter().enumerate() {
        assert!(frame.add_model_choices(SceneEntity::default(), |capture| {
            let Some(root) = composed(capture, native, replacement, &shaders) else {
                return false;
            };
            capture.submit(root)
        }));
        let mut view = super::skeletal::view(index as u32 * 32);
        view.model_lod.bias = 1;
        view.model_time = Some(ThinkTime::Seconds(0.125));
        view.model_replacements[RuleSetId::Quake as usize].enabled = enabled;
        assert!(frame.render_scene(view, &[], &assets, ShaderView::direct()));
    }
    let packet = frame.finish();
    let draws = super::skeletal::draws(&packet);
    assert_eq!(
        draws.iter().map(|draw| draw.mesh).collect::<Vec<_>>(),
        [Some(native), Some(replacement)]
    );
    assert_eq!(
        draws
            .iter()
            .map(|draw| draw.shader.unwrap().source(&()).unwrap())
            .collect::<Vec<_>>(),
        [shaders[4], shaders[2]]
    );
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(64, 32, &assets).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    for (seat, color) in [[68, 203, 76, 255], [34, 229, 38, 255]]
        .into_iter()
        .enumerate()
    {
        assert!(
            cpu.pixels()
                .iter()
                .enumerate()
                .any(|(at, &pixel)| at % 64 / 32 == seat && pixel == u32::from_le_bytes(color))
        );
    }
}

#[test]
fn mapped_skeletal_frames_seal_their_own_interpolation_and_view_clock() {
    let (assets, native, replacement, shaders) = resources();
    let requests = [
        (
            SkeletalFrameMap::Wrap,
            90.,
            ModelFramePose {
                current: 1,
                previous: 0,
                back_lerp: 0.25,
            },
            Vec3([8., -1., -2.]),
        ),
        (
            SkeletalFrameMap::Elapsed { rate: 2. },
            0.375,
            ModelFramePose {
                current: 1,
                previous: 1,
                back_lerp: 0.,
            },
            Vec3([8., 0., -2.]),
        ),
        (
            SkeletalFrameMap::Elapsed { rate: 2. },
            -0.375,
            ModelFramePose {
                current: 1,
                previous: 1,
                back_lerp: 0.,
            },
            Vec3([8., 0., -2.]),
        ),
        (
            SkeletalFrameMap::Elapsed { rate: 2. },
            0.875,
            ModelFramePose {
                current: 0,
                previous: 0,
                back_lerp: 0.,
            },
            Vec3([8., -3., -1.]),
        ),
    ];
    let mut frontend = FrontEnd::load(Limits::default().for_assets(&assets).unwrap()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    for (index, &(mapping, clock, _, _)) in requests.iter().enumerate() {
        assert!(pair(
            &mut frame,
            SceneEntity {
                frame: 5,
                old_frame: 4,
                back_lerp: 0.25,
                sync_base: 0.125,
                ..Default::default()
            },
            part(native, shaders[0]),
            part(replacement, shaders[1]),
            RuleSetId::Quake,
            mapping
        ));
        let mut view = super::skeletal::view(index as u32 * 32);
        view.time_ms = 123_000;
        view.model_time = Some(ThinkTime::Seconds(clock));
        assert!(frame.render_scene(view, &[], &assets, ShaderView::direct()));
    }
    let packet = frame.finish();
    assert_eq!(packet.rejected, 0);
    for (draw, &(_, _, expected, vertex)) in
        super::skeletal::draws(&packet).into_iter().zip(&requests)
    {
        assert_eq!(draw.frames, Some(expected));
        assert_eq!(
            item_frames(&packet, &assets, draw)
                .unwrap()
                .vertex(0)
                .position,
            vertex
        );
        let source = packet.entity(draw.index);
        assert_eq!(
            (
                source.frame,
                source.old_frame,
                source.back_lerp,
                source.sync_base
            ),
            (5, 4, 0.25, 0.125)
        );
    }
}

#[test]
fn composed_capture_capacity_failure_restores_entities_and_nodes_before_retry() {
    let (assets, native, replacement, shaders) = resources();
    let mut frontend = FrontEnd::load(Limits {
        entities: 2,
        model_choices: 9,
        model_joints: 1,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(!frame.add_model_choices(SceneEntity::default(), |capture| {
        let Some(root) = composed(capture, native, replacement, &shaders) else {
            return false;
        };
        capture.submit(root)
    }));
    assert!(!frame.add_model_choices(SceneEntity::default(), |capture| {
        assert!(capture.submit(ModelChoice::part(part(native, shaders[0]))));
        false
    }));
    assert!(pair(
        &mut frame,
        SceneEntity::default(),
        part(native, shaders[0]),
        part(replacement, shaders[1]),
        RuleSetId::Quake,
        SkeletalFrameMap::Wrap
    ));
    assert!(super::model_choices::skin(
        &mut frame,
        SceneEntity::default(),
        native,
        1,
        shaders[1..3].iter().copied().map(Into::into)
    ));
    assert!(frame.render_scene(super::skeletal::view(0), &[], &assets, ShaderView::direct()));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 2);
    assert_eq!(super::skeletal::draws(&packet).len(), 2);
    assert_eq!(packet.entity(0).model, SceneModel::Choice { root: 4 });
    assert_eq!(packet.entity(1).model, SceneModel::Choice { root: 7 });
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(32, 32, &assets).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 2);
    assert!(cpu.inverse_depth().iter().any(|&depth| depth > 0.));
}
