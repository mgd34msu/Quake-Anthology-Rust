//! Received cues use accepted CLIENT snapshots and configuration bindings.
use super::FrameHost;
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{ClientId, ResourceNamespace, SoundAction, SoundEmitter, SoundEvent, Vec3},
};
use qa_network::outputs::{Cue, MuzzleFlash, MuzzleFormat, Sound};
use qa_network::snapshots::ReceivedFrame;

impl FrameHost {
    pub(crate) fn received_cue(
        &mut self,
        client: ClientId,
        cue: Cue,
        current: Option<ReceivedFrame<'_>>,
    ) {
        match cue {
            Cue::Sound(sound) => {
                if let Some(event) = self.native_sound_event(client, sound, current, false) {
                    self.present_sound(event);
                }
            }
            Cue::StaticSound(sound) => {
                if let Some(event) = self.native_sound_event(client, sound, current, true) {
                    self.present_sound(event);
                }
            }
            Cue::MuzzleFlash(flash) => self.muzzle_flash(client, flash, current),
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

    fn cue_position(
        &self,
        client: ClientId,
        slot: u16,
        current: Option<ReceivedFrame<'_>>,
    ) -> Option<Vec3> {
        let projection = self
            .runtime
            .local_snapshots
            .get(client.0 as usize)?
            .as_ref()?
            .entity_projection();
        crate::client_scene::ClientEntities {
            client,
            current,
            previous: None,
            projection,
            interpolation: crate::snapshots::Interpolation::default(),
        }
        .sound_origin(u32::from(slot))
    }

    fn muzzle_flash(
        &mut self,
        client: ClientId,
        flash: MuzzleFlash,
        current: Option<ReceivedFrame<'_>>,
    ) {
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
        let rules = snapshot.protocol.rule_set();
        let Some(resources) = self.runtime.client_resources[client.0 as usize].as_ref() else {
            return;
        };
        let sound_rules = resources.state.sound_rules();
        let Some(position) = self.cue_position(client, flash.entity, current) else {
            return;
        };
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
                    spatial_rules: sound_rules,
                    entity: Some(SoundEmitter::Native {
                        namespace: ResourceNamespace::Client(client),
                        slot: i32::from(flash.entity),
                    }),
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

    fn native_sound_event(
        &mut self,
        client: ClientId,
        wire: Sound,
        current: Option<ReceivedFrame<'_>>,
        static_sound: bool,
    ) -> Option<SoundEvent> {
        let lifetime = self.runtime.server.clients.get(client.0 as usize)?.entity;
        let (sound, sound_rules, static_emitter) = {
            let resources = &mut self
                .runtime
                .client_resources
                .get_mut(client.0 as usize)?
                .as_mut()?
                .state;
            let static_emitter = if static_sound {
                Some(resources.static_emitter(client, lifetime)?)
            } else {
                None
            };
            (
                resources.sound(u32::from(wire.index))?,
                resources.sound_rules(),
                static_emitter,
            )
        };
        if static_sound && !self.runtime.sound_bank.as_ref()?.has_loop_marker(sound) {
            return None;
        }
        let ordinal = if static_sound { None } else { wire.entity() };
        let entity = static_emitter.or_else(|| {
            ordinal.map(|slot| SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: i32::from(slot),
            })
        });
        let position = if let Some(position) = wire.position() {
            position
        } else if ordinal.is_some_and(|slot| slot != 0) {
            self.cue_position(client, ordinal?, current)?
        } else {
            Vec3::default()
        };
        Some(SoundEvent {
            sound,
            spatial_rules: sound_rules,
            entity,
            channel: u16::from(wire.channel),
            position,
            positioned: wire.position().is_some(),
            volume: f32::from(wire.volume) / 255.0,
            attenuation: f32::from(wire.attenuation) / 64.0,
            time_offset: f32::from(wire.time_offset) / 1000.0,
            client: Some(client),
            action: if static_sound {
                SoundAction::StartLoop
            } else {
                SoundAction::Play
            },
            ..SoundEvent::default()
        })
    }
}
