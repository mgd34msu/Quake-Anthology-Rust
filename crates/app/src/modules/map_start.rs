//! Ordered cold entity loading enters the same VM and session function table.
use super::*;
use qa_compat::services::{CallError as ServiceError, ENGINE_CALLS, EngineServices};
use qa_core::{names::NameMatch, text::FixedText};
use qa_formats::entities::EntityLump;
use std::fmt::Write;

fn edit<R>(
    host: &mut FrameHost,
    module: ModuleId,
    call: impl FnOnce(
        &mut QuakeCProgram,
        &mut EngineServices<'_>,
        CallContext,
    ) -> Result<R, ServiceError>,
) -> Result<R, ServiceError> {
    let (row, storage) = host
        .modules
        .as_mut()
        .and_then(|m| m.parts_mut(module))
        .ok_or(ServiceError::Aborted)?;
    let Program::QuakeC(qc) = &mut row.request.program else {
        return Err(ServiceError::Aborted);
    };
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    call(qc, &mut services, row.request.context)
}

impl FrameHost {
    pub(super) fn initialize_quakec_map(&mut self, module: ModuleId) -> Result<(), CallError> {
        self.start_quakec_map(module).map_err(|error| {
            if let Some((row, storage)) = self.modules.as_mut().and_then(|m| m.parts_mut(module)) {
                let mut services =
                    self.runtime
                        .engine_services(&mut self.console, storage, &mut row.scratch);
                let _ = (ENGINE_CALLS.console_formatted)(
                    &mut services,
                    format_args!("module {}: map startup: {error:?}\n", module.0),
                );
            }
            CallError::Rejected
        })
    }

