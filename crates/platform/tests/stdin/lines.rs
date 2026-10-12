use super::{Clock, ConsoleInput, EventKind, SysEvent, SysEventQueue};
use qa_core::sys_events::EventTime;
use std::{
    fs::File,
    io::Write,
    net::Shutdown,
    os::{fd::FromRawFd, fd::OwnedFd, unix::net::UnixStream},
};

unsafe extern "C" {
    fn pipe2(descriptors: *mut i32, flags: i32) -> i32;
}

fn owned_input() -> (ConsoleInput, File) {
    let mut descriptors = [-1; 2];
    // SAFETY: two initialized descriptors are written by pipe2. Success gives
    // each File a distinct owned end; no inherited stdin descriptor is used.
    assert_eq!(
        unsafe { pipe2(descriptors.as_mut_ptr(), 0x800 | 0x80000) },
        0
    );
    // SAFETY: successful pipe2 transferred ownership of both fresh descriptors.
    let (reader, writer) = unsafe {
        (
            File::from_raw_fd(descriptors[0]),
            File::from_raw_fd(descriptors[1]),
        )
    };
    (ConsoleInput::with_file(Some(reader)), writer)
}

fn owned_eof_input() -> (ConsoleInput, UnixStream) {
    let (reader, writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    writer.set_nonblocking(true).unwrap();
    (
        ConsoleInput::with_file(Some(File::from(OwnedFd::from(reader)))),
        writer,
    )
}

fn line(queue: &mut SysEventQueue, expected: &str) -> EventTime {
    let event = queue.pop().unwrap();
    assert_eq!(event.kind, EventKind::ConsoleLine(expected));
    event.time
}

fn finish_intake(clock: &Clock, queue: &mut SysEventQueue) -> EventTime {
    let time = clock.now();
    queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    time
}

#[test]
fn final_budget_byte_publishes_before_packets_and_the_time_marker() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 8192).unwrap();
    let mut bytes = vec![b'x'; 4096];
    bytes[4095] = b'\n';
    writer.write_all(&bytes).unwrap();

    input.poll(&clock, &mut queue);
    let packet_time = clock.now();
    queue
        .push(SysEvent {
            time: packet_time,
            kind: EventKind::Packet {
                socket: 0,
                from: qa_core::sys_events::Peer::Loopback(qa_core::primitives::ClientId(0)),
                bytes: b"packet",
            },
        })
        .unwrap();
    let final_time = finish_intake(&clock, &mut queue);
    assert!(line(&mut queue, std::str::from_utf8(&bytes[..4095]).unwrap()) <= packet_time);
    assert!(matches!(
        queue.pop().unwrap().kind,
        EventKind::Packet {
            bytes: b"packet",
            ..
        }
    ));
    let event = queue.pop().unwrap();
    assert_eq!(event.kind, EventKind::Time);
    assert_eq!(event.time, final_time);
    assert!(final_time >= packet_time);
    assert!(queue.is_empty());
    assert_eq!(input.lines, 1);
    input.poll(&clock, &mut queue);
    assert!(queue.is_empty());
}

#[test]
fn rejected_complete_line_is_counted_once_and_never_retried() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 4).unwrap();
    queue
        .push(SysEvent {
            time: EventTime(0),
            kind: EventKind::ConsoleLine("held"),
        })
        .unwrap();
    writer.write_all(b"lost\nnext\n").unwrap();

    input.poll(&clock, &mut queue);
    assert_eq!(queue.rejected(), 1);
    assert_eq!(input.lines, 0);
    finish_intake(&clock, &mut queue);
    line(&mut queue, "held");
    assert_eq!(queue.pop().unwrap().kind, EventKind::Time);

    input.poll(&clock, &mut queue);
    line(&mut queue, "next");
    assert_eq!(queue.rejected(), 1);
    assert_eq!(input.lines, 1);
    assert_eq!(input.discarded, 0);
    assert!(queue.is_empty());
    input.poll(&clock, &mut queue);
    assert!(queue.is_empty());
}

