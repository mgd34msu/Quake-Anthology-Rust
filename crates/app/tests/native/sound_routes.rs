use super::*;
use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor, modules::ApiCheck};
use qa_compat::native::q2::Game;
use qa_core::{events::NativeReceipt, loopback::Endpoint, primitives::*, sys_events::SeatId};
use qa_network::commands::packet::Protocol;
use std::{num::NonZeroU32, sync::Arc};

struct RemotePeer {
    client: ClientId,
    address: std::net::SocketAddr,
    server_address: std::net::SocketAddr,
    connections: qa_network::ingress::Connections,
    replies: Vec<Vec<u8>>,
    sounds: Vec<qa_network::outputs::Sound>,
    configs: Vec<(u32, Vec<u8>)>,
}
impl RemotePeer {
    fn load(client: ClientId, protocol: Protocol, clients: usize) -> Self {
        let address = ([127, 0, 0, 1], 4000).into();
        let server_address = ([127, 0, 0, 1], 4001).into();
        let mut channel =
            qa_network::channel::Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                .unwrap();
        channel.set_qport(4000);
        let mut connections = qa_network::ingress::Connections::load(clients);
        connections
            .bind(
                client,
                Endpoint::Client,
                qa_network::ingress::Connection {
                    route: qa_network::ingress::Route {
                        socket: 0,
                        peer: qa_core::sys_events::Peer::Socket(server_address),
                    },
                    channel,
                    output: None,
                    pending_output: None,
                    commands: Some(qa_network::commands::connection::Commands::load(protocol)),
                },
            )
            .unwrap();
        Self {
            client,
            address,
            server_address,
            connections,
            replies: Vec::new(),
            sounds: Vec::new(),
            configs: Vec::new(),
        }
    }
}

