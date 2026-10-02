//! Quake II service-record presentation translation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-service-presentation.ts`
//! (`Q2ServicePresentationHost`, `q2ServicePlayerInfo`,
//! `translateQ2ServiceRecords`, `q2ServicePrint`). Presentation effects come
//! from already decoded service records; command/control handling stays with
//! the receiver. The donor emits `SimulationPresentationEvent`, whose
//! canonical port belongs to the simulation lane (see the
//! [`NetworkPresentationEvent`](super::types::NetworkPresentationEvent)
//! precedent); this module mirrors exactly the `q2`, `q2-rerelease`,
//! `q2-player`, and `q2-weapon` envelopes it emits so the merge-time remote
//! owner can forward them unchanged.

use qa_content::contract::ContentId;
use qa_content::q2::foundation::host::{Q2Edition, Q2EffectEvent};
use qa_content::q2::foundation::monsters::ai::angles_vectors;
use qa_content::q2::foundation::monsters::muzzle::muzzle_offset;
use qa_content::q2::foundation::weapons::types::Q2WeaponEvent;
use qa_content::q2::rerelease::types::Q2FogState;
use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_net::q2_net::{Q2ServerEvent, Q2ServerRecord};
use qa_net::q2_variants::{fog_bits, FogData};
use thiserror::Error;

use super::q2_effects::{q2_beam_from_wire, q2_effect_from_wire};
use super::q2_rerelease_hud_events::translate_q2_rerelease_hud_record;

/// Service presentation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2ServiceError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
}

/// Presentation envelope family (`SimulationPresentationEvent['kind']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ServiceFamily {
    /// Base game event (`q2`).
    Q2,
    /// Rerelease event (`q2-rerelease`).
    Q2Rerelease,
    /// Player event (`q2-player`).
    Q2Player,
    /// Weapon event (`q2-weapon`).
    Q2Weapon,
}

/// Player print level (`q2ServicePrint` mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ServicePrintLevel {
    /// Chat line (level 3).
    Chat,
    /// High print (level 2).
    High,
    /// Medium print (level 1).
    Medium,
    /// Low print (anything else).
    Low,
}

/// Service presentation payload (donor inline `event` objects).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2ServiceEvent {
    /// Player userinfo (`userinfo`).
    Userinfo {
        /// Player actor.
        actor: ActorId,
        /// Player slot.
        slot: u32,
        /// Player name.
        name: String,
        /// Player skin.
        skin: String,
    },
    /// Base effect (`q2EffectFromWire`).
    Effect(Q2EffectEvent),
    /// Weapon beam (`q2BeamFromWire`).
    WeaponBeam(Q2WeaponEvent),
    /// World sound (`sound`).
    Sound {
        /// Sound actor, if any.
        actor: Option<ActorId>,
        /// Sound origin.
        origin: Vec3,
        /// Sound path.
        path: String,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
        /// Channel.
        channel: u8,
    },
    /// Monster muzzle flash (`monster-muzzleflash`).
    MonsterMuzzleflash {
        /// Flash actor.
        actor: ActorId,
        /// Flash number.
        flash: i32,
        /// Flash origin.
        origin: Vec3,
        /// Flash direction.
        direction: Vec3,
    },
    /// Player muzzle flash (`muzzleflash`).
    Muzzleflash {
        /// Flash actor.
        actor: ActorId,
        /// Flash number.
        flash: i32,
        /// Silenced flag.
        silenced: bool,
    },
    /// Center print (`centerprint`).
    Centerprint {
        /// Player actor.
        actor: ActorId,
        /// Text.
        text: String,
        /// Instant flag (level 4 prints).
        instant: bool,
    },
    /// Player print (`print`).
    Print {
        /// Print target.
        target: ActorId,
        /// Print level.
        level: Q2ServicePrintLevel,
        /// Text.
        text: String,
    },
    /// Rerelease fog (`fog`).
    Fog {
        /// Player actor.
        actor: ActorId,
        /// Fog state.
        value: Q2FogState,
        /// Transition time in milliseconds.
        transition_milliseconds: u16,
    },
    /// Rerelease achievement (`achievement`).
    Achievement {
        /// Achievement id.
        id: String,
    },
    /// Rerelease help path (`help-path`).
    HelpPath {
        /// Player actor.
        actor: ActorId,
        /// Path start flag.
        first: bool,
        /// Node position.
        position: Vec3,
        /// Node direction.
        direction: Vec3,
    },
    /// Removed point of interest (`remove-poi`).
    RemovePoi {
        /// Player actor.
        actor: ActorId,
        /// POI key.
        key: u16,
    },
    /// Keyed point of interest (`keyed-poi`).
    KeyedPoi {
        /// Player actor.
        actor: ActorId,
        /// POI key.
        key: u16,
        /// Duration.
        duration: u16,
        /// Position.
        position: Vec3,
        /// Image name.
        image: String,
        /// Color.
        color: u8,
        /// Flags.
        flags: u8,
    },
    /// Directional damage indicator (`directional-damage`).
    DirectionalDamage {
        /// Player actor.
        actor: ActorId,
        /// Damage amount.
        damage: u8,
        /// Health damage flag.
        health: bool,
        /// Armor damage flag.
        armor: bool,
        /// Shield damage flag.
        shield: bool,
        /// Attack direction.
        direction: Vec3,
    },
}

