//! Quake II effect wire translation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-effects.ts` (`q2BeamFromWire`,
//! `q2EffectToWire`, `q2EffectFromWire`). Q2 `CL_ParseTEnt`/`g_utils`
//! message shapes; names match the source game presentation imports. Wire
//! state reuses [`Q2TempEntity`](qa_net::q2_net::Q2TempEntity),
//! presentation effects reuse
//! [`Q2EffectEvent`](qa_content::q2::foundation::host::Q2EffectEvent), and
//! beams reuse [`Q2WeaponEvent`](qa_content::q2::foundation::weapons::types::Q2WeaponEvent).
//! Vector fields travel as `[f64; 3]` on the wire and [`Vec3`](qa_core::math::Vec3)
//! in presentation; conversions round through `as` casts.

use qa_content::q2::foundation::host::Q2EffectEvent;
use qa_content::q2::foundation::weapons::types::{Q2WeaponEvent, WeaponBeamEffect};
use qa_core::math::Vec3;
use qa_net::q2_net::{Q2TempEntity, Q2TempField, Q2TempInt, Q2TempType, Q2TempVec};
use thiserror::Error;

/// Effect translation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2EffectError {
    /// Trail endpoints missing.
    #[error("{0}")]
    Message(String),
}

/// Wire effect shape (donor `EffectEncoding['shape']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectShape {
    Position,
    Direction,
    Splash,
}

/// One encodable effect (donor `EffectEncoding`).
struct EffectEncoding {
    name: &'static str,
    temp_type: Q2TempType,
    shape: EffectShape,
}

/// Q2 `CL_ParseTEnt` effect table (donor `effects`).
const EFFECTS: [EffectEncoding; 27] = [
    encoding("gunshot", Q2TempType::Gunshot, EffectShape::Direction),
    encoding("blood", Q2TempType::Blood, EffectShape::Direction),
    encoding("blaster", Q2TempType::Blaster, EffectShape::Direction),
    encoding("shotgun", Q2TempType::Shotgun, EffectShape::Direction),
    encoding("sparks", Q2TempType::Sparks, EffectShape::Direction),
    encoding("screen-sparks", Q2TempType::ScreenSparks, EffectShape::Direction),
    encoding("shield-sparks", Q2TempType::ShieldSparks, EffectShape::Direction),
    encoding("bullet-sparks", Q2TempType::BulletSparks, EffectShape::Direction),
    encoding("greenblood", Q2TempType::Greenblood, EffectShape::Direction),
    encoding("blaster2", Q2TempType::Blaster2, EffectShape::Direction),
    encoding("flechette", Q2TempType::Flechette, EffectShape::Direction),
    encoding("moreblood", Q2TempType::Moreblood, EffectShape::Direction),
    encoding("electric-sparks", Q2TempType::ElectricSparks, EffectShape::Direction),
    encoding("splash", Q2TempType::Splash, EffectShape::Splash),
    encoding("laser-sparks", Q2TempType::LaserSparks, EffectShape::Splash),
    encoding("welding-sparks", Q2TempType::WeldingSparks, EffectShape::Splash),
    encoding("tunnel-sparks", Q2TempType::TunnelSparks, EffectShape::Splash),
    encoding("explosion1", Q2TempType::Explosion1, EffectShape::Position),
    encoding("explosion2", Q2TempType::Explosion2, EffectShape::Position),
    encoding("rocket-explosion", Q2TempType::RocketExplosion, EffectShape::Position),
    encoding("grenade-explosion", Q2TempType::GrenadeExplosion, EffectShape::Position),
    encoding(
        "rocket-explosion-water",
        Q2TempType::RocketExplosionWater,
        EffectShape::Position,
    ),
    encoding(
        "grenade-explosion-water",
        Q2TempType::GrenadeExplosionWater,
        EffectShape::Position,
    ),
    encoding("bfg-explosion", Q2TempType::BfgExplosion, EffectShape::Position),
    encoding("bfg-bigexplosion", Q2TempType::BfgBigexplosion, EffectShape::Position),
    encoding("boss-teleport", Q2TempType::Bosstport, EffectShape::Position),
    encoding("other-teleport", Q2TempType::TeleportEffect, EffectShape::Position),
];