fn sound_module(
    runtime: &mut Runtime,
    module: ModuleId,
    initial_name: &[u8],
    rules: RuleSetId,
    emitted_name: &[u8],
    emitter: u32,
) -> (ModuleRequest, CallbackId) {
    let (request, actions) = sound_module_actions(
        runtime,
        module,
        initial_name,
        rules,
        emitted_name,
        emitter,
        false,
    );
    (request, actions.emit)
}
struct SoundActions {
    emit: CallbackId,
    raw: CallbackId,
    rewrite: CallbackId,
    duplicate: CallbackId,
}
fn sound_module_actions(
    runtime: &mut Runtime,
    module: ModuleId,
    initial_name: &[u8],
    rules: RuleSetId,
    emitted_name: &[u8],
    emitter: u32,
    actions: bool,
) -> (ModuleRequest, SoundActions) {
    sound_module_options(
        runtime,
        module,
        initial_name,
        rules,
        emitted_name,
        emitter,
        actions,
        false,
    )
}
fn sound_module_options(
    runtime: &mut Runtime,
    module: ModuleId,
    initial_name: &[u8],
    rules: RuleSetId,
    emitted_name: &[u8],
    emitter: u32,
    actions: bool,
    dupes: bool,
) -> (ModuleRequest, SoundActions) {
    let rr = rules == RuleSetId::Quake2Rerelease;
    assert!(!dupes || rr);
    let mut file = q2_table_file();
    pe_rva(&mut file, 0x1800, if rr { 2023 } else { 3 }, 4);
    // Capture gi at the genuine GetGameAPI boundary and return the API table.
    pe_text(
        &mut file,
        0x1080,
        &[
            0x48, 0x89, 0x0d, 0x99, 0x0a, 0, 0, 0x48, 0x8d, 0x05, 0x72, 7, 0, 0, 0xc3,
        ],
    );
    pe_text(&mut file, 0x1700, &[0xc3]);
    for index in 0..if rr { 29 } else { 15 } {
        let offset = if !rr || index < 19 {
            8 + index * 8
        } else {
            192 + (index - 19) * 8
        };
        pe_rva(&mut file, 0x1800 + offset, 0x180001700, 8);
    }
    if rr {
        // The native Pmove entry is redirected to the shared engine import;
        // its patchable prologue must not overlap another exported function.
        pe_rva(&mut file, 0x1800 + 192, 0x180001710, 8);
        pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
    }
    // RunFrame mutates native memory so the fixture also proves ongoing dispatch.
    pe_text(&mut file, 0x1480, &[0x83, 0x05, 0xa9, 6, 0, 0, 1, 0xc3]);
    pe_rva(
        &mut file,
        0x1800 + if rr { 136 } else { 112 },
        0x180001480,
        8,
    );
    let header = if rr { 16 } else { 0 };
    let sound_index_offset: i32 = header + if rr { 11 * 8 } else { 9 * 8 };
    let emit_entry = if dupes { 0x1d80 } else { 0x1500 };
    for (entry, name, emit) in [
        (0x1400usize, 0x1bc0usize, false),
        (emit_entry, 0x1d00, true),
    ] {
        let stack = if dupes && emit { 0x60 } else { 0x40 };
        let mut code = vec![0x53, 0x48, 0x83, 0xec, stack]; // rbx + shadow/stack args
        let at = entry + code.len();
        code.extend([0x48, 0x8b, 0x1d]);
        code.extend((0x1b20i32 - (at + 7) as i32).to_le_bytes());
        let at = entry + code.len();
        code.extend([0x48, 0x8d, 0x0d]);
        code.extend((name as i32 - (at + 7) as i32).to_le_bytes());
        code.extend([0x48, 0x8b, 0x83]);
        code.extend(sound_index_offset.to_le_bytes());
        code.extend([0xff, 0xd0]); // gi.soundindex
        if emit && dupes {
            // Two real gi.local_sound calls target seats of one physical client.
            for target in 1..=2u64 {
                code.extend([0x48, 0xb9]);
                code.extend((0x180003000u64 + target * 1472).to_le_bytes());
                code.extend([0x48, 0xba]);
                code.extend(0x180001c00u64.to_le_bytes());
                code.extend([0x49, 0xb8]);
                code.extend((0x180003000u64 + u64::from(emitter) * 1472).to_le_bytes());
                code.extend([0x41, 0xb9, 18, 0, 0, 0]); // reliable + channel2
                for (offset, value) in [
                    (32u8, 1u32),
                    (40, 1.0f32.to_bits()),
                    (48, 1.0f32.to_bits()),
                    (56, 0),
                    (64, 42),
                ] {
                    code.extend([0xc7, 0x44, 0x24, offset]);
                    code.extend(value.to_le_bytes());
                }
                code.extend([0x48, 0x8b, 0x83]);
                code.extend((header + 6 * 8).to_le_bytes());
                code.extend([0xff, 0xd0]);
            }
        } else if emit {
            if actions {
                code.extend([0x41, 0xb9, 2, 0, 0, 0]); // requested duplicate ordinal2
            } else {
                code.extend([0x41, 0x89, 0xc1]); // sound ordinal -> r9d
            }
            code.extend([0x48, 0xb9]);
            code.extend(0x180001c00u64.to_le_bytes()); // supplied position
            code.extend([0x48, 0xba]);
            code.extend(
                (0x180003000u64 + u64::from(emitter) * if rr { 1472 } else { 280 }).to_le_bytes(),
            );
            code.extend([0x41, 0xb8, 10, 0, 0, 0]); // NO_PHS + channel2
            for (offset, value) in [
                (32u8, 1.0f32.to_bits()),
                (40, 1.0f32.to_bits()),
                (48, 0),
                (56, 0),
            ] {
                code.extend([0xc7, 0x44, 0x24, offset]);
                code.extend(value.to_le_bytes());
            }
            code.extend([0x48, 0x8b, 0x83]);
            code.extend((header + 5 * 8).to_le_bytes());
            code.extend([0xff, 0xd0]); // gi.positioned_sound
        }
        code.extend([0x48, 0x83, 0xc4, stack, 0x5b, 0xc3]);
        assert!(!dupes || entry + code.len() < 0x1e80 || !emit);
        pe_text(&mut file, entry, &code);
    }
    pe_rva(&mut file, 0x1800 + if rr { 16 } else { 8 }, 0x180001400, 8);
    pe_rva(
        &mut file,
        0x1800 + if rr { 152 } else { 120 },
        0x180000000 + emit_entry as u64,
        8,
    );
    pe_text(&mut file, 0x1bc0, &[initial_name, b"\0"].concat());
    assert!(emitted_name.len() <= 95);
    pe_text(&mut file, 0x1d00, &[emitted_name, b"\0"].concat());
    let stride = if rr { 1472u64 } else { 280 };
    let metadata = 0x1800 + if rr { 160 } else { 128 };
    pe_rva(&mut file, metadata, 0x180003000, 8);
    pe_rva(&mut file, metadata + 8, stride, if rr { 8 } else { 4 });
    pe_rva(&mut file, metadata + if rr { 16 } else { 12 }, 3, 4);
    pe_rva(&mut file, metadata + if rr { 20 } else { 16 }, 3, 4);
    if actions {
        let begin = |entry: usize| {
            let mut code = vec![0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x1d];
            code.extend((0x1b20i32 - (entry + 12) as i32).to_le_bytes());
            code
        };
        let invoke = |code: &mut Vec<u8>, ordinal: i32| {
            code.extend([0x48, 0x8b, 0x83]);
            code.extend((header + ordinal * 8).to_le_bytes());
            code.extend([0xff, 0xd0]);
        };
        let raw_entry = if dupes { 0x1e80 } else { 0x15b0 };
        let mut raw = begin(raw_entry);
        for target in 1..=if dupes { 2u64 } else { 1 } {
            for value in [9u32, 12, 1] {
                raw.push(0xb9);
                raw.extend(value.to_le_bytes());
                invoke(&mut raw, if rr { 27 } else { 25 });
            }
            raw.push(0xb9);
            raw.extend(((emitter << 3) | 2).to_le_bytes());
            invoke(&mut raw, if rr { 28 } else { 26 });
            raw.extend([0x48, 0xb9]);
            raw.extend(0x180001c00u64.to_le_bytes());
            invoke(&mut raw, if rr { 32 } else { 30 });
            if dupes {
                raw.extend([0x48, 0xb9]);
                raw.extend((0x180003000u64 + target * 1472).to_le_bytes());
                raw.extend([0x31, 0xd2, 0x41, 0xb8]); // best effort, key43
                raw.extend(43u32.to_le_bytes());
                invoke(&mut raw, 25); // gi.unicast
            } else {
                raw.extend([0x31, 0xc9, 0x31, 0xd2, 0x45, 0x31, 0xc0]);
                invoke(&mut raw, if rr { 24 } else { 22 });
            }
        }
        raw.extend([0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3]);
        assert!(raw.len() <= if dupes { 0x180 } else { 0x100 });
        assert!(!dupes || raw_entry + raw.len() <= 0x2000);
        pe_text(&mut file, raw_entry, &raw);
        pe_rva(
            &mut file,
            0x1800 + if rr { 120 } else { 96 },
            0x180000000 + raw_entry as u64,
            8,
        );
        let sound_first = qa_core::configstrings::ConfigLayout::for_rules(rules).resources[1].first;
        for (entry, row, name, export) in [
            (
                0x16b0usize,
                sound_first + 1,
                0x180001d00u64,
                if rr { 104 } else { 80 },
            ),
            (
                0x1740usize,
                sound_first + 2,
                0x180001bc0u64,
                if rr { 112 } else { 88 },
            ),
        ] {
            let mut code = begin(entry);
            code.push(0xb9);
            code.extend(row.to_le_bytes());
            code.extend([0x48, 0xba]);
            code.extend(name.to_le_bytes());
            invoke(&mut code, if rr { 7 } else { 6 });
            code.extend([0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3]);
            pe_text(&mut file, entry, &code);
            pe_rva(&mut file, 0x1800 + export, 0x180000000 + entry as u64, 8);
        }
    }
    let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let mut bytes = std::mem::take(&mut image.bytes).into_vec();
    bytes.resize(0x5000, 0);
    for slot in 1..3 {
        let at = 0x3000 + slot * stride as usize + if rr { 1376 } else { 96 };
        bytes[at] = 1;
    }
    image.bytes = bytes.into_boxed_slice();
    let mut regions = std::mem::take(&mut image.regions).into_vec();
    regions.push(qa_formats::program::native::Region {
        offset: 0x3000,
        length: 0x2000,
        read: true,
        write: true,
        execute: false,
    });
    image.regions = regions.into_boxed_slice();
    let game = Game::map(image, rules, 25, &runtime.geometry, Duration::from_secs(3)).unwrap();
    let initialize = Export::words(CallbackId(game.entry(b"Init").unwrap()), &[]);
    let command = SoundActions {
        emit: CallbackId(game.entry(b"ServerCommand").unwrap()),
        raw: CallbackId(game.entry(b"ClientCommand").unwrap()),
        rewrite: CallbackId(game.entry(b"ClientUserinfoChanged").unwrap()),
        duplicate: CallbackId(game.entry(b"ClientDisconnect").unwrap()),
    };
    let frame = Export::words(CallbackId(game.entry(b"RunFrame").unwrap()), &[1]);
    let imports = game.imports;
    let imports_address = game.imports_address;
    let count = game.vm.export_count();
    let mut request = request(
        runtime,
        module,
        rules,
        TickRate::fixed(25).unwrap(),
        game.vm,
    );
    if let Program::Native { imports: table, .. } = &mut request.program {
        *table = imports;
    }
    runtime.server.entities.columns.native_entity[request.anchor.slot as usize] =
        Some(NativeEntity { module, slot: 0 });
    request.entries = (0..count as u32).collect();
    request.api = Some(ApiCheck::NativeTable {
        export: Export::words(CallbackId(0), &[imports_address]),
    });
    request.initialize = vec![initialize];
    request.frame = frame;
    request.configstrings = qa_core::configstrings::ConfigLayout::for_rules(rules).count;
    (request, command)
}

