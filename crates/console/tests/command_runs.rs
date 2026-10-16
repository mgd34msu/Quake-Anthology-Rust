use qa_console::{
    command_buffer::CAPACITY,
    command_text::{Arguments, ParsedCommand},
    commands::{CommandError, CommandReply, CommandRun, CommandStep, Console, Host, ScriptError},
    views::{CommandProducer, Context, RuleSetId},
};
use qa_core::{
    primitives::{EntityId, ModuleId},
    sys_events::{DeviceId, EventTime, KeySource, SeatId},
};

#[derive(Default)]
struct Capture {
    output: Vec<u8>,
    marks: Vec<(Vec<Vec<u8>>, Context)>,
    live: Vec<CommandProducer>,
    input: qa_input::Input,
    rejected: u64,
}
impl Host for Capture {
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        self.output.extend_from_slice(text.to_string().as_bytes());
    }
    fn print_bytes(&mut self, text: &[u8]) {
        self.output.extend_from_slice(text);
    }
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        if path != "script.cfg" {
            return Err(ScriptError::Missing);
        }
        let bytes = b"mark script\n";
        destination[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
    fn quit(&mut self) {}
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        EventTime(40)
    }
    fn command_producer_live(&self, producer: CommandProducer) -> bool {
        self.live.contains(&producer)
    }
    fn reject_input(&mut self, count: u64) {
        self.rejected += count;
    }
}
fn mark(
    _: &mut Console<Capture>,
    host: &mut Capture,
    args: &Arguments<'_>,
    context: Context,
) -> Result<(), CommandError> {
    host.marks.push((
        args.iter_bytes().skip(1).map(<[u8]>::to_vec).collect(),
        context,
    ));
    Ok(())
}
fn console() -> Console<Capture> {
    let mut console = Console::new(Context::default()).unwrap();
    assert!(console.register("mark", mark));
    console
}
fn single(console: &mut Console<Capture>, host: &mut Capture, text: &[u8], context: Context) {
    let mut scratch = ParsedCommand::default();
    scratch.set_raw(text).unwrap();
    let mut run = CommandRun::single(context);
    assert_eq!(
        console.step(host, &mut run, &mut scratch),
        CommandStep::Done
    );
    assert_eq!(
        console.step(host, &mut run, &mut scratch),
        CommandStep::Done
    );
}
fn labels(host: &Capture) -> Vec<&[u8]> {
    host.marks
        .iter()
        .map(|(args, _)| args[0].as_slice())
        .collect()
}
fn producer() -> CommandProducer {
    CommandProducer {
        module: ModuleId(9),
        anchor: EntityId {
            slot: 7,
            generation: 3,
        },
    }
}

#[test]
fn registered_external_commands_keep_table_precedence_and_one_reply() {
    let mut console = console();
    let mut host = Capture::default();
    single(
        &mut console,
        &mut host,
        b"alias CoNnEcT \"mark alias\"",
        Context::default(),
    );
    let command = console.register_external("connect").unwrap();
    assert_eq!(console.register_external("CONNECT"), Some(command));
    assert!(console.register_external("mark").is_none());
    console
        .append("CONNECT \"server:26000\"; mark after\n", Context::default())
        .unwrap();
    let mut run = CommandRun::drain();
    let mut scratch = ParsedCommand::default();
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::External(command)
    );
    assert_eq!(scratch.arguments().get_bytes(1), b"server:26000");
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::External(command)
    );
    assert!(host.marks.is_empty());
    assert!(run.reply(CommandReply::Handled));
    assert!(!run.reply(CommandReply::Handled));
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Continue
    );
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Continue
    );
    assert_eq!(labels(&host), [b"after".as_slice()]);
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Done
    );
    console.append("cmdlist\n", Context::default()).unwrap();
    console.execute_frame(&mut host);
    assert!(
        host.output
            .windows(b"connect".len())
            .any(|text| text == b"connect")
    );
}

