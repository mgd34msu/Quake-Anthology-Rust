use qa_console::{
    command_buffer::CAPACITY,
    commands::{Console, Host, ScriptError},
    editor::{EditAction, Editor, LINE_BYTES},
    views::Context,
};
use qa_core::sys_events::{EventTime, SeatId, key_catchers};
use std::fmt::Arguments;

#[derive(Default)]
struct Capture {
    input: qa_input::Input,
}
impl Host for Capture {
    fn print(&mut self, _: Arguments<'_>) {}
    fn read_script(&mut self, _: &str, _: &mut [u8]) -> Result<usize, ScriptError> {
        Err(ScriptError::Missing)
    }
    fn quit(&mut self) {}
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        EventTime::default()
    }
}

#[test]
fn editor_repeats_cursor_and_deletion_keep_utf8_boundaries() {
    let seat = SeatId::FIRST;
    let mut editor = Editor::default();
    assert_eq!(editor.key(seat, 53, true, false), EditAction::Caught);
    editor.key(seat, 53, true, true);
    assert_eq!(editor.seat(), Some(seat));
    for value in "aλ猫z".chars() {
        editor.character(seat, value).unwrap();
    }
    editor.key(seat, 80, true, false);
    editor.key(seat, 42, true, true);
    assert_eq!(editor.line(), "aλz");
    assert_eq!(editor.cursor(), "aλ".len());
    editor.key(seat, 80, true, true);
    editor.key(seat, 76, true, false);
    assert_eq!(editor.line(), "az");
    editor.character(seat, 'é').unwrap();
    assert_eq!(editor.line(), "aéz");
    editor.key(seat, 74, true, false);
    assert_eq!(editor.cursor(), 0);
    editor.key(seat, 77, true, false);
    assert_eq!(editor.cursor(), "aéz".len());
    assert_eq!(editor.key(seat, 40, true, false), EditAction::Submit);
    assert_eq!(editor.key(seat, 40, true, true), EditAction::Caught);
}

#[test]
fn editor_is_seat_owned_and_capacity_failure_preserves_text() {
    let mut editor = Editor::default();
    editor.toggle(SeatId::ALL[1]);
    editor.character(SeatId::FIRST, 'x').unwrap();
    assert!(editor.line().is_empty());
    for _ in 0..LINE_BYTES - 1 {
        editor.character(SeatId::ALL[1], 'a').unwrap();
    }
    assert!(editor.character(SeatId::ALL[1], 'λ').is_err());
    assert_eq!(editor.line().len(), LINE_BYTES - 1);
    assert_eq!(editor.cursor(), LINE_BYTES - 1);
    editor.character(SeatId::ALL[1], 'b').unwrap();
    assert_eq!(editor.line().len(), LINE_BYTES);
}

#[test]
fn window_line_uses_existing_commands_and_retains_it_when_buffer_is_full() {
    let mut console = Console::<Capture>::new(Context::default()).unwrap();
    let seat = SeatId::ALL[1];
    console.editor.toggle(seat);
    for value in "cg_fov 123".chars() {
        console.editor.character(seat, value).unwrap();
    }
    console
        .append(&"x".repeat(CAPACITY - 1), Context::default())
        .unwrap();
    assert!(
        console
            .edit_key(seat, 40, true, false, EventTime(8), |_| {})
            .is_err()
    );
    assert_eq!(console.editor.line(), "cg_fov 123");
    let mut host = Capture::default();
    console.execute_frame(&mut host);
    assert!(
        console
            .edit_key(seat, 40, true, false, EventTime(9), |_| {})
            .unwrap()
    );
    assert!(console.editor.line().is_empty());
    console.execute_frame(&mut host);
    assert_eq!(
        console.cvars.value(console.cvars.find("cg_fov").unwrap()),
        123.0
    );
}

#[test]
fn console_catcher_updates_preserve_other_input_owners() {
    let mut console = Console::<Capture>::new(Context::default()).unwrap();
    let mut input = qa_input::Input::load();
    let seat = SeatId::FIRST;
    input.set_catcher(seat, key_catchers::UI, true, EventTime(0));
    console.editor.toggle(seat);
    console.sync_editor_input(&mut input, EventTime(1));
    assert_eq!(
        input.key_catchers(seat),
        key_catchers::CONSOLE | key_catchers::UI
    );
    console.editor.toggle(seat);
    console.sync_editor_input(&mut input, EventTime(2));
    assert_eq!(input.key_catchers(seat), key_catchers::UI);
    input.set_catcher(seat, key_catchers::CGAME, true, EventTime(3));
    input.set_catcher(seat, key_catchers::UI, false, EventTime(4));
    console.sync_editor_input(&mut input, EventTime(5));
    assert_eq!(input.key_catchers(seat), key_catchers::CGAME);
}