const fn encoding(name: &'static str, temp_type: Q2TempType, shape: EffectShape) -> EffectEncoding {
    EffectEncoding { name, temp_type, shape }
}

/// Wire trails carrying endpoints without an owning entity (donor `trails`).
const TRAILS: [(Q2TempType, WeaponBeamEffect); 4] = [
    (Q2TempType::Railtrail, WeaponBeamEffect::Rail),
    (Q2TempType::Bubbletrail, WeaponBeamEffect::BubbleTrail),
    (Q2TempType::BfgLaser, WeaponBeamEffect::BfgLaser),
    (Q2TempType::BfgZap, WeaponBeamEffect::BfgZap),
];

/// Read a vector field by name.
fn vector_field(value: &Q2TempEntity, name: Q2TempVec) -> Option<Vec3> {
    value.fields.iter().find_map(|field| match field {
        Q2TempField::Vector { name: found, value } if *found == name => Some(Vec3 {
            x: value[0] as f32,
            y: value[1] as f32,
            z: value[2] as f32,
        }),
        _ => None,
    })
}

/// Read an integer field by name.
fn integer_field(value: &Q2TempEntity, name: Q2TempInt) -> Option<i32> {
    value.fields.iter().find_map(|field| match field {
        Q2TempField::Integer { name: found, value } if *found == name => Some(*value),
        _ => None,
    })
}

/// Translate a wire trail into a beam event (`q2BeamFromWire`).
///
/// These wire effects carry endpoints without an owning entity number.
pub fn q2_beam_from_wire(value: &Q2TempEntity) -> Result<Option<Q2WeaponEvent>, Q2EffectError> {
    let Some((_, effect)) = TRAILS.iter().find(|(temp_type, _)| value.temp_type == *temp_type as u8) else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (
        vector_field(value, Q2TempVec::Position1),
        vector_field(value, Q2TempVec::Position2),
    ) else {
        return Err(Q2EffectError::Message(
            "Q2 trail requires both source endpoints".to_string(),
        ));
    };
    Ok(Some(Q2WeaponEvent::Beam {
        effect: *effect,
        actor: None,
        start,
        end,
        duration: 0.1,
    }))
}

/// Translate a presentation effect into a wire entity (`q2EffectToWire`).
pub fn q2_effect_to_wire(event: &Q2EffectEvent) -> Option<Q2TempEntity> {
    let name = event.effect.strip_prefix("q2:").unwrap_or(&event.effect);
    let encoding = EFFECTS.iter().find(|value| value.name == name)?;
    let mut fields = Vec::new();
    if encoding.shape == EffectShape::Splash {
        fields.push(Q2TempField::Integer {
            name: Q2TempInt::Count,
            value: event.count,
        });
    }
    fields.push(Q2TempField::Vector {
        name: Q2TempVec::Position1,
        value: [
            f64::from(event.origin.x),
            f64::from(event.origin.y),
            f64::from(event.origin.z),
        ],
    });
    if encoding.shape != EffectShape::Position {
        fields.push(Q2TempField::Vector {
            name: Q2TempVec::Direction,
            value: [
                f64::from(event.direction.x),
                f64::from(event.direction.y),
                f64::from(event.direction.z),
            ],
        });
    }
    if encoding.shape == EffectShape::Splash {
        fields.push(Q2TempField::Integer {
            name: Q2TempInt::Color,
            value: event.color,
        });
    }
    Some(Q2TempEntity {
        temp_type: encoding.temp_type as u8,
        fields,
        raw: Vec::new(),
    })
}

