//! Received cues use accepted CLIENT snapshots and configuration bindings.
use super::FrameHost;
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{ClientId, ResourceNamespace, SoundEmitter, SoundEvent, Vec3},
};
use qa_network::outputs::{Cue, MuzzleFlash, MuzzleFormat, Sound};
use qa_network::{projection::EntityRecord, snapshots::ReceivedFrame};

impl FrameHost {
    pub(crate) fn received_cue(
        &mut self,
        client: ClientId,
        cue: Cue,
        current: Option<ReceivedFrame<'_>>,
    ) {
        match cue {
            Cue::Sound(sound) => {
                if let Some(event) = self.native_sound_event(client, sound, current) {
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
        if slot == 0 {
            return Some(Vec3::default());
        }
        let projection = self
            .runtime
            .local_snapshots
            .get(client.0 as usize)?
            .as_ref()?
            .entity_projection();
        macro_rules! position {
            ($frame:expr) => {{
                let frame = $frame;
                let index = frame
                    .entities
                    .binary_search_by_key(&u32::from(slot), |entity| entity.number)
                    .ok()?;
                let mut record = EntityRecord::default();
                projection
                    .apply(
                        &frame.entities[index].words,
                        &mut record,
                        ResourceNamespace::Client(client),
                    )
                    .then_some(record.position)
            }};
        }
        match current? {
            ReceivedFrame::NetQuake(frame) => position!(frame),
            ReceivedFrame::QuakeWorld(frame) => position!(frame),
            ReceivedFrame::Quake2(frame) => position!(frame),
            ReceivedFrame::Quake2Repro(frame) => position!(frame),
            ReceivedFrame::Quake3(frame) => position!(frame),
        }
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
                    spatial_rules: rules,
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
        &self,
        client: ClientId,
        wire: Sound,
        current: Option<ReceivedFrame<'_>>,
    ) -> Option<SoundEvent> {
        let resources = &self
            .runtime
            .client_resources
            .get(client.0 as usize)?
            .as_ref()?
            .state;
        let sound = resources.sound(u32::from(wire.index))?;
        let ordinal = wire.entity();
        let entity = ordinal
            .filter(|slot| *slot != 0)
            .map(|slot| SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: i32::from(slot),
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
            spatial_rules: resources.layout.rules,
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
}