/// Emitted presentation event (donor `SimulationPresentationEvent` mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ServiceEmittedEvent {
    /// Envelope family.
    pub family: Q2ServiceFamily,
    /// Presentation sequence.
    pub sequence: u64,
    /// Content identity.
    pub content: ContentId,
    /// Presentation time in seconds.
    pub seconds: f64,
    /// Source entity, when the event names one.
    pub source_entity: Option<i32>,
    /// Payload.
    pub event: Q2ServiceEvent,
}

/// Local player binding (`Q2ServicePresentationHost['player']`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ServicePlayer {
    /// Player actor.
    pub actor: ActorId,
    /// Source entity number.
    pub source_entity: i32,
}

/// Entity transform (`Q2ServicePresentationHost['entity']`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ServiceEntity {
    /// World origin.
    pub origin: Vec3,
    /// World angles.
    pub angles: Vec3,
}

/// Service presentation host (`Q2ServicePresentationHost`).
pub trait Q2ServicePresentationHost {
    /// Game edition (classic when the host reports none).
    fn edition(&self) -> Q2Edition {
        Q2Edition::Classic
    }
    /// Content identity.
    fn content(&self) -> ContentId;
    /// Presentation time in seconds.
    fn seconds(&self) -> f64;
    /// Next presentation sequence number.
    fn next_sequence(&mut self) -> u64;
    /// Local player binding, if any.
    fn player(&self) -> Option<Q2ServicePlayer>;
    /// Actor for a source entity, if any.
    fn actor(&self, source_entity: i32) -> Option<ActorId>;
    /// Transform for a source entity, if any.
    fn entity(&self, source_entity: i32) -> Option<Q2ServiceEntity>;
    /// First sound configstring.
    fn sound_config_offset(&self) -> u32;
    /// First image configstring.
    fn image_config_offset(&self) -> u32;
    /// Fog state for wire fog data.
    fn fog(&mut self, value: &FogData) -> Q2FogState;
    /// First player-skin configstring.
    fn player_skin_config_offset(&self) -> u32;
    /// Configstring value, if set.
    fn config_string(&self, index: u32) -> Option<String>;
    /// Set a configstring value.
    fn set_config_string(&mut self, index: u32, value: &str);
    /// Set inventory counts.
    fn set_inventory(&mut self, counts: &[i16]);
    /// Set layout text.
    fn set_layout(&mut self, text: &str);
    /// Emit a presentation event.
    fn emit(&mut self, event: Q2ServiceEmittedEvent);
}

