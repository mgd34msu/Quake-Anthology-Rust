use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    renderer::SceneComposer,
};
use qa_console::{commands::Console, views::Context};
use qa_core::sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue};
use qa_render::{
    Assets, Limits,
    material::world_load::{LoadedWorld, LoadedWorldPresentation, WorldPresentationProfile},
    scene::{Command, Draw2d},
    world::geometry::{GeometryPartition, PatchStats, WorldGeometry},
};
use qa_session::timing::TickRate;
use std::time::Duration;

fn scene(commands: usize) -> SceneComposer {
    SceneComposer::load(
        Limits {
            commands,
            entities: 1,
            model_choices: 0,
            model_joints: 0,
            polys: 1,
            vertices: 3,
            lights: 1,
            particles: 1,
            area_bytes: 1,
            surfaces: 1,
            draws: 0,
        },
        &mut Assets::load().unwrap(),
        vec![],
    )
    .unwrap()
    .0
}

fn empty_world(assets: &mut Assets) -> LoadedWorld {
    use qa_core::primitives::{Bounds, RuleSetId};
    use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};
    let visibility = VisibilityWorld::load(
        vec![],
        vec![],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds: Bounds::default(),
            surfaces: SurfaceSpan::default(),
        }],
        vec![],
        0,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    let world = assets
        .register_world(
            WorldGeometry {
                partition: GeometryPartition::Unpartitioned,
                world_has_lightdata: false,
                vertices: vec![],
                indices: vec![],
                boundaries: vec![],
                surfaces: vec![],
                light_samples: vec![],
                models: vec![],
                patch_stats: PatchStats::default(),
            },
            visibility,
            &[qa_render::world::WorldPresentationInput::new(&[])],
        )
        .unwrap();
    LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: assets.world(world).unwrap().presentations()[0],
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            lighting: Default::default(),
            compile_policy: Default::default(),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    }
}

#[test]
fn in_memory_world_metadata_is_adopted_once_and_stage_requires_that_same_registration() {
    let mut assets = Assets::load().unwrap();
    let first = empty_world(&mut assets);
    let world = first.world;
    let entry = first.presentations[0];
    assert!(assets.world_presentation_registration(entry.id).is_none());
    let (mut owner, _) = SceneComposer::load(Limits::default(), &mut assets, vec![first]).unwrap();
    assert_eq!(
        assets.world_presentation_registration(entry.id),
        Some(entry)
    );
    let mut changed = entry;
    changed.profile.q2_modulate = 2.;
    assert!(
        assets
            .register_world_presentation_metadata(world, &[changed])
            .is_err()
    );
    assert_eq!(
        assets.world_presentation_registration(entry.id),
        Some(entry)
    );

    let second = empty_world(&mut assets);
    let candidate = second.presentations[0];
    assert!(owner.stage_world(&assets, &second).is_err());
    assert!(
        assets
            .register_world_presentation_metadata(second.world, &[candidate, entry])
            .is_err()
    );
    assert!(
        assets
            .world_presentation_registration(candidate.id)
            .is_none()
    );
    assets
        .register_world_presentation_metadata(second.world, &second.presentations)
        .unwrap();
    let mut inconsistent = second;
    inconsistent.presentations[0].defaults.identity_light = 0.5;
    assert!(owner.stage_world(&assets, &inconsistent).is_err());
    inconsistent.presentations[0] = candidate;
    owner.stage_world(&assets, &inconsistent).unwrap().commit();
    assert!(owner.begin_frame());
    let packet = owner.finish_frame().unwrap();
    assert_eq!(packet.frame, 1);
    assert!(owner.recycle(packet).is_ok());
}

#[test]
fn cold_world_stage_waits_for_the_existing_frame_and_both_consumers() {
    let mut assets = Assets::load().unwrap();
    let first = empty_world(&mut assets);
    let (mut owner, _) = SceneComposer::load(Limits::default(), &mut assets, vec![first]).unwrap();
    let second = empty_world(&mut assets);
    assets
        .register_world_presentation_metadata(second.world, &second.presentations)
        .unwrap();
    assert!(owner.begin_frame());
    assert!(owner.stage_world(&assets, &second).is_err());
    let first_packet = owner.finish_frame().unwrap();
    assert!(owner.stage_world(&assets, &second).is_err());
    assert!(owner.begin_frame());
    let second_packet = owner.finish_frame().unwrap();
    assert!(owner.stage_world(&assets, &second).is_err());
    assert!(owner.recycle(first_packet).is_ok());
    assert!(owner.stage_world(&assets, &second).is_err());
    assert_eq!(second_packet.frame, 2);
    assert!(matches!(second_packet.commands(), [Command::Clear(_)]));
    assert!(owner.recycle(second_packet).is_ok());
    owner.stage_world(&assets, &second).unwrap().commit();
    assert!(owner.begin_frame());
    let packet = owner.finish_frame().unwrap();
    assert_eq!(packet.frame, 3);
    assert!(owner.recycle(packet).is_ok());
    // Existing worlds stage frontend capacity only. Dropping that candidate
    // preserves the same command-list owner and frame count.
    drop(owner.stage_world(&assets, &second).unwrap());
    assert!(owner.begin_frame());
    let packet = owner.finish_frame().unwrap();
    assert_eq!(packet.frame, 4);
    assert!(owner.recycle(packet).is_ok());
}

