//! QuakeWorld routed-message presentation without reinterpreting wire
//! opcodes.
//!
//! Ported from donor `src/compat/qc/quakeworld-presentation.ts`
//! (`presentQuakeWorldMessage`).
//!
//! Local mirrors: [`QwLocalHost`] mirrors `QuakeCLocalMessageHost` from
//! `src/app/bootstrap/simulation/quakec-local-messages.ts`;
//! [`QwLocalState`] mirrors `QuakeCLocalMessages.receive`; [`LocalMessage`]
//! and [`QwPresentedEvent`] mirror the local presentation shapes. Routed
//! messages reuse `super::presentation_host`; temporary-entity projection
//! reuses `super::message_effects`.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::message_effects::{quake_temporary_event, QcBroadcastEffect};
use super::presentation_host::{QcRoutedMessage, QwMessage};
use crate::error::GuestError;

/// Intermission screen hold time in seconds.
pub const INTERMISSION_EXIT_AFTER: f64 = 5.0;

/// Muzzle pose for muzzle-flash presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct MuzzlePose {
    /// Muzzle origin.
    pub origin: Vec3,
    /// Muzzle angles.
    pub angles: Vec3,
}

/// Local client message delivered to one recipient.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalMessage {
    /// Console print with level.
    Print {
        /// Print level.
        level: i32,
        /// Message text.
        text: String,
    },
    /// Center-screen print.
    CenterPrint {
        /// Message text.
        text: String,
    },
    /// Client command text.
    StuffText {
        /// Command text.
        text: String,
    },
    /// Finale text.
    Finale {
        /// Finale text.
        text: String,
    },
    /// CD track change.
    CdTrack {
        /// Track number.
        track: u8,
    },
    /// Positional sound.
    Sound {
        /// Sound entity slot.
        entity: u16,
        /// Sound channel.
        channel: u8,
        /// Sound index.
        index: u8,
        /// Sound origin.
        origin: Vec3,
        /// Volume 0-255.
        volume: u8,
        /// Attenuation.
        attenuation: f32,
    },
    /// Stop a looping sound.
    StopSound {
        /// Sound entity slot.
        entity: u16,
        /// Sound channel.
        channel: u8,
    },
    /// Player stat update.
    Stat {
        /// Stat index.
        stat: u8,
        /// Stat value.
        value: i32,
    },
    /// Set entity angles.
    SetAngle {
        /// Entity slot.
        entity: u16,
        /// New angles.
        angles: Vec3,
    },
    /// Light style pattern.
    LightStyle {
        /// Style index.
        style: u8,
        /// Light pattern.
        pattern: String,
    },
    /// Static entity baseline.
    Static {
        /// Model index.
        model: u8,
        /// Frame.
        frame: u8,
        /// Color map.
        colormap: u8,
        /// Skin.
        skin: u8,
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
    },
    /// Static sound emitter.
    StaticSound {
        /// Emitter origin.
        origin: Vec3,
        /// Sound index.
        index: u8,
        /// Volume.
        volume: u8,
        /// Attenuation.
        attenuation: u8,
    },
    /// Damage feedback.
    Damage {
        /// Armor damage.
        armor: u8,
        /// Blood damage.
        blood: u8,
        /// Damage origin.
        origin: Vec3,
    },
    /// Pause flag.
    Pause {
        /// Paused.
        paused: bool,
    },
    /// Monster kill counter.
    KilledMonster,
    /// Secret counter.
    FoundSecret,
    /// End-of-level screen.
    SellScreen,
}

/// Presented effect event.
#[derive(Debug, Clone, PartialEq)]
pub enum QwPresentedEvent {
    /// Broadcast effect (point effect, beam, explosion, particles).
    Broadcast(QcBroadcastEffect),
    /// Intermission camera.
    Intermission {
        /// Camera origin.
        origin: Vec3,
        /// Camera angles.
        angles: Vec3,
        /// Map name.
        map: String,
        /// Seconds until exit.
        exit_after: f64,
        /// CD track.
        track: u8,
    },
    /// Muzzle flash.
    MuzzleFlash {
        /// Flashing actor.
        actor: ActorId,
        /// Flash origin.
        origin: Vec3,
        /// Muzzle pose.
        muzzle: MuzzlePose,
    },
}

/// Local state receipt recorded by [`QwLocalState::receive`].
#[derive(Debug, Clone, PartialEq)]
pub enum QwLocalReceipt {
    /// Intermission marker.
    Intermission,
    /// CD track change.
    CdTrack {
        /// Track number.
        track: u8,
    },
    /// Set view entity with its captured owner.
    SetView {
        /// View entity slot.
        entity: u16,
        /// Captured owner.
        actor: Option<ActorId>,
    },
    /// Passthrough message.
    Message(LocalMessage),
    /// No-op.
    Nop,
}

