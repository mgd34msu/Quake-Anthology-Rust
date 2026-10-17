use qa_console::{
    command_buffer::{CAPACITY, CommandBuffer},
    command_text::{ParsedCommand, TextError},
    commands::{
        BindOutput, CommandError, CommandReply, CommandRun, CommandStep, Console, Host, ScriptError,
    },
    cvars::Cvars,
    views::{CommandProducer, Context, RuleSetId},
};
use qa_core::{
    primitives::{ClientId, EntityId, ModuleId, NameId},
    sys_events::{DeviceId, EventKind, EventTime, KeySource, SeatId, SysEvent},
    text::FixedText,
};
use std::fmt::Arguments;

#[derive(Default)]
struct Capture {
    live: Vec<CommandProducer>,
    clients: Vec<(ClientId, EntityId)>,
    output: Vec<u8>,
    input: qa_input::Input,
    quit: bool,
    now: EventTime,
    rejected: u64,
}
impl Host for Capture {
    fn print(&mut self, text: Arguments<'_>) {
        self.output.extend(text.to_string().as_bytes());
    }
    fn print_bytes(&mut self, text: &[u8]) {
        self.output.extend(text);
    }
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        let bytes = match path {
            "outer.cfg" => b"echo before; exec inner; echo outer-tail\n".as_slice(),
            "inner.cfg" => b"alias route \"wait 100; echo alias-tail\"; route; echo inner-tail\n",
            _ => return Err(ScriptError::Missing),
        };
        destination[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
    fn quit(&mut self) {
        self.quit = true;
    }
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        self.now
    }
    fn command_producer_live(&self, producer: CommandProducer) -> bool {
        self.live.contains(&producer)
    }
    fn client_command_live(&self, client: ClientId, lifetime: EntityId) -> bool {
        self.clients.contains(&(client, lifetime))
    }
    fn reject_input(&mut self, count: u64) {
        self.rejected += count;
    }
}

fn producer(module: u16, generation: u32) -> CommandProducer {
    CommandProducer {
        module: ModuleId(module),
        anchor: EntityId {
            slot: 7,
            generation,
        },
    }
}
fn context(producer: CommandProducer) -> Context {
    Context {
        source: RuleSetId::Quake2,
        producer: Some(producer),
        ..Context::default()
    }
}
fn client_context(generation: u32) -> Context {
    Context {
        source: RuleSetId::QuakeWorld,
        client_lifetime: Some((
            ClientId(2),
            EntityId {
                slot: 9,
                generation,
            },
        )),
        ..Context::default()
    }
}

#[test]
fn reused_client_cannot_resume_alias_wait_or_queued_registered_command() {
    let old = client_context(4);
    let replacement = client_context(5);
    let mut console = Console::new(Context::default()).unwrap();
    let command = console.register_external("server_restart").unwrap();
    let mut host = Capture {
        clients: vec![old.client_lifetime.unwrap()],
        ..Default::default()
    };
    console
        .append(
            "alias deferred \"wait 100; quit\"; deferred; echo stale-tail\n",
            old,
        )
        .unwrap();
    console.append_registered(command, old).unwrap();
    console.execute_frame(&mut host);
    assert!(host.output.is_empty());
    assert!(!host.quit);

    // Same numeric client slot now belongs to a different entity generation.
    host.clients[0] = replacement.client_lifetime.unwrap();
    console
        .append("echo healthy\n", Context::default())
        .unwrap();
    console.append_registered(command, replacement).unwrap();
    let mut run = CommandRun::drain();
    let mut parser = ParsedCommand::default();
    let mut dispatched = 0;
    loop {
        match console.step(&mut host, &mut run, &mut parser) {
            CommandStep::Continue => {}
            CommandStep::External(id) => {
                assert_eq!(id, command);
                assert_eq!(parser.arguments().get_bytes(0), b"server_restart");
                assert_eq!(run.context(), replacement);
                dispatched += 1;
                assert!(run.reply(CommandReply::Handled));
            }
            CommandStep::Fallback => panic!("known commands"),
            CommandStep::Done => break,
        }
    }
    assert_eq!(dispatched, 1);
    assert_eq!(host.output, b"healthy \n");
    assert!(!host.quit);
    assert!(console.idle());
    assert!(matches!(
        console.append_registered(NameId(u32::MAX), replacement),
        Err(CommandError::Usage)
    ));
    assert!(console.idle());
}

