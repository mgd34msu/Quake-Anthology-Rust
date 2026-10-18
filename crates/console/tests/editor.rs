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
    output: Vec<u8>,
}
impl Host for Capture {
    fn print(&mut self, text: Arguments<'_>) {
        self.output.extend(text.to_string().as_bytes());
    }
    fn print_bytes(&mut self, text: &[u8]) {
        self.output.extend(text);
    }
    fn read_script(
        &mut self,
        _: &str,
        _: &mut [u8],
        _context: qa_console::views::Context,
    ) -> Result<usize, ScriptError> {
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
            .edit_key(
                40,
                true,
                false,
                qa_console::views::Context {
                    seat: seat,
                    event_time: Some(EventTime(8)),
                    ..Default::default()
                },
                |_| {}
            )
            .is_err()
    );
    assert_eq!(console.editor.line(), "cg_fov 123");
    let mut host = Capture::default();
    console.execute_frame(&mut host);
    assert!(
        console
            .edit_key(
                40,
                true,
                false,
                qa_console::views::Context {
                    seat: seat,
                    event_time: Some(EventTime(9)),
                    ..Default::default()
                },
                |_| {}
            )
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

#[test]
fn overstrike_replaces_one_unicode_scalar_and_failure_keeps_line_and_cursor() {
    let mut editor = Editor::default();
    let seat = SeatId::FIRST;
    editor.set_active(seat, true);
    for value in "aλ猫z".chars() {
        editor.character(seat, value).unwrap();
    }
    editor.key(seat, 74, true, false);
    editor.key(seat, 79, true, false);
    editor.set_overstrike(true);
    editor.character(seat, 'é').unwrap();
    assert_eq!(editor.line(), "aé猫z");
    assert_eq!(editor.cursor(), "aé".len());
    editor.key(seat, 73, true, false);
    assert!(!editor.overstrike());
    editor.key(seat, 73, true, true);
    assert!(!editor.overstrike());
    editor.set_active(SeatId::ALL[1], true);
    editor.set_active(seat, false);
    assert_eq!(editor.seat(), Some(SeatId::ALL[1]));
    editor.submitted();
    for _ in 0..LINE_BYTES {
        editor.character(SeatId::ALL[1], 'x').unwrap();
    }
    editor.key(SeatId::ALL[1], 74, true, false);
    editor.set_overstrike(true);
    assert!(editor.character(SeatId::ALL[1], 'λ').is_err());
    assert_eq!(editor.line().len(), LINE_BYTES);
    assert_eq!(editor.cursor(), 0);
}

#[test]
fn existing_command_buffer_executes_raw_echo_and_q2_macros_never_expose_private_cvars() {
    use qa_core::primitives::RuleSetId;
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let context = Context {
            source,
            ..Context::default()
        };
        let mut console = Console::<Capture>::new(context).unwrap();
        let mut host = Capture::default();
        let secret = console.cvars.bind("rcon_password", context).unwrap();
        console
            .cvars
            .force_write(secret, "hidden-native-password")
            .unwrap();
        assert!(console.cvars.private(secret));
        console
            .append_line_bytes(b"echo raw\xff \"payload\x80\"", context)
            .unwrap();
        console.append("echo $rcon_password\n", context).unwrap();
        console.execute_frame(&mut host);
        assert_eq!(host.output, b"raw\xff payload\x80 \n\n");
        let value = console.cvars.bind("sensitivity", context).unwrap();
        let before = console.cvars.read(value).unwrap().as_str().to_owned();
        console
            .append_line_bytes(b"sensitivity \xff", context)
            .unwrap();
        console.execute_frame(&mut host);
        assert_eq!(console.cvars.read(value).unwrap().as_str(), before); // Unsupported raw cvar input is rejected, never coerced to empty.
        console
            .append_line_bytes(b"set sensitivity \xff", context)
            .unwrap();
        console.execute_frame(&mut host);
        assert_eq!(console.cvars.read(value).unwrap().as_str(), before);
    }
}

#[test]
fn raw_alias_and_bind_commands_share_byte_storage_and_listing() {
    let context = Context::default();
    let mut console = Console::<Capture>::new(context).unwrap();
    let mut host = Capture::default();
    console
        .append_line_bytes(b"alias raw \"echo alias\xff\"", context)
        .unwrap();
    console.append_line("raw", context).unwrap();
    console
        .append_line_bytes(b"bind a \"echo binding\x80\"", context)
        .unwrap();
    console.append_line("bind a", context).unwrap();
    console.append_line("bindlist", context).unwrap();
    console.append_line("alias", context).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(
        host.input
            .binding(SeatId::FIRST, qa_input::keys::parse("a").unwrap())
            .unwrap()
            .bytes(),
        b"echo binding\x80"
    );
    assert!(
        host.output
            .starts_with(b"alias\xff \n\"a\" = \"echo binding\x80\"\n")
    );
    assert!(
        host.output
            .windows(b"a \"echo binding\x80\"\n".len())
            .any(|row| row == b"a \"echo binding\x80\"\n")
    );
    assert!(host.output.ends_with(b"raw : echo alias\xff\n"));
}
