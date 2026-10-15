//! Cold module-file conversion; map and player roles do not select it.
use super::{ApiCheck, Argument, Export, ModuleRequest, Phase, Program};
use crate::Runtime;
use qa_compat::{
    abi::{Q3_CLIENT, Q3_SERVER, Q3_UI},
    quakec::{Layout, Vm},
    qvm,
    services::CallContext,
};
use qa_console::{
    catalog::Scope,
    views::{Context, Role},
};
use qa_core::primitives::ThinkTime;
use qa_core::primitives::{CallbackId, ClientId, ModuleId, NativeEntity, RuleSetId};
use qa_core::sys_events::SeatId;
use qa_formats::{archive::ArchiveReader, program::quakec::Image};
use qa_session::timing::TickRate;
use qa_world::entities::AllocationPolicy;

mod q2;
pub use q2::{Q2Clients, Q2Spec, load_q2};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_export(call: &qa_compat::native::LifecycleCall) -> Export {
    Export::words(CallbackId(call.ordinal), &call.arguments)
}

pub struct QuakeCSpec {
    pub rules: RuleSetId,
    pub path: String,
}
impl QuakeCSpec {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let (rules, path) = value
            .split_once(':')
            .ok_or("quakec-module needs q1|qw:virtual-file")?;
        let rules = RuleSetId::parse(rules).ok_or("unknown QuakeC module preset")?;
        if !matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) || path.is_empty() {
            return Err("quakec-module needs q1|qw:virtual-file");
        }
        Ok(Self {
            rules,
            path: path.into(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q3Role {
    Game,
    Cgame(SeatId),
    Ui(SeatId),
}
pub struct Q3Spec {
    pub role: Q3Role,
    pub path: String,
    pub player_stats: qa_core::primitives::Q3StatLayout,
}
impl Q3Spec {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let (kind, rest) = value
            .split_once(':')
            .ok_or("q3-module needs game:path, cgame:seat:path or ui:[seat:]path")?;
        let (kind, player_stats) = kind
            .strip_suffix("-ta")
            .map_or((kind, qa_core::primitives::Q3StatLayout::BASE), |kind| {
                (kind, qa_core::primitives::Q3StatLayout::TEAM_ARENA)
            });
        let (role, path) = match kind {
            "game" => (Q3Role::Game, rest),
            "ui" => {
                let (seat, path) = if let Some((seat, path)) = rest.split_once(':') {
                    let seat = seat
                        .parse()
                        .ok()
                        .and_then(SeatId::new)
                        .ok_or("ui seat needs 0..4")?;
                    (seat, path)
                } else {
                    (SeatId::FIRST, rest)
                };
                (Q3Role::Ui(seat), path)
            }
            "cgame" => {
                let (seat, path) = rest.split_once(':').ok_or("cgame module needs seat:path")?;
                let seat = seat
                    .parse()
                    .ok()
                    .and_then(SeatId::new)
                    .ok_or("cgame seat needs 0..4")?;
                (Q3Role::Cgame(seat), path)
            }
            _ => return Err("unknown Q3 module role"),
        };
        if player_stats == qa_core::primitives::Q3StatLayout::TEAM_ARENA && role != Q3Role::Game {
            return Err("Team Arena player stat preset requires a game module");
        }
        if path.is_empty() {
            return Err("Q3 module needs a virtual file");
        }
        Ok(Self {
            role,
            path: path.into(),
            player_stats,
        })
    }
}

fn module_bytes(
    runtime: &Runtime,
    path: &str,
    reader: &mut ArchiveReader,
) -> Result<Vec<u8>, String> {
    runtime
        .vfs
        .read_file(path.as_bytes(), 256 * 1024 * 1024, reader)
        .map_err(|error| format!("module {path}: {error}"))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn parse_native(bytes: &[u8]) -> Result<qa_formats::program::native::Image, String> {
    let base = bytes.starts_with(b"\x7fELF").then_some(0x2000_0000);
    qa_formats::program::native::Image::parse(
        bytes,
        base,
        qa_formats::program::native::LoadRole::Library,
    )
    .map_err(|error| format!("native module: {error:?}"))
}
fn module_id(requests: &[ModuleRequest]) -> Result<ModuleId, String> {
    Ok(ModuleId(
        u16::try_from(requests.len() + 1).map_err(|_| "too many modules")?,
    ))
}
fn allocate_anchor(
    runtime: &mut Runtime,
    module: ModuleId,
    clock: ThinkTime,
    allocation: AllocationPolicy,
) -> Result<qa_core::primitives::EntityId, String> {
    Ok(runtime
        .server
        .entities
        .allocate(clock, module, allocation)
        .ok_or("no module anchor slot")?
        .id)
}
fn call_context(
    module: ModuleId,
    clock: ThinkTime,
    allocation: AllocationPolicy,
    rules: RuleSetId,
    console: Context,
) -> CallContext {
    CallContext {
        native_rules: rules,
        trace_rules: rules,
        sound_policy: qa_gameplay::rules::sound_policy(rules),
        server_frame: 0,
        module,
        clock,
        console,
        allocation,
        link_order: qa_gameplay::rules::link_order(rules),
    }
}

pub fn load_q3(
    runtime: &mut Runtime,
    specs: &[Q3Spec],
    requests: &mut Vec<ModuleRequest>,
    rate: TickRate,
    entity_source: Option<usize>,
) -> Result<(), String> {
    let mut reader = ArchiveReader::default();
    for spec in specs {
        if !spec.player_stats.fits(16) {
            return Err("invalid Q3 player stat layout".into());
        }
        let module = module_id(requests)?;
        let bytes = module_bytes(runtime, &spec.path, &mut reader)?;
        let allocation = AllocationPolicy::for_rules(RuleSetId::Quake3, 0);
        let clock = ThinkTime::module_clock(RuleSetId::Quake3);
        let anchor = allocate_anchor(runtime, module, clock, allocation)?;
        // Native export ordinals live in game/g_public.h, cgame/cg_public.h,
        // and ui/ui_public.h. Each role selects its own import table and phase.
        let mut initialize = [Argument::Word(0); 9];
        let mut client_binding = None;
        let (phase, side, role, imports, init, frame, shutdown, api) = match spec.role {
            Q3Role::Game => {
                initialize[0] = Argument::ClockMilliseconds;
                initialize[1] = Argument::PlatformMilliseconds;
                (
                    Phase::Server,
                    Scope::Server,
                    Role::Game,
                    &Q3_SERVER,
                    0,
                    8,
                    1,
                    None,
                )
            }
            Q3Role::Cgame(seat) => {
                let (index, binding) = runtime
                    .snapshots
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| row.as_ref().map(|row| (index, row)))
                    .find(|(_, row)| row.seat() == Some(seat))
                    .ok_or("cgame needs a connected local seat")?;
                if binding.protocol != qa_network::commands::packet::Protocol::Quake3_68 {
                    return Err("cgame needs a Q3 native client protocol".into());
                }
                if binding.native_client() >= 64 {
                    return Err("cgame native client number needs 0..64".into());
                }
                client_binding = Some(ClientId(index as u32));
                initialize[0] = Argument::ClientMessageSequence;
                initialize[1] = Argument::ClientExecutedCommand;
                initialize[2] = Argument::ClientNativeNumber;
                (
                    Phase::Client,
                    Scope::Client,
                    Role::Cgame,
                    &Q3_CLIENT,
                    0,
                    3,
                    1,
                    None,
                )
            }
            Q3Role::Ui(_) => (
                Phase::Client,
                Scope::Client,
                Role::Engine,
                &Q3_UI,
                1,
                5,
                2,
                Some(ApiCheck::Version {
                    export: Export {
                        client: None,
                        callback: CallbackId(0),
                        arguments: [Argument::Word(0); 9],
                    },
                    accepted: &[4, 6],
                }),
            ),
        };
        let (program, entries, prepare, mut finalize) = if bytes.starts_with(b"MZ")
            || bytes.starts_with(b"\x7fELF")
        {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                use qa_compat::native::{NamedExport, Vm};
                use qa_formats::program::native::Encoding;
                let image = parse_native(&bytes)?;
                if image.target.encoding == Encoding::Elf && image.tls.is_some() {
                    return Err("native TLS provider is not bound".into());
                }
                let mut named: Vec<_> = (0..=10)
                    .map(|command| NamedExport {
                        parameters: &[qa_platform::native::NativeScalar::Word; 13],
                        result: qa_platform::native::NativeScalar::Word,
                        name: b"vmMain",
                        command: Some(command),
                    })
                    .collect();
                named.push(NamedExport {
                    parameters: &[qa_platform::native::NativeScalar::Word; 1],
                    result: qa_platform::native::NativeScalar::Void,
                    name: b"dllEntry",
                    command: None,
                });
                let mut vm = Vm::map_image(image, &named, &[], std::time::Duration::from_secs(3))
                    .map_err(|e| format!("native mapping: {e:?}"))?;
                let mut arguments = [Argument::Word(0); 9];
                arguments[0] = Argument::Word(vm.import_callback());
                let mut prepare: Vec<_> = vm.initializers().iter().map(native_export).collect();
                prepare.push(Export {
                    client: None,
                    callback: CallbackId(11),
                    arguments,
                });
                let finalize: Vec<_> = vm.finalizers().iter().map(native_export).collect();
                if matches!(spec.role, Q3Role::Game) {
                    vm.configure_q3_game(spec.player_stats);
                }
                let count = vm.export_count();
                (
                    Program::Native {
                        vm: Box::new(vm),
                        imports,
                    },
                    (0..count as u32).collect(),
                    prepare,
                    finalize,
                )
            }
            #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
            {
                return Err("native child backend unsupported on this host".into());
            }
        } else {
            let image = qa_formats::program::qvm::Image::parse(&bytes)
                .map_err(|e| format!("QVM module: {e:?}"))?;
            let mut vm = qvm::Vm::load(image).map_err(|e| format!("QVM memory: {e:?}"))?;
            if matches!(spec.role, Q3Role::Game) {
                vm.configure_q3_game(spec.player_stats);
            }
            (
                Program::Qvm {
                    vm: Box::new(vm),
                    imports,
                },
                (0..=10).collect(),
                Vec::new(),
                Vec::new(),
            )
        };
        finalize.insert(
            0,
            Export {
                client: client_binding,
                callback: CallbackId(shutdown),
                arguments: [Argument::Word(0); 9],
            },
        );
        let mut context = call_context(
            module,
            clock,
            allocation,
            RuleSetId::Quake3,
            Context {
                source: RuleSetId::Quake3,
                side,
                role,
                seat: match spec.role {
                    Q3Role::Cgame(seat) | Q3Role::Ui(seat) => seat,
                    Q3Role::Game => SeatId::FIRST,
                },
                ..Context::default()
            },
        );
        if phase == Phase::Client {
            context.sound_policy = qa_gameplay::rules::native_sound_spatial(RuleSetId::Quake3);
        }
        requests.push(ModuleRequest {
            context,
            timing_rules: RuleSetId::Quake3,
            phase,
            rate: if phase == Phase::Server {
                rate
            } else {
                TickRate::FrameDriven
            },
            anchor,
            program,
            entries,
            frame: Export {
                client: client_binding,
                ..Export::clocked(CallbackId(frame))
            },
            prepare,
            initialize: vec![Export {
                client: client_binding,
                callback: CallbackId(init),
                arguments: initialize,
            }],
            shutdown: finalize,
            api,
            instruction_budget: 10_000_000,
            configstrings: 1024,
            files: 32,
            entity_source: if matches!(spec.role, Q3Role::Game) {
                entity_source.map(|index| (index, qa_formats::entities::EntitySyntax::Quake3))
            } else {
                None
            },
        });
    }
    Ok(())
}

/// Map spawn and native frame/think calls use the same session entries.
pub fn load_quakec(
    runtime: &mut Runtime,
    specs: &[QuakeCSpec],
    map: Option<(&str, usize)>,
) -> Result<Vec<ModuleRequest>, String> {
    let map = map
        .map(|(path, index)| {
            runtime
                .entity_sources
                .get(index)
                .map(|source| (path, index, source.syntax))
                .ok_or_else(|| "missing QuakeC map entity source".to_owned())
        })
        .transpose()?;
    let mut reader = ArchiveReader::default();
    let mut requests = Vec::with_capacity(specs.len());
    for spec in specs {
        // Module zero belongs to the existing built-in walk-through provider.
        let module = module_id(&requests)?;
        let bytes = module_bytes(runtime, &spec.path, &mut reader)?;
        let crc = if spec.rules == RuleSetId::QuakeWorld {
            54730
        } else {
            5927
        };
        let image = Image::parse(&bytes, Some(crc)).map_err(|e| format!("QuakeC module: {e:?}"))?;
        let frame = image
            .function(b"StartFrame")
            .ok_or("module has no StartFrame")?;
        let entries = (0..image.functions.len() as u32).collect();
        let vm = Vm::load(
            image,
            Layout {
                entities: runtime.server.entities.capacity(),
                header_bytes: 16,
                extra_string_bytes: 1024 * 1024,
                state_step: 0.1,
                random_seed: 1,
            },
        )
        .map_err(|e| format!("QuakeC memory: {e:?}"))?;
        let allocation = AllocationPolicy::for_rules(spec.rules, 0);
        let clock = ThinkTime::module_clock(spec.rules);
        let anchor = allocate_anchor(runtime, module, clock, allocation)?;
        // Native world edict zero is an explicit ABI binding, not this slot.
        runtime.server.entities.columns.native_entity[anchor.slot as usize] =
            Some(NativeEntity { module, slot: 0 });
        requests.push(ModuleRequest {
            context: call_context(
                module,
                clock,
                allocation,
                spec.rules,
                Context {
                    source: spec.rules,
                    side: Scope::Server,
                    role: Role::Game,
                    ..Context::default()
                },
            ),
            timing_rules: spec.rules,
            phase: Phase::Server,
            rate: TickRate::FrameDriven,
            anchor,
            program: Program::quakec(
                vm,
                map.map(|(path, source, _)| super::QuakeCMap {
                    world_path: path.to_owned(),
                    entity_source: source,
                }),
            ),
            entries,
            frame: Export::clocked(CallbackId(frame)),
            prepare: Vec::new(),
            initialize: Vec::new(),
            api: None,
            shutdown: Vec::new(),
            instruction_budget: 1_000_000,
            configstrings: qa_core::configstrings::ConfigLayout::for_rules(spec.rules).count,
            files: 32,
            entity_source: map.map(|(_, index, syntax)| (index, syntax)),
        });
    }
    Ok(requests)
}