#[test]
fn retired_client_frame_wait_does_not_block_unrelated_commands() {
    let old = client_context(1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture::default();
    {
        let (_, buffer, _) = console.module_parts();
        buffer.wait_next_frame(old);
    }
    console.append("echo ready\n", Context::default()).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"ready \n");
    assert!(console.idle());
}

#[test]
fn client_lifetime_is_rechecked_after_external_reply_and_combines_with_producer() {
    let old = client_context(8);
    let replacement = client_context(9);
    let module = producer(2, 3);
    let combined = Context {
        producer: Some(module),
        ..old
    };
    let mut host = Capture {
        clients: vec![old.client_lifetime.unwrap()],
        live: vec![module],
        ..Default::default()
    };
    assert!(combined.is_live(&host));
    host.live.clear();
    assert!(!combined.is_live(&host));
    assert!(old.is_live(&host));
    host.live.push(module);

    for replied in [false, true] {
        let mut console = Console::new(Context::default()).unwrap();
        let command = console.register_external("server_restart").unwrap();
        let mut parser = ParsedCommand::default();
        parser.set_raw(b"server_restart").unwrap();
        let mut run = CommandRun::single(combined);
        host.clients[0] = old.client_lifetime.unwrap();
        assert_eq!(
            console.step(&mut host, &mut run, &mut parser),
            CommandStep::External(command)
        );
        if replied {
            assert!(run.reply(CommandReply::Unhandled));
        }
        host.clients[0] = replacement.client_lifetime.unwrap();
        assert!(!combined.is_live(&host));
        assert_eq!(
            console.step(&mut host, &mut run, &mut parser),
            CommandStep::Done
        );
        assert!(!run.reply(CommandReply::Handled));
        assert!(host.output.is_empty());
        // A stale connection does not retire the still-live module producer.
        assert!(host.command_producer_live(module));
    }
}
fn line(buffer: &mut CommandBuffer, expected: &[u8], context: Context) {
    let mut text = FixedText::<65536>::default();
    let (actual, result) = buffer.next_line(&mut text).unwrap();
    result.unwrap();
    assert_eq!(actual, context);
    assert_eq!(text.as_bytes(), expected);
}
fn key(console: &mut Console<Capture>, host: &mut Capture, control: u16, down: bool) {
    let (_, buffer, _) = console.module_parts();
    let mut target = BindOutput::new(buffer, Context::default());
    host.input.dispatch(
        SysEvent {
            time: EventTime(20),
            kind: EventKind::Key {
                device: DeviceId::Keyboard(3),
                code: control,
                symbol: 0,
                down,
                repeat: false,
            },
        },
        &mut target,
    );
    assert_eq!(target.outcome(), (0, Ok(())));
}

#[test]
fn exact_producer_retirement_compacts_interleaved_bytes_and_preserves_replacements() {
    let old = producer(1, 2);
    let replacement = context(producer(1, 3));
    let other = context(producer(2, 2));
    let human = Context::default();
    let mut buffer = CommandBuffer::new();
    buffer.append_bytes(b"retired-a\xff", context(old)).unwrap();
    buffer.append_bytes(b"healthy\x80", other).unwrap();
    buffer.append_line("retired-b", context(old)).unwrap();
    buffer.append_line("replacement", replacement).unwrap();
    buffer.insert("retired-insert", context(old)).unwrap();
    buffer.append_line("human", human).unwrap();
    buffer.discard_producer(old);
    line(&mut buffer, b"healthy\x80", other);
    line(&mut buffer, b"replacement", replacement);
    line(&mut buffer, b"human", human);
    assert!(buffer.is_empty());
    // All removed bytes are reusable, without growing another command store.
    buffer.append(&"x".repeat(CAPACITY - 1), human).unwrap();
    assert_eq!(buffer.append("y", human), Err(TextError::TooLong));
}

