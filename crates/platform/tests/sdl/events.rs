use super::*;

fn key(window: u32, which: u32, repeat: bool) -> SdlEvent {
    SdlEvent {
        key: SdlKey {
            common: SdlCommon {
                kind: 0x300,
                ..SdlCommon::default()
            },
            window,
            which,
            scancode: 26,
            key: 119,
            down: true,
            repeat,
            ..SdlKey::default()
        },
    }
}
fn window_event(window: u32, kind: u32) -> SdlEvent {
    SdlEvent {
        window: SdlWindow {
            common: SdlCommon {
                kind,
                ..SdlCommon::default()
            },
            window,
            ..SdlWindow::default()
        },
    }
}
fn device_event(which: u32, kind: u32) -> SdlEvent {
    SdlEvent {
        device: SdlDevice {
            common: SdlCommon {
                kind,
                ..SdlCommon::default()
            },
            which,
        },
    }
}

#[test]
fn keys_keep_physical_ids_and_text_keeps_unknown_identity() {
    let mut input = SdlInput::new(42, true);
    for (id, repeat) in [(17, false), (28, true), (0, false)] {
        let event = key(42, id, repeat);
        assert_eq!(
            unsafe { input.decode(&event) },
            Some(EventKind::Key {
                device: DeviceId::Keyboard(id),
                code: 26,
                symbol: 119,
                down: true,
                repeat,
            })
        );
    }
    let text = SdlEvent {
        text: SdlText {
            common: SdlCommon {
                kind: 0x303,
                ..SdlCommon::default()
            },
            window: 42,
            text: c"aλ".as_ptr(),
        },
    };
    assert_eq!(
        unsafe { input.decode(&text) },
        Some(EventKind::Text {
            device: DeviceId::Keyboard(0),
            text: "aλ"
        })
    );
    let other_key = key(99, 17, false);
    assert_eq!(unsafe { input.decode(&other_key) }, None);
    let other_focus = window_event(99, 0x20f);
    assert_eq!(unsafe { input.decode(&other_focus) }, None);
    assert!(input.focused);
    let other_close = window_event(99, 0x210);
    assert_eq!(unsafe { input.decode(&other_close) }, None);
    let own_close = window_event(42, 0x210);
    assert_eq!(unsafe { input.decode(&own_close) }, Some(EventKind::Quit));
}

#[test]
fn mouse_remainders_are_per_device_and_removal_does_not_clear_other_mice() {
    let mut input = SdlInput::new(42, true);
    input.relative = true;
    for id in [17, 28] {
        assert_eq!(input.motion(id, [0.75, -0.75]), Some([0, 0]));
    }
    let mouse_removed = device_event(17, 0x405);
    assert_eq!(
        unsafe { input.decode(&mouse_removed) },
        Some(EventKind::DeviceRemoved(DeviceId::Mouse(17)))
    );
    assert_eq!(input.motion(17, [0.75, -0.75]), Some([0, 0]));
    assert_eq!(input.motion(28, [0.75, -0.75]), Some([1, -1]));
    let keyboard_removed = device_event(39, 0x306);
    assert_eq!(
        unsafe { input.decode(&keyboard_removed) },
        Some(EventKind::DeviceRemoved(DeviceId::Keyboard(39)))
    );
    let controller_removed = device_event(50, 0x654);
    assert_eq!(
        unsafe { input.decode(&controller_removed) },
        Some(EventKind::DeviceRemoved(DeviceId::Controller(50)))
    );
    let motion = SdlEvent {
        motion: SdlMotion {
            common: SdlCommon {
                kind: 0x400,
                ..SdlCommon::default()
            },
            window: 42,
            which: 17,
            dx: 0.75,
            dy: -0.75,
            ..SdlMotion::default()
        },
    };
    assert_eq!(
        unsafe { input.decode(&motion) },
        Some(EventKind::Mouse {
            device: DeviceId::Mouse(17),
            dx: 1,
            dy: -1
        })
    );
    let other = SdlEvent {
        motion: SdlMotion {
            common: SdlCommon {
                kind: 0x400,
                ..SdlCommon::default()
            },
            window: 99,
            which: 17,
            dx: 10.0,
            ..SdlMotion::default()
        },
    };
    assert_eq!(unsafe { input.decode(&other) }, None);
}

#[test]
fn bounded_residuals_reject_excess_and_invalid_motion_without_aliasing() {
    let mut input = SdlInput::new(42, true);
    for id in 1..=MAX_INPUT_DEVICES as u32 {
        assert_eq!(input.motion(id, [0.75, 0.0]), Some([0, 0]));
    }
    assert_eq!(input.motion(999, [0.75, 0.0]), None);
    assert_eq!(input.motion(1, [f32::NAN, 0.0]), None);
    assert_eq!(input.dropped_mouse, 2);
    assert_eq!(input.motion(1, [0.75, 0.0]), Some([1, 0]));
}