#[test]
fn single_dispatches_one_string_without_separating_or_draining_pending_text() {
    let context = Context::default();
    let mut console = console();
    let mut host = Capture::default();
    console.append("mark queued\n", context).unwrap();
    single(
        &mut console,
        &mut host,
        b"mark immediate; mark hidden\nmark newline",
        context,
    );
    assert_eq!(host.marks.len(), 1);
    assert_eq!(
        host.marks[0].0,
        [
            b"immediate;".to_vec(),
            b"mark".to_vec(),
            b"hidden".to_vec(),
            b"mark".to_vec(),
            b"newline".to_vec()
        ]
    );
    assert!(!console.idle());
    console.execute_frame(&mut host);
    assert_eq!(labels(&host), [b"immediate;".as_slice(), b"queued"]);
}

#[test]
fn single_alias_exec_and_vstr_insert_derived_work_for_a_later_drain() {
    for source in RuleSetId::ALL {
        let context = Context {
            source,
            ..Context::default()
        };
        let mut console = console();
        let mut host = Capture::default();
        single(
            &mut console,
            &mut host,
            b"alias route \"mark alias\"",
            context,
        );
        single(
            &mut console,
            &mut host,
            b"set _qa_run_script \"mark vstr\"",
            context,
        );
        console.append("mark queued\n", context).unwrap();
        single(&mut console, &mut host, b"route", context);
        single(&mut console, &mut host, b"exec script", context);
        single(&mut console, &mut host, b"vstr _qa_run_script", context);
        assert!(host.marks.is_empty());
        console.execute_frame(&mut host);
        assert_eq!(
            labels(&host),
            [b"vstr".as_slice(), b"script", b"alias", b"queued"]
        );
        assert!(host.marks.iter().all(|(_, actual)| *actual == context));
    }
}

#[test]
fn single_ignores_pending_capacity_and_does_not_consume_a_wait() {
    let context = Context::default();
    let mut console = console();
    let mut host = Capture::default();
    console
        .append("mark before; wait 2; mark after\n", context)
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(labels(&host), [b"before".as_slice()]);
    single(&mut console, &mut host, b"mark immediate", context);
    console.execute_frame(&mut host);
    assert_eq!(labels(&host), [b"before".as_slice(), b"immediate"]);
    console.execute_frame(&mut host);
    assert_eq!(
        labels(&host),
        [b"before".as_slice(), b"immediate", b"after"]
    );

    console.append(&"x".repeat(CAPACITY - 1), context).unwrap();
    single(&mut console, &mut host, b"mark full", context);
    assert_eq!(labels(&host).last(), Some(&b"full".as_slice()));
    assert!(!console.idle());
    assert!(host.output.is_empty());
}

#[test]
fn frame_wait_survives_nested_runs_and_repeated_drains_until_the_next_frame() {
    let context = Context::default();
    let mut console = console();
    let mut host = Capture::default();
    console.begin_frame();
    console.append("mark deferred\n", context).unwrap();
    console.module_parts().1.wait_next_frame(context);
    for _ in 0..4 {
        single(&mut console, &mut host, b"mark immediate", context);
        console.execute_frame(&mut host);
        assert_eq!(console.module_parts().1.wait_context(), Some(context));
    }
    assert_eq!(labels(&host), [b"immediate".as_slice(); 4]);
    console.begin_frame();
    console.execute_frame(&mut host);
    assert_eq!(labels(&host).last(), Some(&b"deferred".as_slice()));
    assert!(console.idle());
}

#[test]
fn native_wait_updates_cannot_release_a_frame_retirement_barrier() {
    for count in [0, 1, 2] {
        let context = Context::default();
        let mut console = console();
        let mut host = Capture::default();
        console.begin_frame();
        console.append("mark deferred\n", context).unwrap();
        console.module_parts().1.wait_next_frame(context);
        single(
            &mut console,
            &mut host,
            format!("wait {count}").as_bytes(),
            context,
        );
        for _ in 0..4 {
            console.execute_frame(&mut host);
            assert!(host.marks.is_empty());
            assert_eq!(console.module_parts().1.wait_context(), Some(context));
        }
        console.begin_frame();
        for _ in 0..count {
            console.execute_frame(&mut host);
            assert!(host.marks.is_empty());
        }
        console.execute_frame(&mut host);
        assert_eq!(labels(&host), [b"deferred".as_slice()]);
        assert!(console.idle());
    }
}

