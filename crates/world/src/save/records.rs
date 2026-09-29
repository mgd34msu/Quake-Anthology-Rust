//! Unified-save shared records ported from `src/persistence/save-image.ts`.
//!
//! Magic framing (`QTSAVE3` current, `QTSAVE2` legacy), saved actor/body/
//! armor/combat/inventory/think records, and actor-slot checkpoints with
//! `freedAt` retention. Values read and write the live [`crate`] types
//! directly, so round-trips assert on the same structs the simulation
//! uses. The full [`UnifiedSaveImage`](qa_app::persistence::image) assembly
//! (recipe, guests, mods) lives in `qa-app`, which owns those contracts.

use qa_core::identity::SavedActorId;
use qa_core::time::SourceTime;

use super::shared::{
    read_bounds, read_character, read_time, read_vector, write_bounds, write_character, write_time, write_vector,
};
use super::value::{arr, boolean, int, namespaced, num, obj, str, SaveJson, SaveReader};
use crate::body::BodyFollow;
use crate::combat::{ArmorState, CombatState, PoweredProtection, RegularArmor};
use crate::inventory::{CountArithmetic, CountPolicy, InventoryEntry};
use crate::registry::{ActorSlotCheckpoint, SlotLifetime};
use crate::scheduler::ThinkBoundary;
use crate::session::SavedBodyState;
use crate::WorldError;

/// Current unified-save magic.
pub const SAVE_MAGIC: &[u8] = b"QTSAVE3\n";
/// Legacy unified-save magic (schema 2 payloads).
pub const LEGACY_SAVE_MAGIC: &[u8] = b"QTSAVE2\n";

/// Frame a checkpoint payload with the current magic.
#[must_use]
pub fn encode_framed(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SAVE_MAGIC.len() + payload.len());
    out.extend_from_slice(SAVE_MAGIC);
    out.extend_from_slice(payload);
    out
}

/// Split framing, returning the schema version (2 or 3) and the payload.
pub fn decode_framed(bytes: &[u8]) -> Result<(u32, &[u8]), WorldError> {
    if bytes.starts_with(SAVE_MAGIC) {
        Ok((3, &bytes[SAVE_MAGIC.len()..]))
    } else if bytes.starts_with(LEGACY_SAVE_MAGIC) {
        Ok((2, &bytes[LEGACY_SAVE_MAGIC.len()..]))
    } else {
        Err(WorldError::BadSave(
            "save: unsupported unified save signature/version".to_string(),
        ))
    }
}

/// Read a saved actor id.
pub fn read_saved_actor(reader: SaveReader) -> Result<SavedActorId, WorldError> {
    let slot = reader.field("slot").integer(0)?;
    let generation = reader.field("generation").integer(0)?;
    let slot = u32::try_from(slot).map_err(|_| reader.field("slot").fail("expected an integer in range"))?;
    let generation =
        u32::try_from(generation).map_err(|_| reader.field("generation").fail("expected an integer in range"))?;
    Ok(SavedActorId { slot, generation })
}

/// Write a saved actor id.
#[must_use]
pub fn write_saved_actor(id: SavedActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(id.slot))),
        ("generation", int(i64::from(id.generation))),
    ])
}

/// Read a saved body state.
pub fn read_saved_body(reader: SaveReader) -> Result<SavedBodyState, WorldError> {
    Ok(SavedBodyState {
        origin: read_vector(reader.field("origin"))?,
        angles: read_vector(reader.field("angles"))?,
        velocity: read_vector(reader.field("velocity"))?,
        bounds: read_bounds(reader.field("bounds"))?,
        ground: reader.field("ground").nullable(read_saved_actor)?,
    })
}

/// Write a saved body state.
#[must_use]
pub fn write_saved_body(state: &SavedBodyState) -> SaveJson {
    obj(vec![
        ("origin", write_vector(state.origin)),
        ("angles", write_vector(state.angles)),
        ("velocity", write_vector(state.velocity)),
        ("bounds", write_bounds(state.bounds)),
        ("ground", state.ground.map_or(SaveJson::Null, write_saved_actor)),
    ])
}

