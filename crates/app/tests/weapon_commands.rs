#[path = "support/scalar_import.rs"]
mod scalar_import;
use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
};
use qa_compat::{
    abi::{Addresses, CallTable, Invocation, Q3_CLIENT, Q3_SERVER, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{
        ClientId, EntityId, ModuleId, NativeEntity, RuleSetId, SoundAction, SoundEvent, ThinkTime,
        UserCmd, WeaponId,
    },
    sys_events::{EventTime, SeatId, SysEventQueue},
};
use qa_network::commands::packet::Protocol;
use qa_session::{clients::Connection, timing::TickRate};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};
use std::time::Duration;

#[derive(Default)]
struct RetirementSource {
    retired: Option<(ClientId, EntityId)>,
}
impl FrameSource for RetirementSource {
    fn begin_frame(&mut self) -> Duration {
        panic!("output-only fixture");
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {
        panic!("output-only fixture");
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("output-only fixture");
    }
    fn elapsed(&self) -> Duration {
        panic!("output-only fixture");
    }
    fn present(&mut self) {
        panic!("output-only fixture");
    }
    fn sound(&mut self, event: SoundEvent) -> bool {
        assert_eq!(event.action, SoundAction::RetireClient);
        self.retired = Some((event.client.unwrap(), event.client_lifetime.unwrap()));
        true
    }
}

fn host() -> (FrameHost, [ClientId; 2]) {
    let mut runtime = Runtime::load(2, []).unwrap();
    let clients = std::array::from_fn(|seat| {
        let client = runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: [RuleSetId::QuakeWorld, RuleSetId::Quake3][seat],
                    client: [RuleSetId::QuakeWorld, RuleSetId::Quake3][seat],
                    movement: [RuleSetId::Quake, RuleSetId::Quake2][seat],
                    trace: [RuleSetId::Quake3, RuleSetId::QuakeWorld][seat],
                },
                Protocol::Quake3_68,
                seat as u32,
            )
            .unwrap();
        assert!(runtime.server.bind_client_native(
            client,
            NativeEntity {
                module: ModuleId(7),
                slot: seat as i32
            }
        ));
        client
    });
    let mut console = Console::new(Context::default()).unwrap();
    let maximum = console
        .cvars
        .bind("sv_maxclients", Context::default())
        .unwrap();
    console.cvars.force_write(maximum, "2").unwrap();
    (
        FrameHost::load(console, runtime, TickRate::FrameDriven, vec![]).unwrap(),
        clients,
    )
}

fn abi_call(
    host: &mut FrameHost,
    addresses: Addresses,
    table: &CallTable,
    number: u32,
    seat: SeatId,
    args: impl FnOnce(u64) -> [u64; 2],
) -> (ModuleMemory<'static>, u64) {
    let base = if matches!(addresses, Addresses::Qvm { .. }) {
        0
    } else {
        1 << 40
    };
    let mut memory = ModuleMemory::load(base, 128, &[0xa5; 128]).unwrap();
    let mut storage = ServiceStorage::load_native(
        &[(ModuleId(7), 0, RuleSetId::Quake3)],
        0,
        &host.console.cvars,
    )
    .unwrap();
    storage.bind_client_capacity(ModuleId(7), 2).unwrap();
    let mut scratch = host.runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let args = args(base);
    let mut services = host
        .runtime
        .engine_services(&mut host.console, &mut storage, &mut scratch);
    assert_eq!(
        table
            .invoke(
                number,
                &mut Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    native_pmove: None,
                    native_player: None,
                    context: CallContext {
                        native_rules: qa_core::primitives::RuleSetId::Quake3,
                        trace_rules: qa_core::primitives::RuleSetId::Quake3,
                        sound_policy: qa_gameplay::rules::sound_policy(
                            qa_core::primitives::RuleSetId::Quake3
                        ),
                        module: ModuleId(7),
                        clock: ThinkTime::Milliseconds(0),
                        server_frame: 0,
                        console: Context {
                            seat,
                            ..Context::default()
                        },
                        allocation: AllocationPolicy::EDICT,
                        link_order: LinkOrder::Head,
                    },
                    platform_time: host.time,
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    addresses,
                    arguments: &args,
                },
                &mut unknown
            )
            .map(scalar_import::word),
        Ok(0)
    );
    assert_eq!(unknown.calls, 0);
    (memory, base)
}

