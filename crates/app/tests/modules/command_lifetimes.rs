//! Module imports, central teardown and the sole console own deferred text.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{Export, ModuleRequest, Phase, Program, State},
};
use qa_compat::{abi::Q3_SERVER, services::CallContext};
use qa_console::{
    commands::Console,
    views::{CommandProducer, Context},
};
use qa_core::{primitives::*, sys_events::*};
use qa_formats::program::qvm::Opcode;
use qa_session::timing::TickRate;
use qa_world::entities::AllocationPolicy;
use std::time::Duration;

use super::service_program;

pub(super) type ProducerProgram = (Program, Vec<Export>);
const A: ModuleId = ModuleId(1);
const B: ModuleId = ModuleId(2);
const BLOCKED: &[u8] = b"alias Collision \"set lifetime_stale_alias 1\";wait 100;Collision;set lifetime_stale_tail 1;quit\n";
const PEER: &[u8] =
    b"alias COLLISION \"set lifetime_peer_alias 1\";COLLISION;set lifetime_peer_progress 1\n";
const REPLACEMENT: &[u8] =
    b"alias Collision \"set lifetime_new_alias 1\";Collision;set lifetime_new_tail 1\n";

fn qvm(text: &[u8]) -> ProducerProgram {
    qvm_console(text, None, 2, 2)
}
fn qvm_console(
    text: &[u8],
    only_export: Option<u32>,
    execution: i32,
    trap_export: u32,
) -> ProducerProgram {
    use Opcode::*;
    // Export0 is the ordinary quiet frame. Export1 appends, and export2
    // appends before trapping, so failed initialization cannot hide the queue.
    let mut operations = vec![
        (Enter, 64),
        (Local, 72),
        (Load4, 0),
        (Const, only_export.unwrap_or(0) as i32),
        (if only_export.is_some() { Ne } else { Eq }, 0),
        (Const, execution),
        (Arg, 8),
        (Const, 128),
        (Arg, 12),
        (Const, -15),
        (Call, 0),
        (Pop, 0),
        (Local, 72),
        (Load4, 0),
        (Const, trap_export as i32),
        (Ne, 0),
        (Const, 1),
        (Const, 0),
        (DivI, 0),
        (Pop, 0),
        (Const, 0),
        (Leave, 64),
    ];
    operations[4].1 = 20;
    operations[15].1 = 20;
    let mut data = vec![0; 128];
    data.extend_from_slice(text);
    data.push(0);
    data.resize((data.len() + 3) & !3, 0);
    (
        Program::Qvm {
            vm: Box::new(service_program::program(&operations, &data)),
            imports: &Q3_SERVER,
        },
        vec![],
    )
}
fn request(
    runtime: &mut Runtime,
    module: ModuleId,
    anchor: Option<EntityId>,
    program: ProducerProgram,
) -> ModuleRequest {
    let anchor = anchor.unwrap_or_else(|| {
        runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
            .unwrap()
            .id
    });
    ModuleRequest {
        search: None,
        source: None,
        q3_image: None,
        context: CallContext {
            module,
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::QuakeWorld,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
            console: Context {
                source: RuleSetId::Quake2,
                ..Default::default()
            },
            allocation: AllocationPolicy::q3(0),
            link_order: qa_world::area::LinkOrder::Tail,
        },
        phase: Phase::Server,
        timing_rules: RuleSetId::Quake3,
        rate: TickRate::FrameDriven,
        anchor,
        program: program.0,
        entries: (0..4).collect(),
        frame: Export::words(CallbackId(0), &[]),
        prepare: program.1,
        initialize: vec![],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    }
}
fn fixture(
    make: impl Fn(&[u8]) -> ProducerProgram,
    fail_init: bool,
) -> (Box<FrameHost>, [EntityId; 2]) {
    fixture_commands(make, BLOCKED, fail_init)
}
fn fixture_commands(
    make: impl Fn(&[u8]) -> ProducerProgram,
    commands: &[u8],
    fail_init: bool,
) -> (Box<FrameHost>, [EntityId; 2]) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut first = request(&mut runtime, A, None, make(commands));
    // Shutdown exports are allowed to append, but their closed producer's
    // new text must be retired along with the previously suspended tail.
    first.shutdown = vec![Export::words(CallbackId(1), &[])];
    if fail_init {
        first.initialize = vec![Export::words(CallbackId(2), &[])];
    }
    let peer = request(&mut runtime, B, None, make(PEER));
    let anchors = [first.anchor, peer.anchor];
    let mut host = Box::new(
        FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![first, peer],
        )
        .unwrap(),
    );
    host.initialize_modules(Phase::Server);
    (host, anchors)
}
fn invoke(
    host: &mut FrameHost,
    module: ModuleId,
    entry: u32,
) -> Result<(), qa_session::dispatch::CallError> {
    host.call_module_export(
        module,
        Export::words(CallbackId(entry), &[]),
        ThinkTime::Milliseconds(0),
    )
}
fn text<'a>(host: &'a FrameHost, name: &str) -> Option<&'a str> {
    host.console
        .cvars
        .find(name)
        .map(|id| host.console.cvars.text(id))
}
fn human(host: &mut FrameHost, commands: &str) {
    host.console.append(commands, Context::default()).unwrap();
}
fn run_commands(host: &mut FrameHost) {
    host.execute_commands();
}
struct Source {
    time: EventTime,
    events: Vec<EventKind<'static>>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        for kind in std::mem::take(&mut self.events) {
            queue
                .push(SysEvent {
                    time: self.time,
                    kind,
                })
                .unwrap();
        }
        queue
            .push(SysEvent {
                time: self.time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}
fn frame_with(host: &mut FrameHost, events: Vec<EventKind<'static>>) -> qa_app::host::FrameResult {
    let mut source = Source {
        time: EventTime::from_milliseconds(host.time.milliseconds() + 16),
        events,
    };
    let result = host.frame(&mut source, true);
    assert_eq!(result.drains, 2);
    result
}
fn command_frame(host: &mut FrameHost) {
    let calls = host.module_counts(B).unwrap().calls;
    // The first frame seeds the shared timeline. The following queued frame
    // proves that the healthy provider continues after retirement as well.
    frame_with(host, vec![]);
    let next = frame_with(host, vec![]);
    assert!(next.server_ticks > 0);
    assert!(host.module_counts(B).unwrap().calls > calls);
}
fn parts(host: Box<FrameHost>) -> (Console<Runtime>, Runtime) {
    (host.console, host.runtime)
}
fn assert_stale_absent(host: &FrameHost) {
    assert_eq!(text(host, "lifetime_stale_alias"), None);
    assert_eq!(text(host, "lifetime_stale_tail"), None);
    assert!(!host.runtime.quit);
}

pub(super) fn shutdown_retires_alias_wait_and_shutdown_output(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    let (mut host, _) = fixture(make, false);
    human(
        &mut host,
        "alias collision \"set lifetime_human_alias 1\"\n",
    );
    run_commands(&mut host);
    invoke(&mut host, A, 1).unwrap();
    invoke(&mut host, B, 1).unwrap();
    human(&mut host, "collision;set lifetime_human_progress 1\n");
    run_commands(&mut host);
    assert!(!host.console.idle());
    assert_eq!(text(&host, "lifetime_peer_progress"), None);
    assert_eq!(text(&host, "lifetime_human_progress"), None);
    assert!(host.shutdown_module(A));
    command_frame(&mut host);
    assert_eq!(text(&host, "lifetime_peer_alias"), Some("1"));
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
    assert_eq!(text(&host, "lifetime_human_alias"), Some("1"));
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
    assert_stale_absent(&host);
    assert!(host.console.idle());
    assert_eq!(host.module_state(B), Some(State::Running));
}
pub(super) fn failed_initialization_cannot_leave_queued_work(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    let (mut host, _) = fixture(make, true);
    assert_eq!(host.module_state(A), Some(State::Failed));
    assert_eq!(host.module_counts(A).unwrap().traps, 1);
    invoke(&mut host, B, 1).unwrap();
    human(&mut host, "set lifetime_human_progress 1\n");
    command_frame(&mut host);
    assert_stale_absent(&host);
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
    assert_eq!(host.module_state(B), Some(State::Running));
}
pub(super) fn stale_generation_and_new_publication_use_exact_lifetimes(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    let (mut host, anchors) = fixture(&make, false);
    human(
        &mut host,
        "alias collision \"set lifetime_human_alias 1\"\n",
    );
    run_commands(&mut host);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    assert!(
        host.runtime
            .server
            .entities
            .release(anchors[0], ThinkTime::Milliseconds(0))
    );
    let replacement = host
        .runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), A, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    assert_eq!(replacement.slot, anchors[0].slot);
    assert_ne!(replacement.generation, anchors[0].generation);
    human(&mut host, "set lifetime_human_progress 1\n");
    // Liveness is checked before an owned wait decrements, so a new occupant
    // of the same slot cannot keep unrelated human commands suspended.
    run_commands(&mut host);
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
    assert_stale_absent(&host);
    let stale_context = Context {
        producer: Some(CommandProducer {
            module: A,
            anchor: anchors[0],
        }),
        ..Default::default()
    };
    host.console
        .append(
            "alias orphan \"quit\";wait 100;set lifetime_old_generation 1\n",
            stale_context,
        )
        .unwrap();
    let (console, mut runtime) = parts(host);
    let new_a = request(&mut runtime, A, Some(replacement), make(REPLACEMENT));
    let same_b = request(&mut runtime, B, Some(anchors[1]), make(PEER));
    let mut host = Box::new(
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![new_a, same_b])
            .unwrap(),
    );
    host.initialize_modules(Phase::Server);
    invoke(&mut host, A, 1).unwrap();
    invoke(&mut host, B, 1).unwrap();
    human(&mut host, "collision\n");
    command_frame(&mut host);
    assert_eq!(text(&host, "lifetime_old_generation"), None);
    assert_eq!(text(&host, "lifetime_new_alias"), Some("1"));
    assert_eq!(text(&host, "lifetime_new_tail"), Some("1"));
    assert_eq!(text(&host, "lifetime_peer_alias"), Some("1"));
    assert_eq!(text(&host, "lifetime_human_alias"), Some("1"));
    assert_stale_absent(&host);
}
pub(super) fn absent_cold_row_cannot_lend_its_live_anchor(make: impl Fn(&[u8]) -> ProducerProgram) {
    let (mut host, anchors) = fixture(&make, false);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    assert!(host.runtime.server.entities.resolve(anchors[0]).is_some());
    human(&mut host, "set lifetime_human_progress 1\n");
    let (console, mut runtime) = parts(host);
    let peer = request(&mut runtime, B, Some(anchors[1]), make(PEER));
    let mut host = Box::new(
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![peer]).unwrap(),
    );
    // Entity liveness alone is insufficient: no admitted module row owns the
    // previous console producer after this cold publication.
    assert!(host.runtime.server.entities.resolve(anchors[0]).is_some());
    host.initialize_modules(Phase::Server);
    invoke(&mut host, B, 1).unwrap();
    command_frame(&mut host);
    assert_stale_absent(&host);
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
}
pub(super) fn transferred_owner_invalidates_wait_before_healthy_work(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    let (mut host, anchors) = fixture(make, false);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    host.runtime.server.entities.columns.owner[anchors[0].slot as usize] = B;
    human(&mut host, "set lifetime_human_progress 1\n");
    invoke(&mut host, B, 1).unwrap();
    run_commands(&mut host);
    assert_stale_absent(&host);
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
}
pub(super) fn fatal_export_retires_before_its_queued_text(make: impl Fn(&[u8]) -> ProducerProgram) {
    let (mut host, _) = fixture(make, false);
    assert!(invoke(&mut host, A, 2).is_err());
    assert_eq!(host.module_state(A), Some(State::Failed));
    invoke(&mut host, B, 1).unwrap();
    human(&mut host, "set lifetime_human_progress 1\n");
    command_frame(&mut host);
    assert_stale_absent(&host);
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
}