    fn start_quakec_map(&mut self, module: ModuleId) -> Result<(), ServiceError> {
        let row = self.module_row_mut(module).ok_or(ServiceError::Aborted)?;
        let Program::QuakeC(qc) = &mut row.request.program else {
            return Ok(());
        };
        let Some(map) = qc.map.take() else {
            return Ok(());
        };
        let rules = row.request.context.native_rules;
        let time = row.request.context.clock;
        let anchor = row.request.anchor;
        let frame = row.request.frame;
        let source = self
            .runtime
            .entity_sources
            .get(map.entity_source)
            .ok_or(ServiceError::Entity)?;
        let bytes = std::sync::Arc::clone(&source.bytes);
        let lump = EntityLump::parse_with_match(&bytes, source.syntax, NameMatch::Exact)
            .map_err(|_| ServiceError::Entity)?;
        if lump.records.is_empty() {
            return Err(ServiceError::Entity);
        }
        let world = self
            .runtime
            .collision
            .as_ref()
            .filter(|world| world.index == 0)
            .ok_or(ServiceError::Geometry)?;
        let model_count = self
            .runtime
            .geometry
            .model_count(world.geometry)
            .ok_or(ServiceError::Geometry)?;
        let context = qa_console::views::Context {
            source: rules,
            side: qa_console::catalog::Scope::Server,
            role: qa_console::views::Role::Game,
            ..qa_console::views::Context::default()
        };
        let deathmatch = self
            .console
            .cvars
            .bind("deathmatch", context)
            .ok_or(ServiceError::Cvar)?;
        let coop = self
            .console
            .cvars
            .bind("coop", context)
            .ok_or(ServiceError::Cvar)?;
        let coop_value = self
            .console
            .cvars
            .numeric(coop)
            .map_err(|_| ServiceError::Cvar)?;
        if rules == RuleSetId::Quake && coop_value != 0.0 {
            self.console
                .cvars
                .force_write(deathmatch, "0")
                .map_err(|_| ServiceError::Cvar)?;
        }
        let deathmatch_value = self
            .console
            .cvars
            .numeric(deathmatch)
            .map_err(|_| ServiceError::Cvar)?;
        let skill = self
            .console
            .cvars
            .bind("skill", context)
            .ok_or(ServiceError::Cvar)?;
        let skill_value = self
            .console
            .cvars
            .numeric(skill)
            .map_err(|_| ServiceError::Cvar)?;
        let skill_value = ((f64::from(skill_value) + 0.5) as i32).clamp(0, 3);
        if rules == RuleSetId::Quake {
            self.console
                .cvars
                .force_write(skill, ["0", "1", "2", "3"][skill_value as usize])
                .map_err(|_| ServiceError::Cvar)?;
        }
        let map_name = map
            .world_path
            .strip_prefix("maps/")
            .unwrap_or(&map.world_path);
        let map_name = map_name.strip_suffix(".bsp").unwrap_or(map_name);
        edit(self, module, |qc, services, context| {
            let models = services
                .storage
                .resources(module)
                .ok_or(ServiceError::ConfigString)?[0];
            if model_count == 0 || model_count >= models.count {
                return Err(ServiceError::Capacity);
            }
            let model =
                (ENGINE_CALLS.resource_index)(services, module, models, map.world_path.as_bytes())?;
            if model != 1 {
                return Err(ServiceError::ConfigString);
            }
            for index in 1..model_count {
                let mut name = FixedText::<32>::default();
                write!(&mut name, "*{index}").map_err(|_| ServiceError::Text)?;
                if (ENGINE_CALLS.resource_index)(services, module, models, name.as_bytes())?
                    != index + 1
                {
                    return Err(ServiceError::ConfigString);
                }
            }
            qc.vm
                .initialize_map(time, map_name.as_bytes(), 0.0)
                .map_err(|_| ServiceError::Memory)?;
            if rules == RuleSetId::Quake {
                let (name, value): (&[u8], f32) = if coop_value != 0.0 {
                    (b"coop", coop_value)
                } else {
                    (b"deathmatch", deathmatch_value)
                };
                qc.vm
                    .set_map_number(name, value)
                    .map_err(|_| ServiceError::Memory)?;
            }
            let native = qc
                .vm
                .resolve_entity(&services.server.entities, context.module, 0)?;
            if native != Some(anchor) {
                return Err(ServiceError::Entity);
            }
            qc.vm
                .seed_map_world(0, map.world_path.as_bytes())
                .map_err(|_| ServiceError::Memory)
        })?;
        if rules == RuleSetId::QuakeWorld {
            self.call_module_export(module, frame, time)
                .map_err(|_| ServiceError::Aborted)?;
        }
        let bindings = {
            let row = self.module_row_mut(module).ok_or(ServiceError::Aborted)?;
            let Program::QuakeC(qc) = &mut row.request.program else {
                return Err(ServiceError::Aborted);
            };
            qc.vm.set_time(time).map_err(|_| ServiceError::Memory)?;
            qc.vm
                .bind_map_fields(&lump.names, rules)
                .map_err(|_| ServiceError::Memory)?
        };
        for (index, record) in lump.records.iter().enumerate() {
            let inhibit = qa_gameplay::rules::map_inhibit_flags(
                rules,
                self.console
                    .cvars
                    .numeric(deathmatch)
                    .map_err(|_| ServiceError::Cvar)?
                    != 0.0,
                skill_value,
            );
            let (entity, callback) = edit(self, module, |qc, services, context| {
                let native = if index == 0 {
                    0
                } else {
                    qc.vm.spawn_entity(services, context)?
                };
                let entity = qc
                    .vm
                    .resolve_entity(&services.server.entities, module, native)?
                    .ok_or(ServiceError::Entity)?;
                let admission = qc
                    .vm
                    .admit_map_record(&bindings, native, &lump.fields[record.clone()], |key| {
                        let mut text = FixedText::<8192>::default();
                        const SUFFIX: &[u8] = b"' is not a field\n";
                        let key = &key[..key
                            .len()
                            .min(FixedText::<8192>::CAPACITY - 1 - SUFFIX.len())];
                        if text.append_bytes(b"'").is_ok()
                            && text.append_bytes(key).is_ok()
                            && text.append_bytes(SUFFIX).is_ok()
                        {
                            let _ = (ENGINE_CALLS.console)(services, text.as_bytes());
                        }
                    })
                    .map_err(|_| ServiceError::Memory)?;
                let callback = if !admission.has_pairs || admission.spawn_flags & inhibit != 0 {
                    None
                } else if let Some(classname) = admission.classname {
                    qc.vm
                        .map_spawn_callback(classname)
                        .map_err(|_| ServiceError::Memory)?
                } else {
                    None
                };
                if callback.is_none() {
                    if index == 0 {
                        return Err(ServiceError::Entity);
                    }
                    if admission.has_pairs && admission.spawn_flags & inhibit == 0 {
                        let _ = (ENGINE_CALLS.console_formatted)(
                            services,
                            format_args!("No spawn function for map entity {index}\n"),
                        );
                    }
                    qc.vm.remove_entity(services, context, native)?;
                }
                Ok((entity, callback))
            })?;
            if let Some(callback) = callback {
                // The same lifetime table guards entry; callbacks may spawn
                // or free later ordinals before the next source record.
                self.call_module(callback, CallbackCall::Spawn { entity, time })
                    .map_err(|_| ServiceError::Aborted)?;
            }
        }
        Ok(())
    }
}
