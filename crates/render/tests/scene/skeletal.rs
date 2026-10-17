use super::*;
use qa_core::primitives::{JointPose, SkeletalWeight};
use qa_render::scene::item_frames;
use qa_render::shader::reference::ShaderView;
use qa_render::{CommandList, ModelId, SkeletalMesh, Skeleton};
use std::sync::Arc;

pub(super) fn skeleton(joints: u32) -> Arc<Skeleton> {
    Arc::new(
        Skeleton::new(
            joints,
            [
                JointPose {
                    position: Vec3([8., -2., 0.]),
                    ..Default::default()
                },
                JointPose {
                    position: Vec3([8., 2., 0.]),
                    scale: Vec3([2.; 3]),
                    ..Default::default()
                },
            ]
            .into_iter()
            .flat_map(|pose| std::iter::repeat_n(pose, joints as usize))
            .collect::<Box<_>>(),
        )
        .unwrap(),
    )
}

pub(super) fn mesh(assets: &mut Assets, skeleton: Arc<Skeleton>) -> ModelId {
    let shader = assets
        .register_material(
            "skeleton/red",
            &[Stage {
                rgb_gen: RgbGen::Const([1., 0., 0.]),
                ..Default::default()
            }],
            MaterialSettings {
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let shader = assets
        .register_shader_binding(qa_render::ShaderBinding::material(
            shader,
            LightSource::None,
        ))
        .unwrap();
    let offsets = [[0., -1., -1.], [0., 1., -1.], [0., 0., 1.]].map(Vec3);
    let vertices = offsets.map(|position| Vertex {
        position,
        normal: Vec3([-1., 0., 0.]),
        color: [255; 4],
        ..Default::default()
    });
    let model = assets
        .register_animated_model(&vertices, 3, &[0, 1, 2], shader)
        .unwrap();
    assets
        .set_model_skeleton(
            model,
            SkeletalMesh::new(
                skeleton,
                offsets
                    .map(|offset| SkeletalWeight {
                        joint: 0,
                        bias: 1.,
                        offset,
                    })
                    .into(),
                [0, 1, 2].map(|first| Span { first, count: 1 }).into(),
                Box::new([Vec3([-1., 0., 0.]); 3]),
            )
            .unwrap(),
        )
        .unwrap();
    model
}

pub(super) fn view(x: u32) -> Refdef {
    Refdef {
        viewport: qa_render::Viewport {
            x,
            y: 0,
            width: 32,
            height: 32,
        },
        fov: [90.; 2],
        near: 0.25,
        ..Default::default()
    }
}
pub(super) fn draws(packet: &CommandList) -> Vec<qa_render::DrawItem> {
    packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(packet.draws(view.scene.draws)),
            _ => None,
        })
        .flatten()
        .filter(|draw| draw.kind == DrawKind::Entity)
        .copied()
        .collect()
}
fn pose(model: ModelId, frame: u32, old_frame: u32, back_lerp: f32) -> SceneEntity {
    SceneEntity {
        model: SceneModel::Mesh(model),
        frame,
        old_frame,
        back_lerp,
        ..Default::default()
    }
}

#[test]
fn skeletal_attachment_validates_ranges_and_never_exposes_the_bind_frame_as_an_animation() {
    assert!(Skeleton::new(0, Box::new([JointPose::default()])).is_err());
    assert!(Skeleton::new(2, Box::new([JointPose::default()])).is_err());
    assert!(
        Skeleton::new(
            1,
            Box::new([JointPose {
                orientation: [f32::NAN; 4],
                ..Default::default()
            }])
        )
        .is_err()
    );
    let skeleton = skeleton(1);
    for (joint, range) in [
        (1, Span { first: 0, count: 1 }),
        (0, Span { first: 1, count: 1 }),
    ] {
        assert!(
            SkeletalMesh::new(
                skeleton.clone(),
                Box::new([SkeletalWeight {
                    joint,
                    bias: 1.,
                    offset: Vec3::default(),
                }]),
                Box::new([range]),
                Box::new([Vec3::axis(0, 1.)])
            )
            .is_err()
        );
    }
    // Empty influences have the native zero weighted-loop result.
    assert!(
        SkeletalMesh::new(
            skeleton.clone(),
            Box::new([]),
            Box::new([Span::default()]),
            Box::new([Vec3::default()])
        )
        .is_ok()
    );
    assert!(SkeletalMesh::new(skeleton.clone(), Box::new([]), Box::new([]), Box::new([])).is_ok());
    let mut assets = Assets::load().unwrap();
    let id = mesh(&mut assets, skeleton.clone());
    let model = assets.model(id).unwrap();
    assert_eq!(
        (
            model.frame_count(),
            model.native_frame_count(),
            model.vertices.len()
        ),
        (2, 2, 3)
    );
    assert!(Arc::ptr_eq(model.skeleton().unwrap(), &skeleton));
    assert!(model.frames(0, 0, 0.).is_none());
    assert!(model.posed_frames(&pose(id, 0, 0, 0.)).is_none());
}