pub(super) fn queued_binding_keeps_alias_producer_and_retires_with_it(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let producer = request(
        &mut runtime,
        A,
        None,
        make(b"alias trigger \"set lifetime_bound 1\";bind w trigger\n"),
    );
    let peer = request(&mut runtime, B, None, make(PEER));
    let mut host = Box::new(
        FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![producer, peer],
        )
        .unwrap(),
    );
    host.initialize_modules(Phase::Server);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    assert!(
        host.runtime
            .input
            .assign(DeviceId::Keyboard(10), SeatId::FIRST)
    );
    let key = |down| EventKind::Key {
        device: DeviceId::Keyboard(10),
        code: 26,
        symbol: 119,
        down,
        repeat: false,
    };
    frame_with(&mut host, vec![key(true)]);
    assert_eq!(text(&host, "lifetime_bound"), Some("1"));
    human(&mut host, "set lifetime_bound 0\n");
    run_commands(&mut host);
    assert!(host.shutdown_module(A));
    invoke(&mut host, B, 1).unwrap();
    human(&mut host, "set lifetime_human_progress 1\n");
    frame_with(&mut host, vec![key(false), key(true), key(false)]);
    assert_eq!(text(&host, "lifetime_bound"), Some("0"));
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
    assert_eq!(text(&host, "lifetime_human_progress"), Some("1"));
    assert!(host.runtime.input.binding(SeatId::FIRST, 26).is_none());
    assert!(!host.runtime.quit);
}

