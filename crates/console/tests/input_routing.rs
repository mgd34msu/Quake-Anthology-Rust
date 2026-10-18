use qa_console::{
    command_text::{Arguments, TextError},
    commands::{CommandError, Console, Host, ScriptError},
    views::Context,
};
use qa_core::sys_events::{DeviceId, EventKind, EventTime, KeySource, SeatId, SysEvent};
use qa_input::{Input, InputPolicy, Target};
use std::fmt::Write;

struct Capture {
    input: Input,
    policy: InputPolicy,
    output: String,
    rejected: u64,
    calls: Vec<(String, Context)>,
}
impl Default for Capture {
    fn default() -> Self {
        let vars = qa_console::cvars::Cvars::new().unwrap();
        let handles = qa_console::input_settings::InputHandles::load(&vars).unwrap();
        Self {
            input: Input::load(),
            policy: handles.policy(&vars, qa_core::primitives::RuleSetId::Quake2),
            output: String::new(),
            rejected: 0,
            calls: Vec::new(),
        }
    }
}
impl Host for Capture {
    fn input(&mut self) -> &mut Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        EventTime(0)
    }
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        self.output.write_fmt(text).unwrap();
    }
    fn reject_input(&mut self, count: u64) {
        self.rejected += count;
    }
    fn read_script(
        &mut self,
        path: &str,
        bytes: &mut [u8],
        _context: qa_console::views::Context,
    ) -> Result<usize, ScriptError> {
        let script = match path {
            "held.cfg" => "+forward 999 9999; wait; mark retained\n",
            _ => return Err(ScriptError::Missing),
        };
        bytes[..script.len()].copy_from_slice(script.as_bytes());
        Ok(script.len())
    }
    fn quit(&mut self) {}
}
fn mark(
    _: &mut Console<Capture>,
    host: &mut Capture,
    args: &Arguments<'_>,
    context: Context,
) -> Result<(), CommandError> {
    host.calls.push((args.get(0).to_owned(), context));
    Ok(())
}
struct Enqueue<'a>(&'a mut Console<Capture>);
impl Target for Enqueue<'_> {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        key_source: Option<KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &str,
    ) -> bool {
        self.0
            .append_line(
                text,
                Context {
                    seat,
                    event_time: Some(time),
                    key_source,
                    producer,
                    ..Context::default()
                },
            )
            .is_ok()
    }
    fn cancel_commands(
        &mut self,
        source: KeySource,
        producer: Option<qa_core::primitives::CommandProducer>,
    ) {
        self.0.discard_acquisition(source, producer);
    }
}
fn key(
    console: &mut Console<Capture>,
    host: &mut Capture,
    device: DeviceId,
    code: u16,
    ms: u64,
    down: bool,
    repeat: bool,
) {
    host.input.dispatch(
        SysEvent {
            time: EventTime::from_milliseconds(ms),
            kind: EventKind::Key {
                device,
                code,
                symbol: 0,
                down,
                repeat,
            },
        },
        &mut Enqueue(console),
    );
}
fn frame(host: &mut Capture, ms: u64) -> [qa_core::primitives::UserCmd; SeatId::COUNT] {
    host.input.build_frame_with_policy(
        EventTime::from_milliseconds(ms),
        &[host.policy; SeatId::COUNT],
    )
}
#[test]
fn standard_buttons_wait_for_console_and_keep_distinct_physical_sources() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        0,
        true,
        false,
    );
    assert_eq!(frame(&mut host, 0)[0].movement[0], 0.0);
    console.execute_frame(&mut host);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(28),
        26,
        5,
        true,
        false,
    );
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        10,
        true,
        true,
    );
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        15,
        false,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 200.0);
    host.input.dispatch(
        SysEvent {
            time: EventTime::from_milliseconds(20),
            kind: EventKind::DeviceRemoved(DeviceId::Keyboard(28)),
        },
        &mut Enqueue(&mut console),
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 40)[0].movement[0], 0.0);
    assert!(host.input.assign(DeviceId::Keyboard(29), SeatId::ALL[1]));
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(29),
        26,
        40,
        true,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 60)[1].movement[0], 200.0);
    assert_eq!(frame(&mut host, 80)[0].movement[0], 0.0);
}
#[test]
fn aliases_exec_and_wait_retain_physical_control_and_event_clock() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    assert!(console.register("mark", mark));
    console
        .append(
            "alias +custom \"exec held\"; alias -custom \"-forward\"; bind W +custom\n",
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(812),
        26,
        5,
        true,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 150.0);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(812),
        26,
        20,
        false,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(
        host.calls[0].1.key_source,
        Some(KeySource {
            device: DeviceId::Keyboard(812),
            control: 26
        })
    );
    assert_eq!(
        host.calls[0].1.event_time,
        Some(EventTime::from_milliseconds(5))
    );
    assert_eq!(frame(&mut host, 40)[0].movement[0], 0.0);
}
#[test]
fn custom_button_registration_and_quoted_clauses_share_command_lookup() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    assert!(console.register("+custom", mark));
    assert!(console.register("-custom", mark));
    // Bind text itself stays byte-exact, including the inner quotes.
    host.input
        .bind_text(
            26,
            "+CuStOm; echo \"a;b\"; +jump",
            EventTime(0),
            &mut Enqueue(&mut console),
        )
        .unwrap();
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(81),
        26,
        10,
        true,
        false,
    );
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(81),
        26,
        15,
        true,
        true,
    );
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(81),
        26,
        20,
        false,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(
        host.calls
            .iter()
            .map(|call| call.0.as_str())
            .collect::<Vec<_>>(),
        ["+CuStOm", "-CuStOm"]
    );
    assert!(host.calls.iter().all(|call| call.1.key_source
        == Some(KeySource {
            device: DeviceId::Keyboard(81),
            control: 26
        })));
    assert!(host.output.contains("a;b \n"));
    assert_ne!(
        frame(&mut host, 30)[0].buttons & qa_core::primitives::buttons::JUMP,
        0
    );
}
#[test]
fn full_bind_release_reports_existing_bounded_command_error() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    console
        .append("bind W +forward\n", Context::default())
        .unwrap();
    console.execute_frame(&mut host);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        0,
        true,
        false,
    );
    console.execute_frame(&mut host);
    // Reading the first line frees less than the replacement release requires.
    console.append("unbind w\n", Context::default()).unwrap();
    console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - 11),
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    assert!(
        host.output
            .contains("Command unbind rejected: Text(TooLong)"),
        "{}",
        host.output
    );
    assert!(host.input.binding(26).is_none());
    assert_eq!(host.rejected, 1);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 0.0);
    assert_eq!(frame(&mut host, 40)[0].movement[0], 0.0);
    assert!(matches!(
        console.append(
            &"x".repeat(qa_console::command_buffer::CAPACITY),
            Context::default()
        ),
        Err(TextError::TooLong)
    ));
}

