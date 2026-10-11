//! Received native cues resolve through the existing module resource bindings.
use super::{FrameHost, Program, State};
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{ClientId, NativeEntity, SoundEvent, Vec3},
};
use qa_network::outputs::{Cue, Sound};

impl FrameHost {
    pub(crate) fn received_cue(&mut self, client: ClientId, cue: Cue) {
        if let Cue::Sound(sound) = cue
            && let Some(event) = self.native_sound_event(client, sound)
        {
            // These are received CLIENT cues. They must never be encoded again
            // for SERVER consumers or echoed to other local seats.
            let _ = self
                .runtime
                .server
                .events
                .push_to(OutputTarget::Presentation, FrameEvent::Sound(event));
        }
        // Muzzle sound/effect rules are bound separately from svc_sound.
    }

    fn native_sound_event(&self, client: ClientId, wire: Sound) -> Option<SoundEvent> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let module = self.runtime.server.clients.get(client.0 as usize)?.module;
            let row = self.module_row(module)?;
            if row.state != State::Running {
                return None;
            }
            let sound = row
                .storage
                .sound_id(module, row.native_sounds()?, i32::from(wire.index))
                .ok()??;
            let Program::Native { vm, .. } = &row.request.program else {
                return None;
            };
            let ordinal = wire.entity();
            let entity = if let Some(slot) = ordinal.filter(|&slot| slot != 0) {
                // Preserve the emitter's channel identity. An unknown actor
                // must not become a world sound that replaces another voice.
                Some(vm.entity_binding(
                    &self.runtime.server.entities,
                    NativeEntity {
                        module,
                        slot: i32::from(slot),
                    },
                )?)
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
