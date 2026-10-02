//! Authoritative native protocol output over the presentation history.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/events.ts`
//! (`SimulationEvents`).
//!
//! The presentation history itself (donor
//! `src/app/bootstrap/presentation-state.ts`) lives outside this partition,
//! so it surfaces as the
//! [`PresentationStateSeam`] trait; only the operations the donor subclass
//! calls or overrides are modeled.

use qa_content::contract::{ArmorState, ContentId, ItemId, PresentationOwner, ResolvedResourceReference, ResourceId};
use qa_content::q1::foundation::types::{Q1Event, Q1SoundChannel};
use qa_content::q2::composition::types::Q2CompositionEvent;
use qa_content::q2::foundation::host::Q2PresentationEvent;
use qa_content::q2::foundation::weapons::types::Q2WeaponEvent;
use qa_content::q2::multiplayer::ctf::types::Q2CtfEvent;
use qa_content::q2::multiplayer::lmctf::types::LmctfEvent;
use qa_core::identity::{ActorId, ClientId, ProviderId, SavedActorId, SeatId};
use qa_core::math::{vec3, Vec3};
use qa_core::time::SourceTime;
use qa_world::save::value::{int, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

use super::physics::PhysicsBodies;
use super::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// Mirror of `EventAudience` from donor `src/contracts/session.ts`
/// (canonical home: `qa_net::common::session::EventAudience`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAudience {
    /// Every client.
    World,
    /// One seat.
    Seat {
        /// Seat.
        seat: SeatId,
    },
    /// One client.
    Client {
        /// Client.
        client: ClientId,
    },
}

/// Mirror of `NetworkEvent` from donor `src/contracts/protocol.ts`
/// (canonical home: `qa_net::common::protocol::NetworkEvent`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkEvent {
    /// Console print.
    Print {
        /// Print level.
        level: i32,
        /// Text.
        text: String,
    },
    /// Center print.
    CenterPrint {
        /// Text.
        text: String,
    },
    /// Command text.
    CommandText {
        /// Text.
        text: String,
    },
    /// Config string.
    ConfigString {
        /// Index.
        index: i32,
        /// Value.
        value: String,
    },
    /// Positioned sound.
    Sound {
        /// Entity number.
        entity_number: i32,
        /// Channel.
        channel: i32,
        /// Sound index.
        sound_index: i32,
        /// Origin override.
        origin: Option<Vec3>,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
        /// Delay in seconds.
        delay_seconds: f64,
    },
    /// Q1 damage feedback.
    Q1Damage {
        /// Armor damage.
        armor: f64,
        /// Blood damage.
        blood: f64,
        /// Source origin.
        source: Vec3,
    },
    /// Q1 particle burst.
    Q1Particle {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Count.
        count: i32,
        /// Color.
        color: i32,
    },
    /// Q2 layout program.
    Q2Layout {
        /// Program.
        program: String,
    },
    /// Q2 inventory counts.
    Q2Inventory {
        /// Counts.
        counts: Vec<f64>,
    },
    /// Q2 muzzle flash.
    Q2MuzzleFlash {
        /// Entity number.
        entity_number: i32,
        /// Flash number.
        flash: i32,
        /// Monster flash.
        monster: bool,
    },
    /// Q3 server command.
    Q3ServerCommand {
        /// Sequence.
        sequence: i32,
        /// Text.
        text: String,
    },
    /// Disconnect.
    Disconnect {
        /// Reason.
        reason: String,
    },
}

/// Mirror of `Q2NativeCause` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::Q2NativeCause`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2NativeCause {
    /// Classic cause.
    Classic {
        /// Game.
        game: Q2NativeGame,
        /// Value.
        value: i32,
    },
    /// Rerelease cause.
    Rerelease {
        /// Cause id.
        id: i32,
        /// Friendly fire.
        friendly_fire: bool,
        /// No point loss.
        no_point_loss: bool,
    },
}

/// Mirror of the classic native game from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::Q2NativeGame`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2NativeGame {
    /// Base game.
    Base,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
    /// CTF.
    Ctf,
}

/// Mirror of the attack armor effect from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::AttackArmorEffect`); unify
/// post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    HalfEffectiveness,
}