fn physical_forward(host: &mut FrameHost) {
    human(host, "bind w +forward\n");
    run_commands(host);
    assert!(
        host.runtime
            .input
            .assign(DeviceId::Keyboard(10), SeatId::FIRST)
    );
    frame_with(host, vec![physical_key(26, true)]);
    // The first queued TIME seeds the input span; the next frame has a real
    // elapsed interval for Q3's held-key fraction.
    let held = frame_with(host, vec![]);
    assert!(held.commands[0].movement[0] > 0.0);
}
fn physical_key(control: u16, down: bool) -> EventKind<'static> {
    EventKind::Key {
        device: DeviceId::Keyboard(10),
        code: control,
        symbol: 0,
        down,
        repeat: false,
    }
}
fn release_human_forward(host: &mut FrameHost) {
    frame_with(host, vec![physical_key(26, false)]);
    // Elapsed-key policies may retain the interval ending at that release.
    let quiet = frame_with(host, vec![]);
    assert_eq!(quiet.commands[0].movement[0], 0.0);
}
pub(super) fn manual_hold_retirement_preserves_human_press(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    {
        let (mut module_only, _) = fixture_commands(&make, b"+forward;wait 100;-forward\n", false);
        invoke(&mut module_only, A, 1).unwrap();
        run_commands(&mut module_only);
        frame_with(&mut module_only, vec![]);
        assert!(frame_with(&mut module_only, vec![]).commands[0].movement[0] > 0.0);
        assert!(module_only.shutdown_module(A));
        frame_with(&mut module_only, vec![]);
        assert_eq!(
            frame_with(&mut module_only, vec![]).commands[0].movement[0],
            0.0
        );
    }
    let (mut host, _) = fixture_commands(make, b"+forward;wait 100;-forward\n", false);
    physical_forward(&mut host);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    assert!(!host.console.idle());
    assert!(host.shutdown_module(A));
    invoke(&mut host, B, 1).unwrap();
    let held = frame_with(&mut host, vec![]);
    assert!(held.commands[0].movement[0] > 0.0);
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
    assert!(host.runtime.input.binding(SeatId::FIRST, 26).is_some());
    release_human_forward(&mut host);
    assert!(!host.runtime.quit);
}
pub(super) fn purged_keyup_cannot_leave_module_button_held(
    make: impl Fn(&[u8]) -> ProducerProgram,
    fatal_export: bool,
) {
    let (mut host, _) = fixture_commands(make, b"bind k +forward\n", false);
    physical_forward(&mut host);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    let pressed = frame_with(&mut host, vec![physical_key(14, true)]);
    assert!(pressed.commands[0].movement[0] > 0.0);
    // Consume the physical key-up, queuing the real owned -forward without
    // executing its command phase. Teardown will discard that queued release.
    let at = EventTime::from_milliseconds(host.time.milliseconds() + 1);
    host.queue
        .push(SysEvent {
            time: at,
            kind: physical_key(14, false),
        })
        .unwrap();
    host.drain(&mut Default::default());
    assert!(!host.console.idle());
    if fatal_export {
        assert!(invoke(&mut host, A, 2).is_err());
        assert_eq!(host.module_state(A), Some(State::Failed));
    } else {
        assert!(host.shutdown_module(A));
    }
    invoke(&mut host, B, 1).unwrap();
    let human_held = frame_with(&mut host, vec![]);
    assert!(human_held.commands[0].movement[0] > 0.0);
    assert!(host.runtime.input.binding(SeatId::FIRST, 14).is_none());
    assert!(host.runtime.input.binding(SeatId::FIRST, 26).is_some());
    assert_eq!(text(&host, "lifetime_peer_progress"), Some("1"));
    release_human_forward(&mut host);
    assert!(!host.runtime.quit);
}
pub(super) fn stale_empty_buffer_retires_holds_before_the_next_frame(
    make: impl Fn(&[u8]) -> ProducerProgram,
) {
    let (mut host, anchors) = fixture_commands(make, b"+forward\n", false);
    physical_forward(&mut host);
    invoke(&mut host, A, 1).unwrap();
    run_commands(&mut host);
    assert!(host.console.idle());
    host.runtime.server.entities.columns.owner[anchors[0].slot as usize] = B;
    let healthy_calls = host.module_counts(B).unwrap().calls;
    let held = frame_with(&mut host, vec![]);
    assert!(held.server_ticks > 0);
    assert!(host.module_counts(B).unwrap().calls > healthy_calls);
    assert_eq!(host.module_state(A), Some(State::Failed));
    assert_eq!(host.module_state(B), Some(State::Running));
    assert!(held.commands[0].movement[0] > 0.0);
    assert!(host.runtime.input.binding(SeatId::FIRST, 26).is_some());
    release_human_forward(&mut host);
    assert!(!host.runtime.quit);
}