/// Translate a wire entity into a presentation effect (`q2EffectFromWire`).
///
/// Missing fields decode as zero, matching the donor.
pub fn q2_effect_from_wire(value: &Q2TempEntity) -> Option<Q2EffectEvent> {
    let encoding = EFFECTS.iter().find(|entry| value.temp_type == entry.temp_type as u8)?;
    let zero = Vec3::default();
    Some(Q2EffectEvent {
        effect: encoding.name.to_string(),
        origin: vector_field(value, Q2TempVec::Position1).unwrap_or(zero),
        direction: vector_field(value, Q2TempVec::Direction).unwrap_or(zero),
        count: integer_field(value, Q2TempInt::Count).unwrap_or(0),
        color: integer_field(value, Q2TempInt::Color).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> Vec3 {
        Vec3 { x: 1.0, y: 2.0, z: 3.0 }
    }

    fn direction() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 1.0 }
    }

    #[test]
    fn beam_round_trips_endpoints() {
        let wire = Q2TempEntity {
            temp_type: Q2TempType::Railtrail as u8,
            fields: vec![
                Q2TempField::Vector {
                    name: Q2TempVec::Position1,
                    value: [1.0, 2.0, 3.0],
                },
                Q2TempField::Vector {
                    name: Q2TempVec::Position2,
                    value: [4.0, 5.0, 6.0],
                },
            ],
            raw: Vec::new(),
        };
        let event = q2_beam_from_wire(&wire).expect("beam").expect("some");
        assert!(matches!(
            event,
            Q2WeaponEvent::Beam {
                effect: WeaponBeamEffect::Rail,
                actor: None,
                ..
            }
        ));
        let plain = Q2TempEntity {
            temp_type: Q2TempType::Gunshot as u8,
            fields: Vec::new(),
            raw: Vec::new(),
        };
        assert!(q2_beam_from_wire(&plain).expect("none").is_none());
        let missing = Q2TempEntity {
            temp_type: Q2TempType::BfgZap as u8,
            fields: Vec::new(),
            raw: Vec::new(),
        };
        assert_eq!(
            q2_beam_from_wire(&missing).expect_err("endpoints"),
            Q2EffectError::Message("Q2 trail requires both source endpoints".to_string())
        );
    }

    #[test]
    fn effects_encode_shapes_and_decode_defaults() {
        let splash = Q2EffectEvent {
            effect: "q2:splash".to_string(),
            origin: origin(),
            direction: direction(),
            count: 12,
            color: 3,
        };
        let wire = q2_effect_to_wire(&splash).expect("wire");
        assert_eq!(wire.temp_type, Q2TempType::Splash as u8);
        assert_eq!(wire.fields.len(), 4);
        let back = q2_effect_from_wire(&wire).expect("back");
        assert_eq!(back.effect, "splash");
        assert_eq!(back.count, 12);
        assert_eq!(back.color, 3);
        let gunshot = Q2EffectEvent {
            effect: "gunshot".to_string(),
            origin: origin(),
            direction: direction(),
            count: 0,
            color: 0,
        };
        let wire = q2_effect_to_wire(&gunshot).expect("wire");
        assert_eq!(wire.fields.len(), 2);
        let unknown = Q2EffectEvent {
            effect: "q2:unknown".to_string(),
            origin: origin(),
            direction: direction(),
            count: 0,
            color: 0,
        };
        assert!(q2_effect_to_wire(&unknown).is_none());
        let sparse = Q2TempEntity {
            temp_type: Q2TempType::Explosion1 as u8,
            fields: Vec::new(),
            raw: Vec::new(),
        };
        let back = q2_effect_from_wire(&sparse).expect("back");
        assert_eq!(back.origin, Vec3::default());
        assert_eq!(back.count, 0);
        let unmapped = Q2TempEntity {
            temp_type: Q2TempType::Lightning as u8,
            fields: Vec::new(),
            raw: Vec::new(),
        };
        assert!(q2_effect_from_wire(&unmapped).is_none());
    }
}
