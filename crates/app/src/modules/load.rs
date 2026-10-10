//! Cold module-file conversion; map and player roles do not select it.
use super::{Argument, Export, ModuleRequest, Program};
use crate::Runtime;
use qa_compat::{
    abi::Q3_SERVER,
    quakec::{Layout, Vm},
    qvm,
    services::CallContext,
};
use qa_console::{
    catalog::Scope,
    views::{Context, Role},
};
use qa_core::primitives::{CallbackId, ModuleId, NativeEntity, RuleSetId};
use qa_formats::{archive::ArchiveReader, program::quakec::Image};
use qa_session::timing::TickRate;
use qa_world::{
    area::LinkOrder,
    entities::{AllocationPolicy, EntityTime},
};

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

/// Only the server-game phase is wired here. Client/UI hosts must enter CLIENT.
pub struct QvmSpec {
    pub path: String,
}
impl QvmSpec {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let path = value
            .strip_prefix("game:")
            .filter(|path| !path.is_empty())
            .ok_or("qvm-module needs game:virtual-file")?;
        Ok(Self { path: path.into() })
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

pub fn load_qvm(
    runtime: &mut Runtime,
    specs: &[QvmSpec],
    requests: &mut Vec<ModuleRequest>,
    rate: TickRate,
) -> Result<(), String> {
    let mut reader = ArchiveReader::default();
    for spec in specs {
        let module = ModuleId(u16::try_from(requests.len() + 1).map_err(|_| "too many modules")?);
        let bytes = module_bytes(runtime, &spec.path, &mut reader)?;
        let image = qa_formats::program::qvm::Image::parse(&bytes)
            .map_err(|e| format!("QVM module: {e:?}"))?;
        let vm = qvm::Vm::load(image).map_err(|e| format!("QVM memory: {e:?}"))?;
        let allocation = AllocationPolicy::q3(0);
        let clock = EntityTime::Milliseconds(0);
        let anchor = runtime
            .server
            .entities
            .allocate(clock, module, allocation)
            .ok_or("no module anchor slot")?
            .id;
        // qsrc game/g_public.h gameExport_t: INIT=0, SHUTDOWN=1, RUN_FRAME=8.
        let mut initialize = [Argument::Word(0); 9];
        initialize[0] = Argument::ClockMilliseconds;
        initialize[1] = Argument::PlatformMilliseconds;
        requests.push(ModuleRequest {
            context: CallContext {
                module,
                clock,
                console: Context {
                    source: RuleSetId::Quake3,
                    side: Scope::Server,
                    role: Role::Game,
                    ..Context::default()
                },
                allocation,
                link_order: LinkOrder::Head,
            },
            timing_rules: RuleSetId::Quake3,
            rate,
            anchor,
            program: Program::Qvm {
                vm: Box::new(vm),
                imports: &Q3_SERVER,
            },
            entries: (0..=10).collect(),
            frame: CallbackId(8),
            initialize: Some(Export {
                callback: CallbackId(0),
                arguments: initialize,
            }),
            shutdown: Some(Export {
                callback: CallbackId(1),
                arguments: [Argument::Word(0); 9],
            }),
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
                link_order: if qa_gameplay::rules::link_first(spec.rules) {
                    LinkOrder::Head
                } else {
                    LinkOrder::Tail
                },
            },
            timing_rules: spec.rules,
            rate: TickRate::FrameDriven,
            anchor,
            program: Program::quakec(vm),
            entries,
            frame: CallbackId(frame),
            initialize: None,
            shutdown: None,
            instruction_budget: 1_000_000,
            configstrings: 64,
            files: 32,
        });
    }
    Ok(requests)
}