#[test]
fn dropped_world_stage_allows_suffix_rollback_and_clean_same_id_retry() {
    let mut assets = Assets::load().unwrap();
    let (mut owner, _) = SceneComposer::load(Limits::default(), &mut assets, vec![]).unwrap();
    let first = empty_world(&mut assets);
    assets
        .register_world_presentation_metadata(first.world, &first.presentations)
        .unwrap();
    owner.stage_world(&assets, &first).unwrap().commit();
    let retained = assets.counts();
    let mut rejected = empty_world(&mut assets);
    assets
        .register_world_presentation_metadata(rejected.world, &rejected.presentations)
        .unwrap();
    let candidate_id = rejected.world;
    let original = rejected.presentations[0].id;
    rejected.presentations[0].id = first.presentations[0].id;
    assert!(owner.stage_world(&assets, &rejected).is_err());
    rejected.presentations[0].id = original;
    drop(owner.stage_world(&assets, &rejected).unwrap());
    assets.discard_registration_suffix(retained).unwrap();
    assert!(assets.world(first.world).is_some());
    assert!(assets.world(candidate_id).is_none());

    let retry = empty_world(&mut assets);
    assets
        .register_world_presentation_metadata(retry.world, &retry.presentations)
        .unwrap();
    assert_eq!(retry.world, candidate_id);
    owner.stage_world(&assets, &retry).unwrap().commit();
    let third = empty_world(&mut assets);
    assets
        .register_world_presentation_metadata(third.world, &third.presentations)
        .unwrap();
    owner.stage_world(&assets, &third).unwrap().commit();
    for frame in 1..=4 {
        assert!(owner.begin_frame());
        let packet = owner.finish_frame().unwrap();
        assert_eq!(packet.frame, frame);
        assert!(owner.recycle(packet).is_ok());
    }
}

#[test]
fn scene_packets_retain_owner_and_reset_bounded_submissions_on_reuse() {
    let mut owner = scene(2);
    assert!(owner.begin_frame());
    let frame = owner.frame_mut().unwrap();
    assert!(frame.draw_2d(Draw2d {
        rect: [1.0, 2.0, 3.0, 4.0],
        ..Draw2d::default()
    }));
    assert!(!frame.draw_2d(Draw2d::default()));
    let packet = owner.finish_frame().unwrap();
    assert_eq!(packet.rejected, 1);
    assert!(
        matches!(packet.commands()[1], Command::Draw2d(draw) if draw.rect == [1.0, 2.0, 3.0, 4.0])
    );
    let mut other = scene(2);
    let packet = other
        .recycle(packet)
        .err()
        .expect("other scene cannot adopt the packet");
    assert!(owner.recycle(packet).is_ok());
    for _ in 0..8 {
        assert!(owner.begin_frame());
        let packet = owner.finish_frame().unwrap();
        assert_eq!(packet.rejected, 0);
        assert_eq!(packet.commands().len(), 1);
        assert!(owner.recycle(packet).is_ok());
    }
}

struct Source {
    time: EventTime,
    render: bool,
    submissions: usize,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, span: Duration) -> Duration {
        self.time.0 += span.as_nanos() as u64;
        self.begin_frame()
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn render(
        &mut self,
        scene: Option<&mut SceneComposer>,
        _: Option<qa_app::model_assets::ModelView<'_>>,
        _native: &dyn qa_render::shader::reference::NativeShaders,
        _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        let scene = scene.unwrap();
        assert!(
            scene.frame_mut().is_some(),
            "session opened the shared frame before client dispatch"
        );
        if self.render {
            let packet = scene.finish_frame().unwrap();
            assert!(scene.recycle(packet).is_ok());
            self.submissions += 1;
        }
    }
}

#[test]
fn host_reuses_one_scene_owner_after_presented_and_headless_frames() {
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.scene = Some(scene(2));
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let mut source = Source {
        time: EventTime::from_milliseconds(16),
        render: false,
        submissions: 0,
    };
    for index in 0..8 {
        source.render = index % 2 != 0;
        host.frame(&mut source, true);
        assert!(
            host.runtime.scene.as_mut().unwrap().frame_mut().is_none(),
            "unused headless list retired"
        );
        source.time = EventTime::from_milliseconds((index + 2) * 16);
    }
    assert_eq!(source.submissions, 4);
}