/// Translate a player-skin configstring into a userinfo event
/// (`q2ServicePlayerInfo`).
pub fn q2_service_player_info(host: &mut dyn Q2ServicePresentationHost, index: u32, value: &str) -> bool {
    let offset = host.player_skin_config_offset();
    if index < offset || index >= offset + 256 {
        return false;
    }
    let slot = index - offset;
    let Some(actor) = host.actor(slot as i32 + 1) else {
        return false;
    };
    let (name, skin) = match value.find('\\') {
        Some(split) => (value[..split].to_string(), value[split + 1..].to_string()),
        None => (value.to_string(), String::new()),
    };
    let sequence = host.next_sequence();
    let content = host.content();
    let seconds = host.seconds();
    host.emit(Q2ServiceEmittedEvent {
        family: Q2ServiceFamily::Q2Player,
        sequence,
        content,
        seconds,
        source_entity: Some(slot as i32 + 1),
        event: Q2ServiceEvent::Userinfo {
            actor,
            slot,
            name,
            skin,
        },
    });
    true
}

/// Translate service records into presentation events
/// (`translateQ2ServiceRecords`).
///
/// Returns the records no translator consumed.
pub fn translate_q2_service_records(
    records: &[Q2ServerRecord],
    host: &mut dyn Q2ServicePresentationHost,
) -> Result<Vec<Q2ServerRecord>, Q2ServiceError> {
    let mut remaining = Vec::new();
    for record in records {
        if translate_q2_rerelease_hud_record(record, host)? {
            continue;
        }
        match &record.event {
            Q2ServerEvent::ConfigString { index, value } => {
                let index_u32 = u32::from(*index);
                host.set_config_string(index_u32, value);
                if host.player().is_some() {
                    q2_service_player_info(host, index_u32, value);
                }
            }
            Q2ServerEvent::Print { level, text } => {
                if !q2_service_print(host, *level, text) {
                    remaining.push(record.clone());
                }
            }
            Q2ServerEvent::Inventory { counts } => host.set_inventory(counts),
            Q2ServerEvent::Layout { text } => host.set_layout(text),
            Q2ServerEvent::TempEntity { value } => {
                if let Some(decoded) = q2_effect_from_wire(value) {
                    emit_service(host, Q2ServiceFamily::Q2, None, Q2ServiceEvent::Effect(decoded));
                } else if let Some(beam) =
                    q2_beam_from_wire(value).map_err(|error| Q2ServiceError::Message(error.to_string()))?
                {
                    emit_service(host, Q2ServiceFamily::Q2Weapon, None, Q2ServiceEvent::WeaponBeam(beam));
                } else {
                    remaining.push(record.clone());
                }
            }
            Q2ServerEvent::Sound { sound } => {
                let Some(path) = host.config_string(host.sound_config_offset() + u32::from(sound.index)) else {
                    return Err(Q2ServiceError::Message(format!(
                        "Q2 sound {} has no configstring",
                        sound.index
                    )));
                };
                let entity = i32::try_from(sound.entity).unwrap_or(i32::MAX);
                let placed = host.entity(entity);
                let origin = sound
                    .position
                    .map(|point| Vec3 {
                        x: point[0] as f32,
                        y: point[1] as f32,
                        z: point[2] as f32,
                    })
                    .or_else(|| placed.map(|entry| entry.origin))
                    .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
                let actor = if sound.entity == 0 { None } else { host.actor(entity) };
                emit_service(
                    host,
                    Q2ServiceFamily::Q2,
                    Some(entity),
                    Q2ServiceEvent::Sound {
                        actor,
                        origin,
                        path,
                        volume: sound.volume,
                        attenuation: sound.attenuation,
                        channel: sound.channel,
                    },
                );
            }
            Q2ServerEvent::MuzzleFlash {
                entity,
                flash,
                monster,
                silenced,
            } => {
                if *monster {
                    let (Some(entry), Some(actor)) = (host.entity(*entity), host.actor(*entity)) else {
                        remaining.push(record.clone());
                        continue;
                    };
                    let axes = angles_vectors(entry.angles);
                    let offset = muzzle_offset(host.edition(), usize::try_from(*flash).unwrap_or(usize::MAX));
                    let origin = entry.origin;
                    emit_service(
                        host,
                        Q2ServiceFamily::Q2,
                        Some(*entity),
                        Q2ServiceEvent::MonsterMuzzleflash {
                            actor,
                            flash: *flash,
                            origin: Vec3 {
                                x: origin.x + axes.forward.x * offset.x + axes.right.x * offset.y,
                                y: origin.y + axes.forward.y * offset.x + axes.right.y * offset.y,
                                z: origin.z + axes.forward.z * offset.x + axes.right.z * offset.y + offset.z,
                            },
                            direction: axes.forward,
                        },
                    );
                } else {
                    let Some(actor) = host.actor(*entity) else {
                        remaining.push(record.clone());
                        continue;
                    };
                    emit_service(
                        host,
                        Q2ServiceFamily::Q2Weapon,
                        Some(*entity),
                        Q2ServiceEvent::Muzzleflash {
                            actor,
                            flash: *flash,
                            silenced: *silenced,
                        },
                    );
                }
            }
            Q2ServerEvent::CenterPrint { text } => {
                let Some(player) = host.player() else {
                    remaining.push(record.clone());
                    continue;
                };
                emit_service(
                    host,
                    Q2ServiceFamily::Q2,
                    Some(player.source_entity),
                    Q2ServiceEvent::Centerprint {
                        actor: player.actor,
                        text: text.clone(),
                        instant: false,
                    },
                );
            }
            Q2ServerEvent::Fog { value } => {
                let Some(player) = host.player() else {
                    remaining.push(record.clone());
                    continue;
                };
                let transition = if value.bits & fog_bits::TIME == 0 {
                    0
                } else {
                    value.time
                };
                let fog = host.fog(value);
                emit_service(
                    host,
                    Q2ServiceFamily::Q2Rerelease,
                    Some(player.source_entity),
                    Q2ServiceEvent::Fog {
                        actor: player.actor,
                        value: fog,
                        transition_milliseconds: transition,
                    },
                );
            }
            Q2ServerEvent::Achievement { text } => {
                emit_service(
                    host,
                    Q2ServiceFamily::Q2Rerelease,
                    None,
                    Q2ServiceEvent::Achievement { id: text.clone() },
                );
            }
            Q2ServerEvent::HelpPath { value } => {
                let Some(player) = host.player() else {
                    remaining.push(record.clone());
                    continue;
                };
                emit_service(
                    host,
                    Q2ServiceFamily::Q2Rerelease,
                    Some(player.source_entity),
                    Q2ServiceEvent::HelpPath {
                        actor: player.actor,
                        first: value.start,
                        position: Vec3 {
                            x: value.pos[0],
                            y: value.pos[1],
                            z: value.pos[2],
                        },
                        direction: Vec3 {
                            x: value.dir[0] as f32,
                            y: value.dir[1] as f32,
                            z: value.dir[2] as f32,
                        },
                    },
                );
            }
            _ => remaining.push(record.clone()),
        }
    }
    Ok(remaining)
}

