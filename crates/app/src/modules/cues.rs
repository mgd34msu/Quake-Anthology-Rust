//! Received native cues resolve through the existing module resource bindings.
use super::{FrameHost, Module, Program, State};
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{ClientId, EntityId, NativeEntity, RuleSetId, SoundEvent, Vec3},
};
use qa_network::outputs::{Cue, MuzzleFlash, MuzzleFormat, Sound};

impl FrameHost {
    pub(crate) fn received_cue(&mut self, client: ClientId, cue: Cue) {
        match cue {
            Cue::Sound(sound) => {
                if let Some(event) = self.native_sound_event(client, sound) {
                    self.present_sound(event);
                }
            }
            Cue::MuzzleFlash(flash) => self.muzzle_flash(client, flash),
        }
    }

    fn present_sound(&mut self, event: SoundEvent) {
        // Received CLIENT cues cannot be encoded again for SERVER consumers.
        let _ = self
            .runtime
            .server
            .events
            .push_to(OutputTarget::Presentation, FrameEvent::Sound(event));
    }

    fn cue_module(&self, client: ClientId) -> Option<&Module> {
        let module = self.runtime.server.clients.get(client.0 as usize)?.module;
        self.module_row(module)
            .filter(|row| row.state == State::Running)
    }

    fn cue_entity(&self, row: &Module, slot: u16) -> Option<Option<EntityId>> {
        if slot == 0 {
            return Some(None);
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if let Program::Native { vm, .. } = &row.request.program {
            return vm
                .entity_binding(
                    &self.runtime.server.entities,
                    NativeEntity {
                        module: row.request.context.module,
                        slot: i32::from(slot),
                    },
                )
                .map(Some);
        }
        None
    }

    fn muzzle_flash(&mut self, client: ClientId, flash: MuzzleFlash) {
        use qa_network::commands::packet::Protocol;
        // Product differences belong to this native message boundary. A map,
        // movement rule or provider kind must not select client sound recipes.
        let Some(snapshot) = self
            .runtime
            .local_snapshots
            .get(client.0 as usize)
            .and_then(Option::as_ref)
        else {
            return;
        };
        let rules = match snapshot.protocol {
            Protocol::Quake2_34 => RuleSetId::Quake2,
            Protocol::Quake2Repro1038 => RuleSetId::Quake2Rerelease,
            _ => return,
        };
        let Some(row) = self.cue_module(client) else {
            return;
        };
        let Some(Some(entity)) = self.cue_entity(row, flash.entity) else {
            return;
        };
        let position = self.runtime.server.entities.columns.position[entity.slot as usize];
        let recipes = match flash.format {
            MuzzleFormat::Player => qa_gameplay::rules::q2::sounds::player(rules, flash.flash),
            MuzzleFormat::Monster | MuzzleFormat::MonsterWide => {
                qa_gameplay::rules::q2::sounds::monster(rules, flash.flash)
            }
        };
        for &recipe in recipes {
            if let Some(sound) = self.muzzle_sounds.choose(recipe) {
                self.present_sound(SoundEvent {
                    sound,
                    spatial_rules: rules,
                    entity: Some(entity),
                    channel: recipe.channel,
                    position,
                    volume: recipe.volume(rules, flash.silenced),
                    attenuation: recipe.attenuation,
                    time_offset: recipe.delay,
                    client: Some(client),
                    ..SoundEvent::default()
                });
            }
        }
    }

    fn native_sound_event(&self, client: ClientId, wire: Sound) -> Option<SoundEvent> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let row = self.cue_module(client)?;
            let sound = row
                .storage
                .sound_id(
                    row.request.context.module,
                    row.native_sounds()?,
                    i32::from(wire.index),
                )
                .ok()??;
            let ordinal = wire.entity();
            let entity = if let Some(slot) = ordinal {
                self.cue_entity(row, slot)?
            } else {
                None
            };
            let position = if let Some(position) = wire.position() {
                position
            } else if ordinal.is_some_and(|slot| slot != 0) {
                self.runtime.server.entities.columns.position[entity?.slot as usize]
            } else {
                Vec3::default()
            };
            Some(SoundEvent {
                sound,
                spatial_rules: row.request.context.console.source,
                entity,
                channel: u16::from(wire.channel),
                position,
                volume: f32::from(wire.volume) / 255.0,
                attenuation: f32::from(wire.attenuation) / 64.0,
                time_offset: f32::from(wire.time_offset) / 1000.0,
                client: Some(client),
                ..SoundEvent::default()
            })
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (client, wire);
            None
        }
    }
}
