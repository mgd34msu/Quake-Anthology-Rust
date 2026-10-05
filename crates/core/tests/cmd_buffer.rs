//! Behavior tests for the buffered command queue against the donor
//! `src/core/commands` semantics: wait countdowns, alias expansion, script
//! callbacks, overflow-queue ordering, and multi-seat dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{
    source_filter, BufferError, BufferOptions, BufferServices, CommandBuffer, CommandContext, CommandOrigin,
    ForwardedCommand, FrameHooks, ScriptCompletion, ScriptRead,
};
use qa_core::cvar::{flags, q2_flags, CvarRegistry};
use qa_core::identity::IdentityOwner;

fn owner() -> IdentityOwner {
    IdentityOwner::create("cmd-buffer").unwrap()
}

fn seat_context(owner: &IdentityOwner, index: u32) -> CommandContext {
    CommandContext::new(
        owner.session().clone(),
        CommandOrigin::LocalSeat {
            seat: owner.seat(index),
            client: owner.client(index, 0),
        },
    )
}

#[derive(Clone)]
enum ScriptAnswer {
    Ready(Option<String>),
    Failed(String),
}

struct Services {
    scripts: HashMap<String, ScriptAnswer>,
    forwarded: Vec<String>,
    client_game: bool,
    server_game: bool,
    ui_game: bool,
    game_calls: Vec<String>,
    allowed: Option<bool>,
}

impl Services {
    fn new() -> Self {
        Self {
            scripts: HashMap::new(),
            forwarded: Vec::new(),
            client_game: false,
            server_game: false,
            ui_game: false,
            game_calls: Vec::new(),
            allowed: None,
        }
    }

    fn script(mut self, name: &str, text: &str) -> Self {
        self.scripts
            .insert(name.to_string(), ScriptAnswer::Ready(Some(text.to_string())));
        self
    }
}

impl BufferServices for Services {
    fn read_script(&mut self, name: &str, _source: &CommandContext) -> ScriptRead {
        match self.scripts.get(name).cloned().unwrap_or(ScriptAnswer::Ready(None)) {
            ScriptAnswer::Ready(text) => ScriptRead::Ready(text),
            ScriptAnswer::Failed(error) => ScriptRead::Failed(error),
        }
    }

    fn forward_to_server(&mut self, command: &ForwardedCommand) {
        self.forwarded.push(command.raw.clone());
    }

    fn client_game(&mut self, _command: &ForwardedCommand) -> bool {
        self.game_calls.push("client".to_string());
        self.client_game
    }

    fn server_game(&mut self, _command: &ForwardedCommand) -> bool {
        self.game_calls.push("server".to_string());
        self.server_game
    }

    fn ui_game(&mut self, _command: &ForwardedCommand) -> bool {
        self.game_calls.push("ui".to_string());
        self.ui_game
    }

    fn allow_command(&mut self, _command: &ForwardedCommand) -> bool {
        self.allowed.unwrap_or(true)
    }
}

struct Hooks {
    calls: Rc<RefCell<Vec<String>>>,
    running: Rc<RefCell<bool>>,
}

impl FrameHooks for Hooks {
    fn should_continue(&mut self) -> bool {
        *self.running.borrow()
    }

    fn after_dispatch(&mut self) {
        self.calls.borrow_mut().push("dispatched".to_string());
    }
}

fn buffer_for(dialect: Dialect, context: CommandContext) -> (CommandBuffer, CvarRegistry, Services) {
    let buffer = CommandBuffer::new(dialect, context, BufferOptions::new()).unwrap();
    let cvars = CvarRegistry::new(dialect);
    (buffer, cvars, Services::new())
}

fn record_handler(seen: Rc<RefCell<Vec<String>>>) -> qa_core::cmd_buffer::CommandHandler {
    Rc::new(move |inv: &mut qa_core::cmd_buffer::Invocation| {
        seen.borrow_mut().push(inv.args().join(" "));
    })
}

