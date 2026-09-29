//! Unified save envelope ported from `src/persistence/save-image.ts`.
//!
//! The donor names this module `save-image.ts`: it is the `QTSAVE3`
//! unified-save assembly, not an image codec (content image formats live
//! in `qa-content`'s `images` module). Schema 2 payloads still decode
//! and upgrade to schema 3 with `legacy_armor_layout` set; the framing
//! version must match the payload `schemaVersion`.

use qa_core::identity::SavedActorId;
use qa_core::time::{FrameContext, SourceTime};
use qa_guest::checkpoint::{read_guest, write_guest, GuestCheckpoint};
use qa_world::combat::CombatState;
use qa_world::inventory::InventoryEntry;
use qa_world::registry::ActorSlotCheckpoint;
use qa_world::save::ownership::{read_provider_checkpoint, write_provider_checkpoint, ProviderCheckpoint};
use qa_world::save::records::{
    decode_framed, encode_framed, read_actor_slot, read_body, read_combat, read_configuration, read_inventory_entry,
    read_saved_actor, read_think, write_actor_slot, write_body, write_combat, write_configuration,
    write_inventory_entry, write_saved_actor, write_think, ActorConfiguration, UnifiedBody, UnifiedThink,
};
use qa_world::save::shared::{
    read_frame, read_random, read_time, write_frame, write_random, write_time, SaveRandomState,
};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, obj, str, SaveJson, SaveReader,
};

use super::mods::{read_mod_session, write_mod_session, ModSessionCheckpoint};
use super::native_weapon::{NativeWeaponBehaviorDeclaration, WeaponBehaviorDefinition};
use super::recipe::{read_recipe, write_recipe, ExecutableRecipe};
use super::PersistenceError;

/// Unified save image (donor `SaveImage`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSaveImage {
    /// Mod session.
    pub mods: Option<ModSessionCheckpoint>,
    /// Legacy armor layout flag.
    pub legacy_armor_layout: bool,
    /// Recipe.
    pub recipe: ExecutableRecipe,
    /// Frame.
    pub frame: FrameContext,
    /// Next event sequence.
    pub next_event_sequence: u64,
    /// Clock times by provider.
    pub clocks: Vec<(String, SourceTime)>,
    /// Random states by provider.
    pub random: Vec<(String, SaveRandomState)>,
    /// Actor slots.
    pub actors: Vec<ActorSlotCheckpoint>,
    /// Bodies.
    pub bodies: Vec<UnifiedBody>,
    /// Combat states.
    pub combat: Vec<(SavedActorId, CombatState)>,
    /// Inventories.
    pub inventories: Vec<(SavedActorId, Vec<InventoryEntry>)>,
    /// Configurations.
    pub configurations: Vec<ActorConfiguration>,
    /// Thinks.
    pub thinks: Vec<UnifiedThink>,
    /// Provider records.
    pub providers: Vec<ProviderCheckpoint>,
    /// Guests.
    pub guests: Vec<GuestCheckpoint>,
}

fn nonnegative(reader: SaveReader) -> Result<u64, PersistenceError> {
    let value = reader.integer(0)?;
    u64::try_from(value).map_err(|_| PersistenceError::from(reader.fail("expected an integer in range")))
}

