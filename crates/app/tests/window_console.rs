use qa_app::{Runtime, host::FrameHost};
use qa_console::{commands::Console, views::Context};
use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent};
use qa_session::timing::TickRate;

fn push(host: &mut FrameHost, kind: EventKind<'_>, ms: u64) {
    host.queue
        .push(SysEvent {
            time: EventTime::from_milliseconds(ms),
            kind,
        })
        .unwrap();
}

fn key(host: &mut FrameHost, control: u16, down: bool, repeat: bool, ms: u64) {
    push(
        host,
        EventKind::Key {
            device: DeviceId::Keyboard(0),
            code: control,
            symbol: 0,
            down,
            repeat,
        },
        ms,
    );
}

fn input_host() -> FrameHost {
    FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap()
}

fn forward(host: &mut FrameHost, ms: u64) -> f32 {
    let handles = qa_console::input_settings::InputHandles::load(&host.console.cvars).unwrap();
    let policy = handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake2);
    host.runtime
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(ms), &[policy; SeatId::COUNT])[0]
        .movement[0]
}

#[test]
fn rejected_release_closes_executed_press_at_its_original_time() {
    let mut host = input_host();
    push(&mut host, EventKind::Time, 0);
    key(&mut host, 26, true, false, 10);
    host.drain(&mut Default::default());
    host.execute_commands();
    assert_eq!(forward(&mut host, 20), 100.0);
    host.console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - 1),
            Context::default(),
        )
        .unwrap();
    key(&mut host, 26, false, false, 30);
    host.drain(&mut Default::default());
    host.execute_commands();
    assert_eq!(host.runtime.server.events.rejected_publications(), 1);
    assert_eq!(forward(&mut host, 40), 100.0);
    assert_eq!(forward(&mut host, 60), 0.0);
}

#[test]
fn rejected_release_cancels_press_still_queued_in_the_same_drain() {
    let mut host = input_host();
    push(&mut host, EventKind::Time, 0);
    // Leave room for this exact press, but not its release in the same drain.
    let press_bytes = "+forward 26 10\n".len();
    host.console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - press_bytes - 1),
            Context::default(),
        )
        .unwrap();
    key(&mut host, 26, true, false, 10);
    key(&mut host, 26, false, false, 20);
    host.drain(&mut Default::default());
    host.execute_commands();
    assert_eq!(host.runtime.server.events.rejected_publications(), 1);
    assert_eq!(forward(&mut host, 40), 0.0);
    assert_eq!(forward(&mut host, 60), 0.0);
}

#[test]
fn admitted_quick_tap_keeps_the_native_elapsed_interval() {
    let mut host = input_host();
    push(&mut host, EventKind::Time, 0);
    key(&mut host, 26, true, false, 10);
    key(&mut host, 26, false, false, 20);
    host.drain(&mut Default::default());
    host.execute_commands();
    assert_eq!(forward(&mut host, 40), 50.0);
    assert_eq!(forward(&mut host, 60), 0.0);
    assert_eq!(host.runtime.server.events.rejected_publications(), 0);
}

#[test]
fn queued_window_text_reaches_the_one_console_and_repeat_edits_do_not_move() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let input_handles =
        qa_console::input_settings::InputHandles::load(&host.console.cvars).unwrap();
    push(&mut host, EventKind::Time, 0);
    key(&mut host, 53, true, false, 1);
    key(&mut host, 53, true, true, 2);
    push(
        &mut host,
        EventKind::Text {
            device: DeviceId::Keyboard(0),
            text: "cg_fov 123x",
        },
        3,
    );
    key(&mut host, 42, true, true, 4);
    key(&mut host, 26, true, false, 5);
    key(&mut host, 40, true, false, 6);
    push(&mut host, EventKind::Time, 20);
    host.drain(&mut Default::default());
    host.execute_commands();
    assert_eq!(host.console.editor.seat(), Some(SeatId::FIRST));
    assert_eq!(
        host.console
            .cvars
            .value(host.console.cvars.find("cg_fov").unwrap()),
        123.0
    );
    // This fixture selects the native Q2 input policy: console capture produces
    // no movement or action bits. Q3 intentionally emits its native TALK bit.
    let command = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(20),
        &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake2);
            qa_core::sys_events::SeatId::COUNT],
    )[0];
    assert_eq!(command.movement, [0.0; 3]);
    assert_eq!(command.buttons, 0);
    // A key first pressed in the console does not acquire a game binding when
    // the catcher closes. A fresh press does, via the same physical held state.
    key(&mut host, 41, true, false, 21);
    key(&mut host, 26, false, false, 22);
    key(&mut host, 26, true, false, 23);
    push(&mut host, EventKind::Time, 40);
    host.drain(&mut Default::default());
    // Both native and custom bindings now execute through the queued console.
    host.execute_commands();
    let command = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(40),
        &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake2);
            qa_core::sys_events::SeatId::COUNT],
    )[0];
    assert!(command.movement[0] > 0.0);
}