/// Local QuakeC message state.
#[derive(Debug, Default)]
pub struct QwLocalState {
    log: Vec<(QwLocalReceipt, Option<ActorId>)>,
}

impl QwLocalState {
    /// Fresh state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record received messages for one target.
    pub fn receive(&mut self, receipts: &[QwLocalReceipt], target: Option<&ActorId>) {
        for receipt in receipts {
            self.log.push((receipt.clone(), target.cloned()));
        }
    }

    /// Received receipt log (test inspection).
    #[must_use]
    pub fn log(&self) -> &[(QwLocalReceipt, Option<ActorId>)] {
        &self.log
    }
}

/// Local message host services.
pub trait QwLocalHost {
    /// Deliver a local message to one recipient (`None` broadcasts).
    fn message(&mut self, message: LocalMessage, target: Option<&ActorId>);
    /// Emit a presented event (`None` broadcasts).
    fn emit(&mut self, event: QwPresentedEvent, target: Option<&ActorId>);
    /// Current recipients for broadcast expansion.
    fn recipients(&self) -> Vec<ActorId>;
    /// Current map name.
    fn map(&self) -> String;
    /// Current time in seconds.
    fn seconds(&self) -> f64;
    /// Muzzle pose for an actor, if known.
    fn muzzle(&self, actor: &ActorId) -> Option<MuzzlePose>;
}

/// Present one routed QuakeWorld message to `target` (`None` broadcasts).
/// The match is exhaustive over [`QwMessage`], so no fallback arm is needed.
pub fn present_quake_world_message<H: QwLocalHost>(
    entry: &QcRoutedMessage,
    target: Option<&ActorId>,
    state: &mut QwLocalState,
    host: &mut H,
) -> Result<(), GuestError> {
    match &entry.message {
        QwMessage::Print { level, text } => {
            host.message(
                LocalMessage::Print {
                    level: *level,
                    text: text.clone(),
                },
                target,
            );
        }
        // QW `cl_parse.c` sets -2/-4; `view.c` `DropPunchAngle` subtracts
        // and clamps to zero before `V_CalcRefdef`, so kicks present nothing.
        QwMessage::Kick => {}
        QwMessage::TempEntity { effect } => {
            let event = quake_temporary_event(effect, entry.actor.as_ref(), true)?;
            host.emit(QwPresentedEvent::Broadcast(event), None);
        }
        QwMessage::Intermission { origin, angles } => {
            let map = host.map();
            let exit_after = host.seconds() + INTERMISSION_EXIT_AFTER;
            match target {
                Some(target) => host.emit(
                    QwPresentedEvent::Intermission {
                        origin: *origin,
                        angles: *angles,
                        map,
                        exit_after,
                        track: 0,
                    },
                    Some(target),
                ),
                None => {
                    for actor in host.recipients() {
                        host.emit(
                            QwPresentedEvent::Intermission {
                                origin: *origin,
                                angles: *angles,
                                map: map.clone(),
                                exit_after,
                                track: 0,
                            },
                            Some(&actor),
                        );
                    }
                }
            }
            state.receive(&[QwLocalReceipt::Intermission], target);
        }
        QwMessage::CdTrack { track } => {
            state.receive(&[QwLocalReceipt::CdTrack { track: *track }], target);
            host.message(LocalMessage::CdTrack { track: *track }, target);
        }
        QwMessage::Sound {
            entity,
            channel,
            index,
            origin,
            volume,
            attenuation,
        } => {
            if entry.actor.is_none() {
                return Err(GuestError::invalid("QW sound had no owned actor when written"));
            }
            state.receive(
                &[QwLocalReceipt::Message(LocalMessage::Sound {
                    entity: *entity,
                    channel: *channel,
                    index: *index,
                    origin: *origin,
                    volume: *volume,
                    attenuation: *attenuation,
                })],
                target,
            );
            host.message(
                LocalMessage::Sound {
                    entity: *entity,
                    channel: *channel,
                    index: *index,
                    origin: *origin,
                    volume: *volume,
                    attenuation: *attenuation,
                },
                target,
            );
        }
        QwMessage::StopSound { entity, channel } => {
            if entry.actor.is_none() {
                return Err(GuestError::invalid("QW stop-sound had no owned actor when written"));
            }
            let message = LocalMessage::StopSound {
                entity: *entity,
                channel: *channel,
            };
            state.receive(&[QwLocalReceipt::Message(message.clone())], target);
            host.message(message, target);
        }
        QwMessage::MuzzleFlash { .. } => {
            let Some(actor) = &entry.actor else {
                return Err(GuestError::invalid("QW muzzle flash had no owned actor when written"));
            };
            if let Some(muzzle) = host.muzzle(actor) {
                host.emit(
                    QwPresentedEvent::MuzzleFlash {
                        actor: actor.clone(),
                        origin: muzzle.origin,
                        muzzle,
                    },
                    None,
                );
            }
        }
        QwMessage::SetView { entity } => {
            state.receive(
                &[QwLocalReceipt::SetView {
                    entity: *entity,
                    actor: entry.actor.clone(),
                }],
                target,
            );
        }
        QwMessage::Nop => {
            state.receive(&[QwLocalReceipt::Nop], target);
        }
        QwMessage::Stat { stat, value } => {
            present_local(
                LocalMessage::Stat {
                    stat: *stat,
                    value: *value,
                },
                target,
                state,
                host,
            );
        }
        QwMessage::StuffText { text } => {
            present_local(LocalMessage::StuffText { text: text.clone() }, target, state, host);
        }
        QwMessage::CenterPrint { text } => {
            present_local(LocalMessage::CenterPrint { text: text.clone() }, target, state, host);
        }
        QwMessage::Finale { text } => {
            present_local(LocalMessage::Finale { text: text.clone() }, target, state, host);
        }
        QwMessage::SetAngle { entity, angles } => {
            present_local(
                LocalMessage::SetAngle {
                    entity: *entity,
                    angles: *angles,
                },
                target,
                state,
                host,
            );
        }
        QwMessage::LightStyle { style, pattern } => {
            present_local(
                LocalMessage::LightStyle {
                    style: *style,
                    pattern: pattern.clone(),
                },
                target,
                state,
                host,
            );
        }
        QwMessage::Static {
            model,
            frame,
            colormap,
            skin,
            origin,
            angles,
        } => {
            present_local(
                LocalMessage::Static {
                    model: *model,
                    frame: *frame,
                    colormap: *colormap,
                    skin: *skin,
                    origin: *origin,
                    angles: *angles,
                },
                target,
                state,
                host,
            );
        }
        QwMessage::StaticSound {
            origin,
            index,
            volume,
            attenuation,
        } => {
            present_local(
                LocalMessage::StaticSound {
                    origin: *origin,
                    index: *index,
                    volume: *volume,
                    attenuation: *attenuation,
                },
                target,
                state,
                host,
            );
        }
        QwMessage::Damage { armor, blood, origin } => {
            present_local(
                LocalMessage::Damage {
                    armor: *armor,
                    blood: *blood,
                    origin: *origin,
                },
                target,
                state,
                host,
            );
        }
        QwMessage::Pause { paused } => {
            present_local(LocalMessage::Pause { paused: *paused }, target, state, host);
        }
        QwMessage::KilledMonster => {
            present_local(LocalMessage::KilledMonster, target, state, host);
        }
        QwMessage::FoundSecret => {
            present_local(LocalMessage::FoundSecret, target, state, host);
        }
        QwMessage::SellScreen => {
            present_local(LocalMessage::SellScreen, target, state, host);
        }
    }
    Ok(())
}