#[test]
fn qvm_manual_hold_retirement_preserves_independent_human_hold() {
    manual_hold_retirement_preserves_human_press(qvm);
}
#[test]
fn qvm_shutdown_clears_owned_hold_after_its_queued_keyup_is_purged() {
    purged_keyup_cannot_leave_module_button_held(qvm, false);
}
#[test]
fn qvm_empty_buffer_stale_anchor_retires_hold_without_touching_human() {
    stale_empty_buffer_retires_holds_before_the_next_frame(qvm);
}

#[test]
fn qvm_defined_alias_and_binding_keep_producer_through_queued_keys() {
    queued_binding_keeps_alias_producer_and_retires_with_it(qvm);
}

#[test]
fn qvm_shutdown_retires_deferred_commands_aliases_and_owned_wait() {
    shutdown_retires_alias_wait_and_shutdown_output(qvm);
}
#[test]
fn qvm_failed_init_keeps_healthy_module_and_human_commands() {
    failed_initialization_cannot_leave_queued_work(qvm);
}
#[test]
fn qvm_command_origin_rejects_reused_slots_and_admits_new_generation() {
    stale_generation_and_new_publication_use_exact_lifetimes(qvm);
}
#[test]
fn qvm_cold_publication_prunes_absent_module_with_live_anchor() {
    absent_cold_row_cannot_lend_its_live_anchor(qvm);
}
#[test]
fn qvm_transferred_anchor_cannot_apply_its_old_wait() {
    transferred_owner_invalidates_wait_before_healthy_work(qvm);
}