#[test]
fn copied_skeletal_views_use_current_scale_and_drive_distinct_cpu_pixels_in_either_order() {
    let mut assets = Assets::load().unwrap();
    let model = mesh(&mut assets, skeleton(1));
    let source_vertices = assets.model(model).unwrap().vertices.clone();
    let requests = [(0, 0, 0.), (1, 1, 0.), (1, 0, 0.5)];
    let expected_first = [
        Vec3([8., -3., -1.]),
        Vec3([8., 0., -2.]),
        Vec3([8., -2., -2.]),
    ];
    let mut reference = None;
    for reverse in [false, true] {
        let limits = Limits::default().for_assets(&assets).unwrap();
        let mut frontend = FrontEnd::load(limits).unwrap();
        let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
        for at in 0..requests.len() {
            let index = if reverse { requests.len() - 1 - at } else { at };
            let (current, previous, back) = requests[index];
            assert!(frame.add_entity(pose(model, current, previous, back)));
            assert!(frame.render_scene(
                view(index as u32 * 32),
                &[],
                &assets,
                ShaderView::direct()
            ));
        }
        let packet = frame.finish();
        assert_eq!(packet.rejected, 0);
        let items = draws(&packet);
        assert_eq!(items.len(), 3);
        for (at, item) in items.into_iter().enumerate() {
            let index = if reverse { 2 - at } else { at };
            let posed = item_frames(&packet, &assets, item).unwrap();
            assert_eq!(posed.vertex(0).position, expected_first[index]);
            assert_eq!(posed.vertex(0).normal, Vec3([-1., 0., 0.]));
            let stored = packet.entity(item.index);
            assert_eq!(
                (stored.frame, stored.old_frame, stored.back_lerp),
                requests[index]
            );
        }
        let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(96, 32, &assets).unwrap();
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.triangles, 3);
        let pixels = (0..3)
            .map(|seat| {
                cpu.pixels()
                    .chunks_exact(96)
                    .flat_map(|row| row[seat * 32..(seat + 1) * 32].iter().copied())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let red = u32::from_le_bytes([255, 0, 0, 255]);
        assert!(pixels.iter().all(|seat| seat.contains(&red)));
        assert_ne!(pixels[0], pixels[1]);
        assert_ne!(pixels[1], pixels[2]);
        if let Some(reference) = &reference {
            assert_eq!(&pixels, reference);
        } else {
            reference = Some(pixels);
        }
    }
    assert_eq!(&*assets.model(model).unwrap().vertices, &*source_vertices);
}

#[test]
fn multipart_meshes_share_one_pose_receipt_and_a_rejected_rig_leaves_the_remaining_capacity_usable()
{
    let mut assets = Assets::load().unwrap();
    let shared = skeleton(1);
    let first = mesh(&mut assets, shared.clone());
    let second = mesh(&mut assets, shared);
    let too_large = mesh(&mut assets, skeleton(2));
    let mut frontend = FrontEnd::load(Limits {
        model_joints: 1,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(pose(too_large, 1, 0, 0.5)));
    assert!(frame.add_entity(pose(first, 1, 0, 0.5)));
    assert!(frame.add_entity(pose(second, 1, 0, 0.5)));
    assert!(frame.render_scene(view(0), &[], &assets, ShaderView::direct()));
    let packet = frame.finish();
    let items = draws(&packet);
    assert_eq!(packet.rejected, 1);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].pose, Some(Span { first: 0, count: 1 }));
    assert_eq!(items[0].pose, items[1].pose);
    assert_eq!(
        item_frames(&packet, &assets, items[0])
            .unwrap()
            .vertex(1)
            .position,
        item_frames(&packet, &assets, items[1])
            .unwrap()
            .vertex(1)
            .position
    );
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(32, 32, &assets).unwrap();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.triangles, 2);
    assert!(cpu.pixels().contains(&u32::from_le_bytes([255, 0, 0, 255])));
}

