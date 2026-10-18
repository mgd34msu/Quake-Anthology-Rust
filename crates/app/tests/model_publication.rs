#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{Export, ModuleRequest, Phase, Program},
};
use qa_compat::{abi::Q3_SERVER, services::CallContext};
use qa_console::{commands::Console, views::Context};
use qa_core::{configstrings::ConfigLayout, loopback::Endpoint, primitives::*, sys_events::*};
use qa_formats::program::qvm::Opcode::*;
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Connections, Incoming, Route},
    projection::EntityProjection,
};
use qa_session::{clients::Connection, timing::TickRate};
use qa_world::entities::AllocationPolicy;
use std::{collections::BTreeMap, ops::ControlFlow, time::Duration};

const PRIMARY: ModuleId = ModuleId(7);
const FOREIGN: ModuleId = ModuleId(8);
const PORT: u16 = 24617;
const FIRST: usize = 16;
const REPLACEMENT: usize = 128;
const TOO_LONG: usize = 256;
const INLINE: usize = 384;

fn empty_world(runtime: &mut Runtime) -> GeometryId {
    use qa_world::collision::{
        brushes::{BrushTree, ModelRoot},
        surfaces::SurfaceTable,
    };
    let mut tree = BrushTree::direct(0).unwrap();
    tree.models.push(ModelRoot::Leaf(0));
    runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            tree,
            vec![
                Bounds {
                    mins: Vec3([-1024.0; 3]),
                    maxs: Vec3([1024.0; 3])
                };
                2
            ],
        )
        .unwrap()
}

#[derive(Default)]
struct Source {
    time: EventTime,
    packets: Vec<Vec<u8>>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
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
    fn send_packet(&mut self, socket: u16, peer: std::net::SocketAddr, bytes: &[u8]) -> bool {
        assert_eq!(socket, 0);
        assert_eq!(peer.port(), PORT);
        self.packets.push(bytes.to_vec());
        true
    }
}

fn request(runtime: &mut Runtime, module: ModuleId, first: &[u8]) -> ModuleRequest {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    // A real G_SET_CONFIGSTRING call accepts the index and guest text pointer
    // supplied to export zero; the frame export performs no game mutation.
    let mut code = vec![(Enter, 64), (Local, 72), (Load4, 0), (Const, 0), (Ne, 0)];
    for (local, arg) in [(76, 8), (80, 12)] {
        code.extend([(Local, local), (Load4, 0), (Arg, arg)]);
    }
    code.extend([(Const, -19), (Call, 0), (Pop, 0), (Const, 0), (Leave, 64)]);
    code[4].1 = code.len() as i32;
    code.extend([(Const, 0), (Leave, 64)]);
    let mut data = vec![0; 512];
    for (at, text) in [(FIRST, first), (REPLACEMENT, b"models/replacement.md3"),
        (TOO_LONG, &b"models/abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz.md3"[..]),
        (INLINE, b"*1")] {
        data[at..at + text.len()].copy_from_slice(text);
    }
    let context = Context::default();
    ModuleRequest {
        search: None,
        q3_image: None,
        context: CallContext {
            module,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake3,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            console: context,
            allocation: AllocationPolicy::q3(0),
            link_order: qa_gameplay::rules::link_order(RuleSetId::Quake3),
        },
        phase: Phase::Server,
        timing_rules: RuleSetId::Quake3,
        rate: TickRate::FrameDriven,
        anchor,
        program: Program::Qvm {
            vm: Box::new(service_program::program(&code, &data)),
            imports: &Q3_SERVER,
        },
        entries: vec![0, 1],
        frame: Export::words(CallbackId(1), &[]),
        prepare: vec![],
        initialize: vec![Export::words(CallbackId(0), &[33, FIRST as u64])],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: ConfigLayout::QUAKE3.count,
        files: 0,
        entity_source: None,
    }
}

