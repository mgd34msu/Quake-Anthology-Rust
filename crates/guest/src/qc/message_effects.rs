//! Temporary-entity wire effects projected into presentation events.
//!
//! Ported from donor `src/compat/qc/message-effects.ts`
//! (`quakeTemporaryEvent`).
//!
//! Local mirrors: [`TempEntityEffect`] mirrors the donor `TemporaryEntity`
//! wire shape from `src/network/q1/netquake.ts`; [`QcBroadcastEffect`]
//! mirrors the `Q1Event` effect/beam/colored-explosion/particles subset from
//! `src/content/q1/foundation/types.ts`.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::error::GuestError;

/// Decoded temporary-entity wire effect.
#[derive(Debug, Clone, PartialEq)]
pub enum TempEntityEffect {
    /// Colored explosion (`explosion-colors`).
    ExplosionColors {
        /// Effect origin.
        origin: Vec3,
        /// First palette color.
        color_start: i32,
        /// Palette color span.
        color_length: i32,
    },
    /// Beam between two points.
    Beam {
        /// Beam entity slot captured when the message was written.
        entity: u16,
        /// Wire beam type.
        beam_type: u8,
        /// Beam start.
        start: Vec3,
        /// Beam end.
        end: Vec3,
    },
    /// Point effect addressed by wire type number.
    Point {
        /// Wire effect type.
        effect_type: u8,
        /// Effect origin.
        origin: Vec3,
        /// Effect amount (particle count, blood count, ...).
        count: i32,
    },
}

/// Named point effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointEffect {
    /// Spike (type 0).
    Spike,
    /// Super spike (type 1).
    SuperSpike,
    /// Gunshot (type 2).
    Gunshot,
    /// Explosion (type 3).
    Explosion,
    /// Tar explosion (type 4).
    TarExplosion,
    /// Wizard spike (type 7).
    WizardSpike,
    /// Knight spike (type 8).
    KnightSpike,
    /// Lava splash (type 10).
    LavaSplash,
    /// Teleport splash (type 11).
    Teleport,
}

/// Named beam style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamStyle {
    /// Lightning bolts (type 5).
    Lightning1,
    /// Lightning shafts (type 6).
    Lightning2,
    /// Lightning alternative (type 9).
    Lightning3,
    /// Grapple beam (type 13, NetQuake only).
    Grapple,
}

/// Presentation event projected from a temporary entity.
#[derive(Debug, Clone, PartialEq)]
pub enum QcBroadcastEffect {
    /// Named point effect.
    Effect {
        /// Effect name.
        effect: PointEffect,
        /// Owning actor, if any.
        actor: Option<ActorId>,
        /// Effect origin.
        origin: Vec3,
        /// Effect amount.
        amount: i32,
    },
    /// Beam effect.
    Beam {
        /// Beam style.
        style: BeamStyle,
        /// Owning actor captured when the message was written.
        actor: ActorId,
        /// Beam start.
        start: Vec3,
        /// Beam end.
        end: Vec3,
    },
    /// Colored explosion.
    ColoredExplosion {
        /// Explosion origin.
        origin: Vec3,
        /// First palette color.
        color_start: i32,
        /// Palette color span.
        color_length: i32,
    },
    /// Particle burst.
    Particles {
        /// Burst origin.
        origin: Vec3,
        /// Burst direction.
        direction: Vec3,
        /// Palette color.
        color: i32,
        /// Particle count.
        count: i32,
    },
}

