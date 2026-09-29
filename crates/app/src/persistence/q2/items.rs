//! Quake II item checkpoint ported from `src/persistence/q2-items.ts`.

use qa_core::identity::SavedActorId;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Saved pickup.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PickupCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Classname.
    pub classname: String,
    /// Targets used.
    pub targets_used: bool,
    /// Retained flag.
    pub retained: bool,
    /// Expiry time.
    pub expires_at: Option<f64>,
}

/// Saved power timing.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PowerCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Quad expiry.
    pub quad_until: f64,
    /// Invulnerability expiry.
    pub invulnerability_until: f64,
    /// Breather expiry.
    pub breather_until: f64,
    /// Enviro-suit expiry.
    pub enviro_until: f64,
}

/// Q2 items checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ItemsCheckpoint {
    /// Power cube count.
    pub power_cube_count: f64,
    /// Pickups.
    pub pickups: Vec<Q2PickupCheckpoint>,
    /// Power timings.
    pub powers: Vec<Q2PowerCheckpoint>,
    /// Power-armor bindings.
    pub power_armor_bindings: Vec<SavedActorId>,
}

/// Read a Q2 items checkpoint.
pub fn read_q2_items_checkpoint(reader: SaveReader) -> Result<Q2ItemsCheckpoint, PersistenceError> {
    Ok(Q2ItemsCheckpoint {
        power_cube_count: reader.field("powerCubeCount").number()?,
        pickups: reader
            .field("pickups")
            .list(|value| -> Result<Q2PickupCheckpoint, PersistenceError> {
                Ok(Q2PickupCheckpoint {
                    actor: read_saved_actor(value.field("actor"))?,
                    classname: value.field("classname").string()?,
                    targets_used: value.field("targetsUsed").boolean()?,
                    retained: value.field("retained").boolean()?,
                    expires_at: value
                        .field("expiresAt")
                        .nullable(|expiry| expiry.number().map_err(PersistenceError::from))?,
                })
            })?,
        powers: reader
            .field("powers")
            .list(|value| -> Result<Q2PowerCheckpoint, PersistenceError> {
                let state = value.field("state");
                Ok(Q2PowerCheckpoint {
                    actor: read_saved_actor(value.field("actor"))?,
                    quad_until: state.field("quadUntil").number()?,
                    invulnerability_until: state.field("invulnerabilityUntil").number()?,
                    breather_until: state.field("breatherUntil").number()?,
                    enviro_until: state.field("enviroUntil").number()?,
                })
            })?,
        power_armor_bindings: reader
            .field("powerArmorBindings")
            .list(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
    })
}

/// Write a Q2 items checkpoint.
#[must_use]
pub fn write_q2_items_checkpoint(checkpoint: &Q2ItemsCheckpoint) -> SaveJson {
    obj(vec![
        ("powerCubeCount", num(checkpoint.power_cube_count)),
        (
            "pickups",
            arr(checkpoint
                .pickups
                .iter()
                .map(|pickup| {
                    obj(vec![
                        ("actor", write_saved_actor(pickup.actor)),
                        ("classname", str(&pickup.classname)),
                        ("targetsUsed", boolean(pickup.targets_used)),
                        ("retained", boolean(pickup.retained)),
                        ("expiresAt", pickup.expires_at.map_or(SaveJson::Null, num)),
                    ])
                })
                .collect()),
        ),
        (
            "powers",
            arr(checkpoint
                .powers
                .iter()
                .map(|power| {
                    obj(vec![
                        ("actor", write_saved_actor(power.actor)),
                        (
                            "state",
                            obj(vec![
                                ("quadUntil", num(power.quad_until)),
                                ("invulnerabilityUntil", num(power.invulnerability_until)),
                                ("breatherUntil", num(power.breather_until)),
                                ("enviroUntil", num(power.enviro_until)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "powerArmorBindings",
            arr(checkpoint
                .power_armor_bindings
                .iter()
                .map(|actor| write_saved_actor(*actor))
                .collect()),
        ),
    ])
}

/// Encode a Q2 items checkpoint.
#[must_use]
pub fn encode_q2_items_checkpoint(checkpoint: &Q2ItemsCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_items_checkpoint(checkpoint))
}

/// Decode a Q2 items checkpoint.
pub fn decode_q2_items_checkpoint(bytes: &[u8]) -> Result<Q2ItemsCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_items_checkpoint(SaveReader::at(&payload, "q2-items"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip() {
        let checkpoint = Q2ItemsCheckpoint {
            power_cube_count: 2.0,
            pickups: vec![Q2PickupCheckpoint {
                actor: SavedActorId { slot: 1, generation: 0 },
                classname: "item_quad".to_string(),
                targets_used: true,
                retained: false,
                expires_at: Some(30.0),
            }],
            powers: vec![Q2PowerCheckpoint {
                actor: SavedActorId { slot: 2, generation: 0 },
                quad_until: 10.0,
                invulnerability_until: 0.0,
                breather_until: 0.0,
                enviro_until: 5.0,
            }],
            power_armor_bindings: vec![SavedActorId { slot: 2, generation: 0 }],
        };
        let bytes = encode_q2_items_checkpoint(&checkpoint);
        assert_eq!(decode_q2_items_checkpoint(&bytes).unwrap(), checkpoint);
    }
}
