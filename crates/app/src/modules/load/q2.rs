//! Native game API selection is independent of map and player roles.
use crate::{Runtime, modules::ModuleRequest};
use qa_core::primitives::RuleSetId;

pub struct Q2Spec {
    pub rules: RuleSetId,
    pub path: String,
}
/// Native boundary data selected once; every call enters session dispatch.
#[derive(Clone, Copy)]
pub struct Q2Clients {
    pub module: qa_core::primitives::ModuleId,
    pub rules: RuleSetId,
    pub connect: qa_core::primitives::CallbackId,
    pub begin: qa_core::primitives::CallbackId,
    pub disconnect: qa_core::primitives::CallbackId,
    pub think: qa_core::primitives::CallbackId,
    pub command: qa_core::primitives::CallbackId,
    pub userinfo_changed: qa_core::primitives::CallbackId,
    pub userinfo: u64,
    pub usercmd: u64,
    pub abi: &'static qa_compat::q2_abi::ClientLayout,
    pub social: u64,
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
) -> Result<Vec<Q2Clients>, String> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use super::{module_bytes, native_export};
        use crate::modules::{ApiCheck, Argument, Export, Phase, Program};
        use qa_compat::native::q2::Game;
        use qa_console::{
            catalog::Scope,
            views::{Context, Role},
        };
        use qa_core::primitives::{CallbackId, ThinkTime};
        use qa_formats::archive::ArchiveReader;
        use qa_session::timing::TickRate;
        use qa_world::entities::AllocationPolicy;
        let mut reader = ArchiveReader::default();
        let mut clients = Vec::new();
        for spec in specs {
            let bytes = module_bytes(runtime, &spec.path, &mut reader)?;
            let mut image = super::parse_native(&bytes)?;
            let abi =
                qa_compat::q2_abi::client_layout(spec.rules).ok_or("unsupported Q2 client ABI")?;
            let source = runtime
                .entity_sources
                .get(entity_source)
                .ok_or("missing native entity source")?;
            let map = map.strip_prefix("maps/").unwrap_or(map);
            let map = map.strip_suffix(".bsp").unwrap_or(map);
            let (spawn_arguments, userinfo, usercmd) =
                spawn_strings(&mut image, [map.as_bytes(), &source.bytes, b""], abi)?;
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
                Ok(Export::words(
                    CallbackId(game.entry(name).ok_or("missing Q2 lifecycle export")?),
                    words,
                ))
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
            let module = super::module_id(requests)?;
            let client_entry = |name| {
                game.entry(name)
                    .map(CallbackId)
                    .ok_or("missing Q2 client export")
            };
            clients.push(Q2Clients {
                module,
                rules: spec.rules,
                connect: client_entry(b"ClientConnect")?,
                begin: client_entry(b"ClientBegin")?,
                disconnect: client_entry(b"ClientDisconnect")?,
                think: client_entry(b"ClientThink")?,
                command: client_entry(b"ClientCommand")?,
                userinfo_changed: client_entry(b"ClientUserinfoChanged")?,
                userinfo,
                usercmd,
                abi,
                social: spawn_arguments[2],
            });
            let clock = ThinkTime::module_clock(spec.rules);
            let allocation = AllocationPolicy::for_rules(spec.rules, 0);
            let anchor = super::allocate_anchor(runtime, module, clock, allocation)?;
            let mut api_arguments = [Argument::Word(0); 9];
            api_arguments[0] = Argument::Word(game.imports_address);
            requests.push(ModuleRequest {
                context: super::call_context(
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
                        client: None,
                        callback: CallbackId(0),
                        arguments: api_arguments,
                    },
                }),
                shutdown,
                instruction_budget: 10_000_000,
                configstrings: qa_core::configstrings::ConfigLayout::for_rules(spec.rules).count,
                files: 32,
                entity_source: None,
            });
        }
        Ok(clients)
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (runtime, requests, map, entity_source);
        if specs.is_empty() {
            Ok(Vec::new())
        } else {
            Err("native child backend unsupported on this host".into())
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn spawn_strings(
    image: &mut qa_formats::program::native::Image,
    strings: [&[u8]; 3],
    abi: &qa_compat::q2_abi::ClientLayout,
) -> Result<([u64; 3], u64, u64), String> {
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
        .and_then(|size| size.checked_add(abi.userinfo_bytes))
        .and_then(|size| size.checked_add(3))
        .map(|size| size / 4 * 4)
        .and_then(|size| size.checked_add(abi.command_bytes))
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
    let userinfo = image.base + at as u64;
    let usercmd = (userinfo + abi.userinfo_bytes as u64).div_ceil(4) * 4;
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
    Ok((arguments, userinfo, usercmd))
}