#[test]
fn q3_numeric_wait_counts_down_across_frames() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("record one; wait 2; record two\n", None, None).unwrap();
    assert_eq!(buffer.execute(&mut cvars, &mut services).unwrap(), 2);
    assert_eq!(*seen.borrow(), vec!["one".to_string()]);
    assert_eq!(buffer.execute(&mut cvars, &mut services).unwrap(), 0);
    assert_eq!(*seen.borrow(), vec!["one".to_string()]);
    assert_eq!(buffer.execute(&mut cvars, &mut services).unwrap(), 1);
    assert_eq!(*seen.borrow(), vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn plain_wait_pauses_one_frame_on_every_dialect() {
    for dialect in [
        Dialect::Q1Netquake,
        Dialect::Q1Quakeworld,
        Dialect::Q2Classic,
        Dialect::Q2Rerelease,
        Dialect::Q3,
    ] {
        let owner = owner();
        let (mut buffer, mut cvars, mut services) = buffer_for(dialect, seat_context(&owner, 0));
        let seen = Rc::new(RefCell::new(Vec::new()));
        buffer
            .register("record", Some(record_handler(seen.clone())), None, &cvars)
            .unwrap();
        buffer.append("record one; wait; record two\n", None, None).unwrap();
        buffer.execute(&mut cvars, &mut services).unwrap();
        assert_eq!(*seen.borrow(), vec!["one".to_string()], "{dialect:?}");
        buffer.execute(&mut cvars, &mut services).unwrap();
        assert_eq!(
            *seen.borrow(),
            vec!["one".to_string(), "two".to_string()],
            "{dialect:?}"
        );
    }
}

#[test]
fn advance_program_frame_ticks_empty_waits_and_rejoins_overflow_text() {
    let owner = owner();
    let context = seat_context(&owner, 0);
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, context);
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("wait 2\n", None, None).unwrap();
    assert_eq!(buffer.advance_program_frame(&mut cvars, &mut services).unwrap(), 1);
    assert_eq!(buffer.advance_program_frame(&mut cvars, &mut services).unwrap(), 0);
    assert_eq!(buffer.advance_program_frame(&mut cvars, &mut services).unwrap(), 0);
    assert!(!buffer.has_pending_commands());

    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q2Classic, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("record held\n", None, None).unwrap();
    buffer.copy_to_defer().unwrap();
    assert_eq!(buffer.deferred_text(), "record held\n");
    buffer.advance_program_frame(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["held".to_string()]);
}

#[test]
fn aliases_expand_with_family_rules_and_loop_guard() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q2Classic, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("note", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    cvars.register("word", "expanded", 0).unwrap();
    buffer
        .append(
            "alias old \"note old\"; alias newer \"note $word\"; newer; wait 20; note \"$word\"\n",
            None,
            None,
        )
        .unwrap();
    buffer.copy_to_defer().unwrap();
    buffer.append("note tail\n", None, None).unwrap();
    buffer.insert_from_defer().unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["expanded".to_string()]);
    assert_eq!(buffer.complete("n"), Some("note".to_string()));
    assert_eq!(buffer.alias_names(), vec!["newer".to_string(), "old".to_string()]);
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(
        *seen.borrow(),
        vec!["expanded".to_string(), "$word".to_string(), "tail".to_string()]
    );

    let printed = Rc::new(RefCell::new(Vec::new()));
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    buffer.append("alias again again; again\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert!(printed.borrow().iter().any(|line| line.contains("ALIAS_LOOP_COUNT")));
}

#[test]
fn alias_builtin_lists_and_q3_rejects_aliases() {
    let owner = owner();
    let printed = Rc::new(RefCell::new(Vec::new()));
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q1Netquake, seat_context(&owner, 0));
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    buffer.define_alias("jump", "+jump; -jump\n").unwrap();
    buffer.append("alias\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(
        *printed.borrow(),
        vec![
            "Current alias commands:\n".to_string(),
            "jump : +jump; -jump\n\n".to_string()
        ]
    );

    let (mut buffer, _, _) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    assert_eq!(buffer.define_alias("x", "echo x\n"), Err(BufferError::Q3Alias));
    assert!(buffer.alias_names().is_empty());
}

