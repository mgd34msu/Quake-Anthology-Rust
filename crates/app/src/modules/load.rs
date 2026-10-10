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
use qa_core::primitives::{CallbackId, ModuleId, NativeEntity, RuleSetId};
use qa_core::{loopback::Endpoint, sys_events::SeatId};
use qa_formats::{archive::ArchiveReader, program::quakec::Image};
use qa_session::timing::TickRate;
use qa_world::entities::{AllocationPolicy, EntityTime};

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
    Ui,
}
pub struct Q3Spec {
    pub role: Q3Role,
    pub path: String,
}
impl Q3Spec {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let (kind, rest) = value
            .split_once(':')
            .ok_or("q3-module needs game:path, cgame:seat:path or ui:path")?;
        let (role, path) = match kind {
            "game" => (Q3Role::Game, rest),
            "ui" => (Q3Role::Ui, rest),
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
        if path.is_empty() {
            return Err("Q3 module needs a virtual file");
        }
        Ok(Self {
            role,
            path: path.into(),
        })
    }
}

fn module_bytes(
    runtime: &Runtime,
    path: &str,
    reader: &mut ArchiveReader,
) -> Result<Vec<u8>, String> {
    let file = runtime
        .vfs
        .open(path.as_bytes())
        .ok_or_else(|| format!("module not found: {path}"))?;
    let length = usize::try_from(
        runtime
            .vfs
            .length(file)
            .map_err(|e| format!("module: {e:?}"))?,
    )
    .map_err(|_| "module too large")?;
    if length > 256 * 1024 * 1024 {
        return Err("module too large".into());
    }
    let mut bytes = vec![0; length];
    if runtime
        .vfs
        .read_into_reusing(file, &mut bytes, reader)
        .map_err(|e| format!("module: {e:?}"))?
        != length
    {
        return Err("incomplete module".into());
    }
    Ok(bytes)
}

pub fn load_q3(
    runtime: &mut Runtime,
    specs: &[Q3Spec],
    requests: &mut Vec<ModuleRequest>,
    rate: TickRate,
) -> Result<(), String> {
    let mut reader = ArchiveReader::default();
    for spec in specs {
        let module = ModuleId(u16::try_from(requests.len() + 1).map_err(|_| "too many modules")?);
        let bytes = module_bytes(runtime, &spec.path, &mut reader)?;
        let allocation = AllocationPolicy::q3(0);
        let clock = EntityTime::Milliseconds(0);
        let anchor = runtime
            .server
            .entities
            .allocate(clock, module, allocation)
            .ok_or("no module anchor slot")?
            .id;
        // Native export ordinals live in game/g_public.h, cgame/cg_public.h,
        // and ui/ui_public.h. Each role selects its own import table and phase.
        let mut initialize = [Argument::Word(0); 9];
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
                    .local_snapshots
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| row.as_ref().map(|row| (index, row)))
                    .find(|(_, row)| row.seat == seat)
                    .ok_or("cgame needs a connected local seat")?;
                if binding.protocol != qa_network::commands::packet::Protocol::Quake3_68 {
                    return Err("cgame needs a Q3 native client protocol".into());
                }
                if binding.native_client() >= 64 {
                    return Err("cgame native client number needs 0..64".into());
                }
                let connection = runtime
                    .network
                    .get(
                        qa_core::primitives::ClientId(index as u32),
                        Endpoint::Client,
                    )
                    .ok_or("cgame needs a bound native channel")?;
                // No native server-command execution exists yet. Restrict this
                // initial host to fresh bindings, whose executed sequence is 0.
                if connection
                    .channel
                    .command_state()
                    .is_none_or(|state| state.received != 0)
                {
                    return Err("cgame restart needs native server-command dispatch".into());
                }
                initialize[0] = Argument::Word(u64::from(connection.channel.state().sequence));
                initialize[2] = Argument::Word(u64::from(binding.native_client()));
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
            Q3Role::Ui => (
                Phase::Client,
                Scope::Client,
                Role::Engine,
                &Q3_UI,
                1,
                5,
                2,
                Some(ApiCheck::Version {
                    export: Export {
                        callback: CallbackId(0),
                        arguments: [Argument::Word(0); 9],
                    },
                    accepted: &[4, 6],
                }),
            ),
        };
        let (program, entries, prepare, mut finalize) =
            if bytes.starts_with(b"MZ") || bytes.starts_with(b"\x7fELF") {
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                {
                    use qa_compat::native::{NamedExport, Vm};
                    use qa_formats::program::native::{Encoding, Image, LoadRole};
                    let base = bytes.starts_with(b"\x7fELF").then_some(0x2000_0000);
                    let image = Image::parse(&bytes, base, LoadRole::Library)
                        .map_err(|e| format!("native module: {e:?}"))?;
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
                    let vm = Vm::map_image(image, &named, &[], std::time::Duration::from_secs(3))
                        .map_err(|e| format!("native mapping: {e:?}"))?;
                    let mut arguments = [Argument::Word(0); 9];
                    arguments[0] = Argument::Word(vm.import_callback());
                    let lifecycle = |call: &qa_compat::native::LifecycleCall| {
                        let mut arguments = [Argument::Word(0); 9];
                        for (to, &from) in arguments.iter_mut().zip(&call.arguments) {
                            *to = Argument::Word(from);
                        }
                        Export {
                            callback: CallbackId(call.ordinal),
                            arguments,
                        }
                    };
                    let mut prepare: Vec<_> = vm.initializers().iter().map(lifecycle).collect();
                    prepare.push(Export {
                        callback: CallbackId(11),
                        arguments,
                    });
                    let finalize: Vec<_> = vm.finalizers().iter().map(lifecycle).collect();
                    let count = 12 + vm.initializers().len() + vm.finalizers().len();
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
                let vm = qvm::Vm::load(image).map_err(|e| format!("QVM memory: {e:?}"))?;
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
                callback: CallbackId(shutdown),
                arguments: [Argument::Word(0); 9],
            },
        );
        requests.push(ModuleRequest {
            context: CallContext {
                module,
                clock,
                console: Context {
                    source: RuleSetId::Quake3,
                    side,
                    role,
                    seat: if let Q3Role::Cgame(seat) = spec.role {
                        seat
                    } else {
                        SeatId::FIRST
                    },
                    ..Context::default()
                },
                allocation,
                link_order: qa_gameplay::rules::link_order(RuleSetId::Quake3),
            },
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
            frame: CallbackId(frame),
            prepare,
            initialize: Some(Export {
                callback: CallbackId(init),
                arguments: initialize,
            }),
            shutdown: finalize,
            api,
            instruction_budget: 10_000_000,
            configstrings: 1024,
            files: 32,
        });
    }
    Ok(())
}