/// Parse a unified save image from a decoded payload.
pub fn parse_save_image(
    value: &SaveJson,
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<UnifiedSaveImage, PersistenceError> {
    let reader = SaveReader::at(value, "save");
    let version = reader.field("schemaVersion").choice_i64(&[2, 3])?;
    let legacy_armor = reader.field("legacyArmorLayout");
    if !legacy_armor.is_missing() {
        legacy_armor.literal_bool(true)?;
    }
    let image = UnifiedSaveImage {
        mods: if reader.field("mods").is_missing() {
            None
        } else {
            Some(read_mod_session(reader.field("mods"))?)
        },
        legacy_armor_layout: version == 2 || !legacy_armor.is_missing(),
        recipe: read_recipe(reader.field("recipe"), legacy_native)?,
        frame: read_frame(reader.field("frame"))?,
        next_event_sequence: nonnegative(reader.field("nextEventSequence"))?,
        clocks: reader
            .field("clocks")
            .list(|entry| -> Result<(String, SourceTime), PersistenceError> {
                Ok((namespaced(entry.field("provider"))?, read_time(entry.field("time"))?))
            })?,
        random: reader
            .field("random")
            .list(|entry| -> Result<(String, SaveRandomState), PersistenceError> {
                Ok((namespaced(entry.field("provider"))?, read_random(entry.field("state"))?))
            })?,
        actors: reader
            .field("actors")
            .list(|value| read_actor_slot(value).map_err(PersistenceError::from))?,
        bodies: reader
            .field("bodies")
            .list(|value| read_body(value).map_err(PersistenceError::from))?,
        combat: reader
            .field("combat")
            .list(|entry| -> Result<(SavedActorId, CombatState), PersistenceError> {
                Ok((
                    read_saved_actor(entry.field("actor"))?,
                    read_combat(entry.field("state"))?,
                ))
            })?,
        inventories: reader.field("inventories").list(
            |entry| -> Result<(SavedActorId, Vec<InventoryEntry>), PersistenceError> {
                Ok((
                    read_saved_actor(entry.field("actor"))?,
                    entry
                        .field("entries")
                        .list(|value| read_inventory_entry(value).map_err(PersistenceError::from))?,
                ))
            },
        )?,
        configurations: reader
            .field("configurations")
            .list(|value| read_configuration(value).map_err(PersistenceError::from))?,
        thinks: reader
            .field("thinks")
            .list(|value| read_think(value).map_err(PersistenceError::from))?,
        providers: reader
            .field("providers")
            .list(|value| read_provider_checkpoint(value).map_err(PersistenceError::from))?,
        guests: reader
            .field("guests")
            .list(|value| read_guest(value).map_err(PersistenceError::from))?,
    };
    if !image.recipe.mods.is_empty() && image.mods.is_none() {
        return Err(PersistenceError::from(
            reader
                .field("mods")
                .fail("selected gameplay mods require their saved checkpoint"),
        ));
    }
    Ok(image)
}

/// Write a unified save image payload.
#[must_use]
pub fn write_save_image(image: &UnifiedSaveImage) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    let mut members = vec![("schemaVersion", int(3))];
    if image.legacy_armor_layout {
        members.push(("legacyArmorLayout", boolean(true)));
    }
    members.extend(vec![
        ("recipe", write_recipe(&image.recipe)),
        ("frame", write_frame(image.frame)),
        ("nextEventSequence", int(image.next_event_sequence as i64)),
        (
            "clocks",
            arr(image
                .clocks
                .iter()
                .map(|(provider, time)| obj(vec![("provider", str(provider)), ("time", write_time(*time))]))
                .collect()),
        ),
        (
            "random",
            arr(image
                .random
                .iter()
                .map(|(provider, state)| obj(vec![("provider", str(provider)), ("state", write_random(state))]))
                .collect()),
        ),
        ("actors", arr(image.actors.iter().map(write_actor_slot).collect())),
        ("bodies", arr(image.bodies.iter().map(write_body).collect())),
        (
            "combat",
            arr(image
                .combat
                .iter()
                .map(|(actor, state)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("state", write_combat(state)),
                    ])
                })
                .collect()),
        ),
        (
            "inventories",
            arr(image
                .inventories
                .iter()
                .map(|(actor, entries)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("entries", arr(entries.iter().map(write_inventory_entry).collect())),
                    ])
                })
                .collect()),
        ),
        (
            "configurations",
            arr(image.configurations.iter().map(write_configuration).collect()),
        ),
        ("thinks", arr(image.thinks.iter().map(write_think).collect())),
        (
            "providers",
            arr(image.providers.iter().map(write_provider_checkpoint).collect()),
        ),
        ("guests", arr(image.guests.iter().map(write_guest).collect())),
    ]);
    if let Some(mods) = &image.mods {
        members.push(("mods", write_mod_session(mods)));
    }
    obj(members)
}

/// Encode a unified save image with framing.
#[must_use]
pub fn encode_save_image(image: &UnifiedSaveImage) -> Vec<u8> {
    encode_framed(&encode_checkpoint_value(&write_save_image(image)))
}