#[test]
fn script_completion_follows_nested_scripts_across_wait() {
    let owner = owner();
    let context = seat_context(&owner, 0);
    let mut buffer = CommandBuffer::new(Dialect::Q1Netquake, context.clone(), BufferOptions::new()).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
    let mut services = Services::new()
        .script("quake.rc", "exec default.cfg\nexec config.cfg\nrecord autoexec\n")
        .script("default.cfg", "record default\nwait\n");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let seen_handler = seen.clone();
    buffer
        .register(
            "record",
            Some(Rc::new(move |inv: &mut qa_core::cmd_buffer::Invocation| {
                seen_handler.borrow_mut().push(inv.args().join(" "));
            })),
            None,
            &cvars,
        )
        .unwrap();
    let completed = Rc::new(RefCell::new(Vec::new()));
    let completed_handler = completed.clone();
    buffer.set_on_script_complete(Some(move |event: &ScriptCompletion| {
        completed_handler
            .borrow_mut()
            .push(format!("{}:{:?}", event.name, event.result));
    }));
    buffer.append("exec quake.rc\nrecord caller\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["default".to_string()]);
    assert!(completed.borrow().is_empty());
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(
        *seen.borrow(),
        vec!["default".to_string(), "autoexec".to_string(), "caller".to_string()]
    );
    assert_eq!(
        *completed.borrow(),
        vec![
            "default.cfg:Completed".to_string(),
            "config.cfg:Missing".to_string(),
            "quake.rc:Completed".to_string(),
        ]
    );
}

#[test]
fn nested_exec_reads_run_synchronously_in_order_with_wait_preserved() {
    let owner = owner();
    let context = seat_context(&owner, 0);
    let mut buffer = CommandBuffer::new(Dialect::Q3, context.clone(), BufferOptions::new()).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q3);
    let mut services = Services::new();
    services.scripts.insert(
        "outer.cfg".to_string(),
        ScriptAnswer::Ready(Some("record outer; exec inner; record outer-tail\n".to_string())),
    );
    services.scripts.insert(
        "inner.cfg".to_string(),
        ScriptAnswer::Ready(Some("record inner; wait; record inner-tail\n".to_string())),
    );
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("exec outer; record after\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["outer".to_string(), "inner".to_string()]);
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(
        *seen.borrow(),
        vec!["outer", "inner", "inner-tail", "outer-tail", "after"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn failed_script_reads_report_once_and_continue() {
    let owner = owner();
    let printed = Rc::new(RefCell::new(Vec::new()));
    let context = seat_context(&owner, 0);
    let mut buffer = CommandBuffer::new(Dialect::Q3, context, BufferOptions::new()).unwrap();
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    let mut cvars = CvarRegistry::new(Dialect::Q3);
    let mut services = Services::new();
    services
        .scripts
        .insert("bad.cfg".to_string(), ScriptAnswer::Failed("read denied".to_string()));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    let completed = Rc::new(RefCell::new(Vec::new()));
    let completed_handler = completed.clone();
    buffer.bind_script_completion(move |event: &ScriptCompletion| {
        completed_handler
            .borrow_mut()
            .push(format!("{}:{:?}", event.name, event.result));
    });
    buffer.append("exec bad; record after\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["after".to_string()]);
    assert!(printed
        .borrow()
        .iter()
        .any(|line| line == "couldn't exec bad.cfg: read denied\n"));
    assert_eq!(*completed.borrow(), vec!["bad.cfg:Failed(\"read denied\")".to_string()]);

    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("exec missing; record after\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["after".to_string()]);
}

#[test]
fn q2_scripts_join_caller_bytes_across_completion_nodes() {
    for dialect in [Dialect::Q2Classic, Dialect::Q2Rerelease] {
        let owner = owner();
        let context = seat_context(&owner, 0);
        let mut buffer = CommandBuffer::new(dialect, context.clone(), BufferOptions::new()).unwrap();
        let mut cvars = CvarRegistry::new(dialect);
        let mut services = Services::new()
            .script("outer.cfg", "exec inner.cfg")
            .script("inner.cfg", "record nested");
        let seen = Rc::new(RefCell::new(Vec::new()));
        buffer
            .register("record", Some(record_handler(seen.clone())), None, &cvars)
            .unwrap();
        let completed = Rc::new(RefCell::new(Vec::new()));
        let completed_handler = completed.clone();
        buffer.set_on_script_complete(Some(move |event: &ScriptCompletion| {
            completed_handler.borrow_mut().push(event.name.clone());
        }));
        buffer.append("exec outer.cfg\n\nrecord after\n", None, None).unwrap();
        buffer.execute(&mut cvars, &mut services).unwrap();
        assert_eq!(*seen.borrow(), vec!["nestedrecord after".to_string()], "{dialect:?}");
        assert_eq!(
            *completed.borrow(),
            vec!["inner.cfg".to_string(), "outer.cfg".to_string()]
        );
    }
}

#[test]
fn overflow_queue_orders_q2_and_rejects_other_dialects() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q2Classic, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("record one\n", None, None).unwrap();
    buffer.copy_to_defer().unwrap();
    buffer.append("record two\n", None, None).unwrap();
    assert_eq!(buffer.pending_text(), "record two\n");
    buffer.insert_from_defer().unwrap();
    assert_eq!(buffer.pending_text(), "record one\nrecord two\n");
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["one".to_string(), "two".to_string()]);

    for dialect in [Dialect::Q1Netquake, Dialect::Q1Quakeworld, Dialect::Q3] {
        let (mut buffer, _, _) = buffer_for(dialect, seat_context(&owner, 0));
        assert_eq!(buffer.copy_to_defer(), Err(BufferError::DeferDialect));
        assert_eq!(buffer.insert_from_defer(), Err(BufferError::DeferDialect));
    }
}

#[test]
fn alternating_seats_keep_script_source_through_wait_and_nesting() {
    let owner = owner();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut buffers = Vec::new();
    for index in 0..2 {
        let context = seat_context(&owner, index);
        let mut buffer = CommandBuffer::new(Dialect::Q1Quakeworld, context, BufferOptions::new()).unwrap();
        let calls_handler = calls.clone();
        buffer
            .register(
                "note",
                Some(Rc::new(move |inv: &mut qa_core::cmd_buffer::Invocation| {
                    let (seat, kind) = match &inv.source.origin {
                        CommandOrigin::Script { caller, .. } => match caller.as_ref() {
                            CommandOrigin::LocalSeat { seat, .. } => (seat.index() as i32, "script"),
                            _ => (-1, "script"),
                        },
                        CommandOrigin::LocalSeat { seat, .. } => (seat.index() as i32, "local-seat"),
                        _ => (-1, "other"),
                    };
                    let what = inv.args().first().cloned().unwrap_or_default();
                    calls_handler.borrow_mut().push(format!("{seat}:{what}:{kind}"));
                    if what == "nested" {
                        inv.execute_now("note child").unwrap();
                    }
                })),
                None,
                &CvarRegistry::new(Dialect::Q1Quakeworld),
            )
            .unwrap();
        buffers.push(buffer);
    }
    let mut cvars = [
        CvarRegistry::new(Dialect::Q1Quakeworld),
        CvarRegistry::new(Dialect::Q1Quakeworld),
    ];
    let mut services = [
        Services::new().script("script.cfg", "note script;wait;note resumed"),
        Services::new(),
    ];
    buffers[0].append("exec script.cfg;note nested\n", None, None).unwrap();
    buffers[1].append("note second\n", None, None).unwrap();
    buffers[0].execute(&mut cvars[0], &mut services[0]).unwrap();
    buffers[1].execute(&mut cvars[1], &mut services[1]).unwrap();
    buffers[0].execute(&mut cvars[0], &mut services[0]).unwrap();
    assert_eq!(
        *calls.borrow(),
        vec![
            "0:script:script".to_string(),
            "1:second:local-seat".to_string(),
            "0:resumed:script".to_string(),
            "0:nested:local-seat".to_string(),
            "0:child:local-seat".to_string(),
        ]
    );
}

#[test]
fn disconnected_clients_lose_queued_work() {
    let owner = owner();
    let context = seat_context(&owner, 0);
    let client = owner.client(0, 0);
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q2Classic, context.clone());
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("record kept\n", None, None).unwrap();
    let other = seat_context(&owner, 1);
    buffer.append("record dropped\n", Some(&other), None).unwrap();
    buffer.discard_client(&owner.client(1, 0));
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["kept".to_string()]);
    assert_eq!(
        buffer.append("record late\n", Some(&other), None),
        Err(BufferError::DisconnectedClient)
    );
    let _ = client;
}

#[test]
fn program_revision_tracks_every_mutation() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let start = buffer.program_revision();
    buffer.append("echo hi\n", None, None).unwrap();
    assert!(buffer.program_revision() > start);
    let after_append = buffer.program_revision();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert!(buffer.program_revision() > after_append);
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("record x; wait\n", None, None).unwrap();
    let before_wait = buffer.program_revision();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert!(buffer.program_revision() > before_wait);
}

#[test]
fn insertion_and_overflow_match_family_behavior() {
    for dialect in [
        Dialect::Q1Netquake,
        Dialect::Q1Quakeworld,
        Dialect::Q2Classic,
        Dialect::Q3,
    ] {
        let owner = owner();
        let (mut buffer, _, _) = buffer_for(dialect, seat_context(&owner, 0));
        buffer.append("tail", None, None).unwrap();
        buffer.insert("head", None, None).unwrap();
        let expected = if dialect == Dialect::Q1Quakeworld || dialect == Dialect::Q3 {
            "head\ntail"
        } else {
            "headtail"
        };
        assert_eq!(buffer.pending_text(), expected, "{dialect:?}");
    }
    let owner = owner();
    let printed = Rc::new(RefCell::new(Vec::new()));
    let options = BufferOptions {
        max_buffer_length: Some(8),
        builtins: Some(false),
        ..BufferOptions::new()
    };
    let mut buffer = CommandBuffer::new(Dialect::Q3, seat_context(&owner, 0), options).unwrap();
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    buffer.append("12345678", None, None).unwrap();
    assert_eq!(buffer.pending_text(), "");
    buffer.append("123456", None, None).unwrap();
    buffer.insert("x", None, None).unwrap();
    assert_eq!(buffer.pending_text(), "x\n123456");
    buffer.insert("y", None, None).unwrap();
    assert_eq!(buffer.pending_text(), "x\n123456");
    assert_eq!(printed.borrow().len(), 2);
}

#[test]
fn registration_is_newest_first_and_q3_touches_dispatched_entries() {
    let owner = owner();
    let options = BufferOptions {
        builtins: Some(false),
        ..BufferOptions::new()
    };
    let mut buffer = CommandBuffer::new(Dialect::Q3, seat_context(&owner, 0), options).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q3);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let old = calls.clone();
    buffer
        .register(
            "Choice",
            Some(Rc::new(move |_| old.borrow_mut().push("old".to_string()))),
            None,
            &cvars,
        )
        .unwrap();
    let new = calls.clone();
    buffer
        .register(
            "choice",
            Some(Rc::new(move |_| new.borrow_mut().push("new".to_string()))),
            None,
            &cvars,
        )
        .unwrap();
    buffer
        .register("other", Some(record_handler(calls.clone())), None, &cvars)
        .unwrap();
    assert!(!buffer
        .register("choice", Some(record_handler(calls.clone())), None, &cvars)
        .unwrap());
    assert_eq!(buffer.complete("ch"), Some("choice".to_string()));
    let mut services = Services::new();
    buffer
        .execute_now(Some("CHOICE"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert_eq!(*calls.borrow(), vec!["new".to_string()]);
    assert_eq!(
        buffer.registered_names(),
        vec!["choice".to_string(), "other".to_string(), "Choice".to_string()]
    );
}

#[test]
fn q3_null_commands_fall_back_through_client_server_ui() {
    let owner = owner();
    let context = seat_context(&owner, 0);
    let mut buffer = CommandBuffer::new(Dialect::Q3, context, BufferOptions::new()).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q3);
    cvars.register("rate", "1", 0).unwrap();
    buffer.register("rate", None, None, &cvars).unwrap();
    let mut services = Services::new();
    buffer.append("rate 2; wait 2; unknown\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(cvars.variable_string("rate"), "2");
    assert!(services.game_calls.is_empty());
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert!(services.game_calls.is_empty());
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(
        services.game_calls,
        vec!["client".to_string(), "server".to_string(), "ui".to_string()]
    );
    assert_eq!(services.forwarded, vec![" unknown".to_string()]);
}

#[test]
fn cvar_fallback_reports_and_sets_with_family_text() {
    let owner = owner();
    let printed = Rc::new(RefCell::new(Vec::new()));
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    cvars.register("volume", "0.7", 0).unwrap();
    buffer
        .execute_now(Some("volume"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert!(printed.borrow().last().unwrap().contains("default:"));
    buffer
        .execute_now(Some("volume 0.9"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert_eq!(cvars.variable_string("volume"), "0.9");

    let printed = Rc::new(RefCell::new(Vec::new()));
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q1Netquake, seat_context(&owner, 0));
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    buffer
        .execute_now(Some("bogus"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert_eq!(*printed.borrow(), vec!["Unknown command \"bogus\"\n".to_string()]);

    let printed = Rc::new(RefCell::new(Vec::new()));
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q1Quakeworld, seat_context(&owner, 0));
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    buffer
        .execute_now(Some("bogus"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert!(printed.borrow().is_empty());
    cvars.register("developer", "1", 0).unwrap();
    buffer
        .execute_now(Some("bogus"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert_eq!(*printed.borrow(), vec!["Unknown command \"bogus\"\n".to_string()]);
}

#[test]
fn common_cvar_commands_follow_donor_math() {
    for dialect in [
        Dialect::Q1Netquake,
        Dialect::Q1Quakeworld,
        Dialect::Q2Classic,
        Dialect::Q2Rerelease,
        Dialect::Q3,
    ] {
        let owner = owner();
        let (mut buffer, mut cvars, mut services) = buffer_for(dialect, seat_context(&owner, 0));
        cvars.register("amount", "0", 0).unwrap();
        cvars.register("cycle", "Red", 0).unwrap();
        cvars.register("truth", "0.0", 0).unwrap();
        buffer
            .execute_now(Some("inc amount 0.5"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "0.500000", "{dialect:?}");
        buffer
            .execute_now(Some("inc amount 0.5"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "1", "{dialect:?}");
        buffer
            .execute_now(Some("dec amount"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "0", "{dialect:?}");
        buffer
            .execute_now(Some("toggle truth"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(
            cvars.variable_string("truth"),
            if dialect == Dialect::Q3 { "1" } else { "0.0" },
            "{dialect:?}"
        );
        buffer
            .execute_now(Some("toggle cycle red BLUE"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("cycle"), "BLUE", "{dialect:?}");
        buffer
            .execute_now(Some("toggle cycle red BLUE"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("cycle"), "red", "{dialect:?}");
        cvars.set("amount", "1e3", true).unwrap();
        buffer
            .execute_now(Some("inc amount"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "1e3", "{dialect:?}");
        buffer
            .execute_now(Some("reset amount"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "0", "{dialect:?}");
        buffer
            .execute_now(Some("inc absent"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert!(cvars.get("absent").is_none() || dialect == Dialect::Q3);
        cvars.set("amount", ".", true).unwrap();
        buffer
            .execute_now(Some("inc amount"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "1", "{dialect:?}");
        cvars.set("amount", "16777216", true).unwrap();
        buffer
            .execute_now(Some("inc amount"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "16777216", "{dialect:?}");
        cvars.set("amount", "1", true).unwrap();
        buffer
            .execute_now(Some("inc amount inf"), None, None, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(cvars.variable_string("amount"), "inf", "{dialect:?}");
    }
}

#[test]
fn q2_set_flags_and_private_macros_follow_donor() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q2Classic, seat_context(&owner, 0));
    cvars.register("secret", "hidden", q2_flags::PRIVATE).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("note", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("note $secret\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec![String::new()]);

    buffer
        .execute_now(Some("seta nick old"), None, None, &mut cvars, &mut services)
        .unwrap();
    buffer
        .execute_now(Some("setu nick new name"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert_eq!(cvars.variable_string("nick"), "new name");
    assert!(cvars.userinfo_modified());
    let found = cvars.get("nick").unwrap();
    assert_eq!(found.flags, q2_flags::CUSTOM | q2_flags::ARCHIVE | q2_flags::USER_INFO);
    buffer
        .execute_now(Some("set rom 2"), None, None, &mut cvars, &mut services)
        .unwrap();
    cvars.register("latched", "1", q2_flags::LATCH).unwrap();
    cvars.set_server_active(true);
    buffer
        .execute_now(Some("inc latched"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert_eq!(cvars.get("latched").unwrap().latched_value, Some("2".to_string()));
    let _ = flags::ARCHIVE;
}

#[test]
fn vstr_executes_variable_text_and_cmdlist_filters() {
    let owner = owner();
    let printed = Rc::new(RefCell::new(Vec::new()));
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let sink = printed.clone();
    buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    cvars.register("nextmap", "record dm1", 0).unwrap();
    buffer.append("vstr nextmap\n", None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["dm1".to_string()]);
    buffer
        .execute_now(Some("cmdlist rec*"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert!(printed.borrow().iter().any(|line| line == "record\n"));
    assert!(printed.borrow().iter().any(|line| line == "1 commands\n"));
    buffer
        .execute_now(Some("cvarlist next*"), None, None, &mut cvars, &mut services)
        .unwrap();
    assert!(printed.borrow().iter().any(|line| line.contains("nextmap")));
}

#[test]
fn hooks_observe_every_dispatch_and_can_stop_the_frame() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer.append("record one; record two\n", None, None).unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let running = Rc::new(RefCell::new(true));
    let mut hooks = Hooks {
        calls: calls.clone(),
        running: running.clone(),
    };
    buffer.execute_hooked(&mut cvars, &mut services, &mut hooks).unwrap();
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(*seen.borrow(), vec!["one".to_string(), "two".to_string()]);

    buffer.append("record three; record four\n", None, None).unwrap();
    *running.borrow_mut() = false;
    let mut hooks = Hooks {
        calls: calls.clone(),
        running: running.clone(),
    };
    buffer.execute_hooked(&mut cvars, &mut services, &mut hooks).unwrap();
    assert_eq!(*seen.borrow(), vec!["one".to_string(), "two".to_string()]);
    assert!(!buffer.pending_text().is_empty());
}

#[test]
fn replacement_buffers_take_over_pending_programs() {
    let owner = owner();
    let context = seat_context(&owner, 0);
    let mut original = CommandBuffer::new(Dialect::Q3, context.clone(), BufferOptions::new()).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q3);
    let mut services = Services::new();
    services.scripts.insert(
        "late.cfg".to_string(),
        ScriptAnswer::Ready(Some("record script\n".to_string())),
    );
    let seen = Rc::new(RefCell::new(Vec::new()));
    original
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    // `wait` stops the drain mid-program so the replacement takes over text.
    original.append("exec late; wait; record after\n", None, None).unwrap();
    original.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["script".to_string()]);
    let mut replacement = CommandBuffer::new(Dialect::Q3, context, BufferOptions::new()).unwrap();
    replacement
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    replacement.copy_pending_from(&original).unwrap();
    replacement.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["script".to_string(), "after".to_string()]);
}

#[test]
fn source_filter_matches_donor_wildcards() {
    use qa_core::cmd_buffer::source_filter;
    assert!(source_filter("rec*", "record", false).unwrap());
    assert!(source_filter("REC*", "record", false).unwrap());
    assert!(!source_filter("REC*", "record", true).unwrap());
    assert!(source_filter("re?ord", "record", false).unwrap());
    assert!(source_filter("[a-z]ecord", "record", false).unwrap());
    assert!(!source_filter("[a-z]ecord", "Record", true).unwrap());
    assert!(source_filter("*ord", "record", false).unwrap());
    assert!(!source_filter("other*", "record", false).unwrap());
}

#[test]
fn stuffcmds_inserts_startup_and_command_line_text() {
    let owner = owner();
    let options = BufferOptions {
        startup_command_text: Some("record startup\n".to_string()),
        ..BufferOptions::new()
    };
    let mut buffer = CommandBuffer::new(Dialect::Q1Netquake, seat_context(&owner, 0), options).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
    let mut services = Services::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer
        .execute_now(Some("stuffcmds"), None, None, &mut cvars, &mut services)
        .unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["startup".to_string()]);

    let options = BufferOptions {
        command_line: vec!["quake".to_string(), "+record".to_string(), "cli".to_string()],
        ..BufferOptions::new()
    };
    let mut buffer = CommandBuffer::new(Dialect::Q1Quakeworld, seat_context(&owner, 0), options).unwrap();
    let mut cvars = CvarRegistry::new(Dialect::Q1Quakeworld);
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    buffer
        .execute_now(Some("stuffcmds"), None, None, &mut cvars, &mut services)
        .unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["cli".to_string()]);
}

#[test]
fn high_bytes_count_once_against_buffer_limits() {
    let owner = owner();
    let (mut buffer, _, _) = buffer_for(Dialect::Q2Classic, seat_context(&owner, 0));
    let wide = "ÿ".repeat(5000);
    buffer.append(&wide, None, None).unwrap();
    // 5000 engine bytes queue even though the UTF-8 display text is
    // 10000 bytes: the 8192 limit measures engine bytes.
    assert_eq!(buffer.pending_text(), wide);
    buffer.insert(&"a".repeat(3000), None, None).unwrap();
    assert_eq!(buffer.pending_text().chars().count(), 8000);
    assert!(matches!(
        buffer.insert(&"b".repeat(200), None, None),
        Err(BufferError::InsertOverflow)
    ));

    let (mut buffer, _, _) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let wide = "ÿ".repeat(9000);
    buffer.append(&wide, None, None).unwrap();
    // 9000 engine bytes sit under the 16384 limit; the 18000 display
    // bytes would have overflowed a UTF-8 length check.
    assert_eq!(buffer.pending_text(), wide);
    buffer.insert(&"a".repeat(7000), None, None).unwrap();
    assert_eq!(buffer.pending_text().chars().count(), 16001);
    buffer.insert(&"b".repeat(500), None, None).unwrap();
    assert_eq!(buffer.pending_text().chars().count(), 16001);
}

#[test]
fn q3_long_line_cuts_at_engine_bytes() {
    let owner = owner();
    let (mut buffer, mut cvars, mut services) = buffer_for(Dialect::Q3, seat_context(&owner, 0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    buffer
        .register("record", Some(record_handler(seen.clone())), None, &cvars)
        .unwrap();
    // `;` sits at engine offset 1025, so Quake III cuts the line at
    // 1023 engine bytes: `record ` plus 1016 wide bytes.
    let line = format!("record {}ZY;record done", "ÿ".repeat(1016));
    buffer.insert(&line, None, None).unwrap();
    buffer.execute(&mut cvars, &mut services).unwrap();
    assert_eq!(*seen.borrow(), vec!["ÿ".repeat(1016), "done".to_string()]);
}

#[test]
fn source_filter_star_run_counts_engine_bytes() {
    // 600 engine bytes, 1200 UTF-8 bytes: under the 1024 scratch limit.
    let run = "ÿ".repeat(600);
    assert!(source_filter(&format!("*{run}*"), &run, true).unwrap());
    // 1024 engine bytes trips the scratch limit.
    let run = "ÿ".repeat(1024);
    assert!(source_filter(&format!("*{run}*"), &run, true).is_err());
}