/// Mirror of the environment hazard from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::EnvironmentHazard`); unify
/// post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentHazard {
    /// Fall.
    Fall,
    /// Drown.
    Drown,
    /// Lava.
    Lava,
    /// Slime.
    Slime,
    /// Crush.
    Crush,
    /// Trigger.
    Trigger,
}

/// Mirror of the attack cause from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::AttackCause`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake I cause.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect.
        armor_effect: Option<AttackArmorEffect>,
    },
    /// Quake II cause.
    Q2 {
        /// Canonical engine cause id.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Native cause.
        native: Option<Q2NativeCause>,
    },
    /// Quake III cause.
    Q3 {
        /// Canonical engine cause id.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
    },
    /// Environment cause.
    Environment {
        /// Hazard.
        hazard: EnvironmentHazard,
    },
}

/// Mirror of `AttackProvenance` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::AttackProvenance`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence.
    pub sequence: u64,
    /// Time.
    pub time: SourceTime,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// Powerup owner that already applied its modifier.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
    /// Cause.
    pub cause: AttackCause,
}

/// Mirror of the damage delivery from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageDelivery`); unify
/// post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    /// Direct.
    Direct,
    /// Radius.
    Radius,
}

/// Mirror of `DamageRequest` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageRequest`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target.
    pub target: ActorId,
    /// Amount.
    pub amount: f64,
    /// Knockback.
    pub knockback: f64,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
}

/// Mirror of `DamageMutation` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageMutation`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health change.
    Health {
        /// Health before.
        before: f64,
        /// Health after.
        after: f64,
    },
    /// Armor change.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Source velocity change.
    SourceVelocity {
        /// Velocity before.
        before: Vec3,
        /// Velocity after.
        after: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
    /// Applied impulse.
    Impulse {
        /// Impulse.
        impulse: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
}

/// Mirror of the damage reaction from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageReaction`); unify
/// post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReaction {
    /// None.
    None,
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// Mirror of the damage feedback from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageFeedback`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageFeedback {
    /// Quake II savings.
    Q2 {
        /// Power armor saved.
        power_armor: f64,
        /// Armor saved.
        armor: f64,
        /// Blood.
        blood: f64,
        /// Knockback.
        knockback: f64,
    },
    /// Quake III savings.
    Q3 {
        /// Knockback.
        knockback: f64,
        /// Battlesuit.
        battlesuit: bool,
    },
}

/// Mirror of `DamageDecision` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageDecision`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Request.
    pub request: DamageRequest,
    /// Mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied damage.
    pub applied_damage: f64,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Source savings feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Mirror of `DamageOutcome` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::DamageOutcome`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Stale target.
    StaleTarget {
        /// Request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Survived.
        survived: bool,
    },
}

/// Mirror of `TransitionDecision` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_content::contract::TransitionDecision`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionDecision {
    /// Stay.
    Stay {
        /// Blocking objectives.
        blocked: Vec<String>,
    },
    /// Round.
    Round {
        /// Winner.
        winner: Option<String>,
    },
    /// Travel.
    Travel {
        /// Map.
        map: String,
        /// Spawn point.
        spawn_point: String,
        /// Complete campaign.
        complete_campaign: bool,
    },
    /// Campaign complete.
    CampaignComplete {
        /// Campaign.
        campaign: ProviderId,
    },
}

/// Mirror of `SimulationEventPayload` from donor `src/contracts/session.ts`
/// (canonical home: `qa_net::common::session::SimulationEventPayload`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SimulationEventPayload {
    /// Positioned sound.
    Sound {
        /// Resource.
        resource: ResourceId,
        /// Actor.
        actor: Option<ActorId>,
        /// Origin.
        origin: Vec3,
        /// Channel.
        channel: i32,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
    },
    /// Damage outcome.
    Damage {
        /// Outcome.
        outcome: DamageOutcome,
    },
    /// Transition decision.
    Transition {
        /// Decision.
        decision: TransitionDecision,
    },
    /// Network message.
    Message {
        /// Event.
        event: NetworkEvent,
        /// Source presentation sequence.
        source_presentation_sequence: Option<u64>,
    },
}