#[derive(Default)]
struct AudioSource {
    time: EventTime,
    sounds: [usize; 2],
    mixer: Option<qa_audio::Mixer>,
    entity: Option<SoundEmitter>,
    sound_rules: [Option<RuleSetId>; 2],
    played: Vec<(ClientId, SoundId, Option<SoundEmitter>)>,
    remote: Option<RemotePeer>,
}
impl FrameSource for AudioSource {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        if let Some(remote) = &mut self.remote {
            for packet in remote.replies.drain(..) {
                queue
                    .push(SysEvent {
                        time: self.time,
                        kind: EventKind::Packet {
                            socket: 0,
                            from: qa_core::sys_events::Peer::Socket(remote.address),
                            bytes: &packet,
                        },
                    })
                    .unwrap();
            }
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
    fn send_packet(&mut self, socket: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        let remote = self.remote.as_mut().expect("remote transport configured");
        assert_eq!((socket, to), (0, remote.address));
        let sounds = &mut remote.sounds;
        let configs = &mut remote.configs;
        remote.connections.receive(
            0,
            qa_core::sys_events::Peer::Socket(remote.server_address),
            bytes,
            self.time,
            |client, endpoint, incoming| {
                assert_eq!((client, endpoint), (remote.client, Endpoint::Client));
                match incoming {
                    qa_network::ingress::Incoming::Cue {
                        cue: qa_network::outputs::Cue::Sound(sound),
                        ..
                    } => sounds.push(sound),
                    qa_network::ingress::Incoming::ConfigString(config) => {
                        configs.push((config.index, config.text.to_vec()));
                    }
                    _ => (),
                }
                std::ops::ControlFlow::Continue(())
            },
        );
        let channel = &mut remote
            .connections
            .get_mut(remote.client, Endpoint::Client)
            .unwrap()
            .channel;
        if let Some(packet) = channel.prepare(None, self.time).unwrap() {
            remote.replies.push(packet.bytes.to_vec());
            channel.submitted(self.time).unwrap();
        }
        true
    }
    fn sound(&mut self, sound: SoundEvent) -> bool {
        let client = sound.client.expect("only queued CLIENT playback");
        assert!(sound.positioned);
        assert_eq!(sound.channel, 2);
        if let Some(entity) = self.entity {
            assert_eq!(sound.entity, Some(entity));
        }
        if let Some(rules) = self.sound_rules[client.0 as usize] {
            assert_eq!(sound.spatial_rules, rules);
        }
        self.played.push((client, sound.sound, sound.entity));
        self.sounds[client.0 as usize] += 1;
        self.mixer.as_mut().is_none_or(|mixer| mixer.sound(sound))
    }
}

pub(super) fn remote_native_emitters_keep_the_module_entity_channel() {
    for (rules, protocol) in [
        (RuleSetId::Quake2, Protocol::Quake2_34),
        (RuleSetId::Quake2Rerelease, Protocol::Quake2Repro1038),
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let local = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(rules), None, None, None).unwrap(),
                protocol,
                0,
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            local,
            NativeEntity {
                module: ModuleId(1),
                slot: 1
            }
        ));
        let remote = runtime
            .server
            .connect(
                qa_session::clients::Connection::Remote,
                ModuleId(1),
                PlayerTail::Q2,
                Some(NativeEntity {
                    module: ModuleId(1),
                    slot: 2,
                }),
            )
            .unwrap();
        assert!(runtime.local_snapshots[remote.0 as usize].is_none());
        let peer = RemotePeer::load(remote, protocol, 2);
        let mut channel =
            qa_network::channel::Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
                .unwrap();
        channel.set_qport(4000);
        runtime
            .network
            .bind(
                remote,
                Endpoint::Server,
                qa_network::ingress::Connection {
                    route: qa_network::ingress::Route {
                        socket: 0,
                        peer: qa_core::sys_events::Peer::Socket(peer.address),
                    },
                    channel,
                    output: runtime.server.clients[remote.0 as usize].output,
                    pending_output: None,
                    commands: Some(qa_network::commands::connection::Commands::load(protocol)),
                },
            )
            .unwrap();
        let (request, command) =
            sound_module(&mut runtime, ModuleId(1), b"a.wav", rules, b"b.wav", 1);
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.local_clients[0] = Some(local);
        host.initialize_modules(Phase::Server);
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
        let rate = NonZeroU32::new(44100).unwrap();
        let bank = Arc::new(
            qa_audio::Bank::load(
                rate,
                ["sound/a.wav", "sound/b.wav"]
                    .map(|name| {
                        (
                            name.to_string(),
                            Pcm {
                                rate,
                                channels: PcmChannels::Mono,
                                samples: vec![1024; 2],
                                loop_start: None,
                            },
                        )
                    })
                    .into(),
            )
            .unwrap(),
        );
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        host.call_module_export(
            ModuleId(1),
            Export::words(command, &[]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        let mut source = AudioSource {
            remote: Some(peer),
            ..Default::default()
        };
        for tick in 0..=15 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        let peer = source.remote.as_ref().unwrap();
        assert_eq!(peer.sounds.len(), 1, "{protocol:?} remote queued sound");
        assert_eq!(
            (
                peer.sounds[0].entity(),
                peer.sounds[0].channel,
                peer.sounds[0].index
            ),
            (Some(1), 2, 2)
        );
        assert_eq!(
            source.sounds,
            [1, 0],
            "only local CLIENT presentation is mixed here"
        );
        assert_eq!(
            (peer.connections.command_errors, peer.connections.malformed),
            (0, 0)
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert!(host.runtime.server.events.is_empty());
    }
}

pub(super) fn server_sounds_wait_for_resource_ack_without_stalling_other_clients() {
    for (rules, protocol) in [
        (RuleSetId::Quake2, Protocol::Quake2_34),
        (RuleSetId::Quake2Rerelease, Protocol::Quake2Repro1038),
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let mut ids = Vec::new();
        for seat in 0..2 {
            let id = runtime
                .connect_local(
                    SeatId::new(seat).unwrap(),
                    SpawnAnchor {
                        position: Vec3::default(),
                        angles: Vec3::default(),
                        entity: 0,
                        fixture_fallback: false,
                    },
                    ClientPolicy::select(
                        Some(if seat == 0 { rules } else { RuleSetId::Quake3 }),
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                    protocol,
                    u32::from(seat),
                )
                .unwrap();
            assert!(runtime.server.bind_client_native(
                id,
                NativeEntity {
                    module: ModuleId(1),
                    slot: i32::from(seat) + 1
                }
            ));
            ids.push(id);
        }
        let (request, command) =
            sound_module(&mut runtime, ModuleId(1), b"a.wav", rules, b"b.wav", 1);
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.local_clients[1] = Some(ids[1]); // first peer deliberately sends no ACKs
        host.initialize_modules(Phase::Server);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(State::Running),
            "{protocol:?} native API bound"
        );
        let rate = NonZeroU32::new(44100).unwrap();
        let bank = Arc::new(
            qa_audio::Bank::load(
                rate,
                ["sound/a.wav", "sound/b.wav"]
                    .map(|name| {
                        (
                            name.to_string(),
                            Pcm {
                                rate,
                                channels: PcmChannels::Mono,
                                samples: vec![1000; 2],
                                loop_start: None,
                            },
                        )
                    })
                    .into(),
            )
            .unwrap(),
        );
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        host.runtime
            .server
            .events
            .native_message(
                NativeDelivery {
                    module: ModuleId(1),
                    source: rules,
                    target: NativeTarget::Client(ids[0]),
                    origin: Vec3::default(),
                    reliable: true,
                    dupe_key: 0,
                    emission: None,
                    check_portals: true,
                },
                b"\x0a\x02older flight\0",
            )
            .unwrap();
        let mut source = AudioSource {
            sound_rules: [Some(rules), Some(RuleSetId::Quake3)],
            ..Default::default()
        };
        assert_eq!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .sound_rules(),
            rules
        );
        assert_eq!(
            host.runtime.client_resources[1]
                .as_ref()
                .unwrap()
                .state
                .sound_rules(),
            RuleSetId::Quake3
        );
        host.frame(&mut source, true);
        // Transmit the older reliable flight, allow queued CLIENT receipt, and
        // leave its genuine native ACK withheld at the client endpoint.
        assert!(
            host.runtime
                .send_local_snapshot(ids[0], EventTime(1_000_000), None)
        );
        host.drain(&mut Default::default());
        let old_calls = host.module_counts(ModuleId(1)).unwrap().calls;
        host.call_module_export(
            ModuleId(1),
            Export::words(command, &[]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        for tick in 1..=8 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(
            source.sounds,
            [0, 1],
            "{protocol:?} healthy client progresses"
        );
        assert!(host.module_counts(ModuleId(1)).unwrap().calls > old_calls + 6);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        let sound_row = host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .layout
            .resources[1]
            .get(2)
            .unwrap();
        assert!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .configs
                .config(sound_row)
                .unwrap()
                .text
                .is_empty(),
            "new resource has not bypassed older unACKed flight"
        );
        // Re-enable normal usercmd transmission: native packet ACKs retire the
        // old flight, then the resource flight, and only then admit the sound.
        host.local_clients[0] = Some(ids[0]);
        for tick in 9..=20 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(
            source.sounds,
            [1, 1],
            "{protocol:?} each queued client plays once"
        );
        assert_eq!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .sound(2),
            bank.find("sound/b.wav")
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        let stale_receipt = NativeReceipt(u64::MAX);
        assert_eq!(
            host.runtime.server.events.acknowledge(
                host.runtime.server.clients[0].output.unwrap(),
                stale_receipt
            ),
            0
        );
        assert!(host.runtime.loopback.pending(Endpoint::Client) <= 1);
    }
}

pub(super) fn unsupported_resource_names_drop_only_the_unrepresentable_client_delivery() {
    let name = [b"n".repeat(68), b".wav".to_vec()].concat();
    let mut runtime = Runtime::load(2, []).unwrap();
    let mut clients = Vec::new();
    for (seat, protocol) in [(0, Protocol::Quake2_34), (1, Protocol::Quake2Repro1038)] {
        let client = runtime
            .connect_local(
                SeatId::new(seat).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(RuleSetId::Quake2Rerelease), None, None, None).unwrap(),
                protocol,
                u32::from(seat),
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            client,
            NativeEntity {
                module: ModuleId(1),
                slot: i32::from(seat) + 1
            }
        ));
        clients.push(client);
    }
    let (request, command) = sound_module(
        &mut runtime,
        ModuleId(1),
        b"a.wav",
        RuleSetId::Quake2Rerelease,
        &name,
        1,
    );
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(clients[0]);
    host.local_clients[1] = Some(clients[1]);
    host.initialize_modules(Phase::Server);
    let rate = NonZeroU32::new(44100).unwrap();
    let path = format!("sound/{}", std::str::from_utf8(&name).unwrap());
    let bank = Arc::new(
        qa_audio::Bank::load(
            rate,
            ["sound/a.wav".to_string(), path]
                .map(|name| {
                    (
                        name,
                        Pcm {
                            rate,
                            channels: PcmChannels::Mono,
                            samples: vec![1; 2],
                            loop_start: None,
                        },
                    )
                })
                .into(),
        )
        .unwrap(),
    );
    host.runtime.sound_bank = Some(Arc::clone(&bank));
    host.bind_sounds(&bank).unwrap();
    host.call_module_export(
        ModuleId(1),
        Export::words(command, &[]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
    let mut source = AudioSource::default();
    let mut unsupported = 0;
    for tick in 0..=20 {
        source.time = EventTime::from_milliseconds(tick * 25);
        unsupported += host.frame(&mut source, true).output.native_unsupported;
    }
    assert_eq!(source.sounds, [0, 1]);
    assert_eq!(unsupported, 1);
    assert!(host.module_counts(ModuleId(1)).unwrap().calls > 20);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert!(
        host.runtime.server.events.is_empty(),
        "unsupported sound cannot pin the ring"
    );
}

pub(super) fn world_edict_sounds_keep_the_native_channel_replacement_key() {
    for (rules, protocol) in [
        (RuleSetId::Quake2, Protocol::Quake2_34),
        (RuleSetId::Quake2Rerelease, Protocol::Quake2Repro1038),
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let client = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(rules), None, None, None).unwrap(),
                protocol,
                0,
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            client,
            NativeEntity {
                module: ModuleId(1),
                slot: 1
            }
        ));
        let (request, command) =
            sound_module(&mut runtime, ModuleId(1), b"a.wav", rules, b"b.wav", 0);
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.local_clients[0] = Some(client);
        host.initialize_modules(Phase::Server);
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
        let rate = NonZeroU32::new(44100).unwrap();
        let bank = Arc::new(
            qa_audio::Bank::load(
                rate,
                ["sound/a.wav", "sound/b.wav"]
                    .map(|name| {
                        (
                            name.to_string(),
                            Pcm {
                                rate,
                                channels: PcmChannels::Mono,
                                samples: vec![1024; 2],
                                loop_start: None,
                            },
                        )
                    })
                    .into(),
            )
            .unwrap(),
        );
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        for _ in 0..2 {
            host.call_module_export(
                ModuleId(1),
                Export::words(command, &[]),
                ThinkTime::module_clock(rules),
            )
            .unwrap();
        }
        let emitter = SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 0,
        };
        let mut mixer = qa_audio::Mixer::load(Arc::clone(&bank), 2, 1);
        mixer.listen(
            &[Some(qa_audio::Listener {
                client,
                entity: None,
                origin: Vec3::default(),
                right: Vec3([1.0, 0.0, 0.0]),
            })],
            1.0,
        );
        let mut source = AudioSource {
            mixer: Some(mixer),
            entity: Some(emitter),
            ..Default::default()
        };
        for tick in 0..=10 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(source.sounds, [2, 0]);
        let mixer = source.mixer.as_ref().unwrap();
        assert_eq!((mixer.counts.started, mixer.counts.replaced), (2, 1));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