/// Emit one service event with a fresh envelope stamp.
fn emit_service(
    host: &mut dyn Q2ServicePresentationHost,
    family: Q2ServiceFamily,
    source_entity: Option<i32>,
    event: Q2ServiceEvent,
) {
    let sequence = host.next_sequence();
    let content = host.content();
    let seconds = host.seconds();
    host.emit(Q2ServiceEmittedEvent {
        family,
        sequence,
        content,
        seconds,
        source_entity,
        event,
    });
}

/// Translate a print record (`q2ServicePrint`).
pub fn q2_service_print(host: &mut dyn Q2ServicePresentationHost, level: u8, text: &str) -> bool {
    let Some(player) = host.player() else {
        return false;
    };
    if level == 4 || level == 5 {
        emit_service(
            host,
            Q2ServiceFamily::Q2,
            Some(player.source_entity),
            Q2ServiceEvent::Centerprint {
                actor: player.actor,
                text: text.to_string(),
                instant: level == 4,
            },
        );
    } else {
        let mapped = match level {
            3 => Q2ServicePrintLevel::Chat,
            2 => Q2ServicePrintLevel::High,
            1 => Q2ServicePrintLevel::Medium,
            _ => Q2ServicePrintLevel::Low,
        };
        emit_service(
            host,
            Q2ServiceFamily::Q2Player,
            Some(player.source_entity),
            Q2ServiceEvent::Print {
                target: player.actor,
                level: mapped,
                text: text.to_string(),
            },
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use qa_core::identity::IdentityOwner;

    pub(crate) struct ServiceHost {
        pub(crate) emitted: Vec<Q2ServiceEmittedEvent>,
        pub(crate) strings: HashMap<u32, String>,
        pub(crate) inventory: Vec<i16>,
        pub(crate) layout: String,
        pub(crate) sequence: u64,
        pub(crate) actor: ActorId,
    }

    impl ServiceHost {
        pub(crate) fn new() -> Self {
            Self {
                emitted: Vec::new(),
                strings: HashMap::new(),
                inventory: Vec::new(),
                layout: String::new(),
                sequence: 0,
                actor: IdentityOwner::create("q2-service-test").expect("owner").actor(1, 0),
            }
        }

        fn record(event: Q2ServerEvent) -> Q2ServerRecord {
            Q2ServerRecord {
                seat: 0,
                opcode: 0,
                raw: Vec::new(),
                event,
            }
        }
    }

    impl Q2ServicePresentationHost for ServiceHost {
        fn content(&self) -> ContentId {
            ContentId("q2:test:baseq3:0".to_string())
        }

        fn seconds(&self) -> f64 {
            1.5
        }

        fn next_sequence(&mut self) -> u64 {
            self.sequence += 1;
            self.sequence
        }

        fn player(&self) -> Option<Q2ServicePlayer> {
            Some(Q2ServicePlayer {
                actor: self.actor.clone(),
                source_entity: 3,
            })
        }

        fn actor(&self, _source_entity: i32) -> Option<ActorId> {
            Some(self.actor.clone())
        }

        fn entity(&self, _source_entity: i32) -> Option<Q2ServiceEntity> {
            Some(Q2ServiceEntity {
                origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                angles: Vec3 {
                    x: 0.0,
                    y: 90.0,
                    z: 0.0,
                },
            })
        }

        fn sound_config_offset(&self) -> u32 {
            100
        }

        fn image_config_offset(&self) -> u32 {
            200
        }

        fn fog(&mut self, _value: &FogData) -> Q2FogState {
            Q2FogState::default()
        }

        fn player_skin_config_offset(&self) -> u32 {
            300
        }

        fn config_string(&self, index: u32) -> Option<String> {
            self.strings.get(&index).cloned()
        }

        fn set_config_string(&mut self, index: u32, value: &str) {
            self.strings.insert(index, value.to_string());
        }

        fn set_inventory(&mut self, counts: &[i16]) {
            self.inventory = counts.to_vec();
        }

        fn set_layout(&mut self, text: &str) {
            self.layout = text.to_string();
        }

        fn emit(&mut self, event: Q2ServiceEmittedEvent) {
            self.emitted.push(event);
        }
    }

    #[test]
    fn player_info_splits_name_and_skin() {
        let mut host = ServiceHost::new();
        assert!(q2_service_player_info(&mut host, 301, "doe\\male/grunt"));
        let event = host.emitted.pop().expect("emitted");
        assert_eq!(event.family, Q2ServiceFamily::Q2Player);
        assert_eq!(event.source_entity, Some(2));
        assert!(matches!(
            event.event,
            Q2ServiceEvent::Userinfo { slot: 1, ref name, ref skin, .. }
            if name == "doe" && skin == "male/grunt"
        ));
        assert!(!q2_service_player_info(&mut host, 99, "doe"));
    }

    #[test]
    fn print_levels_map_to_centerprint_and_player_prints() {
        let mut host = ServiceHost::new();
        assert!(q2_service_print(&mut host, 4, "instant"));
        assert!(q2_service_print(&mut host, 3, "chat"));
        assert!(q2_service_print(&mut host, 9, "low"));
        assert!(matches!(
            host.emitted[0].event,
            Q2ServiceEvent::Centerprint { instant: true, .. }
        ));
        assert!(matches!(
            host.emitted[1].event,
            Q2ServiceEvent::Print {
                level: Q2ServicePrintLevel::Chat,
                ..
            }
        ));
        assert!(matches!(
            host.emitted[2].event,
            Q2ServiceEvent::Print {
                level: Q2ServicePrintLevel::Low,
                ..
            }
        ));
    }

    #[test]
    fn service_records_store_state_and_consume_translated_events() {
        let mut host = ServiceHost::new();
        host.strings.insert(103, "weapons/blast.wav".to_string());
        let records = vec![
            ServiceHost::record(Q2ServerEvent::ConfigString {
                index: 103,
                value: "x".to_string(),
            }),
            ServiceHost::record(Q2ServerEvent::Inventory { counts: vec![1, 2] }),
            ServiceHost::record(Q2ServerEvent::Layout {
                text: "hud".to_string(),
            }),
            ServiceHost::record(Q2ServerEvent::CenterPrint { text: "go".to_string() }),
            ServiceHost::record(Q2ServerEvent::Achievement {
                text: "ace".to_string(),
            }),
            ServiceHost::record(Q2ServerEvent::Nop),
        ];
        let remaining = translate_q2_service_records(&records, &mut host).expect("translate");
        assert_eq!(host.strings.get(&103).map(String::as_str), Some("x"));
        assert_eq!(host.inventory, vec![1, 2]);
        assert_eq!(host.layout, "hud");
        assert_eq!(remaining.len(), 1);
        assert!(matches!(remaining[0].event, Q2ServerEvent::Nop));
        assert!(host
            .emitted
            .iter()
            .any(|event| matches!(event.event, Q2ServiceEvent::Achievement { ref id } if id == "ace")));
    }

    #[test]
    fn sound_without_configstring_fails_with_donor_text() {
        let mut host = ServiceHost::new();
        let records = vec![ServiceHost::record(Q2ServerEvent::Sound {
            sound: qa_net::q2_net::Q2SoundMessage {
                flags: 0,
                index: 7,
                entity: 1,
                channel: 0,
                position: None,
                volume: 1.0,
                attenuation: 1.0,
                delay_seconds: 0.0,
            },
        })];
        let error = translate_q2_service_records(&records, &mut host).expect_err("missing sound");
        assert_eq!(
            error,
            Q2ServiceError::Message("Q2 sound 7 has no configstring".to_string())
        );
    }

    #[test]
    fn monster_muzzleflash_offsets_from_entity_axes() {
        let mut host = ServiceHost::new();
        let records = vec![ServiceHost::record(Q2ServerEvent::MuzzleFlash {
            entity: 2,
            flash: 1,
            monster: true,
            silenced: false,
        })];
        let remaining = translate_q2_service_records(&records, &mut host).expect("translate");
        assert!(remaining.is_empty());
        assert!(matches!(
            host.emitted[0].event,
            Q2ServiceEvent::MonsterMuzzleflash { flash: 1, .. }
        ));
    }
}