struct Harness {
    host: FrameHost,
    peer: Connections,
    client: ClientId,
    actor: EntityId,
    source: Source,
    configs: BTreeMap<u32, Vec<u8>>,
    world: GeometryId,
}
impl Harness {
    fn new() -> Self {
        let mut runtime = Runtime::load(1, []).unwrap();
        let world = empty_world(&mut runtime);
        runtime.collision = Some(qa_app::WorldCollision::new(&runtime.geometry, world, 0));
        let client = runtime
            .server
            .connect(Connection::Remote, PRIMARY, None)
            .unwrap();
        assert!(runtime.server.bind_client_native(
            client,
            NativeEntity {
                module: PRIMARY,
                slot: 1
            }
        ));
        runtime
            .bind_connection(
                client,
                NativeConnection {
                    phase: qa_network::ingress::ConnectionPhase::Connected,
                    route: Route {
                        socket: 0,
                        peer: Peer::Socket(([127, 0, 0, 1], PORT).into()),
                    },
                    channel: Channel::load(
                        Protocol::Quake2_34.channel(),
                        Endpoint::Server,
                        8192,
                        16,
                    )
                    .unwrap(),
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(Protocol::Quake2_34, 8192)),
                },
                qa_app::snapshots::NativePlayerBinding::Supplied(0),
                None,
            )
            .unwrap();
        let requests = vec![
            request(&mut runtime, PRIMARY, b"models/primary.md3"),
            request(&mut runtime, FOREIGN, b"models/foreign.md3"),
        ];
        let actor = runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), FOREIGN, AllocationPolicy::q3(0))
            .unwrap()
            .id;
        let slot = actor.slot as usize;
        runtime.server.entities.columns.native_entity[slot] = Some(NativeEntity {
            module: FOREIGN,
            slot: 42,
        });
        runtime.server.entities.columns.position[slot] = Vec3([123.0, 17.0, 29.0]);
        runtime.server.entities.columns.appearance[slot].source = RuleSetId::Quake3;
        runtime.server.entities.columns.appearance[slot].models[0] =
            ModelRef::native(ResourceNamespace::Module(FOREIGN), 1);
        runtime.server.clients[client.0 as usize]
            .player
            .view_weapon
            .model = ModelRef::native(ResourceNamespace::Module(FOREIGN), 1);
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            requests,
        )
        .unwrap();
        host.initialize_modules(Phase::Server);
        for module in [PRIMARY, FOREIGN] {
            assert_eq!(host.module_counts(module).unwrap().traps, 0);
        }
        let mut peer = Connections::load(1);
        peer.bind(
            client,
            Endpoint::Client,
            NativeConnection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: 0,
                    peer: Peer::Socket(([127, 0, 0, 1], PORT + 1).into()),
                },
                channel: Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16)
                    .unwrap(),
                output: None,
                pending_output: None,
                commands: Some(Commands::load(Protocol::Quake2_34, 8192)),
            },
        )
        .unwrap();
        Self {
            host,
            peer,
            client,
            actor,
            source: Source::default(),
            configs: BTreeMap::new(),
            world,
        }
    }
    fn step(&mut self, acknowledge: bool) {
        self.source.time.0 += 100_000_000;
        if acknowledge {
            let channel = &mut self
                .peer
                .get_mut(self.client, Endpoint::Client)
                .unwrap()
                .channel;
            if let Some(packet) = channel.prepare(None, self.source.time).unwrap() {
                let bytes = packet.bytes.to_vec();
                channel.submitted(self.source.time).unwrap();
                self.host
                    .queue
                    .push(SysEvent {
                        time: self.source.time,
                        kind: EventKind::Packet {
                            socket: 0,
                            from: Peer::Socket(([127, 0, 0, 1], PORT).into()),
                            bytes: &bytes,
                        },
                    })
                    .unwrap();
            }
        }
        self.host.frame(&mut self.source, true);
        for bytes in self.source.packets.drain(..) {
            self.peer.receive(
                0,
                Peer::Socket(([127, 0, 0, 1], PORT + 1).into()),
                &bytes,
                self.source.time,
                |_, _, incoming| {
                    if let Incoming::ConfigString(row) = incoming {
                        self.configs.insert(row.index, row.text.to_vec());
                    }
                    ControlFlow::Continue(())
                },
            );
        }
        assert_eq!(self.peer.malformed, 0);
        assert_eq!(self.peer.command_errors, 0);
        assert_eq!(self.host.runtime.network.command_errors, 0);
    }
    fn models(&self) -> (u32, u32) {
        let frame = self
            .peer
            .get(self.client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        let qa_network::snapshots::ReceivedFrame::Quake2(frame) = frame else {
            panic!("Q2 frame")
        };
        let mut actor_model = None;
        for entity in frame.entities {
            let mut record = EntityRecord::default();
            assert!(EntityProjection::load(Protocol::Quake2_34).apply(
                &entity.words,
                &mut record,
                ResourceNamespace::Client(self.client),
            ));
            if record.position == Vec3([123.0, 17.0, 29.0]) {
                actor_model = Some(record.appearance.models[0].map_or(0, |model| model.ordinal));
            }
        }
        (
            actor_model.expect("foreign actor retained"),
            frame.player[22],
        )
    }
    fn rewrite(&mut self, module: ModuleId, ordinal: u32, pointer: usize) {
        self.host
            .call_module_export(
                module,
                Export::words(
                    CallbackId(0),
                    &[
                        (ConfigLayout::QUAKE3.resources[0].first + ordinal) as u64,
                        pointer as u64,
                    ],
                ),
                ThinkTime::module_clock(RuleSetId::Quake3).at(self.source.time),
            )
            .unwrap();
        assert_eq!(self.host.module_counts(module).unwrap().traps, 0);
    }
}