#[test]
fn waits_remain_owned_when_their_command_span_has_been_consumed() {
    let owner = context(producer(1, 1));
    let other = producer(2, 1);
    let mut buffer = CommandBuffer::new();
    buffer.append_line("healthy", Context::default()).unwrap();
    buffer.wait(2, owner);
    buffer.discard_producer(other);
    assert_eq!(buffer.wait_context(), Some(owner));
    assert!(!buffer.ready());
    assert!(!buffer.ready());
    assert!(buffer.ready());
    assert!(buffer.wait_context().is_none());
    buffer.wait(100, owner);
    buffer.discard_producer(owner.producer.unwrap());
    assert!(buffer.ready());
    line(&mut buffer, b"healthy", Context::default());
    buffer.wait(0, owner);
    assert!(buffer.ready());
    buffer.wait(1, Context::default());
    buffer.discard_producer(owner.producer.unwrap());
    assert!(!buffer.ready());
    assert!(buffer.ready());
    buffer.wait(5, owner);
    buffer.clear();
    assert!(buffer.wait_context().is_none());
}

#[test]
fn frame_barrier_and_native_wait_retire_only_their_own_producers() {
    let frame_owner = context(producer(1, 1));
    let native_owner = context(producer(2, 1));
    for retire_frame in [false, true] {
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        buffer.wait_next_frame(frame_owner);
        buffer.wait(2, native_owner);
        if retire_frame {
            buffer.discard_producer(frame_owner.producer.unwrap());
            assert_eq!(buffer.wait_context(), Some(native_owner));
            assert!(!buffer.ready());
            assert!(!buffer.ready());
            assert!(buffer.ready());
        } else {
            buffer.discard_producer(native_owner.producer.unwrap());
            for _ in 0..4 {
                assert_eq!(buffer.wait_context(), Some(frame_owner));
                assert!(!buffer.ready());
            }
            buffer.begin_frame();
            assert!(buffer.ready());
        }
        assert!(buffer.wait_context().is_none());
    }
}

#[test]
fn key_cancellation_and_producer_retirement_are_independent_and_share_compaction() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let key = KeySource {
        device: DeviceId::Keyboard(11),
        control: 26,
    };
    let other_key = KeySource {
        device: DeviceId::Keyboard(12),
        control: 26,
    };
    let keyed_a = Context {
        key_source: Some(key),
        ..context(a)
    };
    let keyed_b = Context {
        key_source: Some(other_key),
        ..context(b)
    };
    let same_key_human = Context {
        key_source: Some(key),
        ..Context::default()
    };
    let same_key_replacement = Context {
        key_source: Some(key),
        ..context(producer(1, 2))
    };
    for key_first in [false, true] {
        let mut buffer = CommandBuffer::new();
        buffer.append_line("a", context(a)).unwrap();
        buffer.append_line("a-key", keyed_a).unwrap();
        buffer.append_line("b-key", keyed_b).unwrap();
        buffer
            .append_line("same-key-human", same_key_human)
            .unwrap();
        buffer
            .append_line("same-key-replacement", same_key_replacement)
            .unwrap();
        buffer.append_line("human", Context::default()).unwrap();
        buffer.wait(100, keyed_a);
        if key_first {
            buffer.discard_acquisition(key, Some(a));
            assert!(buffer.wait_context().is_none());
            buffer.discard_producer(a);
        } else {
            buffer.discard_producer(a);
            assert!(buffer.wait_context().is_none());
            buffer.discard_acquisition(key, Some(a));
        }
        line(&mut buffer, b"b-key", keyed_b);
        line(&mut buffer, b"same-key-human", same_key_human);
        line(&mut buffer, b"same-key-replacement", same_key_replacement);
        line(&mut buffer, b"human", Context::default());
        assert!(buffer.is_empty());
    }
}