pub(super) fn secondary_modules_share_the_connection_sound_namespace() {
    for (rules, protocol) in [
        (RuleSetId::Quake2, Protocol::Quake2_34),
        (RuleSetId::Quake2Rerelease, Protocol::Quake2Repro1038),
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let local = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(rules), None, None, None).unwrap(),
                protocol,
                0,
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            local,
            NativeEntity {
                module: ModuleId(1),
                slot: 1
            }
        ));
        let remote = runtime
            .server
            .connect(
                qa_session::clients::Connection::Remote,
                ModuleId(1),
                PlayerTail::Q2,
                Some(NativeEntity {
                    module: ModuleId(1),
                    slot: 2,
                }),
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            remote,
            NativeEntity {
                module: ModuleId(1),
                slot: 2
            }
        ));
        let peer = RemotePeer::load(remote, protocol, 2);
        let mut channel =
            qa_network::channel::Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
                .unwrap();
        channel.set_qport(4000);
        runtime
            .network
            .bind(
                remote,
                Endpoint::Server,
                qa_network::ingress::Connection {
                    route: qa_network::ingress::Route {
                        socket: 0,
                        peer: qa_core::sys_events::Peer::Socket(peer.address),
                    },
                    channel,
                    output: runtime.server.clients[remote.0 as usize].output,
                    pending_output: None,
                    commands: Some(qa_network::commands::connection::Commands::load(protocol)),
                },
            )
            .unwrap();
        let (first, primary_emit) =
            sound_module(&mut runtime, ModuleId(1), b"a.wav", rules, b"a.wav", 1);
        let (second, secondary_emit) =
            sound_module(&mut runtime, ModuleId(2), b"b.wav", rules, b"b.wav", 1);
        let world = second.anchor;
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![first, second],
        )
        .unwrap();
        host.local_clients[0] = Some(local);
        host.initialize_modules(Phase::Server);
        let rate = NonZeroU32::new(44100).unwrap();
        let bank = Arc::new(
            qa_audio::Bank::load(
                rate,
                ["sound/a.wav", "sound/b.wav", "sound/c.wav"]
                    .map(|path| {
                        (
                            path.into(),
                            Pcm {
                                rate,
                                channels: PcmChannels::Mono,
                                samples: vec![1024; 2],
                                loop_start: None,
                            },
                        )
                    })
                    .into(),
            )
            .unwrap(),
        );
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        let a = bank.find("sound/a.wav").unwrap();
        let b = bank.find("sound/b.wav").unwrap();
        let mut source = AudioSource {
            remote: Some(peer),
            ..Default::default()
        };
        for tick in 0..=12 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        host.call_module_export(
            ModuleId(1),
            Export::words(primary_emit, &[]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        host.call_module_export(
            ModuleId(2),
            Export::words(secondary_emit, &[]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        for tick in 13..=18 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(
            source.played.len(),
            2,
            "{protocol:?} typed cross-module sounds"
        );
        assert_eq!((source.played[0].1, source.played[1].1), (a, b));
        let remote_sounds = &source.remote.as_ref().unwrap().sounds;
        assert_eq!(remote_sounds.len(), 2);
        assert_eq!(
            remote_sounds[0].index, 1,
            "primary source ordinal unchanged"
        );
        assert_ne!(
            remote_sounds[1].index, 1,
            "secondary ordinal1 cannot alias primary"
        );
        assert_eq!(remote_sounds[0].entity(), Some(1));
        assert_ne!(remote_sounds[1].entity(), Some(1));
        let secondary_ordinal = remote_sounds[1].index;
        let secondary_entity = remote_sounds[1].entity().unwrap();
        assert_eq!(
            source.played[1].2,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(local),
                slot: i32::from(secondary_entity)
            })
        );
        let layout = qa_core::configstrings::ConfigLayout::for_rules(rules);
        assert!(
            source
                .remote
                .as_ref()
                .unwrap()
                .configs
                .iter()
                .any(|(index, name)| *index
                    == layout.resources[1].first + u32::from(secondary_ordinal)
                    && name == b"b.wav")
        );
        // The staged native path uses the very same ordinal and emitter mapper.
        let mut bytes = [0; 64];
        let length = qa_network::outputs::write_cue(
            protocol,
            qa_network::outputs::Cue::Sound(qa_network::outputs::Sound {
                flags: 12,
                index: 1,
                volume: 255,
                attenuation: 64,
                time_offset: 0,
                entity: 1,
                channel: 2,
                position: Vec3::default(),
            }),
            &mut bytes,
        )
        .unwrap();
        let delivery = NativeDelivery {
            module: ModuleId(2),
            source: rules,
            target: NativeTarget::All,
            origin: Vec3::default(),
            reliable: false,
            dupe_key: 0,
            emission: None,
            check_portals: false,
        };
        let foreign_emitter = host
            .runtime
            .server
            .entities
            .find_native(
                NativeEntity {
                    module: ModuleId(2),
                    slot: 1,
                },
                None,
            )
            .unwrap();
        NativeCueBinding {
            sound: Some(b),
            entity: Some(foreign_emitter),
        }
        .write_capture(0, &mut bytes[length..length + NATIVE_CUE_CAPTURE_BYTES])
        .unwrap();
        host.runtime
            .server
            .events
            .native_message_bound(
                delivery,
                length,
                &bytes[..length + NATIVE_CUE_CAPTURE_BYTES],
            )
            .unwrap();
        for tick in 19..=23 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        let native = source.remote.as_ref().unwrap().sounds.last().unwrap();
        assert_eq!(
            (native.index, native.entity(), native.channel),
            (secondary_ordinal, Some(secondary_entity), 2)
        );
        assert_eq!(source.played.last().unwrap().1, b);
        // World0 in the foreign module has its own lifetime and mapped key.
        assert_eq!(
            host.runtime.server.entities.columns.native_entity[world.slot as usize],
            Some(NativeEntity {
                module: ModuleId(2),
                slot: 0
            })
        );
        let world_number = host.runtime.native_numbers[local.0 as usize]
            .project(&host.runtime.server.entities, world)
            .unwrap();
        assert_ne!(world_number, 0);
        host.runtime
            .server
            .events
            .push_delivered(
                delivery,
                FrameEvent::Sound(SoundEvent {
                    sound: b,
                    entity: Some(SoundEmitter::Native {
                        namespace: ResourceNamespace::Module(ModuleId(2)),
                        slot: 0,
                    }),
                    channel: 2,
                    position: Vec3::default(),
                    positioned: true,
                    ..Default::default()
                }),
            )
            .unwrap();
        for tick in 24..=28 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(
            source.played.last().unwrap().2,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(local),
                slot: world_number as i32
            })
        );
        assert_ne!(
            source
                .remote
                .as_ref()
                .unwrap()
                .sounds
                .last()
                .unwrap()
                .entity(),
            Some(0)
        );
        for module in [ModuleId(1), ModuleId(2)] {
            assert_eq!(host.module_counts(module).unwrap().traps, 0);
            assert!(host.module_counts(module).unwrap().calls > 20);
        }
    }
}