/// Decode a framed unified save image.
pub fn decode_save_image(
    bytes: &[u8],
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<UnifiedSaveImage, PersistenceError> {
    let (version, payload) = decode_framed(bytes)?;
    let value = decode_checkpoint_value(payload)?;
    #[allow(clippy::cast_possible_wrap)]
    SaveReader::at(&value, "save")
        .field("schemaVersion")
        .literal_i64(version as i64)?;
    parse_save_image(&value, legacy_native)
}

/// Read a unified save image from disk.
pub fn read_save_image(
    path: &str,
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<UnifiedSaveImage, PersistenceError> {
    decode_save_image(&std::fs::read(path)?, legacy_native)
}

/// Write a unified save image atomically (sibling file plus rename).
pub fn write_save_image_file(path: &str, image: &UnifiedSaveImage) -> Result<(), PersistenceError> {
    let target = std::path::Path::new(path);
    let parent = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    let leaf = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{leaf}.{}.tmp", std::process::id()));
    let result: Result<(), std::io::Error> = (|| {
        std::fs::write(&temporary, encode_save_image(image))?;
        std::fs::rename(&temporary, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_core::math::{Bounds, Vec3};
    use qa_world::registry::SlotLifetime;
    use qa_world::session::SavedBodyState;

    fn frame() -> FrameContext {
        FrameContext {
            frame: 3,
            time: SourceTime::Milliseconds(48),
            elapsed: SourceTime::Milliseconds(16),
            phase: qa_core::time::FramePhase::FrameEntry,
        }
    }

    // A minimal recipe is verbose to construct inline; reuse a tiny builder.
    fn recipe_json() -> SaveJson {
        // Built by the shared recipe fixture through JSON round-trip below.
        crate::persistence::recipe::tests_fixture_recipe_json()
    }

    #[test]
    fn images_round_trip_with_world_state() {
        let json = recipe_json();
        let recipe = read_recipe(SaveReader::new(&json), &|_| None).unwrap();
        let actor = SavedActorId { slot: 0, generation: 0 };
        let image = UnifiedSaveImage {
            mods: None,
            legacy_armor_layout: false,
            recipe,
            frame: frame(),
            next_event_sequence: 7,
            clocks: vec![("q3:game".to_string(), SourceTime::Milliseconds(48))],
            random: vec![("q3:game".to_string(), SaveRandomState::Q3Lcg { seed: 1, draws: 2 })],
            actors: vec![ActorSlotCheckpoint {
                slot: 0,
                generation: 0,
                lifetime: SlotLifetime::Active {
                    owner: ProviderId::new("q3", "game"),
                    definition: "q3:soldier".to_string(),
                },
            }],
            bodies: vec![UnifiedBody {
                actor,
                body: SavedBodyState {
                    origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                    angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    bounds: Bounds {
                        min: Vec3 {
                            x: -1.0,
                            y: -1.0,
                            z: -1.0,
                        },
                        max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                    },
                    ground: None,
                },
                attachment: None,
                link_count: 1,
                linked: None,
            }],
            combat: vec![(actor, CombatState::default())],
            inventories: vec![(actor, Vec::new())],
            configurations: Vec::new(),
            thinks: Vec::new(),
            providers: vec![ProviderCheckpoint {
                provider: "world:actors".to_string(),
                schema: "world:source-slots".to_string(),
                version: 1,
                bytes: encode_checkpoint_value(&arr(Vec::new())),
            }],
            guests: Vec::new(),
        };
        let bytes = encode_save_image(&image);
        assert!(bytes.starts_with(b"QTSAVE3\n"));
        let back = decode_save_image(&bytes, &|_| None).unwrap();
        assert_eq!(back, image);
        // Legacy framing upgrades with the layout flag.
        let mut legacy = b"QTSAVE2\n".to_vec();
        let mut payload = write_save_image(&image);
        if let SaveJson::Object(members) = &mut payload {
            for (key, value) in members.iter_mut() {
                if key == "schemaVersion" {
                    *value = int(2);
                }
            }
        }
        legacy.extend_from_slice(&encode_checkpoint_value(&payload));
        let back = decode_save_image(&legacy, &|_| None).unwrap();
        assert!(back.legacy_armor_layout);
        assert_eq!(back.frame, frame());
    }
}
