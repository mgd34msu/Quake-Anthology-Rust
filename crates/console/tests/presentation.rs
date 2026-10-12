use qa_console::{
    editor::Editor,
    presentation::{Cell, ConsoleView, Layout, SCROLLBACK_RECORDS, Scrollback, Style},
};
use qa_core::{
    events::{EventRing, FrameEvent, OutputSubmission, OutputTarget, TextStore},
    primitives::{PrintKind, RuleSetId},
    sys_events::{EventTime, SeatId},
};

#[test]
fn display_lease_survives_event_retirement_and_eviction_releases_the_oldest() {
    let mut ring = EventRing::load(2, 1, SCROLLBACK_RECORDS + 1, 8).unwrap();
    let consumer = ring.bind(OutputTarget::Presentation).unwrap();
    let mut log = Scrollback::default();
    let mut first = None;
    for index in 0..=SCROLLBACK_RECORDS {
        ring.print_bytes(None, PrintKind::Console, &[index as u8, 0x80])
            .unwrap();
        let mut batch = ring.batch(consumer).unwrap();
        let record = ring.next(&mut batch).unwrap();
        let FrameEvent::Print(print) = record.event else {
            panic!("expected print")
        };
        if index == 0 {
            first = Some(print.text);
        }
        assert!(log.append(&mut ring.texts, print.text));
        assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
        assert_eq!(
            ring.texts.get(print.text),
            Some([index as u8, 0x80].as_slice())
        );
        assert!(ring.next(&mut ring.batch(consumer).unwrap()).is_none());
    }
    assert_eq!(log.len(), SCROLLBACK_RECORDS);
    assert_eq!(log.evicted, 1);
    assert!(ring.texts.get(first.unwrap()).is_none());
    assert_eq!(log.chunks(&ring.texts).next(), Some([1, 0x80].as_slice()));
    let bytes = log.chunks(&ring.texts).last().unwrap();
    assert_eq!(bytes, [SCROLLBACK_RECORDS as u8, 0x80]);
    log.clear(&mut ring.texts);
    assert!(log.is_empty());
    for _ in 0..=SCROLLBACK_RECORDS {
        assert!(ring.texts.insert(b"new").is_some());
    }
}

fn append(log: &mut Scrollback, texts: &mut TextStore, bytes: &[u8]) {
    let lease = texts.insert(bytes).unwrap();
    assert!(log.append(texts, lease.id()));
    texts.release(lease);
}
fn view<'a>(
    editor: &'a Editor,
    log: &'a Scrollback,
    texts: &'a TextStore,
    time_ms: u64,
) -> ConsoleView<'a> {
    ConsoleView {
        editor,
        scrollback: log,
        texts,
        rules: RuleSetId::Quake3,
        time: EventTime::from_milliseconds(time_ms),
    }
}
fn glyphs(layout: &Layout) -> Vec<Vec<u8>> {
    layout
        .lines()
        .map(|(_, row)| row.iter().map(|cell| cell.glyph).collect())
        .collect()
}

#[test]
fn bounded_layout_keeps_full_last_rows_across_chunks_and_carriage_returns() {
    let mut texts = TextStore::load(8, 64).unwrap();
    let mut log = Scrollback::default();
    let editor = Editor::default();
    let mut layout = Layout::load(
        4,
        2,
        Style {
            colors: false,
            cursor_period_ms: 250,
            blink_cursor: false,
        },
    )
    .unwrap();
    append(&mut log, &mut texts, b"abcd");
    append(&mut log, &mut texts, b"ef\x80h");
    layout.prepare(view(&editor, &log, &texts, 0));
    assert_eq!(glyphs(&layout), [b"abcd".to_vec(), b"ef\x80h".to_vec()]);
    layout.prepare(view(&editor, &log, &texts, 250));
    assert_eq!(glyphs(&layout), [b"abcd".to_vec(), b"ef\x80h".to_vec()]);
    append(&mut log, &mut texts, b"\nlong\rX\n");
    layout.prepare(view(&editor, &log, &texts, 0));
    // An exactly full row wraps before the CR; the newer line retains its own row.
    assert_eq!(glyphs(&layout), [b"long".to_vec(), b"X   ".to_vec()]);
    log.clear(&mut texts);
    append(&mut log, &mut texts, b"abcd\rX\n");
    layout.prepare(view(&editor, &log, &texts, 0));
    assert_eq!(glyphs(&layout), [b"abcd".to_vec(), b"X   ".to_vec()]);
}

#[test]
fn color_prefix_split_across_publications_and_utf8_cursor_use_shared_state() {
    let mut texts = TextStore::load(8, 64).unwrap();
    let mut log = Scrollback::default();
    append(&mut log, &mut texts, b"^1r");
    append(&mut log, &mut texts, b"ed ^");
    append(&mut log, &mut texts, b"2green\n");
    let mut editor = Editor::default();
    editor.toggle(SeatId::ALL[1]);
    for value in "abcλdef".chars() {
        editor.character(SeatId::ALL[1], value).unwrap();
    }
    let mut layout = Layout::load(
        6,
        2,
        Style {
            colors: true,
            cursor_period_ms: 256,
            blink_cursor: true,
        },
    )
    .unwrap();
    layout.prepare(view(&editor, &log, &texts, 0));
    let rows: Vec<_> = layout.lines().map(|(_, row)| row.to_vec()).collect();
    assert_eq!(
        rows[0][0],
        Cell {
            glyph: b'r',
            color: 1
        }
    );
    assert_eq!(
        rows[1][0],
        Cell {
            glyph: b'g',
            color: 2
        }
    );
    assert_eq!(
        layout
            .input()
            .iter()
            .map(|cell| cell.glyph)
            .collect::<Vec<_>>(),
        b"]?def\x0a"
    );
    layout.prepare(view(&editor, &log, &texts, 256));
    assert_eq!(layout.input().last().unwrap().glyph, b' ');
    editor.key(SeatId::ALL[1], 80, true, false);
    layout.prepare(view(&editor, &log, &texts, 0));
    assert_eq!(layout.input().last().unwrap().glyph, 10);
}

#[test]
fn failed_lease_does_not_replace_retained_output_and_invalid_layout_is_rejected() {
    let mut texts = TextStore::load(1, 8).unwrap();
    let lease = texts.insert(b"old").unwrap();
    let old = lease.id();
    texts.release(lease);
    let _new = texts.insert(b"new").unwrap();
    let mut log = Scrollback::default();
    assert!(!log.append(&mut texts, old));
    assert_eq!(log.rejected, 1);
    assert_eq!(log.len(), 0);
    let style = Style {
        colors: false,
        cursor_period_ms: 250,
        blink_cursor: false,
    };
    assert!(Layout::load(1, 1, style).is_err());
    assert!(Layout::load(65536, 2, style).is_err());
    assert!(Layout::load(2, 0, style).is_err());
}