fn native_weapon(host: &mut FrameHost, slot: i32, addresses: Addresses) -> u8 {
    let (memory, base) = abi_call(host, addresses, &Q3_SERVER, 36, SeatId::FIRST, |base| {
        [slot as u64, base + 32 + if base == 0 { 128 } else { 0 }]
    });
    assert_eq!(memory.read(base + 31, 1).unwrap(), &[0xa5]);
    assert_eq!(memory.read(base + 56, 1).unwrap(), &[0xa5]);
    memory.read(base + 52, 1).unwrap()[0]
}

#[test]
fn queued_q3_weapon_selection_uses_the_registry_for_independent_seats_and_module_abis() {
    let (mut host, clients) = host();
    let input_handles =
        qa_console::input_settings::InputHandles::load(&host.console.cvars).unwrap();
    let registry = &host.runtime.catalog.registry;
    let rail = registry.native_weapon(RuleSetId::Quake3, 7).unwrap();
    let bfg = registry.native_weapon(RuleSetId::Quake3, 9).unwrap();
    let foreign = registry.native_weapon(RuleSetId::Quake2, 7).unwrap();
    assert_ne!(rail, WeaponId(7));
    let rounds = [
        ([Some(rail), Some(bfg)], [Some(rail), Some(bfg)], [7, 9]),
        ([None, Some(foreign)], [None, None], [0, 0]),
        (
            [Some(WeaponId(u32::MAX)), Some(rail)],
            [None, Some(rail)],
            [0, 7],
        ),
    ];
    for (round, (selected, expected, native)) in rounds.into_iter().enumerate() {
        host.time = EventTime::from_milliseconds((round as u64 + 1) * 16);
        let previous = clients.map(|id| host.runtime.server.clients[id.0 as usize].command.weapon);
        let commands = if round == 0 {
            for addresses in [
                Addresses::Qvm { mask: 127 },
                Addresses::Native {
                    pointer_bytes: 8,
                    abi: qa_platform::native::NativeAbi::SystemV,
                },
                Addresses::Native {
                    pointer_bytes: 8,
                    abi: qa_platform::native::NativeAbi::Microsoft,
                },
            ] {
                for (seat, number) in native.into_iter().enumerate() {
                    let (memory, base) = abi_call(
                        &mut host,
                        addresses,
                        &Q3_CLIENT,
                        56,
                        SeatId::new(seat as u8).unwrap(),
                        |_| [u64::from(number), u64::from(1.0f32.to_bits())],
                    );
                    assert_eq!(memory.read(base, 128).unwrap(), &[0xa5; 128]);
                }
            }
            let commands = host.runtime.input.build_frame_with_policy(
                host.time,
                &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake3);
                    qa_core::sys_events::SeatId::COUNT],
            );
            assert_eq!([commands[0].weapon, commands[1].weapon], selected);
            commands
        } else {
            std::array::from_fn(|seat| UserCmd {
                duration_ms: 16,
                server_time_ms: host.time.milliseconds() as i32,
                weapon: selected.get(seat).copied().flatten(),
                ..Default::default()
            })
        };
        for (seat, client) in clients.into_iter().enumerate() {
            assert!(
                host.runtime
                    .send_local_command(client, &commands[seat], host.time)
            );
        }
        assert_eq!(
            clients.map(|id| host.runtime.server.clients[id.0 as usize].command.weapon),
            previous
        );
        host.drain(&mut FrameResult::default());
        assert_eq!(
            clients.map(|id| host.runtime.server.clients[id.0 as usize].command.weapon),
            expected
        );
        for addresses in [
            Addresses::Qvm { mask: 127 },
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::Microsoft,
            },
        ] {
            for (seat, number) in native.into_iter().enumerate() {
                assert_eq!(native_weapon(&mut host, seat as i32, addresses), number);
            }
        }
    }
    assert_eq!(host.runtime.network.commands, 6);
    assert_eq!(host.runtime.network.command_errors, 0);
    for (seat, client) in clients.into_iter().enumerate() {
        assert_eq!(
            host.runtime.server.clients[client.0 as usize]
                .player
                .movement_rules,
            [RuleSetId::Quake, RuleSetId::Quake2][seat]
        );
        assert_eq!(
            host.runtime.server.clients[client.0 as usize]
                .player
                .trace_rules,
            [RuleSetId::Quake3, RuleSetId::QuakeWorld][seat]
        );
    }
}