#[test]
fn fallback_preserves_context_and_byte_arguments_across_nested_run_and_reply() {
    let owner = producer();
    let context = Context {
        source: RuleSetId::QuakeWorld,
        seat: SeatId::ALL[1],
        key_source: Some(KeySource {
            device: DeviceId::Keyboard(4),
            control: 26,
        }),
        event_time: Some(EventTime(20)),
        producer: Some(owner),
        ..Context::default()
    };
    let mut console = console();
    let mut host = Capture {
        live: vec![owner],
        ..Capture::default()
    };
    console
        .append_line_bytes(b"native \"quoted bytes\" \xff", context)
        .unwrap();
    console.append("mark next\n", Context::default()).unwrap();
    let mut scratch = ParsedCommand::default();
    let mut run = CommandRun::drain();
    assert!(!run.reply(CommandReply::Handled));
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Fallback
    );
    assert_eq!(run.context(), context);
    assert_eq!(
        scratch.arguments().iter_bytes().collect::<Vec<_>>(),
        [b"native".as_slice(), b"quoted bytes", b"\xff"]
    );
    assert_eq!(scratch.arguments().raw_bytes(), b"\"quoted bytes\" \xff");
    single(&mut console, &mut host, b"mark nested", Context::default());
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Fallback
    );
    assert_eq!(scratch.arguments().get_bytes(2), b"\xff");
    assert!(host.output.is_empty());
    assert!(run.reply(CommandReply::Handled));
    assert!(!run.reply(CommandReply::Unhandled));
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Continue
    );
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Continue
    );
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Done
    );
    assert_eq!(labels(&host), [b"nested".as_slice(), b"next"]);
    assert!(host.output.is_empty());

    scratch.set_raw(b"unclaimed arg").unwrap();
    let mut run = CommandRun::single(context);
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Fallback
    );
    assert!(host.output.is_empty());
    assert!(run.reply(CommandReply::Unhandled));
    assert!(host.output.is_empty());
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Done
    );
    assert!(!host.output.is_empty());
}

#[test]
fn producer_retirement_during_fallback_discards_only_its_pending_work_and_aliases() {
    let owner = producer();
    let context = Context {
        producer: Some(owner),
        ..Context::default()
    };
    let mut console = console();
    let mut host = Capture {
        live: vec![owner],
        ..Capture::default()
    };
    single(
        &mut console,
        &mut host,
        b"alias route \"mark stale\"",
        context,
    );
    console
        .append("native; route; mark stale\n", context)
        .unwrap();
    console
        .append("mark healthy\n", Context::default())
        .unwrap();
    let mut scratch = ParsedCommand::default();
    let mut run = CommandRun::drain();
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Fallback
    );
    host.live.clear();
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Continue
    );
    assert!(!run.reply(CommandReply::Unhandled));
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Continue
    );
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Done
    );
    assert_eq!(labels(&host), [b"healthy".as_slice()]);
    assert!(host.output.is_empty());
    host.live.push(owner);
    scratch.set_raw(b"route").unwrap();
    let mut run = CommandRun::single(context);
    assert_eq!(
        console.step(&mut host, &mut run, &mut scratch),
        CommandStep::Fallback
    );
}