/// Saved body attachment (anchor as a saved actor id).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedBodyAttachment {
    /// Anchor actor.
    pub anchor: SavedActorId,
    /// Follow rule.
    pub follow: BodyFollow,
}

/// Read a saved body attachment.
pub fn read_saved_body_attachment(reader: SaveReader) -> Result<SavedBodyAttachment, WorldError> {
    let follow = reader.field("follow");
    let kind = follow
        .field("kind")
        .choice_str(&["translation", "center", "bounds-min"])?;
    let rule = match kind.as_str() {
        "center" => BodyFollow::Center,
        "translation" => BodyFollow::Translation {
            offset: read_vector(follow.field("offset"))?,
        },
        _ => BodyFollow::BoundsMin {
            offset: read_vector(follow.field("offset"))?,
        },
    };
    Ok(SavedBodyAttachment {
        anchor: read_saved_actor(reader.field("anchor"))?,
        follow: rule,
    })
}

/// Write a saved body attachment.
#[must_use]
pub fn write_saved_body_attachment(attachment: &SavedBodyAttachment) -> SaveJson {
    let follow = match attachment.follow {
        BodyFollow::Center => obj(vec![("kind", str("center"))]),
        BodyFollow::Translation { offset } => obj(vec![("kind", str("translation")), ("offset", write_vector(offset))]),
        BodyFollow::BoundsMin { offset } => obj(vec![("kind", str("bounds-min")), ("offset", write_vector(offset))]),
    };
    obj(vec![
        ("anchor", write_saved_actor(attachment.anchor)),
        ("follow", follow),
    ])
}

/// Read a regular armor state.
pub fn read_regular_armor(reader: SaveReader) -> Result<RegularArmor, WorldError> {
    let kind = reader.field("kind").choice_str(&["none", "q1", "q2", "q3", "source"])?;
    match kind.as_str() {
        "none" => Ok(RegularArmor::None),
        "source" => Ok(RegularArmor::Source {
            points: reader.field("points").number()?,
            item: reader.field("item").nullable(namespaced)?,
        }),
        "q1" => Ok(RegularArmor::Q1 {
            points: reader.field("points").number()?,
            absorption: reader.field("absorption").number()?,
            item: namespaced(reader.field("item"))?,
        }),
        "q3" => Ok(RegularArmor::Q3 {
            points: reader.field("points").number()?,
            protection: reader.field("protection").number()?,
        }),
        _ => Ok(RegularArmor::Q2 {
            points: reader.field("points").number()?,
            normal_protection: reader.field("normalProtection").number()?,
            energy_protection: reader.field("energyProtection").number()?,
            item: namespaced(reader.field("item"))?,
        }),
    }
}

/// Read powered protection state.
pub fn read_powered_protection(reader: SaveReader) -> Result<PoweredProtection, WorldError> {
    let kind = reader.field("kind").choice_str(&["none", "screen", "shield"])?;
    if kind == "none" {
        return Ok(PoweredProtection::None);
    }
    // The donor stores cell counts as plain numbers; saves in the wild are
    // integral, and this port requires integral counts for the `i32` store.
    let cells = reader.field("cells").number()?;
    if !cells.is_finite() || cells.trunc() != cells || cells < f64::from(i32::MIN) || cells > f64::from(i32::MAX) {
        return Err(reader.field("cells").fail("expected an integer in range"));
    }
    #[allow(clippy::cast_possible_truncation)]
    let cells = cells as i32;
    if kind == "screen" {
        Ok(PoweredProtection::Screen { cells })
    } else {
        Ok(PoweredProtection::Shield { cells })
    }
}

/// Read armor, accepting the legacy flat Q2 layout (`powerArmor` member).
pub fn read_armor(reader: SaveReader) -> Result<ArmorState, WorldError> {
    if !reader.field("regular").is_missing() {
        return Ok(ArmorState {
            regular: read_regular_armor(reader.field("regular"))?,
            powered: read_powered_protection(reader.field("powered"))?,
        });
    }
    let regular = read_regular_armor(reader.clone())?;
    let powered = if matches!(regular, RegularArmor::Q2 { .. }) {
        read_powered_protection(reader.field("powerArmor"))?
    } else {
        PoweredProtection::None
    };
    Ok(ArmorState { regular, powered })
}