#[test]
fn stale_wait_is_removed_before_it_can_delay_another_producer() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a, b],
        ..Capture::default()
    };
    console
        .append(
            "alias stale \"echo stale-alias\"; wait 100; quit\n",
            context(a),
        )
        .unwrap();
    console.append("echo healthy\n", context(b)).unwrap();
    console.execute_frame(&mut host);
    assert!(host.output.is_empty());
    host.live.retain(|&current| current != a);
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"healthy \n");
    assert!(!host.quit);
    assert!(console.idle());
    // Losing the lifetime discarded the alias along with its queued tail.
    host.live.push(a);
    host.output.clear();
    console.append("stale\n", context(a)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"Unknown command \"stale\"\n");
}

#[test]
fn stale_lines_do_not_execute_and_same_module_new_lifetime_remains_live() {
    let old = producer(1, 4);
    let replacement = producer(1, 5);
    let other_owner = producer(2, 4);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![replacement, other_owner],
        ..Capture::default()
    };
    console
        .append("quit; echo retired\n", context(old))
        .unwrap();
    console.append("echo new\n", context(replacement)).unwrap();
    console
        .append("echo other\n", context(other_owner))
        .unwrap();
    console.append("echo human\n", Context::default()).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"new \nother \nhuman \n");
    assert!(!host.quit);
}

#[test]
fn aliases_are_scoped_by_exact_lifetime_and_retired_slots_are_reusable() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let replacement = producer(1, 2);
    let mut console = Console::with_alias_capacity(Context::default(), 3).unwrap();
    let mut host = Capture {
        live: vec![a, b, replacement],
        ..Capture::default()
    };
    for (origin, text) in [
        (Context::default(), "alias shared \"echo human\"; shared\n"),
        (context(a), "alias SHARED \"echo a\"; shared\n"),
        (context(b), "alias Shared \"echo b\"; shared\n"),
        (context(a), "alias sHaReD \"echo changed-a\"; shared\n"),
    ] {
        console.append(text, origin).unwrap();
        console.execute_frame(&mut host);
    }
    assert_eq!(host.output, b"human \na \nb \nchanged-a \n");
    host.output.clear();
    console.append("alias\n", context(a)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"sHaReD : echo changed-a\n");
    // A is the middle row of the same-name chain. B and human remain.
    console.discard_producer(&mut host, a).unwrap();
    host.output.clear();
    console.append("shared\n", Context::default()).unwrap();
    console.append("shared\n", context(b)).unwrap();
    console
        .append(
            "alias shared \"echo replacement\"; shared\n",
            context(replacement),
        )
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"human \nb \nreplacement \n");
    // B can remove only its own definition, even while another is the head.
    console.append("unalias shared\n", context(b)).unwrap();
    console.execute_frame(&mut host);
    host.output.clear();
    console.append("shared\n", Context::default()).unwrap();
    console.append("shared\n", context(replacement)).unwrap();
    console.append("shared\n", context(b)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(
        host.output,
        b"human \nreplacement \nUnknown command \"shared\"\n"
    );
}

#[test]
fn alias_capacity_failure_does_not_replace_a_different_owner() {
    let a = producer(1, 1);
    let mut console = Console::with_alias_capacity(Context::default(), 1).unwrap();
    let mut host = Capture {
        live: vec![a],
        ..Capture::default()
    };
    console
        .append("alias same \"echo human\"\n", Context::default())
        .unwrap();
    console.execute_frame(&mut host);
    console
        .append("alias same \"echo module\"\n", context(a))
        .unwrap();
    console.execute_frame(&mut host);
    assert!(host.output.windows(7).any(|bytes| bytes == b"TooLong"));
    host.output.clear();
    console.discard_producer(&mut host, a).unwrap();
    console.append("same\n", Context::default()).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"human \n");
}

