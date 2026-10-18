use super::*;

#[test]
fn cold_prints_reach_the_console_once_without_retiring_audio_or_native_delivery() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let id = runtime
        .server
        .connect(Connection::Remote, ModuleId(1), None)
        .unwrap();
    let native = runtime.server.clients[id.0 as usize].output.unwrap();
    let mut console = qa_console::commands::Console::<Runtime>::new(Default::default()).unwrap();
    let events = &mut runtime.server.events;
    let audio = events
        .push_to(
            qa_core::events::OutputTarget::Presentation,
            FrameEvent::Sound(qa_core::primitives::SoundEvent::default()),
        )
        .unwrap();
    let print = events
        .print_bytes(None, PrintKind::Console, b"startup detail\x80\n")
        .unwrap();
    let counts = flush_console(
        &mut runtime,
        &mut console.scrollback,
        &[None; SeatId::COUNT],
    );
    assert_eq!(counts.prints, 1);
    assert_eq!(counts.stale_texts, 0);
    assert_eq!(
        console
            .scrollback
            .chunks(&runtime.server.events.texts)
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        b"startup detail\x80\n"
    );
    let counts = flush_console(
        &mut runtime,
        &mut console.scrollback,
        &[None; SeatId::COUNT],
    );
    assert_eq!(counts.prints, 0);
    let mut pending = runtime
        .server
        .events
        .batch(runtime.server.presentation)
        .unwrap();
    let record = runtime.server.events.next(&mut pending).unwrap();
    assert_eq!(record.sequence, audio);
    assert!(matches!(record.event, FrameEvent::Sound(_)));
    assert!(runtime.server.events.next(&mut pending).is_none());
    let mut pending = runtime.server.events.batch(native).unwrap();
    let record = runtime.server.events.next(&mut pending).unwrap();
    assert_eq!(record.sequence, print);
    assert!(matches!(record.event, FrameEvent::Print(_)));
}