#[test]
fn remote_foreign_models_follow_config_packets_and_real_ack_for_entities_and_weapons() {
    let mut h = Harness::new();
    assert!(h.host.runtime.client_resources[h.client.0 as usize].is_none());
    h.step(false);
    assert_eq!(h.models(), (0, 0));
    h.step(false);
    assert_eq!(
        h.models(),
        (0, 0),
        "native submission alone cannot ready a resource"
    );
    h.step(true);
    let first = h.models();
    assert_eq!(first, (126, 126));
    assert_eq!(h.configs.get(&(32 + 1)).unwrap(), b"models/primary.md3");
    assert_eq!(
        h.configs.get(&(32 + first.0)).unwrap(),
        b"models/foreign.md3"
    );
    // The live actor and weapon keep their source ModelRefs. Only their wire
    // binding changes when a primary native row takes back the alias ordinal.
    h.rewrite(PRIMARY, first.0, REPLACEMENT);
    h.step(false);
    assert_eq!(h.models(), (0, 0));
    h.step(true);
    let relocated = h.models();
    assert_eq!(relocated, (125, 125));
    assert_eq!(
        h.configs.get(&(32 + first.0)).unwrap(),
        b"models/replacement.md3"
    );
    assert_eq!(
        h.configs.get(&(32 + relocated.0)).unwrap(),
        b"models/foreign.md3"
    );
    assert_eq!(
        h.host.runtime.server.entities.columns.appearance[h.actor.slot as usize].models[0],
        ModelRef::native(ResourceNamespace::Module(FOREIGN), 1)
    );
    // Q3 accepts this text, classic Q2 cannot. Retaining an ACK for the old
    // filename must never make the rewritten resource display that old file.
    h.rewrite(FOREIGN, 1, TOO_LONG);
    h.step(false);
    assert_eq!(h.models(), (0, 0));
    h.step(true);
    assert_eq!(h.models(), (0, 0));
}

#[test]
fn remote_inline_models_require_the_actual_bound_world_for_primary_and_foreign_sources() {
    for (module, expected) in [(PRIMARY, 1), (FOREIGN, 126)] {
        let mut h = Harness::new();
        h.rewrite(module, 1, INLINE);
        let reference = ModelRef::native(ResourceNamespace::Module(module), 1);
        h.host.runtime.server.entities.columns.appearance[h.actor.slot as usize].models[0] =
            reference;
        h.host.runtime.server.clients[h.client.0 as usize]
            .player
            .view_weapon
            .model = reference;
        h.step(false);
        assert_eq!(h.models(), (0, 0));
        h.step(true);
        assert_eq!(h.models(), (expected, expected));
        assert_eq!(h.configs.get(&(32 + expected)).unwrap(), b"*1");
        let other_world = empty_world(&mut h.host.runtime);
        h.host.runtime.collision = Some(qa_app::WorldCollision::new(
            &h.host.runtime.geometry,
            other_world,
            0,
        ));
        h.step(false);
        assert_eq!(
            h.models(),
            (0, 0),
            "the same *1 text cannot select another world's inline model"
        );
        h.host.runtime.collision = Some(qa_app::WorldCollision::new(
            &h.host.runtime.geometry,
            h.world,
            0,
        ));
        h.step(false);
        assert_eq!(h.models(), (0, 0));
        // The restored world's new publication can wait behind a flight from
        // the intervening world; acknowledge actual peer receipts until ready.
        for _ in 0..8 {
            h.step(true);
            if h.models() == (expected, expected) {
                break;
            }
        }
        assert_eq!(h.models(), (expected, expected));
    }
}