pub(super) fn native_sound_ordinals_and_retained_captures_survive_resource_rewrites() {
    for (rules, protocol) in [
        (RuleSetId::Quake2, Protocol::Quake2_34),
        (RuleSetId::Quake2Rerelease, Protocol::Quake2Repro1038),
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let client = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(rules), None, None, None).unwrap(),
                protocol,
                0,
            )
            .unwrap();
        runtime.server.bind_client_native(
            client,
            NativeEntity {
                module: ModuleId(1),
                slot: 1,
            },
        );
        let remote = runtime
            .server
            .connect(
                qa_session::clients::Connection::Remote,
                ModuleId(1),
                PlayerTail::Q2,
                Some(NativeEntity {
                    module: ModuleId(1),
                    slot: 2,
                }),
            )
            .unwrap();
        let peer = RemotePeer::load(remote, protocol, 2);
        let mut channel =
            qa_network::channel::Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
                .unwrap();
        channel.set_qport(4000);
        runtime
            .network
            .bind(
                remote,
                Endpoint::Server,
                qa_network::ingress::Connection {
                    route: qa_network::ingress::Route {
                        socket: 0,
                        peer: qa_core::sys_events::Peer::Socket(peer.address),
                    },
                    channel,
                    output: runtime.server.clients[remote.0 as usize].output,
                    pending_output: None,
                    commands: Some(qa_network::commands::connection::Commands::load(protocol)),
                },
            )
            .unwrap();
        let (request, actions) = sound_module_actions(
            &mut runtime,
            ModuleId(1),
            b"a.wav",
            rules,
            b"b.wav",
            1,
            true,
        );
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.local_clients[0] = Some(client);
        host.initialize_modules(Phase::Server);
        let rate = NonZeroU32::new(44100).unwrap();
        let bank = Arc::new(
            qa_audio::Bank::load(
                rate,
                ["sound/a.wav", "sound/b.wav"]
                    .map(|path| {
                        (
                            path.into(),
                            Pcm {
                                rate,
                                channels: PcmChannels::Mono,
                                samples: vec![1024; 2],
                                loop_start: None,
                            },
                        )
                    })
                    .into(),
            )
            .unwrap(),
        );
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        let a = bank.find("sound/a.wav").unwrap();
        let b = bank.find("sound/b.wav").unwrap();
        let mut source = AudioSource {
            remote: Some(peer),
            ..Default::default()
        };
        for tick in 0..=10 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        host.call_module_export(
            ModuleId(1),
            Export::words(actions.duplicate, &[0]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        for tick in 11..=20 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        host.call_module_export(
            ModuleId(1),
            Export::words(actions.emit, &[]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        for tick in 21..=30 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(
            source.played.last().unwrap().1,
            a,
            "requested primary ordinal2 must still name A"
        );
        assert_eq!(
            source.remote.as_ref().unwrap().sounds[0].index,
            2,
            "typed emission keeps the exact requested duplicate ordinal"
        );
        let layout = qa_core::configstrings::ConfigLayout::for_rules(rules);
        assert_eq!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .sound(2),
            Some(a)
        );
        // Capture raw ordinal1 while it is A, then rewrite it before dispatch.
        host.call_module_export(
            ModuleId(1),
            Export::words(actions.raw, &[0]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        host.call_module_export(
            ModuleId(1),
            Export::words(actions.rewrite, &[0, 0]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        for tick in 31..=42 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(source.played.len(), 2);
        assert_eq!(
            source.played[1].1, a,
            "retained raw cue keeps captured A, not rewritten B"
        );
        assert_eq!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .sound(1),
            Some(b)
        );
        assert_eq!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .configs
                .config(layout.resources[1].first as usize + 1)
                .unwrap()
                .text,
            b"b.wav"
        );
        // Stopping the publishing module cannot erase an already owned record.
        host.call_module_export(
            ModuleId(1),
            Export::words(actions.raw, &[0]),
            ThinkTime::module_clock(rules),
        )
        .unwrap();
        host.shutdown_modules();
        for tick in 43..=48 {
            source.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(source.played.len(), 3);
        assert_eq!(source.played[2].1, b);
    }
}

/// Real rerelease local_sound and unicast imports share one physical output.
pub(super) fn split_seat_native_dupes_wait_for_ack_and_play_once() {
    let rules = RuleSetId::Quake2Rerelease;
    let protocol = Protocol::Quake2Repro1038;
    let mut runtime = Runtime::load(2, []).unwrap();
    let mut ids = [ClientId(0); 2];
    for (seat, id) in ids.iter_mut().enumerate() {
        *id = runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(rules), None, None, None).unwrap(),
                protocol,
                seat as u32,
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            *id,
            NativeEntity {
                module: ModuleId(1),
                slot: seat as i32 + 1
            }
        ));
    }
    let (request, actions) = sound_module_options(
        &mut runtime,
        ModuleId(1),
        b"a.wav",
        rules,
        b"a.wav",
        1,
        true,
        true,
    );
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.local_clients[1] = Some(ids[1]);
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = Arc::new(
        qa_audio::Bank::load(
            rate,
            vec![(
                "sound/a.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![1000; 2],
                    loop_start: None,
                },
            )],
        )
        .unwrap(),
    );
    host.runtime.sound_bank = Some(Arc::clone(&bank));
    host.bind_sounds(&bank).unwrap();
    let mut source = AudioSource::default();
    // Actual resource publication and native packet ACKs, with no readiness override.
    for tick in 0..=12 {
        source.time = EventTime::from_milliseconds(tick * 25);
        host.frame(&mut source, true);
    }
    for id in ids {
        assert_eq!(
            host.runtime.client_resources[id.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .sound(1),
            bank.find("sound/a.wav")
        );
    }
    host.call_module_export(
        ModuleId(1),
        Export::words(actions.emit, &[]),
        ThinkTime::module_clock(rules),
    )
    .unwrap();
    for tick in 13..=24 {
        source.time = EventTime::from_milliseconds(tick * 25);
        host.frame(&mut source, true);
    }
    assert_eq!(
        source.played.len(),
        1,
        "reliable gi.local_sound duplicate waits for native ACK then suppresses sibling"
    );
    assert_eq!(source.played[0].1, bank.find("sound/a.wav").unwrap());
    host.call_module_export(
        ModuleId(1),
        Export::words(actions.raw, &[0]),
        ThinkTime::module_clock(rules),
    )
    .unwrap();
    for tick in 25..=36 {
        source.time = EventTime::from_milliseconds(tick * 25);
        host.frame(&mut source, true);
    }
    assert_eq!(
        source.played.len(),
        2,
        "raw best-effort gi.unicast uses the same coordinator"
    );
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    host.call_module_export(
        ModuleId(1),
        Export::words(actions.emit, &[]),
        ThinkTime::module_clock(rules),
    )
    .unwrap();
    for tick in 37..=48 {
        source.time = EventTime::from_milliseconds(tick * 25);
        host.frame(&mut source, true);
    }
    assert_eq!(
        source.played.len(),
        3,
        "same key is eligible in a later native provider frame"
    );
    assert!(host.module_counts(ModuleId(1)).unwrap().calls > calls + 8);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}