#[test]
fn full_command_buffer_reports_queued_stdin_rejection_through_the_output_ring() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.console
        .append(
            &"x".repeat(qa_console::command_buffer::CAPACITY - 1),
            Context::default(),
        )
        .unwrap();
    push(&mut host, EventKind::ConsoleLine("cg_fov 123"), 1);
    host.drain(&mut Default::default());
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
    let record = host.runtime.server.events.next(&mut batch).unwrap();
    let qa_core::events::FrameEvent::Print(print) = record.event else {
        panic!("stdin rejection must use the one print ring");
    };
    assert!(
        host.runtime
            .server
            .events
            .texts
            .get(print.text)
            .unwrap()
            .starts_with(b"Input command:")
    );
}

#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod source_files;

#[test]
fn queued_stdin_bind_and_editor_exec_keep_the_connections_publication_time_files() {
    use qa_app::{
        client_policy::ClientPolicy, client_resources::ClientResources, map::SpawnAnchor,
    };
    use qa_core::primitives::{RuleSetId, Vec3};
    use qa_network::commands::packet::Protocol;
    let mut low = source_files::Fixture::new(&[("source.cfg", b"cg_fov 91\n".to_vec())]);
    let high = source_files::Fixture::new(&[("source.cfg", b"cg_fov 131\n".to_vec())]);
    let low_mount = low.vfs.mounts().next().unwrap().id;
    let root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = low.vfs.mount_directory(&root, 10).unwrap();
    let low_view = low.vfs.search_view([low_mount]).unwrap();
    let high_view = low.vfs.search_view([high_mount]).unwrap();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut low.vfs);
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(Some(RuleSetId::Quake3), None, None, None, None).unwrap(),
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    host.console
        .append("bind x \"exec source.cfg\"\n", Context::default())
        .unwrap();
    host.execute_commands();
    for route in 0..3 {
        host.runtime.client_resources[client.0 as usize] = ClientResources::load(
            client,
            Protocol::Quake3_68,
            RuleSetId::Quake3,
            &host.runtime.vfs,
            Some(&low_view),
        )
        .unwrap();
        match route {
            0 => push(&mut host, EventKind::ConsoleLine("exec source.cfg"), 1),
            1 => {
                key(&mut host, 27, true, false, 1);
                key(&mut host, 27, false, false, 2);
            }
            _ => {
                host.console.editor.set_active(SeatId::FIRST, true);
                push(
                    &mut host,
                    EventKind::Text {
                        device: DeviceId::Keyboard(0),
                        text: "exec source.cfg",
                    },
                    1,
                );
                key(&mut host, 40, true, false, 2);
            }
        }
        host.drain(&mut Default::default());
        // Changing the connection after admission cannot retarget queued exec.
        host.runtime.client_resources[client.0 as usize] = ClientResources::load(
            client,
            Protocol::Quake3_68,
            RuleSetId::Quake3,
            &host.runtime.vfs,
            Some(&high_view),
        )
        .unwrap();
        host.execute_commands();
        let fov = host.console.cvars.find("cg_fov").unwrap();
        assert_eq!(host.console.cvars.value(fov), 91., "input route {route}");
        host.console
            .append("cg_fov 131\n", Context::default())
            .unwrap();
        host.execute_commands();
    }
}