/// Write armor in the current nested layout.
#[must_use]
pub fn write_armor(armor: &ArmorState) -> SaveJson {
    obj(vec![
        ("regular", write_regular_armor(&armor.regular)),
        ("powered", write_powered_protection(&armor.powered)),
    ])
}

fn write_regular_armor(armor: &RegularArmor) -> SaveJson {
    match armor {
        RegularArmor::None => obj(vec![("kind", str("none"))]),
        RegularArmor::Source { points, item } => obj(vec![
            ("kind", str("source")),
            ("points", num(*points)),
            ("item", item.as_ref().map_or(SaveJson::Null, |item| str(item))),
        ]),
        RegularArmor::Q1 {
            points,
            absorption,
            item,
        } => obj(vec![
            ("kind", str("q1")),
            ("points", num(*points)),
            ("absorption", num(*absorption)),
            ("item", str(item)),
        ]),
        RegularArmor::Q3 { points, protection } => obj(vec![
            ("kind", str("q3")),
            ("points", num(*points)),
            ("protection", num(*protection)),
        ]),
        RegularArmor::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        } => obj(vec![
            ("kind", str("q2")),
            ("points", num(*points)),
            ("normalProtection", num(*normal_protection)),
            ("energyProtection", num(*energy_protection)),
            ("item", str(item)),
        ]),
    }
}

fn write_powered_protection(powered: &PoweredProtection) -> SaveJson {
    match powered {
        PoweredProtection::None => obj(vec![("kind", str("none"))]),
        PoweredProtection::Screen { cells } => obj(vec![("kind", str("screen")), ("cells", int(i64::from(*cells)))]),
        PoweredProtection::Shield { cells } => obj(vec![("kind", str("shield")), ("cells", int(i64::from(*cells)))]),
    }
}

/// Read combat state (`noKnockback` defaults to false when absent).
pub fn read_combat(reader: SaveReader) -> Result<CombatState, WorldError> {
    Ok(CombatState {
        health: reader.field("health").number()?,
        armor: read_armor(reader.field("armor"))?,
        mass: reader.field("mass").number()?,
        can_take_damage: reader.field("canTakeDamage").boolean()?,
        invulnerable: reader.field("invulnerable").boolean()?,
        no_knockback: if reader.field("noKnockback").is_missing() {
            false
        } else {
            reader.field("noKnockback").boolean()?
        },
        team: reader.field("team").nullable(|value| value.string())?,
    })
}

/// Write combat state (`noKnockback` omitted when false).
#[must_use]
pub fn write_combat(state: &CombatState) -> SaveJson {
    let mut members = vec![
        ("health", num(state.health)),
        ("armor", write_armor(&state.armor)),
        ("mass", num(state.mass)),
        ("canTakeDamage", boolean(state.can_take_damage)),
        ("invulnerable", boolean(state.invulnerable)),
        ("team", state.team.as_ref().map_or(SaveJson::Null, |team| str(team))),
    ];
    if state.no_knockback {
        members.push(("noKnockback", boolean(true)));
    }
    obj(members)
}

/// Read one inventory entry.
pub fn read_inventory_entry(reader: SaveReader) -> Result<InventoryEntry, WorldError> {
    let policy = reader.field("countPolicy");
    let count_policy = if policy.is_missing() {
        None
    } else {
        let kind = policy.field("kind").choice_str(&["stack", "source-counter"])?;
        if kind == "stack" {
            Some(CountPolicy::Stack)
        } else {
            let arithmetic = policy
                .field("arithmetic")
                .choice_str(&["binary32", "binary64", "int32"])?;
            Some(CountPolicy::SourceCounter(match arithmetic.as_str() {
                "binary32" => CountArithmetic::Binary32,
                "binary64" => CountArithmetic::Binary64,
                _ => CountArithmetic::Int32,
            }))
        }
    };
    Ok(InventoryEntry {
        item: namespaced(reader.field("item"))?,
        count: reader.field("count").number()?,
        capacity: reader.field("capacity").number()?,
        count_policy,
    })
}