fn present_local<H: QwLocalHost>(
    message: LocalMessage,
    target: Option<&ActorId>,
    state: &mut QwLocalState,
    host: &mut H,
) {
    state.receive(&[QwLocalReceipt::Message(message.clone())], target);
    host.message(message, target);
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::message_effects::TempEntityEffect;

    struct FakeHost {
        messages: Vec<(LocalMessage, Option<ActorId>)>,
        events: Vec<(QwPresentedEvent, Option<ActorId>)>,
        everyone: Vec<ActorId>,
    }

    impl QwLocalHost for FakeHost {
        fn message(&mut self, message: LocalMessage, target: Option<&ActorId>) {
            self.messages.push((message, target.cloned()));
        }

        fn emit(&mut self, event: QwPresentedEvent, target: Option<&ActorId>) {
            self.events.push((event, target.cloned()));
        }

        fn recipients(&self) -> Vec<ActorId> {
            self.everyone.clone()
        }

        fn map(&self) -> String {
            "e1m1".to_string()
        }

        fn seconds(&self) -> f64 {
            100.0
        }

        fn muzzle(&self, actor: &ActorId) -> Option<MuzzlePose> {
            Some(MuzzlePose {
                origin: vec3(actor.slot() as f32, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            })
        }
    }

    fn harness() -> (IdentityOwner, QwLocalState, FakeHost) {
        let owner = IdentityOwner::create("qw-presentation").unwrap();
        let everyone = vec![owner.actor(1, 1), owner.actor(2, 1)];
        (
            owner,
            QwLocalState::new(),
            FakeHost {
                messages: Vec::new(),
                events: Vec::new(),
                everyone,
            },
        )
    }

    #[test]
    fn print_kick_and_temp_entities_present() {
        let (owner, mut state, mut host) = harness();
        let target = owner.actor(1, 1);
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::Print {
                    level: 2,
                    text: "hi".to_string(),
                },
                actor: None,
            },
            Some(&target),
            &mut state,
            &mut host,
        )
        .unwrap();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::Kick,
                actor: None,
            },
            Some(&target),
            &mut state,
            &mut host,
        )
        .unwrap();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::TempEntity {
                    effect: TempEntityEffect::Point {
                        effect_type: 3,
                        origin: vec3(1.0, 2.0, 3.0),
                        count: 1,
                    },
                },
                actor: None,
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        assert_eq!(host.messages.len(), 1);
        assert_eq!(host.events.len(), 1);
        assert!(matches!(host.events[0].0, QwPresentedEvent::Broadcast(_)));
        assert!(state.log().is_empty());
    }

    #[test]
    fn intermission_expands_to_recipients() {
        let (_owner, mut state, mut host) = harness();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::Intermission {
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 90.0, 0.0),
                },
                actor: None,
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        assert_eq!(host.events.len(), 2);
        assert!(matches!(
            &host.events[0].0,
            QwPresentedEvent::Intermission { map, exit_after, track: 0, .. }
            if map == "e1m1" && *exit_after == 105.0
        ));
        assert_eq!(state.log(), &[(QwLocalReceipt::Intermission, None)]);
    }

    #[test]
    fn sounds_and_muzzle_require_owners() {
        let (owner, mut state, mut host) = harness();
        let actor = owner.actor(1, 1);
        let sound = QwMessage::Sound {
            entity: 1,
            channel: 1,
            index: 2,
            origin: vec3(0.0, 0.0, 0.0),
            volume: 200,
            attenuation: 1.0,
        };
        assert!(present_quake_world_message(
            &QcRoutedMessage {
                message: sound.clone(),
                actor: None
            },
            None,
            &mut state,
            &mut host
        )
        .is_err());
        present_quake_world_message(
            &QcRoutedMessage {
                message: sound,
                actor: Some(actor.clone()),
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::MuzzleFlash { entity: 1 },
                actor: Some(actor),
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        assert!(host
            .events
            .iter()
            .any(|(event, _)| matches!(event, QwPresentedEvent::MuzzleFlash { .. })));
        assert!(present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::StopSound { entity: 1, channel: 1 },
                actor: None
            },
            None,
            &mut state,
            &mut host
        )
        .is_err());
    }

    #[test]
    fn set_view_records_owner_and_passthroughs_reach_host() {
        let (owner, mut state, mut host) = harness();
        let actor = owner.actor(2, 1);
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::SetView { entity: 2 },
                actor: Some(actor.clone()),
            },
            Some(&actor),
            &mut state,
            &mut host,
        )
        .unwrap();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::CdTrack { track: 3 },
                actor: None,
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::Stat { stat: 1, value: 100 },
                actor: None,
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        present_quake_world_message(
            &QcRoutedMessage {
                message: QwMessage::Damage {
                    armor: 10,
                    blood: 20,
                    origin: vec3(0.0, 0.0, 0.0),
                },
                actor: None,
            },
            None,
            &mut state,
            &mut host,
        )
        .unwrap();
        assert_eq!(
            state.log()[0],
            (
                QwLocalReceipt::SetView {
                    entity: 2,
                    actor: Some(actor)
                },
                Some(owner.actor(2, 1))
            )
        );
        assert!(state
            .log()
            .iter()
            .any(|(receipt, _)| matches!(receipt, QwLocalReceipt::CdTrack { track: 3 })));
        assert_eq!(host.messages.len(), 3);
    }

    #[test]
    fn remaining_passthroughs_present_without_loss() {
        let (_owner, mut state, mut host) = harness();
        let messages = vec![
            QwMessage::Nop,
            QwMessage::StuffText { text: "x".to_string() },
            QwMessage::CenterPrint { text: "y".to_string() },
            QwMessage::Finale { text: "z".to_string() },
            QwMessage::SetAngle {
                entity: 1,
                angles: vec3(0.0, 0.0, 0.0),
            },
            QwMessage::LightStyle {
                style: 1,
                pattern: "a".to_string(),
            },
            QwMessage::Static {
                model: 1,
                frame: 0,
                colormap: 0,
                skin: 0,
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            },
            QwMessage::StaticSound {
                origin: vec3(0.0, 0.0, 0.0),
                index: 1,
                volume: 1,
                attenuation: 1,
            },
            QwMessage::Pause { paused: true },
            QwMessage::KilledMonster,
            QwMessage::FoundSecret,
            QwMessage::SellScreen,
        ];
        for message in &messages {
            present_quake_world_message(
                &QcRoutedMessage {
                    message: message.clone(),
                    actor: None,
                },
                None,
                &mut state,
                &mut host,
            )
            .unwrap();
        }
        assert_eq!(state.log().len(), messages.len());
        assert_eq!(host.messages.len(), messages.len() - 1);
    }
}