#[test]
fn queued_weapon_selection_cannot_cross_a_reused_connection_generation() {
    let (mut host, clients) = host();
    let client = clients[0];
    let rail = host
        .runtime
        .catalog
        .registry
        .native_weapon(RuleSetId::Quake3, 7)
        .unwrap();
    assert!(host.runtime.send_local_command(
        client,
        &UserCmd {
            server_time_ms: 16,
            weapon: Some(rail),
            ..Default::default()
        },
        EventTime(0)
    ));
    assert!(host.runtime.server.disconnect(client));
    assert_eq!(
        host.runtime
            .server
            .connect(Connection::Local, ModuleId(7), None),
        Some(client)
    );
    host.drain(&mut FrameResult::default());
    assert_eq!(host.runtime.network.commands, 1);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .command
            .weapon,
        None
    );
}

#[test]
fn cgame_selection_narrows_native_bytes_and_resets_only_the_reused_seat() {
    let (mut host, clients) = host();
    let input_handles =
        qa_console::input_settings::InputHandles::load(&host.console.cvars).unwrap();
    let rail = host
        .runtime
        .catalog
        .registry
        .native_weapon(RuleSetId::Quake3, 7)
        .unwrap();
    let bfg = host
        .runtime
        .catalog
        .registry
        .native_weapon(RuleSetId::Quake3, 9)
        .unwrap();
    let addresses = Addresses::Native {
        pointer_bytes: 8,
        abi: qa_platform::native::NativeAbi::SystemV,
    };
    for (seat, number) in [263, 9].into_iter().enumerate() {
        abi_call(
            &mut host,
            addresses,
            &Q3_CLIENT,
            56,
            SeatId::new(seat as u8).unwrap(),
            |_| [number, u64::from(1.0f32.to_bits())],
        );
    }
    let commands = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(16),
        &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake3);
            qa_core::sys_events::SeatId::COUNT],
    );
    assert_eq!(
        [commands[0].weapon, commands[1].weapon],
        [Some(rail), Some(bfg)]
    );
    let old = host.runtime.server.clients[clients[0].0 as usize].entity;
    let healthy = host.runtime.server.clients[clients[1].0 as usize].entity;
    assert!(host.runtime.disconnect_client(clients[0]));
    assert_eq!(
        host.runtime.server.clients[clients[0].0 as usize]
            .audio_retirement
            .unwrap()
            .lifetime,
        old
    );
    let mut source = RetirementSource::default();
    let output = qa_app::output::dispatch(
        &mut host.runtime,
        &mut source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    );
    assert_eq!(source.retired, Some((clients[0], old)));
    assert_eq!(output.unhandled_sounds, 0);
    assert!(
        host.runtime.server.clients[clients[0].0 as usize]
            .audio_retirement
            .is_none()
    );
    assert_eq!(
        host.runtime.server.clients[clients[1].0 as usize].entity,
        healthy
    );
    assert_eq!(
        host.runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false
                },
                ClientPolicy {
                    input: RuleSetId::Quake3,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake2,
                    trace: RuleSetId::Quake
                },
                Protocol::Quake3_68,
                0,
            )
            .unwrap(),
        clients[0]
    );
    let commands = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(32),
        &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake3);
            qa_core::sys_events::SeatId::COUNT],
    );
    assert_eq!([commands[0].weapon, commands[1].weapon], [None, Some(bfg)]);
    abi_call(
        &mut host,
        addresses,
        &Q3_CLIENT,
        56,
        SeatId::new(1).unwrap(),
        |_| [256, u64::from(1.0f32.to_bits())],
    );
    let commands = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(48),
        &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake3);
            qa_core::sys_events::SeatId::COUNT],
    );
    assert_eq!([commands[0].weapon, commands[1].weapon], [None, None]);
}