#[test]
fn nested_exec_alias_and_vstr_descendants_keep_their_producer() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a, b],
        ..Capture::default()
    };
    console.append("set _qa_owned_script \"exec outer; echo vstr-tail\"; vstr _qa_owned_script; echo a-tail\n", context(a)).unwrap();
    console.append("echo healthy\n", context(b)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"before \n");
    console.discard_producer(&mut host, a).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"before \nhealthy \n");
    assert!(console.idle());
    host.output.clear();
    console.append("route\n", context(a)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"Unknown command \"route\"\n");
}

#[test]
fn cold_publication_prunes_absent_keys_without_trusting_anchor_liveness() {
    let old = producer(1, 1);
    let replacement = producer(1, 2);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![old, replacement],
        ..Capture::default()
    };
    console
        .append("alias scoped \"echo old\"; wait 100; quit\n", context(old))
        .unwrap();
    console
        .append("echo replacement\n", context(replacement))
        .unwrap();
    console.append("echo human\n", Context::default()).unwrap();
    console.execute_frame(&mut host);
    assert!(host.output.is_empty());
    console
        .retain_producers(&mut host, |producer| producer == replacement)
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"replacement \nhuman \n");
    assert!(!host.quit);
    host.output.clear();
    console.append("scoped\n", context(old)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"Unknown command \"scoped\"\n");
}

#[test]
fn raw_alias_bytes_and_native_cvar_handles_do_not_gain_per_producer_copies() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a, b],
        ..Capture::default()
    };
    console
        .append_line_bytes(b"alias bytes \"echo \xff\x80\"; bytes", context(a))
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"\xff\x80 \n");
    host.output.clear();
    console.append("bytes\n", context(b)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"Unknown command \"bytes\"\n");

    let mut vars = Cvars::new().unwrap();
    let first = vars
        .bind(
            "cg_fov",
            Context {
                producer: Some(a),
                ..Context::default()
            },
        )
        .unwrap();
    let second = vars
        .bind(
            "cg_fov",
            Context {
                producer: Some(b),
                event_time: Some(EventTime(32)),
                ..Context::default()
            },
        )
        .unwrap();
    assert_eq!(
        vars.native_view(first).unwrap(),
        vars.native_view(second).unwrap()
    );
}

#[test]
fn window_editor_submits_a_human_origin_even_with_a_module_context_selected() {
    let a = producer(1, 1);
    let mut console = Console::new(Context {
        key_source: Some(KeySource {
            device: DeviceId::Keyboard(3),
            control: 26,
        }),
        ..context(a)
    })
    .unwrap();
    let mut host = Capture::default();
    console.editor.set_active(SeatId::FIRST, true);
    for value in "echo human".chars() {
        console.editor.character(SeatId::FIRST, value).unwrap();
    }
    console
        .edit_key(SeatId::FIRST, 40, true, false, EventTime(9), |_| {})
        .unwrap();
    console.discard_producer(&mut host, a).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"human \n");
}