/// Mirror of `SimulationEvent` from donor `src/contracts/session.ts`
/// (canonical home: `qa_net::common::session::SimulationEvent`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationEvent {
    /// Sequence.
    pub sequence: u64,
    /// Time.
    pub time: SourceTime,
    /// Audience.
    pub audience: EventAudience,
    /// Payload.
    pub payload: SimulationEventPayload,
}

/// Presentation history failures observed through the seam.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresentationStateError {
    /// Save requires consumed source output.
    #[error("Save requires consumed source output")]
    OutputPending,
    /// Registered resource path does not match its source request.
    #[error("Registered resource path does not match its source request")]
    ResourcePathMismatch,
}

/// Presentation history surface used by simulation events.
///
/// Seam over donor `PresentationState`
/// (`src/app/bootstrap/presentation-state.ts`); only the operations the
/// donor subclass calls or overrides are modeled. The implementor owns the
/// clock, source slots, resource table, and Q1 fog wiring the donor
/// constructor receives.
pub trait PresentationStateSeam {
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Source slot for an actor.
    fn source_slot(&self, actor: &ActorId) -> Option<i32>;
    /// Emit an owned presentation event into the shared history.
    fn emit_owned(
        &mut self,
        owner: Option<PresentationOwner>,
        content: &ContentId,
        source: SourcePresentationEvent,
        time: SourceTime,
        recipient: Option<ActorId>,
    ) -> SimulationPresentationEvent;
    /// Resolve a resource by content and requested path.
    fn resource_by_path(&self, content: &ContentId, path: &str) -> Option<ResolvedResourceReference>;
    /// Register a resolved resource (donor `registerResource`).
    fn register_resource(
        &mut self,
        content: &ContentId,
        path: &str,
        resource: ResolvedResourceReference,
    ) -> Result<(), PresentationStateError>;
    /// Animated light styles (donor `lightStyles`).
    fn light_styles(&self, seconds: f64) -> Vec<super::types::SceneLightStyle>;
    /// Take pending presentation output.
    fn take_presentation(&mut self) -> Vec<SimulationPresentationEvent>;
    /// Assert all output was consumed before a save.
    fn assert_output_consumed(&self) -> Result<(), PresentationStateError>;
    /// Capture the history checkpoint as a JSON object.
    fn capture_state(&self) -> SaveJson;
    /// Restore a captured history checkpoint.
    fn restore_state(
        &mut self,
        reader: &SaveReader,
        reference: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<(), WorldError>;
}

/// Simulation event failures.
#[derive(Debug, Error)]
pub enum EventsError {
    /// Save requires consumed source output.
    #[error("Save requires consumed source output")]
    OutputPending,
    /// Presentation history capture is not a JSON object.
    #[error("Presentation history capture is not a JSON object")]
    BadPresentationCapture,
}

impl From<PresentationStateError> for EventsError {
    fn from(_: PresentationStateError) -> Self {
        EventsError::OutputPending
    }
}

/// Map an actor to its connected client, if any.
pub type ClientForActor = Box<dyn Fn(&ActorId) -> Option<ClientId>>;

/// Authoritative native protocol output over the shared presentation history.
pub struct SimulationEvents {
    bodies: Box<dyn PhysicsBodies>,
    client_for: ClientForActor,
    presentation: Box<dyn PresentationStateSeam>,
    sequence: u64,
    emitted: Vec<SimulationEvent>,
}

struct SoundEmission<'a> {
    content: &'a ContentId,
    path: &'a str,
    actor: Option<ActorId>,
    origin: Vec3,
    channel: i32,
    volume: f64,
    attenuation: f64,
    recipient: Option<&'a ActorId>,
}

impl SimulationEvents {
    /// Create event state over bodies, client routing, and history.
    pub fn new(
        bodies: Box<dyn PhysicsBodies>,
        client_for: ClientForActor,
        presentation: Box<dyn PresentationStateSeam>,
    ) -> Self {
        Self {
            bodies,
            client_for,
            presentation,
            sequence: 0,
            emitted: Vec::new(),
        }
    }