/// Project a decoded temporary entity into a presentation event.
///
/// `actor` is the owner captured when the message bytes were written.
/// `quakeworld` selects the QuakeWorld blood opcodes, which differ from the
/// NetQuake explosion2/beam opcodes.
pub fn quake_temporary_event(
    effect: &TempEntityEffect,
    actor: Option<&ActorId>,
    quakeworld: bool,
) -> Result<QcBroadcastEffect, GuestError> {
    match effect {
        TempEntityEffect::ExplosionColors {
            origin,
            color_start,
            color_length,
        } => Ok(QcBroadcastEffect::ColoredExplosion {
            origin: *origin,
            color_start: *color_start,
            color_length: *color_length,
        }),
        TempEntityEffect::Beam {
            entity,
            beam_type,
            start,
            end,
        } => {
            let Some(actor) = actor else {
                return Err(GuestError::invalid(format!(
                    "QC beam entity {entity} had no owned actor when written"
                )));
            };
            let style = match *beam_type {
                5 => BeamStyle::Lightning1,
                6 => BeamStyle::Lightning2,
                9 => BeamStyle::Lightning3,
                13 if !quakeworld => BeamStyle::Grapple,
                _ => return Err(GuestError::invalid(format!("Unsupported QC beam {beam_type}"))),
            };
            Ok(QcBroadcastEffect::Beam {
                style,
                actor: actor.clone(),
                start: *start,
                end: *end,
            })
        }
        TempEntityEffect::Point {
            effect_type,
            origin,
            count,
        } => {
            if quakeworld && matches!(*effect_type, 2 | 12 | 13) {
                let (color, count) = match *effect_type {
                    2 => (0, 20 * *count),
                    12 => (73, 20 * *count),
                    _ => (225, 50),
                };
                return Ok(QcBroadcastEffect::Particles {
                    origin: *origin,
                    direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    color,
                    count,
                });
            }
            let name = match *effect_type {
                0 => PointEffect::Spike,
                1 => PointEffect::SuperSpike,
                2 => PointEffect::Gunshot,
                3 => PointEffect::Explosion,
                4 => PointEffect::TarExplosion,
                7 => PointEffect::WizardSpike,
                8 => PointEffect::KnightSpike,
                10 => PointEffect::LavaSplash,
                11 => PointEffect::Teleport,
                _ => {
                    return Err(GuestError::invalid(format!(
                        "Unsupported QC point effect {effect_type}"
                    )))
                }
            };
            Ok(QcBroadcastEffect::Effect {
                effect: name,
                actor: None,
                origin: *origin,
                amount: *count,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    #[test]
    fn point_effect_table_matches_donor() {
        let origin = vec3(1.0, 2.0, 3.0);
        let cases = [
            (0, PointEffect::Spike),
            (1, PointEffect::SuperSpike),
            (2, PointEffect::Gunshot),
            (3, PointEffect::Explosion),
            (4, PointEffect::TarExplosion),
            (7, PointEffect::WizardSpike),
            (8, PointEffect::KnightSpike),
            (10, PointEffect::LavaSplash),
            (11, PointEffect::Teleport),
        ];
        for (effect_type, expected) in cases {
            let event = quake_temporary_event(
                &TempEntityEffect::Point {
                    effect_type,
                    origin,
                    count: 6,
                },
                None,
                false,
            )
            .unwrap();
            assert_eq!(
                event,
                QcBroadcastEffect::Effect {
                    effect: expected,
                    actor: None,
                    origin,
                    amount: 6
                }
            );
        }
        assert!(quake_temporary_event(
            &TempEntityEffect::Point {
                effect_type: 5,
                origin,
                count: 1
            },
            None,
            false
        )
        .is_err());
    }

    #[test]
    fn quakeworld_blood_opcodes_become_particles() {
        let origin = vec3(0.0, 0.0, 8.0);
        let event = quake_temporary_event(
            &TempEntityEffect::Point {
                effect_type: 2,
                origin,
                count: 3,
            },
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            event,
            QcBroadcastEffect::Particles {
                origin,
                direction: vec3(0.0, 0.0, 0.0),
                color: 0,
                count: 60
            }
        );
        let event = quake_temporary_event(
            &TempEntityEffect::Point {
                effect_type: 13,
                origin,
                count: 9,
            },
            None,
            true,
        )
        .unwrap();
        assert!(matches!(
            event,
            QcBroadcastEffect::Particles {
                color: 225,
                count: 50,
                ..
            }
        ));
        // Same wire type on NetQuake is a beam opcode, rejected as a point effect.
        assert!(quake_temporary_event(
            &TempEntityEffect::Point {
                effect_type: 13,
                origin,
                count: 9
            },
            None,
            false
        )
        .is_err());
    }

    #[test]
    fn beams_require_owner_and_known_style() {
        let owner = IdentityOwner::create("message-effects").unwrap();
        let actor = owner.actor(1, 1);
        let start = vec3(0.0, 0.0, 0.0);
        let end = vec3(0.0, 0.0, 64.0);
        for (beam_type, style) in [
            (5, BeamStyle::Lightning1),
            (6, BeamStyle::Lightning2),
            (9, BeamStyle::Lightning3),
        ] {
            let event = quake_temporary_event(
                &TempEntityEffect::Beam {
                    entity: 4,
                    beam_type,
                    start,
                    end,
                },
                Some(&actor),
                true,
            )
            .unwrap();
            assert_eq!(
                event,
                QcBroadcastEffect::Beam {
                    style,
                    actor: actor.clone(),
                    start,
                    end
                }
            );
        }
        assert!(quake_temporary_event(
            &TempEntityEffect::Beam {
                entity: 4,
                beam_type: 5,
                start,
                end
            },
            None,
            true
        )
        .is_err());
        assert!(quake_temporary_event(
            &TempEntityEffect::Beam {
                entity: 4,
                beam_type: 7,
                start,
                end
            },
            Some(&actor),
            true
        )
        .is_err());
    }

    #[test]
    fn grapple_exists_only_on_netquake() {
        let owner = IdentityOwner::create("message-effects").unwrap();
        let actor = owner.actor(1, 1);
        let beam = TempEntityEffect::Beam {
            entity: 2,
            beam_type: 13,
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(8.0, 0.0, 0.0),
        };
        assert!(matches!(
            quake_temporary_event(&beam, Some(&actor), false).unwrap(),
            QcBroadcastEffect::Beam {
                style: BeamStyle::Grapple,
                ..
            }
        ));
        assert!(quake_temporary_event(&beam, Some(&actor), true).is_err());
    }

    #[test]
    fn explosion_colors_pass_through() {
        let origin = vec3(5.0, 5.0, 5.0);
        let event = quake_temporary_event(
            &TempEntityEffect::ExplosionColors {
                origin,
                color_start: 10,
                color_length: 4,
            },
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            event,
            QcBroadcastEffect::ColoredExplosion {
                origin,
                color_start: 10,
                color_length: 4
            }
        );
    }
}