#[test]
fn queued_disconnect_runs_client_callback_on_the_active_arena_and_retires_after_its_trap() {
    use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_network::{commands::packet::Protocol, ingress::ConnectionPhase};

    let mut runtime = Runtime::load(2, []).unwrap();
    let clients = [SeatId::FIRST, SeatId::new(1).unwrap()].map(|seat| {
        runtime
            .connect_local(
                seat,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    client: RuleSetId::Quake3,
                    input: RuleSetId::QuakeWorld,
                    movement: RuleSetId::Quake2,
                    trace: RuleSetId::Quake,
                },
                Protocol::Quake3_68,
                seat.index() as u32,
            )
            .unwrap()
    });
    // Reuse the existing QVM instruction builder. Only ClientDisconnect emits
    // EXEC_NOW, then traps after its nested console activation has returned.
    let mut first = request(
        &mut runtime,
        A,
        None,
        qvm_console(b"set lifetime_disconnect_callback 1\n", Some(5), 0, 5),
    );
    first.entries = (0..=7).collect();
    first.initialize = vec![Export::words(CallbackId(0), &[])];
    let Program::Qvm { vm, .. } = &mut first.program else {
        panic!("QVM fixture")
    };
    vm.configure_q3_game(Q3StatLayout::BASE);
    let peer = request(&mut runtime, B, None, qvm(PEER));
    let mut host = Box::new(
        FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![first, peer],
        )
        .unwrap(),
    );
    host.local_clients[0] = Some(clients[0]);
    host.local_clients[1] = Some(clients[1]);
    host.initialize_modules(Phase::Server);
    assert_eq!(
        host.connect_module_client(A, clients[0], b"\\name\\disconnect"),
        Ok(0)
    );
    let retired = host.runtime.server.clients[clients[0].0 as usize].entity;
    let healthy = host.runtime.server.clients[clients[1].0 as usize].entity;
    let before = host.module_counts(A).unwrap().calls;
    let traps = host.module_counts(A).unwrap().traps;
    let peer_before = host.module_counts(B).unwrap().calls;
    frame_with(
        &mut host,
        vec![EventKind::ConsoleLine(
            "disconnect;set lifetime_disconnect_human 1\n",
        )],
    );

    // A recursive driver would find the execution arena already taken and
    // could not reach this callback or its nested EXEC_NOW command.
    assert_eq!(text(&host, "lifetime_disconnect_callback"), Some("1"));
    assert_eq!(host.module_counts(A).unwrap().calls, before + 1);
    assert_eq!(host.module_counts(A).unwrap().traps, traps + 1);
    assert_eq!(host.module_state(A), Some(State::Running));
    let human_view = host
        .console
        .cvars
        .bind(
            "lifetime_disconnect_human",
            Context {
                cvar_seat: Some(SeatId::FIRST),
                ..Context::default()
            },
        )
        .unwrap();
    assert_eq!(host.console.cvars.read(human_view).unwrap().as_str(), "1");
    assert_eq!(text(&host, "lifetime_disconnect_human"), Some(""));
    assert!(host.local_clients[0].is_none());
    assert!(
        host.runtime.server.clients[clients[0].0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.server.entities.resolve(retired).is_none());
    let replacement = host.runtime.server.clients[clients[0].0 as usize].entity;
    assert_ne!(replacement, retired);
    assert_eq!(host.local_clients[1], Some(clients[1]));
    assert_eq!(
        host.runtime.server.clients[clients[1].0 as usize].entity,
        healthy
    );
    for endpoint in [
        qa_core::loopback::Endpoint::Client,
        qa_core::loopback::Endpoint::Server,
    ] {
        assert_eq!(
            host.runtime
                .network
                .get(clients[1], endpoint)
                .unwrap()
                .phase,
            ConnectionPhase::Connected
        );
        assert!(host.runtime.network.get(clients[0], endpoint).is_none());
    }

    human(&mut host, "disconnect\n");
    run_commands(&mut host);
    assert_eq!(host.module_counts(A).unwrap().calls, before + 1);
    assert_eq!(
        host.runtime.server.clients[clients[0].0 as usize].entity,
        replacement
    );
    command_frame(&mut host);
    assert_eq!(host.module_counts(A).unwrap().traps, traps + 1);
    assert!(host.module_counts(B).unwrap().calls > peer_before);
    assert_eq!(host.module_state(B), Some(State::Running));
}

/// The caller supplies the existing QC fixture; native custom harnesses reuse
/// the queue assertions without building a second QuakeC program fixture.
pub(super) fn quakec_localcmd_retirement(make: fn(RuleSetId, f32, [&[u8]; 2]) -> FrameHost) {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut host = make(rules, 2.0, [b"lifetime probe", b"\n"]);
        host.initialize_modules(Phase::Server);
        host.console
            .append(
                "fov 77\n",
                Context {
                    source: rules,
                    ..Default::default()
                },
            )
            .unwrap();
        host.call_module_export(A, Export::clocked(CallbackId(0)), ThinkTime::Seconds(0.1))
            .unwrap();
        assert!(!host.console.idle());
        assert!(host.shutdown_module(A));
        run_commands(&mut host);
        let fov = host.console.cvars.find("cg_fov").unwrap();
        assert_eq!(host.console.cvars.value_in(fov, rules), 77.0);
        assert!(host.console.idle());
        assert!(!host.runtime.quit);
    }
}