/// Write one inventory entry.
#[must_use]
pub fn write_inventory_entry(entry: &InventoryEntry) -> SaveJson {
    let mut members = vec![
        ("item", str(&entry.item)),
        ("count", num(entry.count)),
        ("capacity", num(entry.capacity)),
    ];
    if let Some(policy) = entry.count_policy {
        let policy_json = match policy {
            CountPolicy::Stack => obj(vec![("kind", str("stack"))]),
            CountPolicy::SourceCounter(arithmetic) => obj(vec![
                ("kind", str("source-counter")),
                (
                    "arithmetic",
                    str(match arithmetic {
                        CountArithmetic::Binary32 => "binary32",
                        CountArithmetic::Binary64 => "binary64",
                        CountArithmetic::Int32 => "int32",
                    }),
                ),
            ]),
        };
        members.push(("countPolicy", policy_json));
    }
    obj(members)
}

/// Read an actor slot checkpoint (free slots retain `freedAt`).
pub fn read_actor_slot(reader: SaveReader) -> Result<ActorSlotCheckpoint, WorldError> {
    let slot = reader.field("slot").integer(0)?;
    let generation = reader.field("generation").integer(0)?;
    let lifetime = reader.field("lifetime");
    let kind = lifetime.field("kind").choice_str(&["active", "free"])?;
    Ok(ActorSlotCheckpoint {
        slot: u32::try_from(slot).map_err(|_| reader.field("slot").fail("expected an integer in range"))?,
        generation: u32::try_from(generation)
            .map_err(|_| reader.field("generation").fail("expected an integer in range"))?,
        lifetime: if kind == "free" {
            SlotLifetime::Free {
                freed_at: lifetime.field("freedAt").nullable(read_time)?,
            }
        } else {
            SlotLifetime::Active {
                owner: {
                    let name = namespaced(lifetime.field("owner"))?;
                    let (namespace, id) = name.split_once(':').expect("validated namespaced id");
                    qa_core::identity::ProviderId::new(namespace, id)
                },
                definition: namespaced(lifetime.field("definition"))?,
            }
        },
    })
}

/// Write an actor slot checkpoint.
#[must_use]
pub fn write_actor_slot(slot: &ActorSlotCheckpoint) -> SaveJson {
    let lifetime = match &slot.lifetime {
        SlotLifetime::Free { freed_at } => obj(vec![
            ("kind", str("free")),
            ("freedAt", freed_at.map_or(SaveJson::Null, write_time)),
        ]),
        SlotLifetime::Active { owner, definition } => obj(vec![
            ("kind", str("active")),
            ("owner", str(&format!("{}:{}", owner.namespace, owner.name))),
            ("definition", str(definition)),
        ]),
    };
    obj(vec![
        ("slot", int(i64::from(slot.slot))),
        ("generation", int(i64::from(slot.generation))),
        ("lifetime", lifetime),
    ])
}

/// Source-slot binding checkpoint (donor `SourceActorCheckpoint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSlotBinding {
    /// Source provider.
    pub provider: String,
    /// Source slot.
    pub source_slot: u32,
    /// Bound actor.
    pub actor: SavedActorId,
}

/// Read source-slot bindings from a `world:source-slots` provider record.
pub fn read_source_slots(
    record: &super::ownership::ProviderCheckpoint,
    payload: &SaveJson,
) -> Result<Vec<SourceSlotBinding>, WorldError> {
    if record.provider != "world:actors" || record.schema != "world:source-slots" || record.version != 1 {
        return Err(WorldError::BadSave(
            "source-slots: unsupported source actor checkpoint".to_string(),
        ));
    }
    SaveReader::at(payload, "source-slots").list(|entry| -> Result<SourceSlotBinding, WorldError> {
        let source_slot = entry.field("sourceSlot").integer(0)?;
        Ok(SourceSlotBinding {
            provider: namespaced(entry.field("provider"))?,
            source_slot: u32::try_from(source_slot)
                .map_err(|_| entry.field("sourceSlot").fail("expected an integer in range"))?,
            actor: read_saved_actor(entry.field("actor"))?,
        })
    })
}

