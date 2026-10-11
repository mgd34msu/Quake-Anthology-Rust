//! Native game API selection is independent of map and player roles.
use crate::{Runtime, modules::ModuleRequest};
use qa_core::primitives::RuleSetId;

pub struct Q2Spec {
    pub rules: RuleSetId,
    pub path: String,
}
impl Q2Spec {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let (rules, path) = value
            .split_once(':')
            .ok_or("q2-module needs q2|q2rr:virtual-file")?;
        let rules = RuleSetId::parse(rules).ok_or("unknown Q2 module preset")?;
        if !matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) || path.is_empty() {
            return Err("q2-module needs q2|q2rr:virtual-file");
        }
        Ok(Self {
            rules,
            path: path.into(),
        })
    }
}

pub fn load_q2(
    runtime: &mut Runtime,
    specs: &[Q2Spec],
    requests: &mut Vec<ModuleRequest>,
    map: &str,
    entity_source: usize,
) -> Result<(), String> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use super::{module_bytes, native_export};
        use crate::modules::{ApiCheck, Argument, Export, Phase, Program};
        use qa_compat::{native::q2::Game, services::CallContext};
        use qa_console::{
            catalog::Scope,
            views::{Context, Role},
        };
        use qa_core::primitives::{CallbackId, ModuleId, ThinkTime};
        use qa_formats::{
            archive::ArchiveReader,
            program::native::{Image, LoadRole},
        };
        use qa_session::timing::TickRate;
        use qa_world::entities::AllocationPolicy;
        let mut reader = ArchiveReader::default();
        for spec in specs {
            let bytes = module_bytes(runtime, &spec.path, &mut reader)?;
            let base = bytes.starts_with(b"\x7fELF").then_some(0x2000_0000);
            let mut image = Image::parse(&bytes, base, LoadRole::Library)
                .map_err(|e| format!("Q2 native module: {e:?}"))?;
            let source = runtime
                .entity_sources
                .get(entity_source)
                .ok_or("missing native entity source")?;
            let map = map.strip_prefix("maps/").unwrap_or(map);
            let map = map.strip_suffix(".bsp").unwrap_or(map);
            let spawn_arguments = spawn_strings(&mut image, [map.as_bytes(), &source.bytes, b""])?;
            let interval = qa_gameplay::rules::tick_millis(spec.rules, std::num::NonZeroU32::MIN)
                .ok_or("Q2 module needs a fixed native rate")?;
            let game = Game::map(
                image,
                spec.rules,
                interval.get(),
                &runtime.geometry,
                std::time::Duration::from_secs(3),
            )
            .map_err(|e| format!("Q2 native mapping: {e:?}"))?;
            let export = |name: &[u8], words: &[u64]| -> Result<Export, String> {
                let mut arguments = [Argument::Word(0); 9];
                for (to, &from) in arguments.iter_mut().zip(words) {
                    *to = Argument::Word(from);
                }
                Ok(Export {
                    callback: CallbackId(game.entry(name).ok_or("missing Q2 lifecycle export")?),
                    arguments,
                })
            };
            let mut initialize = Vec::new();
            if spec.rules == RuleSetId::Quake2Rerelease {
                initialize.push(export(b"PreInit", &[])?);
            }
            initialize.push(export(b"Init", &[])?);
            initialize.push(export(b"SpawnEntities", &spawn_arguments)?);
            // SV_SpawnServer settles two frames before clients connect. RR's
            // false main_loop argument permits those frames without players.
            let settle = export(b"RunFrame", &[0])?;
            initialize.extend([settle; 2]);
            let frame = export(
                b"RunFrame",
                &[u64::from(spec.rules == RuleSetId::Quake2Rerelease)],
            )?;
            let mut shutdown = vec![export(b"Shutdown", &[])?];
            shutdown.extend(game.vm.finalizers().iter().map(native_export));
            let prepare = game.vm.initializers().iter().map(native_export).collect();
            let count = game.vm.export_count();
            let module =
                ModuleId(u16::try_from(requests.len() + 1).map_err(|_| "too many modules")?);
            let clock = if spec.rules == RuleSetId::Quake2 {
                ThinkTime::Seconds(0.0)
            } else {
                ThinkTime::Milliseconds(0)
            };
            let allocation = AllocationPolicy::EDICT;
            let anchor = runtime
                .server
                .entities
                .allocate(clock, module, allocation)
                .ok_or("no module anchor slot")?
                .id;
            let mut api_arguments = [Argument::Word(0); 9];
            api_arguments[0] = Argument::Word(game.imports_address);
            requests.push(ModuleRequest {
                context: CallContext {
                    server_frame: 0,
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
                phase: Phase::Server,
                timing_rules: spec.rules,
                rate: TickRate::FixedMilliseconds(interval),
                anchor,
                program: Program::Native {
                    vm: Box::new(game.vm),
                    imports: game.imports,
                },
                entries: (0..count as u32).collect(),
                frame,
                prepare,
                initialize,
                api: Some(ApiCheck::NativeTable {
                    export: Export {
                        callback: CallbackId(0),
                        arguments: api_arguments,
                    },
                }),
                shutdown,
                instruction_budget: 10_000_000,
                configstrings: if spec.rules == RuleSetId::Quake2 {
                    2080
                } else {
                    32768
                },
                files: 32,
            });
        }
        Ok(())
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (runtime, requests, map, entity_source);
        if specs.is_empty() {
            Ok(())
        } else {
            Err("native child backend unsupported on this host".into())
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn spawn_strings(
    image: &mut qa_formats::program::native::Image,
    strings: [&[u8]; 3],
) -> Result<[u64; 3], String> {
    use qa_formats::program::native::Region;
    use qa_platform::native::PAGE_BYTES;
    let offset = image
        .bytes
        .len()
        .div_ceil(PAGE_BYTES)
        .checked_mul(PAGE_BYTES)
        .ok_or("Q2 spawn strings too large")?;
    let size = strings
        .iter()
        .try_fold(0usize, |total, string| {
            total.checked_add(string.len())?.checked_add(1)
        })
        .ok_or("Q2 spawn strings too large")?;
    let length = size
        .div_ceil(PAGE_BYTES)
        .checked_mul(PAGE_BYTES)
        .ok_or("Q2 spawn strings too large")?;
    let end = offset
        .checked_add(length)
        .filter(|&end| end <= 512 * 1024 * 1024)
        .ok_or("Q2 spawn strings too large")?;
    image
        .base
        .checked_add(end as u64)
        .ok_or("Q2 spawn strings too large")?;
    let mut bytes = std::mem::take(&mut image.bytes).into_vec();
    bytes.resize(end, 0);
    let mut at = offset;
    let arguments = strings.map(|string| {
        let address = image.base + at as u64;
        bytes[at..at + string.len()].copy_from_slice(string);
        at += string.len() + 1;
        address
    });
    image.bytes = bytes.into_boxed_slice();
    let mut regions = std::mem::take(&mut image.regions).into_vec();
    regions.push(Region {
        offset,
        length,
        read: true,
        write: true,
        execute: false,
    });
    image.regions = regions.into_boxed_slice();
    Ok(arguments)
}