#[test]
fn rejected_single_expansion_cancels_its_acquisition_without_draining_a_peer() {
    let source = KeySource {
        device: DeviceId::Keyboard(4),
        control: 26,
    };
    let context = Context {
        source: RuleSetId::Quake2,
        key_source: Some(source),
        event_time: Some(EventTime::from_milliseconds(20)),
        ..Context::default()
    };
    let mut console = console();
    let mut host = Capture::default();
    host.input.seed(EventTime::default());
    console.append("mark stale\n", context).unwrap();
    console
        .append("mark healthy\n", Context::default())
        .unwrap();
    host.input.button(
        SeatId::FIRST,
        qa_input::Action::Forward,
        true,
        Some(source),
        None,
        EventTime::from_milliseconds(10),
    );
    single(
        &mut console,
        &mut host,
        &vec![b'x'; qa_console::text::MAX_TEXT],
        context,
    );
    assert_eq!(host.rejected, 1);
    assert!(host.marks.is_empty());
    let handles = qa_console::input_settings::InputHandles::load(&console.cvars).unwrap();
    let policy = handles.policy(&console.cvars, RuleSetId::Quake2);
    host.input
        .build_frame_with_policy(EventTime::from_milliseconds(40), &[policy; SeatId::COUNT]);
    let released = host
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(64), &[policy; SeatId::COUNT])[0];
    assert_eq!(released.movement[0], 0.0);
    console.execute_frame(&mut host);
    assert_eq!(labels(&host), [b"healthy".as_slice()]);
}

#[test]
fn malformed_single_macro_keeps_queued_tail_and_drain_expands_only_its_current_line() {
    let context = Context {
        source: RuleSetId::Quake2,
        ..Context::default()
    };
    let mut console = console();
    let mut host = Capture::default();
    single(
        &mut console,
        &mut host,
        b"set _qa_run_value expanded",
        context,
    );
    console.append("mark $_qa_run_value\n", context).unwrap();
    single(&mut console, &mut host, b"mark \"unfinished", context);
    assert!(host.marks.is_empty());
    assert!(!host.output.is_empty());
    console.execute_frame(&mut host);
    assert_eq!(labels(&host), [b"expanded".as_slice()]);
}

#[test]
fn drain_limits_survive_fallback_suspensions_and_reset_for_the_next_run() {
    let context = Context::default();
    let mut console = console();
    let mut host = Capture::default();
    single(
        &mut console,
        &mut host,
        b"alias recursive \"native; recursive\"",
        context,
    );
    console.append("recursive; mark after\n", context).unwrap();
    let mut run = CommandRun::drain();
    let mut scratch = ParsedCommand::default();
    let mut fallbacks = 0;
    loop {
        match console.step(&mut host, &mut run, &mut scratch) {
            CommandStep::Continue => {}
            CommandStep::Fallback => {
                fallbacks += 1;
                assert!(run.reply(CommandReply::Handled));
            }
            CommandStep::External(_) => panic!("no external command registered"),
            CommandStep::Done => break,
        }
    }
    assert_eq!(fallbacks, 16);
    assert_eq!(labels(&host), [b"after".as_slice()]);
    host.marks.clear();
    console
        .append(&"mark bounded\n".repeat(4097), context)
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.marks.len(), 4096);
    assert!(!console.idle());
    console.execute_frame(&mut host);
    assert_eq!(host.marks.len(), 4097);
    assert!(console.idle());
}

#[test]
fn parsed_command_engine_entry_reuses_lexical_rules_without_native_argument_limit() {
    let mut scratch = ParsedCommand::default();
    let line = "mark ".repeat(100);
    scratch.parse(line.as_bytes(), RuleSetId::Quake).unwrap();
    assert_eq!(scratch.arguments().len(), 80);
    scratch
        .parse_engine(line.as_bytes(), RuleSetId::Quake)
        .unwrap();
    assert_eq!(scratch.arguments().len(), 100);
    scratch.set_raw(b"native \xff").unwrap();
    assert!(scratch.arguments().is_empty());
    scratch
        .parse_engine(b"native \xff", RuleSetId::Quake3)
        .unwrap();
    assert_eq!(scratch.arguments().get_bytes(1), b"\xff");
}