/// Encode source-slot bindings as a `world:source-slots` provider record.
#[must_use]
pub fn source_actors_checkpoint(bindings: &[SourceSlotBinding]) -> super::ownership::ProviderCheckpoint {
    super::ownership::ProviderCheckpoint {
        provider: "world:actors".to_string(),
        schema: "world:source-slots".to_string(),
        version: 1,
        bytes: super::value::encode_checkpoint_value(&write_source_slots(bindings)),
    }
}

/// Write source-slot bindings as a `world:source-slots` payload.
#[must_use]
pub fn write_source_slots(bindings: &[SourceSlotBinding]) -> SaveJson {
    arr(bindings
        .iter()
        .map(|binding| {
            obj(vec![
                ("provider", str(&binding.provider)),
                ("sourceSlot", int(i64::from(binding.source_slot))),
                ("actor", write_saved_actor(binding.actor)),
            ])
        })
        .collect())
}

/// Unified think record (donor `ThinkCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedThink {
    /// Actor.
    pub actor: SavedActorId,
    /// Callback (`namespace:name`).
    pub callback: String,
    /// Due time.
    pub due: SourceTime,
    /// Execution boundary.
    pub boundary: ThinkBoundary,
    /// Owning provider.
    pub provider: String,
    /// Invocation sequence.
    pub sequence: u64,
    /// Execution provider override.
    pub execution_provider: Option<String>,
}

/// Read a unified think record.
pub fn read_think(reader: SaveReader) -> Result<UnifiedThink, WorldError> {
    let boundary = reader
        .field("boundary")
        .choice_str(&["before-physics", "during-physics", "after-physics"])?;
    let sequence = reader.field("sequence").integer(0)?;
    Ok(UnifiedThink {
        actor: read_saved_actor(reader.field("actor"))?,
        callback: namespaced(reader.field("callback"))?,
        due: read_time(reader.field("due"))?,
        boundary: match boundary.as_str() {
            "before-physics" => ThinkBoundary::BeforePhysics,
            "during-physics" => ThinkBoundary::DuringPhysics,
            _ => ThinkBoundary::AfterPhysics,
        },
        provider: namespaced(reader.field("provider"))?,
        sequence: u64::try_from(sequence).map_err(|_| reader.field("sequence").fail("expected an integer in range"))?,
        execution_provider: if reader.field("executionProvider").is_missing() {
            None
        } else {
            Some(namespaced(reader.field("executionProvider"))?)
        },
    })
}

/// Write a unified think record.
#[must_use]
pub fn write_think(think: &UnifiedThink) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    let sequence = int(think.sequence as i64);
    let mut members = vec![
        ("actor", write_saved_actor(think.actor)),
        ("callback", str(&think.callback)),
        ("due", write_time(think.due)),
        (
            "boundary",
            str(match think.boundary {
                ThinkBoundary::BeforePhysics => "before-physics",
                ThinkBoundary::DuringPhysics => "during-physics",
                ThinkBoundary::AfterPhysics => "after-physics",
            }),
        ),
        ("provider", str(&think.provider)),
        ("sequence", sequence),
    ];
    if let Some(provider) = &think.execution_provider {
        members.push(("executionProvider", str(provider)));
    }
    obj(members)
}

/// Unified body record (donor `BodyCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedBody {
    /// Actor.
    pub actor: SavedActorId,
    /// Body state.
    pub body: SavedBodyState,
    /// Attachment.
    pub attachment: Option<SavedBodyAttachment>,
    /// Link count.
    pub link_count: u64,
    /// Linked snapshot.
    pub linked: Option<(SavedBodyState, qa_core::math::Bounds)>,
}