#[test]
fn rejected_release_retains_another_physical_holder() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    for device in [DeviceId::Keyboard(17), DeviceId::Keyboard(28)] {
        key(&mut console, &mut host, device, 26, 0, true, false);
    }
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 200.0);
    console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - 1),
            Context::default(),
        )
        .unwrap();
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        30,
        false,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 40)[0].movement[0], 200.0);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(28),
        26,
        40,
        false,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 60)[0].movement[0], 0.0);
}

#[test]
fn rejected_rebind_release_clears_old_action_without_acquiring_held_replacement() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        0,
        true,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 200.0);
    let replacement = "bind w +jump\n";
    console
        .append(
            replacement,
            Context {
                event_time: Some(EventTime::from_milliseconds(30)),
                ..Context::default()
            },
        )
        .unwrap();
    console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - replacement.len() - 1),
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.rejected, 1);
    assert_eq!(host.input.binding(26).unwrap().text(), "+jump");
    assert_eq!(frame(&mut host, 40)[0].movement[0], 100.0);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        45,
        true,
        true,
    );
    console.execute_frame(&mut host);
    let command = frame(&mut host, 50)[0];
    assert_eq!(command.movement[0], 0.0);
    assert_eq!(command.buttons & qa_core::primitives::buttons::JUMP, 0);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        55,
        false,
        false,
    );
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        60,
        true,
        false,
    );
    console.execute_frame(&mut host);
    assert_ne!(
        frame(&mut host, 70)[0].buttons & qa_core::primitives::buttons::JUMP,
        0
    );
}

#[test]
fn rejected_unbind_release_discards_old_press_waiting_behind_the_unbind() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    let unbind = "unbind w\n";
    console.append(unbind, Context::default()).unwrap();
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        10,
        true,
        false,
    );
    let press = "+forward 2074 10\n";
    console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - unbind.len() - press.len() - 1),
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.rejected, 1);
    assert!(host.input.binding(26).is_none());
    assert_eq!(frame(&mut host, 20)[0].movement[0], 0.0);
    assert_eq!(frame(&mut host, 40)[0].movement[0], 0.0);
}

#[test]
fn rejected_release_cancels_inherited_alias_commands_waiting_after_a_wait() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    console
        .append(
            "alias +custom \"wait; +forward\"; alias -custom \"-forward\"; bind w +custom\n",
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        10,
        true,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 0.0);
    // Alias storage and native Q3 insertion each supply their own newline.
    let pending = " +forward\n\n";
    console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - pending.len() - 1),
            Context::default(),
        )
        .unwrap();
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        30,
        false,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 40)[0].movement[0], 0.0);
}

#[test]
fn rejected_alias_expansion_closes_an_already_executed_press() {
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    host.input.seed(EventTime(0));
    console
        .append(
            &format!(
                "alias +custom \"+forward\"; alias -custom \"-forward; echo {}\"; bind w +custom\n",
                "x".repeat(800)
            ),
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        10,
        true,
        false,
    );
    console.execute_frame(&mut host);
    assert_eq!(frame(&mut host, 20)[0].movement[0], 100.0);
    key(
        &mut console,
        &mut host,
        DeviceId::Keyboard(17),
        26,
        30,
        false,
        false,
    );
    let release = "-custom 2074 30\n";
    console
        .append(
            &" ".repeat(qa_console::command_buffer::CAPACITY - release.len() - 1),
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.rejected, 1);
    assert!(
        host.output
            .contains("Command -custom rejected: Text(TooLong)")
    );
    assert_eq!(frame(&mut host, 40)[0].movement[0], 100.0);
    assert_eq!(frame(&mut host, 60)[0].movement[0], 0.0);
}