    /// Next emission sequence.
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.sequence
    }

    /// Emit into the shared history, snooping protocol output.
    pub fn emit_owned(
        &mut self,
        owner: Option<PresentationOwner>,
        content: &ContentId,
        source: SourcePresentationEvent,
        time: SourceTime,
        recipient: Option<ActorId>,
    ) -> SimulationPresentationEvent {
        let presentation = self
            .presentation
            .emit_owned(owner, content, source.clone(), time, recipient.clone());
        self.snoop(content, &source, presentation.sequence, recipient);
        presentation
    }

    fn snoop(
        &mut self,
        content: &ContentId,
        source: &SourcePresentationEvent,
        sequence: u64,
        recipient: Option<ActorId>,
    ) {
        match source {
            SourcePresentationEvent::Q2Composition(event) => match event {
                Q2CompositionEvent::Ctf(Q2CtfEvent::Scoreboard { actor, layout, .. }) => {
                    self.message(
                        NetworkEvent::Q2Layout {
                            program: layout.clone(),
                        },
                        Some(actor),
                        None,
                    );
                }
                Q2CompositionEvent::Ctf(Q2CtfEvent::MatchStatus { text }) => {
                    self.message(
                        NetworkEvent::Print {
                            level: 2,
                            text: text.clone(),
                        },
                        None,
                        None,
                    );
                }
                Q2CompositionEvent::Lmctf(LmctfEvent::Scoreboard { actor, layout, .. }) => {
                    self.message(
                        NetworkEvent::Q2Layout {
                            program: layout.clone(),
                        },
                        Some(actor),
                        None,
                    );
                }
                Q2CompositionEvent::Lmctf(LmctfEvent::Hud { actor, layout, .. }) => {
                    self.message(
                        NetworkEvent::Q2Layout {
                            program: layout.clone(),
                        },
                        Some(actor),
                        None,
                    );
                }
                _ => {}
            },
            SourcePresentationEvent::Q1(event) => self.q1(content, event, sequence, recipient),
            SourcePresentationEvent::Q2(event) => self.q2(content, event, sequence),
            SourcePresentationEvent::Q2Weapon(Q2WeaponEvent::Muzzleflash { actor, flash, .. }) => {
                if let Some(entity_number) = self.presentation.source_slot(actor) {
                    self.message(
                        NetworkEvent::Q2MuzzleFlash {
                            entity_number,
                            flash: *flash,
                            monster: false,
                        },
                        None,
                        None,
                    );
                }
            }
            _ => {}
        }
    }

    /// Append a payload for a world or client audience.
    pub fn append(&mut self, payload: SimulationEventPayload, actor: Option<&ActorId>) {
        let client = actor.and_then(|actor| (self.client_for)(actor));
        if actor.is_some() && client.is_none() {
            return;
        }
        let sequence = self.sequence;
        self.sequence += 1;
        self.emitted.push(SimulationEvent {
            sequence,
            time: self.presentation.now(),
            payload,
            audience: match client {
                None => EventAudience::World,
                Some(client) => EventAudience::Client { client },
            },
        });
    }

    /// Append a network message.
    pub fn message(&mut self, event: NetworkEvent, actor: Option<&ActorId>, source_presentation_sequence: Option<u64>) {
        self.append(
            SimulationEventPayload::Message {
                event,
                source_presentation_sequence,
            },
            actor,
        );
    }

    /// Take pending simulation events.
    pub fn take(&mut self) -> Vec<SimulationEvent> {
        std::mem::take(&mut self.emitted)
    }

    /// Register a resolved resource (donor `registerResource`).
    pub fn register_resource(
        &mut self,
        content: &ContentId,
        path: &str,
        resource: ResolvedResourceReference,
    ) -> Result<(), PresentationStateError> {
        self.presentation.register_resource(content, path, resource)
    }

    /// Animated light styles (donor `lightStyles`).
    pub fn light_styles(&self, seconds: f64) -> Vec<super::types::SceneLightStyle> {
        self.presentation.light_styles(seconds)
    }

    /// Take pending presentation output.
    pub fn take_presentation(&mut self) -> Vec<SimulationPresentationEvent> {
        self.presentation.take_presentation()
    }

    /// Assert all output was consumed before a save.
    pub fn assert_output_consumed(&self) -> Result<(), EventsError> {
        self.presentation.assert_output_consumed()?;
        if !self.emitted.is_empty() {
            return Err(EventsError::OutputPending);
        }
        Ok(())
    }

    /// Capture the event sequence alongside the history checkpoint.
    pub fn capture(&self) -> Result<SaveJson, EventsError> {
        let SaveJson::Object(mut members) = self.presentation.capture_state() else {
            return Err(EventsError::BadPresentationCapture);
        };
        members.push(("sequence".to_string(), int(self.sequence as i64)));
        Ok(SaveJson::Object(members))
    }

    /// Restore the event sequence and history checkpoint.
    pub fn restore(
        &mut self,
        reader: &SaveReader,
        reference: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<(), WorldError> {
        self.sequence = reader.field("sequence").integer(0)? as u64;
        self.emitted.clear();
        self.presentation.restore_state(reader, reference)
    }

    fn sound(&mut self, emission: SoundEmission<'_>) {
        let resource = self
            .presentation
            .resource_by_path(emission.content, emission.path)
            .or_else(|| {
                self.presentation
                    .resource_by_path(emission.content, &format!("sound/{}", emission.path))
            });
        if let Some(resource) = resource {
            self.append(
                SimulationEventPayload::Sound {
                    resource: resource.id,
                    actor: emission.actor,
                    origin: emission.origin,
                    channel: emission.channel,
                    volume: emission.volume,
                    attenuation: emission.attenuation,
                },
                emission.recipient,
            );
        }
    }

    fn q1(&mut self, content: &ContentId, event: &Q1Event, sequence: u64, recipient: Option<ActorId>) {
        match event {
            Q1Event::Sound {
                origin,
                actor,
                path,
                channel,
                attenuation,
                volume,
                ..
            } => {
                let channel = match channel {
                    Q1SoundChannel::Auto => 0,
                    Q1SoundChannel::Weapon => 1,
                    Q1SoundChannel::Voice => 2,
                    Q1SoundChannel::Item => 3,
                    Q1SoundChannel::Body => 4,
                    Q1SoundChannel::Raw(channel) => *channel,
                };
                // SV_StartSound captures origin + 0.5 * (mins + maxs) before
                // the source edict can move or disappear.
                let center = match self.bodies.read(actor) {
                    None => vec3(0.0, 0.0, 0.0),
                    Some(body) => vec3(
                        (f64::from(body.origin.x) + f64::from(body.bounds.min.x + body.bounds.max.x) * 0.5) as f32,
                        (f64::from(body.origin.y) + f64::from(body.bounds.min.y + body.bounds.max.y) * 0.5) as f32,
                        (f64::from(body.origin.z) + f64::from(body.bounds.min.z + body.bounds.max.z) * 0.5) as f32,
                    ),
                };
                self.sound(SoundEmission {
                    content,
                    path,
                    actor: Some(actor.clone()),
                    origin: origin.unwrap_or(center),
                    channel,
                    volume: *volume,
                    attenuation: *attenuation,
                    recipient: recipient.as_ref(),
                });
            }
            Q1Event::Ambient {
                origin,
                path,
                volume,
                attenuation,
                ..
            } => {
                self.sound(SoundEmission {
                    content,
                    path,
                    actor: None,
                    origin: *origin,
                    channel: 0,
                    volume: *volume,
                    attenuation: *attenuation,
                    recipient: recipient.as_ref(),
                });
            }
            Q1Event::Message {
                player, text, center, ..
            } => {
                self.message(
                    if *center {
                        NetworkEvent::CenterPrint { text: text.clone() }
                    } else {
                        NetworkEvent::Print {
                            level: 2,
                            text: text.clone(),
                        }
                    },
                    Some(player),
                    Some(sequence),
                );
            }
            _ => {}
        }
    }

    fn q2(&mut self, content: &ContentId, event: &Q2PresentationEvent, sequence: u64) {
        match event {
            Q2PresentationEvent::Sound(event) => {
                self.sound(SoundEmission {
                    content,
                    path: &event.path,
                    actor: event.actor.clone(),
                    origin: event.origin,
                    channel: event.channel,
                    volume: event.volume,
                    attenuation: event.attenuation,
                    recipient: None,
                });
            }
            Q2PresentationEvent::CenterPrint { actor, text, .. } => {
                self.message(
                    NetworkEvent::CenterPrint { text: text.clone() },
                    Some(actor),
                    Some(sequence),
                );
            }
            Q2PresentationEvent::Help { text, .. } => {
                self.message(
                    NetworkEvent::Print {
                        level: 2,
                        text: text.clone(),
                    },
                    None,
                    Some(sequence),
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_content::contract::{
        ContentDigest, LooseMount, MountId, MountIdentity, MountPlanId, ResourceProvenance, ResourceResolution,
    };
    use qa_content::q2::foundation::host::Q2SoundEvent;
    use qa_content::q2::foundation::weapons::types::Q2WeaponEvent;
    use qa_core::identity::IdentityOwner;
    use qa_world::body::BodyState;
    use qa_world::save::value::{obj, str as save_str};

    use super::super::physics::fakes::{test_body, FakeActors, FakeBodies};
    use super::*;

    struct FakeSeam {
        now: SourceTime,
        slots: HashMap<ActorId, i32>,
        resources: HashMap<String, ResolvedResourceReference>,
        emitted: Vec<SimulationPresentationEvent>,
        sequence: u64,
        pending: bool,
    }

    impl FakeSeam {
        fn new() -> Self {
            Self {
                now: SourceTime::Milliseconds(500),
                slots: HashMap::new(),
                resources: HashMap::new(),
                emitted: Vec::new(),
                sequence: 0,
                pending: false,
            }
        }

        fn resource(id: &str, path: &str) -> ResolvedResourceReference {
            ResolvedResourceReference {
                id: ResourceId(format!("resource:{id}")),
                requested_path: path.to_string(),
                provenance: ResourceProvenance::Loose {
                    mount: LooseMount {
                        identity: MountIdentity {
                            id: MountId("mount:q1:id1".to_string()),
                            content: ContentId("q1:id1:e1m1:1".to_string()),
                            generation: 1,
                        },
                        root_path: "/tmp".to_string(),
                    },
                    member_path: path.to_string(),
                },
                digest: ContentDigest("sha256:00".to_string()),
                byte_length: 8,
                resolution: ResourceResolution::DefaultOrder {
                    plan: MountPlanId("mount-plan:q1:1".to_string()),
                    rank: 0,
                },
            }
        }
    }

    impl PresentationStateSeam for FakeSeam {
        fn now(&self) -> SourceTime {
            self.now
        }

        fn source_slot(&self, actor: &ActorId) -> Option<i32> {
            self.slots.get(actor).copied()
        }

        fn emit_owned(
            &mut self,
            owner: Option<PresentationOwner>,
            content: &ContentId,
            source: SourcePresentationEvent,
            time: SourceTime,
            recipient: Option<ActorId>,
        ) -> SimulationPresentationEvent {
            let _ = time;
            let sequence = self.sequence;
            self.sequence += 1;
            let event = SimulationPresentationEvent {
                event: source,
                owner,
                recipient,
                sequence,
                content: content.clone(),
                seconds: 0.5,
                source_entity: None,
            };
            self.emitted.push(event.clone());
            event
        }

        fn resource_by_path(&self, content: &ContentId, path: &str) -> Option<ResolvedResourceReference> {
            self.resources.get(&format!("{}/{}", content.as_str(), path)).cloned()
        }

        fn register_resource(
            &mut self,
            content: &ContentId,
            path: &str,
            resource: ResolvedResourceReference,
        ) -> Result<(), PresentationStateError> {
            if resource.requested_path != path {
                return Err(PresentationStateError::ResourcePathMismatch);
            }
            self.resources
                .insert(format!("{}/{}", content.as_str(), path), resource);
            Ok(())
        }

        fn light_styles(&self, _seconds: f64) -> Vec<super::super::types::SceneLightStyle> {
            Vec::new()
        }

        fn take_presentation(&mut self) -> Vec<SimulationPresentationEvent> {
            std::mem::take(&mut self.emitted)
        }

        fn assert_output_consumed(&self) -> Result<(), PresentationStateError> {
            if self.pending {
                return Err(PresentationStateError::OutputPending);
            }
            Ok(())
        }

        fn capture_state(&self) -> SaveJson {
            obj(vec![("history", save_str("kept"))])
        }

        fn restore_state(
            &mut self,
            _reader: &SaveReader,
            _reference: &dyn Fn(SavedActorId) -> ActorId,
        ) -> Result<(), WorldError> {
            self.emitted.clear();
            Ok(())
        }
    }

    struct Rig {
        events: SimulationEvents,
        states: Rc<RefCell<HashMap<ActorId, BodyState>>>,
    }

    fn make_rig(client: Option<ClientId>) -> (FakeActors, Rig) {
        let actors = FakeActors::new();
        let bodies = FakeBodies::new();
        let states = bodies.states.clone();
        let events = SimulationEvents::new(
            Box::new(bodies),
            Box::new(move |_| client.clone()),
            Box::new(FakeSeam::new()),
        );
        (actors, Rig { events, states })
    }

    fn content() -> ContentId {
        ContentId("q1:id1:e1m1:1".to_string())
    }

    #[test]
    fn append_routes_world_and_client_audiences() {
        let owner = IdentityOwner::create("events").expect("owner");
        let actor = owner.actor(1, 0);
        let (mut actors, mut rig) = make_rig(None);
        let minted = actors.mint();
        assert_eq!(minted.id().slot(), actor.slot());
        rig.events.append(
            SimulationEventPayload::Transition {
                decision: TransitionDecision::Stay { blocked: Vec::new() },
            },
            None,
        );
        // No client mapping drops actor-targeted payloads.
        rig.events.append(
            SimulationEventPayload::Transition {
                decision: TransitionDecision::Stay { blocked: Vec::new() },
            },
            Some(minted.id()),
        );
        let taken = rig.events.take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].audience, EventAudience::World);
        assert_eq!(taken[0].sequence, 0);
    }

    #[test]
    fn q1_sound_captures_body_center() {
        let owner = IdentityOwner::create("events").expect("owner");
        let actor = owner.actor(1, 0);
        let (mut actors, mut rig) = make_rig(None);
        let minted = actors.mint();
        let mut body = test_body();
        body.origin = vec3(10.0, 0.0, 0.0);
        rig.states.borrow_mut().insert(minted.id().clone(), body);
        let content = content();
        rig.events.presentation = Box::new({
            let mut seam = FakeSeam::new();
            seam.resources.insert(
                format!("{}/weapons/shotgun.wav", content.as_str()),
                FakeSeam::resource("shotgun", "weapons/shotgun.wav"),
            );
            seam
        });
        let presentation = rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q1(Q1Event::Sound {
                origin: None,
                actor: minted.id().clone(),
                path: "weapons/shotgun.wav".to_string(),
                channel: Q1SoundChannel::Weapon,
                attenuation: 1.0,
                volume: 1.0,
            }),
            SourceTime::Milliseconds(500),
            None,
        );
        assert_eq!(presentation.sequence, 0);
        let taken = rig.events.take();
        assert_eq!(taken.len(), 1);
        match &taken[0].payload {
            SimulationEventPayload::Sound {
                origin,
                channel,
                actor: sound_actor,
                ..
            } => {
                // Donor SV_StartSound: origin + 0.5 * (mins + maxs); z = 0 + 0.5 * 8.
                assert_eq!(*origin, vec3(10.0, 0.0, 4.0));
                assert_eq!(*channel, 1);
                assert_eq!(sound_actor.as_ref(), Some(minted.id()));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        assert_eq!(actor.slot(), minted.id().slot());
    }

    #[test]
    fn q1_message_selects_center_print() {
        let (mut actors, mut rig) = make_rig(None);
        let minted = actors.mint();
        let content = content();
        rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q1(Q1Event::Message {
                player: minted.id().clone(),
                text: "hello".to_string(),
                center: true,
                args: None,
                parts: None,
            }),
            SourceTime::Milliseconds(500),
            None,
        );
        // Actor-targeted messages drop without a client mapping.
        assert!(rig.events.take().is_empty());
        let client_owner = IdentityOwner::create("events-client").expect("owner");
        let (mut actors, mut rig) = make_rig(Some(client_owner.client(1, 0)));
        let minted = actors.mint();
        rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q1(Q1Event::Message {
                player: minted.id().clone(),
                text: "hello".to_string(),
                center: false,
                args: None,
                parts: None,
            }),
            SourceTime::Milliseconds(500),
            None,
        );
        let taken = rig.events.take();
        assert_eq!(taken.len(), 1);
        match &taken[0].payload {
            SimulationEventPayload::Message {
                event,
                source_presentation_sequence,
            } => {
                assert!(matches!(event, NetworkEvent::Print { level: 2, .. }));
                assert_eq!(*source_presentation_sequence, Some(0));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn q2_routes_sound_centerprint_help() {
        let client_owner = IdentityOwner::create("events-client").expect("owner");
        let (mut actors, mut rig) = make_rig(Some(client_owner.client(9, 0)));
        let minted = actors.mint();
        let content = ContentId("q2:re:base:1".to_string());
        rig.events.presentation = Box::new({
            let mut seam = FakeSeam::new();
            seam.resources.insert(
                format!("{}/sound/weapons/blaster.wav", content.as_str()),
                FakeSeam::resource("blaster", "sound/weapons/blaster.wav"),
            );
            seam
        });
        rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q2(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(minted.id().clone()),
                origin: vec3(1.0, 2.0, 3.0),
                path: "weapons/blaster.wav".to_string(),
                channel: 1,
                volume: 0.8,
                attenuation: 1.0,
                reliable: false,
                loop_: qa_content::q2::foundation::host::Q2SoundLoop::Once,
                loop_owner: None,
            })),
            SourceTime::Milliseconds(500),
            None,
        );
        rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q2(Q2PresentationEvent::CenterPrint {
                actor: minted.id().clone(),
                text: "cp".to_string(),
                instant: false,
                duration_seconds: None,
            }),
            SourceTime::Milliseconds(500),
            None,
        );
        rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q2(Q2PresentationEvent::Help {
                slot: 1,
                text: "help".to_string(),
            }),
            SourceTime::Milliseconds(500),
            None,
        );
        let taken = rig.events.take();
        assert_eq!(taken.len(), 3);
        assert!(matches!(taken[0].payload, SimulationEventPayload::Sound { .. }));
        assert!(matches!(
            taken[1].payload,
            SimulationEventPayload::Message {
                event: NetworkEvent::CenterPrint { .. },
                ..
            }
        ));
        assert!(matches!(
            taken[2].payload,
            SimulationEventPayload::Message {
                event: NetworkEvent::Print { level: 2, .. },
                ..
            }
        ));
    }

    #[test]
    fn muzzleflash_needs_a_source_slot() {
        let (mut actors, mut rig) = make_rig(None);
        let minted = actors.mint();
        let content = content();
        rig.events.emit_owned(
            None,
            &content,
            SourcePresentationEvent::Q2Weapon(Q2WeaponEvent::Muzzleflash {
                actor: minted.id().clone(),
                flash: 3,
                silenced: false,
            }),
            SourceTime::Milliseconds(500),
            None,
        );
        assert!(rig.events.take().is_empty());
    }

    #[test]
    fn capture_restore_round_trips_sequence() {
        let (_, mut rig) = make_rig(None);
        rig.events.message(
            NetworkEvent::Disconnect {
                reason: "bye".to_string(),
            },
            None,
            None,
        );
        assert_eq!(rig.events.next_sequence(), 1);
        let json = rig.events.capture().expect("capture");
        let taken = rig.events.take();
        assert_eq!(taken.len(), 1);
        rig.events.message(
            NetworkEvent::Disconnect {
                reason: "again".to_string(),
            },
            None,
            None,
        );
        let owner = IdentityOwner::create("events").expect("owner");
        rig.events
            .restore(&SaveReader::new(&json), &|saved| {
                owner.actor(saved.slot, saved.generation)
            })
            .expect("restore");
        assert_eq!(rig.events.next_sequence(), 1);
        assert!(rig.events.take().is_empty());
        rig.events.assert_output_consumed().expect("consumed");
    }

    #[test]
    fn pending_output_blocks_saves() {
        let (_, mut rig) = make_rig(None);
        rig.events.message(
            NetworkEvent::Disconnect {
                reason: "bye".to_string(),
            },
            None,
            None,
        );
        assert!(matches!(
            rig.events.assert_output_consumed(),
            Err(EventsError::OutputPending)
        ));
    }
}