#[test]
fn physical_bindings_keep_the_module_alias_owner_until_exact_retirement() {
    let a = producer(1, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a],
        ..Capture::default()
    };
    console
        .append(
            "alias route \"echo human\"; bind q route\n",
            Context::default(),
        )
        .unwrap();
    console
        .append("alias route \"echo module\"; bind w route\n", context(a))
        .unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.input.binding(26).unwrap().producer(), Some(a));
    assert_eq!(host.input.binding(20).unwrap().producer(), None);
    key(&mut console, &mut host, 26, true);
    console.execute_frame(&mut host);
    key(&mut console, &mut host, 26, false);
    key(&mut console, &mut host, 20, true);
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"module \nhuman \n");
    key(&mut console, &mut host, 20, false);
    // A pending physical acquisition is dropped by the same producer teardown.
    key(&mut console, &mut host, 26, true);
    let (_, buffer, _) = console.module_parts();
    let mut target = BindOutput::new(buffer, Context::default());
    host.input.retire_bindings(a, EventTime(21), &mut target);
    assert_eq!(target.outcome(), (0, Ok(())));
    console.discard_producer(&mut host, a).unwrap();
    assert!(host.input.binding(26).is_none());
    assert!(host.input.binding(20).is_some());
    console.execute_frame(&mut host);
    key(&mut console, &mut host, 26, false);
    key(&mut console, &mut host, 26, true);
    key(&mut console, &mut host, 20, true);
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"module \nhuman \nhuman \n");
}

#[test]
fn replacing_a_held_binding_releases_with_the_acquired_producer() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a, b],
        ..Capture::default()
    };
    console.append("bind g +forward\n", context(a)).unwrap();
    console.execute_frame(&mut host);
    key(&mut console, &mut host, 10, true);
    console.execute_frame(&mut host);
    let (_, buffer, _) = console.module_parts();
    // This replacement target belongs to B, but the old acquisition belongs to A.
    let mut target = BindOutput::new(buffer, context(b));
    host.input
        .bind_bytes_owned(10, b"+back", Some(b), EventTime(23), &mut target)
        .unwrap();
    assert_eq!(target.outcome(), (0, Ok(())));
    let (_, buffer, _) = console.module_parts();
    line(
        buffer,
        // Keyboard0 and Mouse0 occupy the first two default device rows.
        b"-forward 2058 0",
        Context {
            key_source: Some(KeySource {
                device: DeviceId::Keyboard(3),
                control: 10,
            }),
            event_time: Some(EventTime(23)),
            ..context(a)
        },
    );
    assert!(buffer.is_empty());
    assert_eq!(host.input.binding(10).unwrap().producer(), Some(b));
}

#[test]
fn retiring_an_old_instance_keeps_the_replacement_binding_and_alias() {
    let old = producer(1, 1);
    let replacement = producer(1, 2);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![old, replacement],
        ..Capture::default()
    };
    for (origin, text) in [
        (context(old), "alias route \"echo old\"; bind w route\n"),
        (
            context(replacement),
            "alias route \"echo new\"; bind w route\n",
        ),
    ] {
        console.append(text, origin).unwrap();
        console.execute_frame(&mut host);
    }
    let (_, buffer, _) = console.module_parts();
    let mut target = BindOutput::new(buffer, Context::default());
    host.input.retire_bindings(old, EventTime(20), &mut target);
    assert_eq!(target.outcome(), (0, Ok(())));
    console.discard_producer(&mut host, old).unwrap();
    key(&mut console, &mut host, 26, true);
    console.execute_frame(&mut host);
    assert_eq!(
        host.input.binding(26).unwrap().producer(),
        Some(replacement)
    );
    assert_eq!(host.output, b"new \n");
}

