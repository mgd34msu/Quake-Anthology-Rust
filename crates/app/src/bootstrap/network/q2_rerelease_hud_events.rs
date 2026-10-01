//! Quake II rerelease HUD record translation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-rerelease-hud-events.ts`
//! (`translateQ2RereleaseHudRecord`). Points of interest and directional
//! damage indicators become `q2-rerelease` presentation events; every other
//! record is left for the remaining service translators.

use qa_core::math::Vec3;
use qa_net::q2_net::{Q2ServerEvent, Q2ServerRecord};

use super::q2_service_presentation::{
    Q2ServiceEmittedEvent, Q2ServiceError, Q2ServiceEvent, Q2ServiceFamily, Q2ServicePresentationHost,
};

/// Translate one rerelease HUD record (`translateQ2RereleaseHudRecord`).
///
/// Returns whether the record was a HUD record (consumed or not, HUD
/// records never reach the remaining translators).
pub fn translate_q2_rerelease_hud_record(
    record: &Q2ServerRecord,
    host: &mut dyn Q2ServicePresentationHost,
) -> Result<bool, Q2ServiceError> {
    if !matches!(record.event, Q2ServerEvent::Poi { .. } | Q2ServerEvent::Damage { .. }) {
        return Ok(false);
    }
    let Some(player) = host.player() else {
        return Ok(false);
    };
    let sequence = host.next_sequence();
    let content = host.content();
    let seconds = host.seconds();
    match &record.event {
        Q2ServerEvent::Poi { value } => {
            if value.time == 65535 {
                host.emit(Q2ServiceEmittedEvent {
                    family: Q2ServiceFamily::Q2Rerelease,
                    sequence,
                    content,
                    seconds,
                    source_entity: None,
                    event: Q2ServiceEvent::RemovePoi {
                        actor: player.actor,
                        key: value.key,
                    },
                });
            } else {
                let Some(image) = host.config_string(host.image_config_offset() + u32::from(value.image)) else {
                    return Err(Q2ServiceError::Message(format!(
                        "Q2 POI image {} has no configstring",
                        value.image
                    )));
                };
                host.emit(Q2ServiceEmittedEvent {
                    family: Q2ServiceFamily::Q2Rerelease,
                    sequence,
                    content,
                    seconds,
                    source_entity: None,
                    event: Q2ServiceEvent::KeyedPoi {
                        actor: player.actor,
                        key: value.key,
                        duration: value.time,
                        position: Vec3 {
                            x: value.pos[0],
                            y: value.pos[1],
                            z: value.pos[2],
                        },
                        image,
                        color: value.color,
                        flags: value.flags,
                    },
                });
            }
        }
        Q2ServerEvent::Damage { indicators } => {
            for value in indicators {
                let sequence = host.next_sequence();
                let content = host.content();
                host.emit(Q2ServiceEmittedEvent {
                    family: Q2ServiceFamily::Q2Rerelease,
                    sequence,
                    content,
                    seconds,
                    source_entity: None,
                    event: Q2ServiceEvent::DirectionalDamage {
                        actor: player.actor.clone(),
                        damage: value.damage,
                        health: value.health,
                        armor: value.armor,
                        shield: value.shield,
                        direction: Vec3 {
                            x: value.direction[0] as f32,
                            y: value.direction[1] as f32,
                            z: value.direction[2] as f32,
                        },
                    },
                });
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use qa_content::contract::ContentId;
    use qa_content::q2::rerelease::types::Q2FogState;
    use qa_core::identity::{ActorId, IdentityOwner};
    use qa_net::q2_variants::{FogData, KexDamageIndicator, KexPoi};

    use super::super::q2_service_presentation::Q2ServicePlayer;

    struct HudHost {
        emitted: Vec<Q2ServiceEmittedEvent>,
        strings: HashMap<u32, String>,
        sequence: u64,
        actor: ActorId,
    }

    impl HudHost {
        fn new() -> Self {
            Self {
                emitted: Vec::new(),
                strings: HashMap::new(),
                sequence: 0,
                actor: IdentityOwner::create("q2-service-test").expect("owner").actor(2, 0),
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

    impl Q2ServicePresentationHost for HudHost {
        fn content(&self) -> ContentId {
            ContentId("q2:test:baseq2:0".to_string())
        }

        fn seconds(&self) -> f64 {
            2.0
        }

        fn next_sequence(&mut self) -> u64 {
            self.sequence += 1;
            self.sequence
        }

        fn player(&self) -> Option<Q2ServicePlayer> {
            Some(Q2ServicePlayer {
                actor: self.actor.clone(),
                source_entity: 1,
            })
        }

        fn actor(&self, _source_entity: i32) -> Option<ActorId> {
            Some(self.actor.clone())
        }

        fn entity(&self, _source_entity: i32) -> Option<super::super::q2_service_presentation::Q2ServiceEntity> {
            None
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

        fn set_inventory(&mut self, _counts: &[i16]) {}

        fn set_layout(&mut self, _text: &str) {}

        fn emit(&mut self, event: Q2ServiceEmittedEvent) {
            self.emitted.push(event);
        }
    }

    #[test]
    fn non_hud_records_pass_through() {
        let mut host = HudHost::new();
        let record = HudHost::record(Q2ServerEvent::Nop);
        assert!(!translate_q2_rerelease_hud_record(&record, &mut host).expect("pass"));
        assert!(host.emitted.is_empty());
    }

    #[test]
    fn expired_poi_emits_removal() {
        let mut host = HudHost::new();
        let record = HudHost::record(Q2ServerEvent::Poi {
            value: KexPoi {
                key: 9,
                time: 65535,
                pos: [0.0, 0.0, 0.0],
                image: 0,
                color: 0,
                flags: 0,
            },
        });
        assert!(translate_q2_rerelease_hud_record(&record, &mut host).expect("poi"));
        assert!(matches!(
            host.emitted[0].event,
            Q2ServiceEvent::RemovePoi { key: 9, .. }
        ));
    }

    #[test]
    fn live_poi_resolves_its_image() {
        let mut host = HudHost::new();
        host.strings.insert(204, "pics/poi.pcx".to_string());
        let record = HudHost::record(Q2ServerEvent::Poi {
            value: KexPoi {
                key: 3,
                time: 30,
                pos: [1.0, 2.0, 3.0],
                image: 4,
                color: 7,
                flags: 1,
            },
        });
        assert!(translate_q2_rerelease_hud_record(&record, &mut host).expect("poi"));
        assert!(matches!(
            host.emitted[0].event,
            Q2ServiceEvent::KeyedPoi { key: 3, duration: 30, ref image, color: 7, flags: 1, .. }
            if image == "pics/poi.pcx"
        ));
    }

    #[test]
    fn poi_without_image_fails_with_donor_text() {
        let mut host = HudHost::new();
        let record = HudHost::record(Q2ServerEvent::Poi {
            value: KexPoi {
                key: 3,
                time: 30,
                pos: [0.0, 0.0, 0.0],
                image: 4,
                color: 0,
                flags: 0,
            },
        });
        let error = translate_q2_rerelease_hud_record(&record, &mut host).expect_err("missing image");
        assert_eq!(
            error,
            Q2ServiceError::Message("Q2 POI image 4 has no configstring".to_string())
        );
    }

    #[test]
    fn damage_emits_one_event_per_indicator_with_fresh_sequences() {
        let mut host = HudHost::new();
        let record = HudHost::record(Q2ServerEvent::Damage {
            indicators: vec![
                KexDamageIndicator {
                    damage: 10,
                    health: true,
                    armor: false,
                    shield: false,
                    direction: [1.0, 0.0, 0.0],
                },
                KexDamageIndicator {
                    damage: 5,
                    health: false,
                    armor: true,
                    shield: true,
                    direction: [0.0, 1.0, 0.0],
                },
            ],
        });
        assert!(translate_q2_rerelease_hud_record(&record, &mut host).expect("damage"));
        assert_eq!(host.emitted.len(), 2);
        assert_ne!(host.emitted[0].sequence, host.emitted[1].sequence);
        assert!(matches!(
            host.emitted[1].event,
            Q2ServiceEvent::DirectionalDamage {
                damage: 5,
                armor: true,
                shield: true,
                ..
            }
        ));
    }
}