/// Read a unified body record.
pub fn read_body(reader: SaveReader) -> Result<UnifiedBody, WorldError> {
    Ok(UnifiedBody {
        actor: read_saved_actor(reader.field("actor"))?,
        body: read_saved_body(reader.field("body"))?,
        attachment: reader.field("attachment").nullable(read_saved_body_attachment)?,
        link_count: u64::try_from(reader.field("linkCount").integer(0)?)
            .map_err(|_| reader.field("linkCount").fail("expected an integer in range"))?,
        linked: reader.field("linked").nullable(|link| {
            Ok((
                read_saved_body(link.field("state"))?,
                read_bounds(link.field("absoluteBounds"))?,
            ))
        })?,
    })
}

/// Write a unified body record.
#[must_use]
pub fn write_body(body: &UnifiedBody) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    let link_count = int(body.link_count as i64);
    obj(vec![
        ("actor", write_saved_actor(body.actor)),
        ("body", write_saved_body(&body.body)),
        (
            "attachment",
            body.attachment
                .as_ref()
                .map_or(SaveJson::Null, write_saved_body_attachment),
        ),
        ("linkCount", link_count),
        (
            "linked",
            body.linked.as_ref().map_or(SaveJson::Null, |(state, bounds)| {
                obj(vec![
                    ("state", write_saved_body(state)),
                    ("absoluteBounds", write_bounds(*bounds)),
                ])
            }),
        ),
    ])
}

/// Actor configuration record (donor `ActorConfigurationCheckpoint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorConfiguration {
    /// Actor.
    pub actor: SavedActorId,
    /// Movement provider.
    pub movement: super::shared::ProviderRef,
    /// Character selection.
    pub character: super::shared::CharacterSelection,
    /// Weapon providers.
    pub weapons: Vec<super::shared::ProviderRef>,
    /// Inventory provider.
    pub inventory: super::shared::ProviderRef,
}

/// Read an actor configuration record.
pub fn read_configuration(reader: SaveReader) -> Result<ActorConfiguration, WorldError> {
    use super::shared::read_provider_ref;
    Ok(ActorConfiguration {
        actor: read_saved_actor(reader.field("actor"))?,
        movement: read_provider_ref(reader.field("movement"))?,
        character: read_character(reader.field("character"))?,
        weapons: reader.field("weapons").list(read_provider_ref)?,
        inventory: read_provider_ref(reader.field("inventory"))?,
    })
}