#[test]
fn stale_manual_forward_wait_releases_only_its_producer_actions_and_bindings() {
    let a = producer(1, 1);
    let b = producer(2, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a, b],
        now: EventTime::from_milliseconds(1),
        ..Capture::default()
    };
    host.input.seed(EventTime::default());
    console
        .append("+forward 444 1; +moveup\n", Context::default())
        .unwrap();
    console.append("+moveleft\n", context(b)).unwrap();
    console
        .append(
            "bind w +forward; +forward; wait 100; -forward\n",
            context(a),
        )
        .unwrap();
    console
        .append("echo healthy\n", Context::default())
        .unwrap();
    console.execute_frame(&mut host);
    let handles = qa_console::input_settings::InputHandles::load(&console.cvars).unwrap();
    let policy = handles.policy(&console.cvars, RuleSetId::Quake2);
    let before = host
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(16), &[policy; SeatId::COUNT])[0];
    assert!(before.movement[0] > 0.0);
    assert!(before.movement[1] < 0.0);
    assert!(before.movement[2] > 0.0);
    host.live.retain(|&producer| producer != a);
    host.now = EventTime::from_milliseconds(20);
    console.execute_frame(&mut host);
    // The native elapsed fraction before retirement may contribute once.
    host.input
        .build_frame_with_policy(EventTime::from_milliseconds(32), &[policy; SeatId::COUNT]);
    let after = host
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(48), &[policy; SeatId::COUNT])[0];
    assert!(after.movement[0] > 0.0);
    assert!(after.movement[1] < 0.0);
    assert!(after.movement[2] > 0.0);
    // The separate human holder survived. Removing only that holder must not
    // reveal a stuck manual hold from the retired module.
    console
        .append("-forward 444 48\n", Context::default())
        .unwrap();
    console.execute_frame(&mut host);
    host.input
        .build_frame_with_policy(EventTime::from_milliseconds(64), &[policy; SeatId::COUNT]);
    let released = host
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(80), &[policy; SeatId::COUNT])[0];
    assert_eq!(released.movement[0], 0.0);
    assert!(released.movement[1] < 0.0);
    assert!(released.movement[2] > 0.0);
    assert!(host.input.binding(26).is_none());
    assert_eq!(host.output, b"healthy \n");
    assert_eq!(host.rejected, 0);
}

#[test]
fn saturated_release_admission_is_counted_after_complete_input_and_alias_cleanup() {
    let a = producer(1, 1);
    let mut console = Console::new(Context::default()).unwrap();
    let mut host = Capture {
        live: vec![a],
        ..Capture::default()
    };
    console
        .append("alias owned \"echo stale\"; bind g +forward\n", context(a))
        .unwrap();
    console.execute_frame(&mut host);
    key(&mut console, &mut host, 10, true);
    console.execute_frame(&mut host);
    let shared_key = KeySource {
        device: DeviceId::Keyboard(3),
        control: 10,
    };
    host.input.button(
        SeatId::FIRST,
        qa_input::Action::Forward,
        true,
        Some(shared_key),
        None,
        EventTime(20),
    );
    let human = Context {
        key_source: Some(shared_key),
        ..Context::default()
    };
    console.append(&"x".repeat(CAPACITY - 1), human).unwrap();
    assert!(console.discard_producer(&mut host, a).is_err());
    assert_eq!(host.rejected, 1);
    assert!(host.input.binding(10).is_none());
    // Consume the unrelated too-long human line before checking alias cleanup.
    let (_, buffer, _) = console.module_parts();
    let mut text = FixedText::<65536>::default();
    let (origin, result) = buffer.next_line(&mut text).unwrap();
    result.unwrap();
    assert_eq!(origin, human);
    assert_eq!(text.as_bytes().len(), CAPACITY - 1);
    console.append("owned\n", context(a)).unwrap();
    console.execute_frame(&mut host);
    assert_eq!(host.output, b"Unknown command \"owned\"\n");
    let handles = qa_console::input_settings::InputHandles::load(&console.cvars).unwrap();
    let policy = handles.policy(&console.cvars, RuleSetId::Quake2);
    host.input
        .build_frame_with_policy(EventTime::from_milliseconds(16), &[policy; SeatId::COUNT]);
    let retained = host
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(32), &[policy; SeatId::COUNT])[0];
    assert!(retained.movement[0] > 0.0);
    host.input.button(
        SeatId::FIRST,
        qa_input::Action::Forward,
        false,
        Some(shared_key),
        None,
        EventTime::from_milliseconds(32),
    );
    let after = host
        .input
        .build_frame_with_policy(EventTime::from_milliseconds(48), &[policy; SeatId::COUNT])[0];
    assert_eq!(after.movement[0], 0.0);
}
