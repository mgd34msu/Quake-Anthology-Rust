use qa_content::journal::{Recorder, Replay};
use qa_core::{primitives::ClientId, sys_events::*};
use qa_formats::FormatError;

#[test]
fn owned_files_replay_two_drains_and_retain_payloads_across_queue_backpressure() {
    let directory = std::env::temp_dir().join(format!("qa-journal-{}", std::process::id()));
    qa_platform::files::create_dir_all(&directory).unwrap();
    let path = directory.join("events.qaj");
    if path.exists() {
        qa_platform::files::remove_file(&path).unwrap();
    }
    let mut recorder = Recorder::create(&path, 4096).unwrap();
    let events = [
        SysEvent {
            time: EventTime(1),
            kind: EventKind::ConsoleLine("echo preserved"),
        },
        SysEvent {
            time: EventTime(2),
            kind: EventKind::Text {
                device: DeviceId::Keyboard(42),
                text: "ééé",
            },
        },
        SysEvent {
            time: EventTime(3),
            kind: EventKind::Time,
        },
        SysEvent {
            time: EventTime(4),
            kind: EventKind::Packet {
                socket: 1,
                from: Peer::Loopback(ClientId(u32::MAX)),
                bytes: &[0, 255, 4],
            },
        },
        SysEvent {
            time: EventTime(5),
            kind: EventKind::Time,
        },
    ];
    for event in events {
        recorder.append(event).unwrap();
    }
    recorder.flush().unwrap();
    assert_eq!(recorder.events(), 5);
    assert!(matches!(
        Recorder::create(&path, 4096),
        Err(FormatError::Io(std::io::ErrorKind::AlreadyExists))
    ));
    let mut replay = Replay::open(&path, 1024).unwrap();
    let mut queue = SysEventQueue::load(2, 1024).unwrap();
    assert!(replay.enqueue_drain(&mut queue).unwrap());
    assert_eq!(queue.pop().unwrap(), events[0]);
    assert_eq!(replay.events(), 1);
    // The rejected text is still present, with its original bytes and device.
    assert_eq!(replay.peek().unwrap().unwrap(), events[1]);
    assert!(replay.enqueue_drain(&mut queue).unwrap());
    assert_eq!(queue.pop().unwrap(), events[1]);
    assert_eq!(queue.pop().unwrap(), events[2]);
    assert!(replay.enqueue_drain(&mut queue).unwrap());
    assert_eq!(queue.pop().unwrap(), events[3]);
    assert_eq!(queue.pop().unwrap(), events[4]);
    assert!(!replay.enqueue_drain(&mut queue).unwrap());
    assert_eq!(replay.events(), 5);
    qa_platform::files::remove_dir_all(directory).unwrap();
}

#[test]
fn replay_rejects_truncated_records_and_load_sized_payload_overflow() {
    let path = std::env::temp_dir().join(format!("qa-journal-invalid-{}", std::process::id()));
    let event = SysEvent {
        time: EventTime(1),
        kind: EventKind::ConsoleLine("too long"),
    };
    let mut bytes = qa_formats::journal::FILE_HEADER.to_vec();
    bytes.extend(qa_formats::journal::encode_header(event).unwrap());
    bytes.extend(event.kind.payload());
    qa_platform::files::write(&path, &bytes).unwrap();
    let mut small = Replay::open(&path, 2).unwrap();
    assert_eq!(small.peek(), Err(FormatError::InvalidRange));
    bytes.pop();
    qa_platform::files::write(&path, &bytes).unwrap();
    let mut truncated = Replay::open(&path, 1024).unwrap();
    assert_eq!(truncated.peek(), Err(FormatError::Truncated));
    qa_platform::files::remove_file(path).unwrap();
}