/// Write an actor configuration record.
#[must_use]
pub fn write_configuration(configuration: &ActorConfiguration) -> SaveJson {
    use super::shared::write_provider_ref;
    obj(vec![
        ("actor", write_saved_actor(configuration.actor)),
        ("movement", write_provider_ref(&configuration.movement)),
        ("character", write_character(&configuration.character)),
        (
            "weapons",
            arr(configuration.weapons.iter().map(write_provider_ref).collect()),
        ),
        ("inventory", write_provider_ref(&configuration.inventory)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::value::{decode_checkpoint_value, encode_checkpoint_value};
    use qa_core::identity::ProviderId;
    use qa_core::math::{Bounds, Vec3};

    fn round_trip(value: &SaveJson) -> SaveJson {
        decode_checkpoint_value(&encode_checkpoint_value(value)).unwrap()
    }

    fn sample_body() -> SavedBodyState {
        SavedBodyState {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            angles: Vec3 {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
            velocity: Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
            bounds: Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            },
            ground: Some(SavedActorId { slot: 0, generation: 0 }),
        }
    }

    #[test]
    fn framing_selects_schema_versions() {
        let framed = encode_framed(b"{}");
        assert_eq!(decode_framed(&framed).unwrap(), (3, b"{}".as_slice()));
        let mut legacy = LEGACY_SAVE_MAGIC.to_vec();
        legacy.extend_from_slice(b"{}");
        assert_eq!(decode_framed(&legacy).unwrap(), (2, b"{}".as_slice()));
        assert!(decode_framed(b"NOPE").is_err());
    }

    #[test]
    fn body_armor_combat_inventory_round_trip() {
        let body = sample_body();
        let json = write_saved_body(&body);
        assert_eq!(read_saved_body(SaveReader::at(&round_trip(&json), "b")).unwrap(), body);
        for follow in [
            BodyFollow::Center,
            BodyFollow::Translation {
                offset: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            },
            BodyFollow::BoundsMin {
                offset: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            },
        ] {
            let attachment = SavedBodyAttachment {
                anchor: SavedActorId { slot: 2, generation: 1 },
                follow,
            };
            let json = write_saved_body_attachment(&attachment);
            assert_eq!(
                read_saved_body_attachment(SaveReader::at(&round_trip(&json), "a")).unwrap(),
                attachment
            );
        }
        let combat = CombatState {
            health: 75.0,
            armor: ArmorState {
                regular: RegularArmor::Q2 {
                    points: 50.0,
                    normal_protection: 0.6,
                    energy_protection: 0.4,
                    item: "q2:armor-jacket".to_string(),
                },
                powered: PoweredProtection::Screen { cells: 12 },
            },
            mass: 200.0,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: true,
            team: Some("red".to_string()),
        };
        let json = write_combat(&combat);
        assert_eq!(read_combat(SaveReader::at(&round_trip(&json), "c")).unwrap(), combat);
        // Legacy flat Q2 armor layout.
        let legacy = obj(vec![
            ("kind", str("q2")),
            ("points", num(10.0)),
            ("normalProtection", num(0.5)),
            ("energyProtection", num(0.5)),
            ("item", str("q2:armor")),
            ("powerArmor", obj(vec![("kind", str("shield")), ("cells", int(3))])),
        ]);
        let armor = read_armor(SaveReader::new(&legacy)).unwrap();
        assert!(matches!(armor.powered, PoweredProtection::Shield { cells: 3 }));
        let entry = InventoryEntry {
            item: "q3:rockets".to_string(),
            count: 10.0,
            capacity: 20.0,
            count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
        };
        let json = write_inventory_entry(&entry);
        assert_eq!(
            read_inventory_entry(SaveReader::at(&round_trip(&json), "i")).unwrap(),
            entry
        );
        let plain = InventoryEntry {
            count_policy: None,
            ..entry.clone()
        };
        let json = write_inventory_entry(&plain);
        assert!(json.get("countPolicy").is_none());
        assert_eq!(
            read_inventory_entry(SaveReader::at(&round_trip(&json), "i")).unwrap(),
            plain
        );
    }

    #[test]
    fn slots_thinks_bodies_round_trip() {
        let slots = vec![
            ActorSlotCheckpoint {
                slot: 0,
                generation: 0,
                lifetime: SlotLifetime::Active {
                    owner: ProviderId::new("q3", "game"),
                    definition: "q3:soldier".to_string(),
                },
            },
            ActorSlotCheckpoint {
                slot: 1,
                generation: 2,
                lifetime: SlotLifetime::Free {
                    freed_at: Some(SourceTime::Milliseconds(100)),
                },
            },
        ];
        for slot in &slots {
            let json = write_actor_slot(slot);
            assert_eq!(read_actor_slot(SaveReader::at(&round_trip(&json), "s")).unwrap(), *slot);
        }
        let think = UnifiedThink {
            actor: SavedActorId { slot: 0, generation: 0 },
            callback: "q3:run".to_string(),
            due: SourceTime::Milliseconds(16),
            boundary: ThinkBoundary::DuringPhysics,
            provider: "q3:game".to_string(),
            sequence: 9,
            execution_provider: None,
        };
        let json = write_think(&think);
        assert!(json.get("executionProvider").is_none());
        assert_eq!(read_think(SaveReader::at(&round_trip(&json), "t")).unwrap(), think);
        let body = UnifiedBody {
            actor: SavedActorId { slot: 0, generation: 0 },
            body: sample_body(),
            attachment: None,
            link_count: 3,
            linked: None,
        };
        let json = write_body(&body);
        assert_eq!(read_body(SaveReader::at(&round_trip(&json), "b")).unwrap(), body);
    }
}
