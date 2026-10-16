use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    renderer::SceneComposer,
};
use qa_console::{commands::Console, views::Context};
use qa_core::sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue};
use qa_render::{
    Assets, Limits,
    scene::{Command, Draw2d},
};
use qa_session::timing::TickRate;
use std::time::Duration;

fn scene(commands: usize) -> SceneComposer {
    SceneComposer::load(
        Limits {
            commands,
            entities: 1,
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