#[test]
fn active_capacity_staging_preserves_prepared_pose_receipts_on_drop_and_commit() {
    let mut assets = Assets::load().unwrap();
    let model = mesh(&mut assets, skeleton(1));
    let mut frontend = FrontEnd::load(Limits {
        model_joints: 1,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(pose(model, 0, 0, 0.)));
    assert!(frame.render_scene(view(0), &[], &assets, ShaderView::direct()));
    let old = frontend.limits();
    let larger = Limits {
        model_joints: 2,
        ..old
    };
    drop(
        frontend
            .stage_capacity_with_frame(larger, &mut frame)
            .unwrap(),
    );
    assert_eq!(frontend.limits(), old);
    frontend
        .stage_capacity_with_frame(larger, &mut frame)
        .unwrap()
        .commit();
    assert!(frame.add_entity(pose(model, 1, 1, 0.)));
    assert!(frame.render_scene(view(32), &[], &assets, ShaderView::direct()));
    let packet = frame.finish();
    let items = draws(&packet);
    assert_eq!(
        items
            .iter()
            .map(|draw| draw.pose.unwrap().first)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(
        item_frames(&packet, &assets, items[0])
            .unwrap()
            .vertex(0)
            .position,
        Vec3([8., -3., -1.])
    );
    assert_eq!(
        item_frames(&packet, &assets, items[1])
            .unwrap()
            .vertex(0)
            .position,
        Vec3([8., 0., -2.])
    );
}

#[test]
fn skeletal_portal_and_parent_share_the_prepared_pose_instead_of_reading_raw_bind_vertices() {
    let mut assets = Assets::load().unwrap();
    let model = mesh(&mut assets, skeleton(1));
    let portal = assets
        .register_material(
            "skeleton/portal",
            &[Stage {
                alpha_gen: AlphaGen::Const(0.),
                blend: Some(StageBlend::ALPHA),
                depth_write: false,
                ..Default::default()
            }],
            MaterialSettings {
                portal: true,
                sort: 1.,
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let world = world(
        &mut assets,
        &[SurfaceMaterial {
            shader: qa_render::ShaderBinding::material(portal, LightSource::None),
            ..Default::default()
        }],
    );
    let presentation = presentation(&assets, world);
    let mut scratch = qa_render::world::WorldView::load(assets.world(world).unwrap());
    // Exactly one joint: child and parent must reuse the original portal-probe receipt.
    let mut frontend = FrontEnd::load(Limits {
        model_joints: 1,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(pose(model, 1, 0, 0.5)));
    assert!(frame.add_entity(SceneEntity {
        origin: Vec3([1., 0., 0.]),
        model: SceneModel::Portal {
            camera: Vec3([1., 0., 0.]),
            rotation: 0
        },
        ..Default::default()
    }));
    assert!(frame.add_world(
        world,
        presentation,
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.render_world_scene(
        world,
        presentation,
        view(0),
        &[],
        &mut scratch,
        &assets,
        ShaderView::direct()
    ));
    let packet = frame.finish();
    let views = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(views.len(), 2);
    assert!(views[0].refdef.portal);
    assert!(!views[1].refdef.portal);
    assert_eq!(packet.rejected, 0);
    let items = draws(&packet);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].pose, items[1].pose);
    for item in items {
        assert_eq!(
            item_frames(&packet, &assets, item)
                .unwrap()
                .vertex(0)
                .position,
            Vec3([8., -2., -2.])
        );
    }
}

#[test]
fn weighted_vertices_use_native_rotation_rows_and_do_not_scale_their_normals() {
    let mut assets = Assets::load().unwrap();
    let skeleton = Arc::new(
        Skeleton::new(
            2,
            Box::new([
                JointPose {
                    position: Vec3([4., 0., 0.]),
                    scale: Vec3([2.; 3]),
                    ..Default::default()
                },
                JointPose {
                    position: Vec3([8., 0., 0.]),
                    orientation: [0., 0., 1., 0.],
                    scale: Vec3([3.; 3]),
                },
            ]),
        )
        .unwrap(),
    );
    let model = assets
        .register_model(&[Vertex::default(); 3], &[0, 1, 2], ShaderBindingId(0))
        .unwrap();
    assets
        .set_model_skeleton(
            model,
            SkeletalMesh::new(
                skeleton,
                Box::new([
                    SkeletalWeight {
                        joint: 0,
                        bias: 0.25,
                        offset: Vec3([0., 1., 0.]),
                    },
                    SkeletalWeight {
                        joint: 1,
                        bias: 0.75,
                        offset: Vec3([0., 1., 0.]),
                    },
                ]),
                Box::new([Span { first: 0, count: 2 }; 3]),
                Box::new([Vec3([0., 1., 0.]); 3]),
            )
            .unwrap(),
        )
        .unwrap();
    let mut frontend = FrontEnd::load(Limits {
        model_joints: 2,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(pose(model, 0, 0, 0.)));
    assert!(frame.render_scene(view(0), &[], &assets, ShaderView::direct()));
    let packet = frame.finish();
    let vertex = item_frames(&packet, &assets, draws(&packet)[0])
        .unwrap()
        .vertex(0);
    assert_eq!(vertex.position, Vec3([7., -1.75, 0.]));
    assert_eq!(vertex.normal, Vec3([0., -0.5, 0.]));
}

#[test]
fn failed_portal_attempt_retires_tail_pose_receipts_before_other_rigs_reuse_the_arena() {
    let mut assets = Assets::load().unwrap();
    let first = mesh(&mut assets, skeleton(1));
    let other = mesh(
        &mut assets,
        Arc::new(
            Skeleton::new(
                1,
                Box::new([JointPose {
                    position: Vec3([8., 12., 0.]),
                    ..Default::default()
                }]),
            )
            .unwrap(),
        ),
    );
    let portal = assets
        .register_material(
            "skeleton/rejected-portal",
            &[Stage {
                alpha_gen: AlphaGen::Const(0.),
                blend: Some(StageBlend::ALPHA),
                depth_write: false,
                ..Default::default()
            }],
            MaterialSettings {
                portal: true,
                sort: 1.,
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let world = world(
        &mut assets,
        &[SurfaceMaterial {
            shader: qa_render::ShaderBinding::material(portal, LightSource::None),
            ..Default::default()
        }],
    );
    let presentation = presentation(&assets, world);
    let mut scratch = qa_render::world::WorldView::load(assets.world(world).unwrap());
    let mut frontend = FrontEnd::load(Limits {
        area_bytes: 1,
        model_joints: 2,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(pose(first, 1, 0, 0.5)));
    assert!(frame.add_entity(SceneEntity {
        origin: Vec3([1., 0., 0.]),
        model: SceneModel::Portal {
            camera: Vec3([1., 0., 0.]),
            rotation: 0
        },
        ..Default::default()
    }));
    assert!(frame.add_world(
        world,
        presentation,
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(!frame.render_world_scene(
        world,
        presentation,
        view(0),
        &[0, 0],
        &mut scratch,
        &assets,
        ShaderView::direct()
    ));
    frame.clear_scene();
    assert!(frame.add_entity(pose(other, 0, 0, 0.)));
    assert!(frame.render_scene(view(0), &[], &assets, ShaderView::direct()));
    assert!(frame.add_entity(pose(first, 1, 0, 0.5)));
    assert!(frame.render_scene(view(32), &[], &assets, ShaderView::direct()));
    let packet = frame.finish();
    let items = draws(&packet);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].pose, Some(Span { first: 0, count: 1 }));
    assert_eq!(items[1].pose, Some(Span { first: 1, count: 1 }));
    assert_eq!(
        item_frames(&packet, &assets, items[0])
            .unwrap()
            .vertex(0)
            .position,
        Vec3([8., 11., -1.])
    );
    assert_eq!(
        item_frames(&packet, &assets, items[1])
            .unwrap()
            .vertex(0)
            .position,
        Vec3([8., -2., -2.])
    );
}

#[test]
fn equal_frames_and_zero_backlerp_keep_the_native_direct_nonunit_orientation() {
    let mut assets = Assets::load().unwrap();
    let skeleton = Arc::new(
        Skeleton::new(
            1,
            Box::new([
                JointPose {
                    position: Vec3([8., 0., 0.]),
                    orientation: [0., 0., 0., 0.5],
                    ..Default::default()
                },
                JointPose {
                    position: Vec3([8., 0., 0.]),
                    orientation: [0., 0., 0., 0.5],
                    ..Default::default()
                },
            ]),
        )
        .unwrap(),
    );
    let model = mesh(&mut assets, skeleton);
    let mut frontend = FrontEnd::load(Limits {
        model_joints: 3,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    for (current, previous, back) in [(0, 0, 0.5), (1, 0, 0.), (1, 0, 1.)] {
        assert!(frame.add_entity(pose(model, current, previous, back)));
    }
    assert!(frame.render_scene(view(0), &[], &assets, ShaderView::direct()));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 0);
    for draw in draws(&packet) {
        let vertex = item_frames(&packet, &assets, draw).unwrap().vertex(0);
        assert_eq!(vertex.position, Vec3([8., 0.5, 0.5]));
        assert_eq!(vertex.normal, Vec3([0.5, 0., 0.]));
    }
}