#[test]
fn capture_waits_for_queue_room_and_focus_loss_releases_without_another_slot() {
    let mut input = SdlInput::new(42, true);
    input.capture_requested = true;
    let mut queue = SysEventQueue::load(2, 8).unwrap();
    queue
        .push(SysEvent {
            time: EventTime(1),
            kind: EventKind::Focus(true),
        })
        .unwrap();
    let mut calls = Vec::new();
    let mut apply = |enabled| {
        calls.push(enabled);
        true
    };
    input.reconcile_capture(&mut queue, EventTime(2), true, &mut apply);
    assert!(!input.relative);
    assert_eq!(queue.pop().unwrap().kind, EventKind::Focus(true));
    input.reconcile_capture(&mut queue, EventTime(3), true, &mut apply);
    assert!(input.relative);
    assert_eq!(queue.pop().unwrap().kind, EventKind::MouseCapture(true));
    assert_eq!(input.motion(17, [0.75, 0.0]), Some([0, 0]));

    let lost = window_event(42, 0x20f);
    queue
        .push(SysEvent {
            time: EventTime(4),
            kind: unsafe { input.decode(&lost) }.unwrap(),
        })
        .unwrap();
    assert!(!queue.has_ordinary_room());
    input.reconcile_capture(&mut queue, EventTime(4), false, &mut apply);
    assert!(!input.relative);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.pop().unwrap().kind, EventKind::Focus(false));
    let unfocused_key = key(42, 17, false);
    assert_eq!(unsafe { input.decode(&unfocused_key) }, None);
    let gained = window_event(42, 0x20e);
    queue
        .push(SysEvent {
            time: EventTime(5),
            kind: unsafe { input.decode(&gained) }.unwrap(),
        })
        .unwrap();
    input.reconcile_capture(&mut queue, EventTime(5), true, &mut apply);
    assert!(!input.relative);
    queue.pop().unwrap();
    input.reconcile_capture(&mut queue, EventTime(6), true, &mut apply);
    assert!(input.relative);
    assert_eq!(queue.pop().unwrap().kind, EventKind::MouseCapture(true));
    assert_eq!(input.motion(17, [0.75, 0.0]), Some([0, 0]));
    input.capture_requested = false;
    input.reconcile_capture(&mut queue, EventTime(7), true, &mut apply);
    assert!(!input.relative);
    assert_eq!(queue.pop().unwrap().kind, EventKind::MouseCapture(false));
    assert_eq!(calls, [true, false, true, false]);
    assert_eq!(queue.rejected(), 0);
}

#[test]
fn failed_capture_is_counted_and_not_published_as_success() {
    let mut input = SdlInput::new(42, true);
    input.capture_requested = true;
    let mut queue = SysEventQueue::load(2, 8).unwrap();
    input.reconcile_capture(&mut queue, EventTime(1), true, &mut |_| false);
    assert_eq!(input.capture_errors, 1);
    assert!(!input.relative);
    assert!(queue.is_empty());
}

#[test]
fn initially_unfocused_window_does_not_capture_until_its_focus_event() {
    let mut input = SdlInput::new(42, false);
    input.capture_requested = true;
    let mut queue = SysEventQueue::load(3, 8).unwrap();
    let mut calls = Vec::new();
    let mut apply = |enabled| {
        calls.push(enabled);
        true
    };
    input.reconcile_capture(&mut queue, EventTime(1), true, &mut apply);
    assert!(!input.relative);
    assert!(queue.is_empty());
    let gained = window_event(42, 0x20e);
    queue
        .push(SysEvent {
            time: EventTime(2),
            kind: unsafe { input.decode(&gained) }.unwrap(),
        })
        .unwrap();
    input.reconcile_capture(&mut queue, EventTime(2), true, &mut apply);
    assert!(input.relative);
    assert_eq!(queue.pop().unwrap().kind, EventKind::Focus(true));
    assert_eq!(queue.pop().unwrap().kind, EventKind::MouseCapture(true));
    assert_eq!(calls, [true]);
}

#[test]
fn failed_sync_restores_previous_mode_and_does_not_claim_capture() {
    use std::cell::RefCell;
    let calls = RefCell::new(Vec::new());
    let mut syncs = 0;
    let mut input = SdlInput::new(42, true);
    input.capture_requested = true;
    let mut queue = SysEventQueue::load(2, 8).unwrap();
    input.reconcile_capture(&mut queue, EventTime(1), true, &mut |desired| {
        synchronized_capture(
            false,
            desired,
            |mode| {
                calls
                    .borrow_mut()
                    .push(if mode { "enable" } else { "disable" });
                true
            },
            || {
                calls.borrow_mut().push("sync");
                syncs += 1;
                syncs == 2
            },
        )
    });
    assert_eq!(*calls.borrow(), ["enable", "sync", "disable", "sync"]);
    assert_eq!(input.capture_errors, 1);
    assert!(!input.relative);
    assert!(queue.is_empty());
}