#[test]
fn eof_publishes_the_partial_line_in_the_intake_that_observes_it() {
    let (mut input, mut writer) = owned_eof_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 64).unwrap();
    writer.write_all(b"tail").unwrap();
    // Shutdown reaches every duplicate of this socket endpoint. Closing a
    // pipe writer alone can race another test's fork until that child execs.
    writer.shutdown(Shutdown::Write).unwrap();
    drop(writer);

    input.poll(&clock, &mut queue);
    let final_time = finish_intake(&clock, &mut queue);
    assert!(line(&mut queue, "tail") <= final_time);
    assert_eq!(queue.pop().unwrap().kind, EventKind::Time);
    assert_eq!(input.lines, 1);
    input.poll(&clock, &mut queue);
    assert!(queue.is_empty());
}

#[test]
fn rejected_eof_line_does_not_survive_stream_shutdown() {
    let (mut input, mut writer) = owned_eof_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 4).unwrap();
    queue
        .push(SysEvent {
            time: EventTime(0),
            kind: EventKind::ConsoleLine("held"),
        })
        .unwrap();
    writer.write_all(b"lost").unwrap();
    writer.shutdown(Shutdown::Write).unwrap();
    drop(writer);

    input.poll(&clock, &mut queue);
    assert_eq!(queue.rejected(), 1);
    line(&mut queue, "held");
    input.poll(&clock, &mut queue);
    assert!(queue.is_empty());
    assert_eq!(queue.rejected(), 1);
    assert_eq!(input.lines, 0);
}

#[test]
fn partial_utf8_and_crlf_framing_survive_the_byte_boundary() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 8192).unwrap();
    let mut bytes = vec![b'x'; 4096];
    bytes[4095] = 0xc3;
    writer.write_all(&bytes).unwrap();
    input.poll(&clock, &mut queue);
    assert!(queue.is_empty());
    writer.write_all(b"\xa9\r\nnext\n").unwrap();
    input.poll(&clock, &mut queue);
    bytes.push(0xa9);
    line(&mut queue, std::str::from_utf8(&bytes).unwrap());
    line(&mut queue, "next");
    assert_eq!(input.lines, 2);
    assert_eq!(input.discarded, 0);
    assert!(queue.is_empty());
}

#[test]
fn cr_at_the_byte_limit_is_published_before_its_lf_is_read() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 8192).unwrap();
    let mut bytes = vec![b'x'; 4096];
    bytes[4095] = b'\r';
    writer.write_all(&bytes).unwrap();
    input.poll(&clock, &mut queue);
    line(&mut queue, std::str::from_utf8(&bytes[..4095]).unwrap());
    writer.write_all(b"\nnext\n").unwrap();
    input.poll(&clock, &mut queue);
    line(&mut queue, "next");
    assert_eq!(input.lines, 2);
    assert!(queue.is_empty());
}

#[test]
fn line_budget_yields_before_consuming_the_next_command() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(16, 64).unwrap();
    writer.write_all(b"0\n1\n2\n3\n4\n5\n6\n7\n8\n").unwrap();
    input.poll(&clock, &mut queue);
    finish_intake(&clock, &mut queue);
    for index in 0..8 {
        line(&mut queue, &index.to_string());
    }
    assert_eq!(queue.pop().unwrap().kind, EventKind::Time);
    input.poll(&clock, &mut queue);
    line(&mut queue, "8");
    assert_eq!(input.lines, 9);
    assert!(queue.is_empty());
}

#[test]
fn ordinary_slot_limit_preserves_the_time_slot_and_unread_command() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(2, 64).unwrap();
    writer.write_all(b"first\nsecond\n").unwrap();
    input.poll(&clock, &mut queue);
    finish_intake(&clock, &mut queue);
    line(&mut queue, "first");
    assert_eq!(queue.pop().unwrap().kind, EventKind::Time);
    input.poll(&clock, &mut queue);
    line(&mut queue, "second");
    assert_eq!(queue.rejected(), 0);
    assert_eq!(input.lines, 2);
    assert!(queue.is_empty());
}

#[test]
fn invalid_completed_lines_are_discarded_without_poisoning_the_next_line() {
    let (mut input, mut writer) = owned_input();
    let clock = Clock::new();
    let mut queue = SysEventQueue::load(8, 64).unwrap();
    writer.write_all(b"bad\0line\n\xff\nok\n").unwrap();
    input.poll(&clock, &mut queue);
    line(&mut queue, "ok");
    assert_eq!(input.lines, 1);
    assert_eq!(input.discarded, 2);
    assert_eq!(queue.rejected(), 0);
    assert!(queue.is_empty());
}