/// The initial normal-app entry is StartFrame; native spawning stays separate.
pub fn load_quakec(
    runtime: &mut Runtime,
    specs: &[QuakeCSpec],
) -> Result<Vec<ModuleRequest>, String> {
    let mut reader = ArchiveReader::default();
    let mut requests = Vec::with_capacity(specs.len());
    for (index, spec) in specs.iter().enumerate() {
        // Module zero belongs to the existing built-in walk-through provider.
        let module = ModuleId(u16::try_from(index + 1).map_err(|_| "too many modules")?);
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
            },
        )
        .map_err(|e| format!("QuakeC memory: {e:?}"))?;
        let allocation = if spec.rules == RuleSetId::QuakeWorld {
            AllocationPolicy::QUAKEWORLD
        } else {
            AllocationPolicy::EDICT
        };
        let clock = EntityTime::Seconds(0.0);
        let anchor = runtime
            .server
            .entities
            .allocate(clock, module, allocation)
            .ok_or("no module anchor slot")?
            .id;
        // Native world edict zero is an explicit ABI binding, not this slot.
        runtime.server.entities.columns.native_entity[anchor.slot as usize] =
            Some(NativeEntity { module, slot: 0 });
        requests.push(ModuleRequest {
            context: CallContext {
                module,
                clock,
                console: Context {
                    source: spec.rules,
                    side: Scope::Server,
                    role: Role::Game,
                    ..Context::default()
                },
                allocation,
                link_order: qa_gameplay::rules::link_order(spec.rules),
            },
            timing_rules: spec.rules,
            phase: Phase::Server,
            rate: TickRate::FrameDriven,
            anchor,
            program: Program::quakec(vm),
            entries,
            frame: CallbackId(frame),
            prepare: Vec::new(),
            initialize: None,
            api: None,
            shutdown: Vec::new(),
            instruction_budget: 1_000_000,
            configstrings: 64,
            files: 32,
        });
    }
    Ok(requests)
}
