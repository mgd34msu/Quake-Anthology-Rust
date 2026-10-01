//! Unified frame value readers.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-frame-values.ts`
//! (`wireActor`, `actor`, `vector`, `color`, `axis`, `readPlayerView`,
//! `readPlayerUi`, `readModel`, `readCharacterView`, `readWorldText`,
//! `readNativeCameraView`).
//!
//! Wire keys match the donor exactly; values reuse the `network::types`
//! presentation mirrors and [`qa_world::save`] readers. The donor's
//! held-weapon and item-icon readers live in `qa-content` against its own
//! `SaveReader` port, so this module mirrors their exact checks
//! (rotation-grip axes, SHA256 digests, content match, WAD lump names) on
//! the `qa-world` reader instead of duplicating their crates.

use qa_content::contract::{
    ArmorState, ContentDigest, ContentId, HeldWeaponDeclaration, HeldWeaponModel, HeldWeaponPart, InventoryCountPolicy,
    InventoryEntry, ModelTransform, PoweredProtectionState, ProviderReference, RegularArmorState, ResourceRequest,
    SourceCounterArithmetic, WeaponHudIcon,
};
use qa_content::paths::normalize_resource_path;
use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{Axis, Vec3, Vec4};
use qa_world::save::shared::read_content_id;
use qa_world::save::value::{arr, boolean, int, namespaced, num, obj, str as json_str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::types::{
    ArsenalWarning, NativeCameraBlock, NativeCameraEdition, NativeInventory, NativeInventoryItem, NativeInventoryKind,
    NativeInventoryPresentation, NativeModCameraView, PlayerAmmo, PlayerArsenalItem, PlayerArsenalKind, PlayerPowerup,
    PlayerUi, PlayerView, PresentationFamily, PresentationFlare, PresentationFovOffset, PresentationGrappleCable,
    PresentationIndexedSkin, PresentationModel, PresentationModelAnchor, PresentationModelAttachment,
    PresentationModelBeam, PresentationPlayerColors, PresentationQ3Weapon, PresentationShaderBeam, WeaponAmmoStatus,
    WeaponStatus, WorldText, WorldTextFont, WorldTextOrientation,
};
use super::unified_types::{
    UnifiedCharacterAnimation, UnifiedCharacterTeam, UnifiedCharacterView, UnifiedIdentityDecoder,
};

/// Encode an actor reference for the wire (`wireActor`).
#[must_use]
pub fn wire_actor(actor: &ActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot()))),
        ("generation", int(i64::from(actor.generation()))),
    ])
}

fn wire_u32(reader: SaveReader, name: &str) -> Result<u32, WorldError> {
    let value = reader.field(name).integer(0)?;
    u32::try_from(value).map_err(|_| reader.field(name).fail("actor reference exceeds its range"))
}

/// Resolve a wire actor reference (`actor`).
pub fn read_actor(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<ActorId, WorldError> {
    Ok(identity.actor(wire_u32(reader.clone(), "slot")?, wire_u32(reader, "generation")?))
}

/// Read a vector (`vector`).
#[allow(clippy::cast_possible_truncation)]
pub fn read_vector(reader: SaveReader) -> Result<Vec3, WorldError> {
    Ok(Vec3 {
        x: reader.field("x").finite()? as f32,
        y: reader.field("y").finite()? as f32,
        z: reader.field("z").finite()? as f32,
    })
}

/// Read a color (`color`).
#[allow(clippy::cast_possible_truncation)]
pub fn read_color(reader: SaveReader) -> Result<Vec4, WorldError> {
    Ok(Vec4 {
        x: reader.field("x").finite()? as f32,
        y: reader.field("y").finite()? as f32,
        z: reader.field("z").finite()? as f32,
        w: reader.field("w").finite()? as f32,
    })
}

/// Read three basis axes (`axis`).
pub fn read_axis(reader: SaveReader) -> Result<Axis, WorldError> {
    let values = reader.list(read_vector)?;
    if values.len() != 3 {
        return Err(reader.fail("expected three axes"));
    }
    Ok([values[0], values[1], values[2]])
}

fn optional<T>(
    reader: &SaveReader,
    name: &str,
    read: impl FnOnce(SaveReader) -> Result<T, WorldError>,
) -> Result<Option<T>, WorldError> {
    if reader.field(name).is_missing() {
        Ok(None)
    } else {
        read(reader.field(name)).map(Some)
    }
}

fn read_provider(reader: SaveReader) -> Result<ProviderReference, WorldError> {
    let provider = namespaced(reader.field("provider"))?;
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    if namespace.is_empty() || name.is_empty() {
        return Err(reader.field("provider").fail("invalid provider reference"));
    }
    let content = read_content_id(reader.field("content"))?;
    Ok(ProviderReference {
        provider: ProviderId::new(namespace, name),
        content: ContentId(content),
    })
}

fn read_content(reader: SaveReader) -> Result<ContentId, WorldError> {
    let content = read_content_id(reader)?;
    Ok(ContentId(content))
}

/// Read armor into the shared `qa-content` contract shape.
///
/// The wire layout matches [`qa_world::save::records::read_armor`] exactly
/// (including the legacy flat Q2 layout); only the target types differ.
fn read_contract_armor(reader: SaveReader) -> Result<ArmorState, WorldError> {
    fn regular(reader: SaveReader) -> Result<RegularArmorState, WorldError> {
        let kind = reader.field("kind").choice_str(&["none", "q1", "q2", "q3", "source"])?;
        match kind.as_str() {
            "none" => Ok(RegularArmorState::None),
            "source" => Ok(RegularArmorState::Source {
                points: reader.field("points").number()?,
                item: reader.field("item").nullable(namespaced)?,
            }),
            "q1" => Ok(RegularArmorState::Q1 {
                points: reader.field("points").number()?,
                absorption: reader.field("absorption").number()?,
                item: namespaced(reader.field("item"))?,
            }),
            "q3" => Ok(RegularArmorState::Q3 {
                points: reader.field("points").number()?,
                protection: reader.field("protection").number()?,
            }),
            _ => Ok(RegularArmorState::Q2 {
                points: reader.field("points").number()?,
                normal_protection: reader.field("normalProtection").number()?,
                energy_protection: reader.field("energyProtection").number()?,
                item: namespaced(reader.field("item"))?,
            }),
        }
    }
    fn powered(reader: SaveReader) -> Result<PoweredProtectionState, WorldError> {
        let kind = reader.field("kind").choice_str(&["none", "screen", "shield"])?;
        match kind.as_str() {
            "screen" => Ok(PoweredProtectionState::Screen {
                cells: reader.field("cells").number()?,
            }),
            "shield" => Ok(PoweredProtectionState::Shield {
                cells: reader.field("cells").number()?,
            }),
            _ => Ok(PoweredProtectionState::None),
        }
    }
    if !reader.field("regular").is_missing() {
        return Ok(ArmorState {
            regular: regular(reader.field("regular"))?,
            powered: powered(reader.field("powered"))?,
        });
    }
    let state = regular(reader.clone())?;
    let protection = if matches!(state, RegularArmorState::Q2 { .. }) {
        powered(reader.field("powerArmor"))?
    } else {
        PoweredProtectionState::None
    };
    Ok(ArmorState {
        regular: state,
        powered: protection,
    })
}

/// Read one inventory entry into the shared `qa-content` contract shape.
fn read_contract_inventory_entry(reader: SaveReader) -> Result<InventoryEntry, WorldError> {
    let policy = reader.field("countPolicy");
    let count_policy = if policy.is_missing() {
        None
    } else {
        let kind = policy.field("kind").choice_str(&["stack", "source-counter"])?;
        if kind == "stack" {
            Some(InventoryCountPolicy::Stack)
        } else {
            let arithmetic = policy
                .field("arithmetic")
                .choice_str(&["binary32", "binary64", "int32"])?;
            Some(InventoryCountPolicy::SourceCounter(match arithmetic.as_str() {
                "binary32" => SourceCounterArithmetic::Binary32,
                "binary64" => SourceCounterArithmetic::Binary64,
                _ => SourceCounterArithmetic::Int32,
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

/// Read a player view (`readPlayerView`).
pub fn read_player_view(reader: SaveReader) -> Result<PlayerView, WorldError> {
    let drift = optional(&reader, "pitchDrift", |drift| {
        Ok(super::types::PlayerPitchDrift {
            grounded: drift.field("grounded").boolean()?,
            ideal_pitch: drift.field("idealPitch").finite()?,
            disabled: drift.field("disabled").boolean()?,
        })
    })?;
    Ok(PlayerView {
        origin: read_vector(reader.field("origin"))?,
        angles: read_vector(reader.field("angles"))?,
        view_height: reader.field("viewHeight").finite()?,
        blend: optional(&reader, "blend", read_color)?,
        damage_blend: optional(&reader, "damageBlend", read_color)?,
        kick_angles: optional(&reader, "kickAngles", read_vector)?,
        field_of_view: optional(&reader, "fieldOfView", |value| value.finite())?,
        client_view_offset_delta: optional(&reader, "clientViewOffsetDelta", read_vector)?,
        foreign_character_death: optional(&reader, "foreignCharacterDeath", |value| value.literal_bool(true))?,
        pitch_drift: drift,
    })
}

fn dot3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross_z(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let cross = Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    };
    dot3(cross, c)
}

fn read_grip(grip: SaveReader) -> Result<ModelTransform, WorldError> {
    let axis = read_axis(grip.field("axis"))?;
    let orthonormal = axis.iter().all(|value| (dot3(*value, *value) - 1.0).abs() <= 0.001)
        && dot3(axis[0], axis[1]).abs() <= 0.001
        && dot3(axis[0], axis[2]).abs() <= 0.001
        && dot3(axis[1], axis[2]).abs() <= 0.001
        && cross_z(axis[0], axis[1], axis[2]) >= 0.999;
    if !orthonormal {
        return Err(grip.fail("Model grip axes must form a rotation"));
    }
    let scale_field = grip.field("scale");
    let scale = if scale_field.is_missing() {
        Vec3 { x: 1.0, y: 1.0, z: 1.0 }
    } else {
        read_vector(scale_field)?
    };
    if scale.x == 0.0 || scale.y == 0.0 || scale.z == 0.0 {
        return Err(grip.fail("Model grip scale must be invertible"));
    }
    Ok(ModelTransform {
        origin: read_vector(grip.field("origin"))?,
        axis,
        scale,
    })
}

fn is_content_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Read a held weapon declaration (donor `readHeldWeaponDeclaration`).
pub fn read_held_weapon(reader: SaveReader) -> Result<HeldWeaponDeclaration, WorldError> {
    if reader.field("kind").choice_str(&["none", "model"])? == "none" {
        return Ok(HeldWeaponDeclaration::None);
    }
    let model = reader.field("model");
    let part = model.field("part");
    let subset = if part.is_missing() {
        None
    } else {
        let digests: Vec<String> = part.field("digests").list(|value| {
            let parsed = value.string()?;
            if !is_content_digest(&parsed) {
                return Err(value.fail("held model requires a SHA256 digest"));
            }
            Ok(parsed)
        })?;
        let vertices: Vec<i64> = part.field("vertices").list(|value| value.integer(0))?;
        let mut distinct = vertices.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if digests.is_empty() || vertices.is_empty() || distinct.len() != vertices.len() {
            return Err(part.fail("held model subset requires source digests and distinct vertices"));
        }
        #[allow(clippy::cast_precision_loss)]
        let subset = HeldWeaponPart {
            digests,
            vertices: vertices.into_iter().map(|vertex| vertex as f64).collect(),
        };
        Some(subset)
    };
    let path = normalize_resource_path(&model.field("path").string()?)
        .map_err(|error| model.field("path").fail(&error.to_string()))?;
    #[allow(clippy::cast_precision_loss)]
    let reference_frame = model.field("referenceFrame").integer(0)? as f64;
    let grip = read_grip(model.field("grip"))?;
    let digest_field = model.field("digest");
    let digest = if digest_field.is_missing() {
        None
    } else {
        let parsed = digest_field.string()?;
        if !is_content_digest(&parsed) {
            return Err(digest_field.fail("held model requires a SHA256 digest"));
        }
        Some(ContentDigest(parsed))
    };
    let fallback_field = model.field("fallback");
    let fallback = if fallback_field.is_missing() {
        None
    } else {
        Some(
            normalize_resource_path(&fallback_field.string()?)
                .map_err(|error| fallback_field.fail(&error.to_string()))?,
        )
    };
    Ok(HeldWeaponDeclaration::Model(HeldWeaponModel {
        digest,
        path,
        reference_frame,
        grip,
        fallback,
        part: subset,
    }))
}

/// Read a source item icon (donor `readSourceItemIcon`).
pub fn read_item_icon(reader: SaveReader, content: &ContentId) -> Result<WeaponHudIcon, WorldError> {
    let kind = reader.field("kind").choice_str(&["image", "wad-picture", "shader"])?;
    let resource = if kind == "shader" {
        reader.clone()
    } else {
        reader.field("resource")
    };
    if resource.field("content").string()? != content.as_str() {
        return Err(reader.fail("Item icon belongs to another content source"));
    }
    if kind == "shader" {
        let name = normalize_resource_path(&reader.field("name").string()?)
            .map_err(|error| reader.field("name").fail(&error.to_string()))?;
        return Ok(WeaponHudIcon::Shader {
            content: content.clone(),
            name,
        });
    }
    let path = normalize_resource_path(&resource.field("path").string()?)
        .map_err(|error| resource.field("path").fail(&error.to_string()))?;
    if kind == "image" {
        return Ok(WeaponHudIcon::Image {
            resource: ResourceRequest {
                content: content.clone(),
                path,
            },
        });
    }
    let lump = reader.field("lump").string()?;
    if lump.is_empty() || lump.encode_utf16().count() > 16 || lump.contains('\0') {
        return Err(reader.fail("Item icon requires a valid WAD lump name"));
    }
    Ok(WeaponHudIcon::WadPicture {
        resource: ResourceRequest {
            content: content.clone(),
            path,
        },
        lump,
    })
}

/// Read player HUD state (`readPlayerUi`).
pub fn read_player_ui(reader: SaveReader) -> Result<PlayerUi, WorldError> {
    let native_inventory = optional(&reader, "nativeInventory", |inventory| {
        let presentation = optional(&inventory, "presentation", |presentation| {
            let source = read_provider(presentation.field("source"))?;
            let kind = presentation
                .field("kind")
                .choice_str(&["weapon", "ammunition", "item"])?;
            match kind.as_str() {
                "item" => Ok(NativeInventoryPresentation {
                    source: source.clone(),
                    kind: NativeInventoryKind::Item,
                    icon: presentation
                        .field("icon")
                        .nullable(|icon| read_item_icon(icon, &source.content))?,
                    weapon: None,
                }),
                "weapon" => Ok(NativeInventoryPresentation {
                    source,
                    kind: NativeInventoryKind::Weapon,
                    icon: None,
                    weapon: Some(namespaced(presentation.field("weapon"))?),
                }),
                _ => Ok(NativeInventoryPresentation {
                    source,
                    kind: NativeInventoryKind::Ammunition,
                    icon: None,
                    weapon: Some(namespaced(presentation.field("weapon"))?),
                }),
            }
        })?;
        Ok(NativeInventory {
            items: inventory.field("items").list(|row| {
                Ok(NativeInventoryItem {
                    item: namespaced(row.field("item"))?,
                    label: row.field("label").string()?,
                    count: row.field("count").finite()?,
                })
            })?,
            selected: inventory.field("selected").nullable(namespaced)?,
            presentation,
        })
    })?;
    let items = reader.field("items").list(|row| {
        let kind = row.field("kind").choice_str(&["weapon", "powerup"])?;
        Ok(PlayerArsenalItem {
            id: namespaced(row.field("id"))?,
            label: row.field("label").string()?,
            kind: if kind == "weapon" {
                PlayerArsenalKind::Weapon
            } else {
                PlayerArsenalKind::Powerup
            },
            source_ordinal: row.field("sourceOrdinal").finite()?,
            owned: row.field("owned").boolean()?,
            has_ammo: row.field("hasAmmo").boolean()?,
            count: row.field("count").nullable(|count| count.finite())?,
            warning_count: row.field("warningCount").finite()?,
        })
    })?;
    let warning = reader.field("arsenalWarning").choice_str(&["none", "low", "empty"])?;
    let weapon_status = reader.field("weaponStatus").nullable(|status| {
        let ammo_field = status.field("ammo");
        let kind = ammo_field.field("kind").choice_str(&["unmetered", "finite"])?;
        Ok(WeaponStatus {
            source: read_provider(status.field("source"))?,
            item: namespaced(status.field("item"))?,
            label: status.field("label").string()?,
            ammo: if kind == "unmetered" {
                WeaponAmmoStatus::Unmetered
            } else {
                WeaponAmmoStatus::finite(
                    namespaced(ammo_field.field("item"))?,
                    ammo_field.field("count").finite()?,
                    ammo_field.field("hasAmmoToStart").boolean()?,
                    ammo_field.field("low").boolean()?,
                )
            },
        })
    })?;
    Ok(PlayerUi {
        selected_arsenal: optional(&reader, "selectedArsenal", |value| value.literal_bool(true))?,
        native_inventory,
        health: reader.field("health").finite()?,
        armor: read_contract_armor(reader.field("armor"))?,
        active_weapon: reader.field("activeWeapon").nullable(namespaced)?,
        ammo: reader.field("ammo").nullable(|ammo| {
            Ok(PlayerAmmo {
                item: namespaced(ammo.field("item"))?,
                count: ammo.field("count").finite()?,
            })
        })?,
        inventory: reader.field("inventory").list(read_contract_inventory_entry)?,
        arsenal_warning: match warning.as_str() {
            "low" => ArsenalWarning::Low,
            "empty" => ArsenalWarning::Empty,
            _ => ArsenalWarning::None,
        },
        powerups: reader.field("powerups").list(|row| {
            Ok(PlayerPowerup {
                item: namespaced(row.field("item"))?,
                label: row.field("label").string()?,
                remaining_seconds: row.field("remainingSeconds").finite()?,
            })
        })?,
        items,
        weapon_status,
    })
}

fn read_flare(reader: SaveReader) -> Result<PresentationFlare, WorldError> {
    Ok(PresentationFlare {
        image: reader.field("image").string()?,
        fade_start: reader.field("fadeStart").finite()?,
        fade_end: reader.field("fadeEnd").finite()?,
        scale: reader.field("scale").finite()?,
        color: read_vector(reader.field("color"))?,
        rim_color: reader.field("rimColor").nullable(read_vector)?,
        lock_angle: reader.field("lockAngle").boolean()?,
    })
}

/// Read one presentation model (`readModel`).
pub fn read_model(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<PresentationModel, WorldError> {
    let family = reader.field("family").choice_str(&["q1", "q2", "q3"])?;
    let indexed_skin = optional(&reader, "indexedSkin", |skin| {
        let width = skin.field("width").integer(1)?;
        let height = skin.field("height").integer(1)?;
        let pixels = skin.field("pixels").bytes()?;
        let expected = width.checked_mul(height).unwrap_or(i64::MAX);
        if i64::try_from(pixels.len()).ok().is_none_or(|len| len != expected) {
            return Err(skin.fail("indexed skin length differs"));
        }
        Ok(PresentationIndexedSkin {
            name: skin.field("name").string()?,
            width,
            height,
            pixels,
        })
    })?;
    let q3_grapple_cable = optional(&reader, "q3GrappleCable", |cable| {
        Ok(PresentationGrappleCable {
            owner: read_actor(cable.field("owner"), identity)?,
            owner_origin: read_vector(cable.field("ownerOrigin"))?,
            owner_angles: read_vector(cable.field("ownerAngles"))?,
            view_height: cable.field("viewHeight").finite()?,
            offhand: cable.field("offhand").boolean()?,
            attached: cable.field("attached").boolean()?,
            flight: cable.field("flight").string()?,
            pull: cable.field("pull").string()?,
            hold: cable.field("hold").string()?,
            segment_length: cable.field("segmentLength").integer(1)?,
        })
    })?;
    let q3_weapon = optional(&reader, "q3Weapon", |weapon| {
        Ok(PresentationQ3Weapon {
            time_milliseconds: weapon.field("timeMilliseconds").finite()?,
            torso_animation: weapon.field("torsoAnimation").integer(i64::MIN)?,
            last_fire_milliseconds: weapon.field("lastFireMilliseconds").nullable(|value| value.finite())?,
            firing: weapon.field("firing").boolean()?,
            horizontal_speed: weapon.field("horizontalSpeed").finite()?,
            bob_cycle: weapon.field("bobCycle").finite()?,
            weapon: weapon.field("weapon").integer(i64::MIN)?,
        })
    })?;
    Ok(PresentationModel {
        actor: read_actor(reader.field("actor"), identity)?,
        content: read_content(reader.field("content"))?,
        family: match family.as_str() {
            "q1" => PresentationFamily::Q1,
            "q2" => PresentationFamily::Q2,
            _ => PresentationFamily::Q3,
        },
        path: reader.field("path").string()?,
        frame: reader.field("frame").integer(i64::MIN)?,
        old_frame: reader.field("oldFrame").integer(i64::MIN)?,
        skin: reader.field("skin").integer(i64::MIN)?,
        effects: reader.field("effects").integer(i64::MIN)?,
        render_flags: reader.field("renderFlags").integer(i64::MIN)?,
        origin: read_vector(reader.field("origin"))?,
        angles: read_vector(reader.field("angles"))?,
        scale: reader.field("scale").finite()?,
        visible: reader.field("visible").boolean()?,
        view_weapon: reader.field("viewWeapon").boolean()?,
        replaces_body: optional(&reader, "replacesBody", |value| value.literal_bool(true))?,
        render_owner: optional(&reader, "renderOwner", |value| {
            value.literal_str("source-client")?;
            Ok(true)
        })?,
        held_weapon: optional(&reader, "heldWeapon", read_held_weapon)?,
        native_held_weapon: optional(&reader, "nativeHeldWeapon", |value| value.literal_bool(true))?,
        weapon_item: optional(&reader, "weaponItem", namespaced)?,
        flare: optional(&reader, "flare", read_flare)?,
        back_lerp: optional(&reader, "backLerp", |value| value.finite())?,
        skin_path: optional(&reader, "skinPath", |value| value.nullable(|skin| skin.string()))?,
        indexed_skin,
        player_colors: optional(&reader, "playerColors", |colors| {
            Ok(PresentationPlayerColors {
                top: colors.field("top").integer(i64::MIN)?,
                bottom: colors.field("bottom").integer(i64::MIN)?,
            })
        })?,
        previous_origin: optional(&reader, "previousOrigin", read_vector)?,
        model_beam: optional(&reader, "modelBeam", |beam| {
            Ok(PresentationModelBeam {
                segment_length: beam.field("segmentLength").finite()?,
            })
        })?,
        shader_beam: optional(&reader, "shaderBeam", |beam| {
            Ok(PresentationShaderBeam {
                path: beam.field("path").string()?,
                end: read_vector(beam.field("end"))?,
                width: beam.field("width").integer(1)?,
            })
        })?,
        model_attachments: optional(&reader, "modelAttachments", |attachments| {
            attachments.list(|entry| {
                Ok(PresentationModelAttachment {
                    path: entry.field("path").string()?,
                    tag: entry.field("tag").string()?,
                })
            })
        })?,
        model_anchor: optional(&reader, "modelAnchor", |anchor| {
            let fov = anchor.field("fovOffset");
            Ok(PresentationModelAnchor {
                path: anchor.field("path").string()?,
                tag: anchor.field("tag").string()?,
                offset: read_vector(anchor.field("offset"))?,
                fov_offset: PresentationFovOffset {
                    above: fov.field("above").integer(1)?,
                    scale: fov.field("scale").finite()?,
                },
            })
        })?,
        q3_grapple_cable,
        alpha: optional(&reader, "alpha", |value| value.finite())?,
        q3_weapon,
    })
}

/// Read a character view (`readCharacterView`).
pub fn read_character_view(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedCharacterView, WorldError> {
    let animation = reader.field("animation");
    animation.field("kind").literal_str("q3")?;
    Ok(UnifiedCharacterView {
        actor: read_actor(reader.field("actor"), identity)?,
        origin: read_vector(reader.field("origin"))?,
        angles: read_vector(reader.field("angles"))?,
        velocity: read_vector(reader.field("velocity"))?,
        movement_direction: reader.field("movementDirection").integer(i64::MIN)?,
        animation: UnifiedCharacterAnimation {
            legs: animation.field("legs").finite()?,
            torso: animation.field("torso").finite()?,
            legs_timer_milliseconds: animation.field("legsTimerMilliseconds").finite()?,
            torso_timer_milliseconds: animation.field("torsoTimerMilliseconds").finite()?,
        },
        source_flags: reader.field("sourceFlags").integer(i64::MIN)?,
        powerups: reader.field("powerups").integer(i64::MIN)?,
        team: reader.field("team").nullable(|team| {
            Ok(match team.choice_str(&["red", "blue"])?.as_str() {
                "red" => UnifiedCharacterTeam::Red,
                _ => UnifiedCharacterTeam::Blue,
            })
        })?,
        color: read_color(reader.field("color"))?,
        scale: optional(&reader, "scale", |value| value.finite())?,
        opacity: optional(&reader, "opacity", |value| value.finite())?,
    })
}

/// Read world text (`readWorldText`).
pub fn read_world_text(reader: SaveReader) -> Result<WorldText, WorldError> {
    let orientation = reader.field("orientation");
    let kind = orientation.field("kind").choice_str(&["billboard", "fixed"])?;
    let font = reader.field("font").choice_str(&["classic", "selected"])?;
    Ok(WorldText {
        content: read_content(reader.field("content"))?,
        text: reader.field("text").string()?,
        origin: read_vector(reader.field("origin"))?,
        color: read_color(reader.field("color"))?,
        cell_size: reader.field("cellSize").finite()?,
        orientation: if kind == "billboard" {
            WorldTextOrientation::Billboard
        } else {
            WorldTextOrientation::Fixed {
                angles: read_vector(orientation.field("angles"))?,
            }
        },
        depth_test: reader.field("depthTest").boolean()?,
        font: if font == "classic" {
            WorldTextFont::Classic
        } else {
            WorldTextFont::Selected
        },
        distance_cull_factor: optional(&reader, "distanceCullFactor", |value| value.finite())?,
    })
}

/// Read a native camera view (`readNativeCameraView`).
pub fn read_native_camera_view(reader: SaveReader) -> Result<NativeModCameraView, WorldError> {
    let native = reader.field("native");
    let edition = native.field("edition").choice_str(&["classic", "rerelease"])?;
    let view = read_player_view(reader.clone())?;
    if edition == "classic" && view.damage_blend.is_some() {
        return Err(reader.fail("Classic camera cannot publish rerelease damage blend"));
    }
    Ok(NativeModCameraView {
        view,
        native: NativeCameraBlock {
            edition: if edition == "classic" {
                NativeCameraEdition::Classic
            } else {
                NativeCameraEdition::Rerelease
            },
            movement_origin: read_vector(native.field("movementOrigin"))?,
            render_flags: native.field("renderFlags").integer(0)?,
            position_prediction: native.field("positionPrediction").boolean()?,
            angular_prediction: native.field("angularPrediction").boolean()?,
            weapon_visible: native.field("weaponVisible").boolean()?,
        },
    })
}

/// Write a vector for the wire.
#[must_use]
pub fn write_vector(value: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
    ])
}

/// Write a color for the wire.
#[must_use]
pub fn write_color(value: Vec4) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
        ("w", num(f64::from(value.w))),
    ])
}

/// Write a player view for the wire (mirrors `read_player_view`).
#[must_use]
pub fn write_player_view(view: &PlayerView) -> SaveJson {
    let mut members = vec![
        ("origin", write_vector(view.origin)),
        ("angles", write_vector(view.angles)),
        ("viewHeight", num(view.view_height)),
    ];
    if let Some(blend) = view.blend {
        members.push(("blend", write_color(blend)));
    }
    if let Some(blend) = view.damage_blend {
        members.push(("damageBlend", write_color(blend)));
    }
    if let Some(kick) = view.kick_angles {
        members.push(("kickAngles", write_vector(kick)));
    }
    if let Some(fov) = view.field_of_view {
        members.push(("fieldOfView", num(fov)));
    }
    if let Some(delta) = view.client_view_offset_delta {
        members.push(("clientViewOffsetDelta", write_vector(delta)));
    }
    if let Some(death) = view.foreign_character_death {
        members.push(("foreignCharacterDeath", boolean(death)));
    }
    if let Some(drift) = &view.pitch_drift {
        members.push((
            "pitchDrift",
            obj(vec![
                ("grounded", boolean(drift.grounded)),
                ("idealPitch", num(drift.ideal_pitch)),
                ("disabled", boolean(drift.disabled)),
            ]),
        ));
    }
    obj(members)
}

/// Write a native camera view for the wire (mirrors `read_native_camera_view`).
#[must_use]
pub fn write_native_camera_view(view: &NativeModCameraView) -> SaveJson {
    let SaveJson::Object(mut members) = write_player_view(&view.view) else {
        unreachable!("player view always encodes as an object");
    };
    let edition = if view.native.edition == NativeCameraEdition::Classic {
        "classic"
    } else {
        "rerelease"
    };
    members.push((
        "native".to_string(),
        obj(vec![
            ("edition", json_str(edition)),
            ("movementOrigin", write_vector(view.native.movement_origin)),
            ("renderFlags", int(view.native.render_flags)),
            ("positionPrediction", boolean(view.native.position_prediction)),
            ("angularPrediction", boolean(view.native.angular_prediction)),
            ("weaponVisible", boolean(view.native.weapon_visible)),
        ]),
    ));
    SaveJson::Object(members)
}

/// Write a provider reference for the wire (mirrors the local provider read).
#[must_use]
pub fn write_provider(value: &ProviderReference) -> SaveJson {
    obj(vec![
        (
            "provider",
            json_str(&format!("{}:{}", value.provider.namespace, value.provider.name)),
        ),
        ("content", json_str(value.content.as_str())),
    ])
}

/// Write regular armor for the wire (canonical nested layout).
#[must_use]
pub fn write_regular_armor(value: &qa_content::contract::RegularArmorState) -> SaveJson {
    use qa_content::contract::RegularArmorState as Regular;
    match value {
        Regular::None => obj(vec![("kind", json_str("none"))]),
        Regular::Source { points, item } => obj(vec![
            ("kind", json_str("source")),
            ("points", num(*points)),
            ("item", item.as_ref().map_or(SaveJson::Null, |item| json_str(item))),
        ]),
        Regular::Q1 {
            points,
            absorption,
            item,
        } => obj(vec![
            ("kind", json_str("q1")),
            ("points", num(*points)),
            ("absorption", num(*absorption)),
            ("item", json_str(item)),
        ]),
        Regular::Q3 { points, protection } => obj(vec![
            ("kind", json_str("q3")),
            ("points", num(*points)),
            ("protection", num(*protection)),
        ]),
        Regular::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        } => obj(vec![
            ("kind", json_str("q2")),
            ("points", num(*points)),
            ("normalProtection", num(*normal_protection)),
            ("energyProtection", num(*energy_protection)),
            ("item", json_str(item)),
        ]),
    }
}

/// Write powered protection for the wire.
#[must_use]
pub fn write_powered_protection(value: &qa_content::contract::PoweredProtectionState) -> SaveJson {
    use qa_content::contract::PoweredProtectionState as Powered;
    match value {
        Powered::None => obj(vec![("kind", json_str("none"))]),
        Powered::Screen { cells } => obj(vec![("kind", json_str("screen")), ("cells", num(*cells))]),
        Powered::Shield { cells } => obj(vec![("kind", json_str("shield")), ("cells", num(*cells))]),
    }
}

/// Write armor for the wire.
#[must_use]
pub fn write_contract_armor(value: &qa_content::contract::ArmorState) -> SaveJson {
    obj(vec![
        ("regular", write_regular_armor(&value.regular)),
        ("powered", write_powered_protection(&value.powered)),
    ])
}

/// Write an inventory entry for the wire.
#[must_use]
pub fn write_contract_inventory_entry(value: &qa_content::contract::InventoryEntry) -> SaveJson {
    let mut members = vec![
        ("item", json_str(&value.item)),
        ("count", num(value.count)),
        ("capacity", num(value.capacity)),
    ];
    if let Some(policy) = &value.count_policy {
        members.push((
            "countPolicy",
            match policy {
                InventoryCountPolicy::Stack => obj(vec![("kind", json_str("stack"))]),
                InventoryCountPolicy::SourceCounter(arithmetic) => obj(vec![
                    ("kind", json_str("source-counter")),
                    (
                        "arithmetic",
                        json_str(match arithmetic {
                            SourceCounterArithmetic::Binary32 => "binary32",
                            SourceCounterArithmetic::Binary64 => "binary64",
                            SourceCounterArithmetic::Int32 => "int32",
                        }),
                    ),
                ]),
            },
        ));
    }
    obj(members)
}

/// Write an item icon for the wire (mirrors `read_item_icon`).
#[must_use]
pub fn write_item_icon(value: &WeaponHudIcon) -> SaveJson {
    match value {
        WeaponHudIcon::Image { resource } => obj(vec![
            ("kind", json_str("image")),
            (
                "resource",
                obj(vec![
                    ("content", json_str(resource.content.as_str())),
                    ("path", json_str(&resource.path)),
                ]),
            ),
        ]),
        WeaponHudIcon::WadPicture { resource, lump } => obj(vec![
            ("kind", json_str("wad-picture")),
            (
                "resource",
                obj(vec![
                    ("content", json_str(resource.content.as_str())),
                    ("path", json_str(&resource.path)),
                ]),
            ),
            ("lump", json_str(lump)),
        ]),
        WeaponHudIcon::Shader { content, name } => obj(vec![
            ("kind", json_str("shader")),
            ("content", json_str(content.as_str())),
            ("name", json_str(name)),
        ]),
    }
}

/// Write a model grip for the wire.
#[must_use]
pub fn write_grip(value: &ModelTransform) -> SaveJson {
    obj(vec![
        (
            "axis",
            arr(vec![
                write_vector(value.axis[0]),
                write_vector(value.axis[1]),
                write_vector(value.axis[2]),
            ]),
        ),
        ("scale", write_vector(value.scale)),
        ("origin", write_vector(value.origin)),
    ])
}

/// Write a held weapon declaration for the wire.
#[must_use]
pub fn write_held_weapon(value: &HeldWeaponDeclaration) -> SaveJson {
    match value {
        HeldWeaponDeclaration::None => obj(vec![("kind", json_str("none"))]),
        HeldWeaponDeclaration::Model(model) => {
            let mut members = vec![
                ("path", json_str(&model.path)),
                ("referenceFrame", num(model.reference_frame)),
                ("grip", write_grip(&model.grip)),
            ];
            if let Some(digest) = &model.digest {
                members.push(("digest", json_str(digest.as_str())));
            }
            if let Some(fallback) = &model.fallback {
                members.push(("fallback", json_str(fallback)));
            }
            if let Some(part) = &model.part {
                members.push((
                    "part",
                    obj(vec![
                        (
                            "digests",
                            arr(part.digests.iter().map(|digest| json_str(digest)).collect()),
                        ),
                        ("vertices", arr(part.vertices.iter().copied().map(num).collect())),
                    ]),
                ));
            }
            obj(vec![("kind", json_str("model")), ("model", obj(members))])
        }
    }
}

/// Write player HUD state for the wire (mirrors `read_player_ui`).
#[must_use]
pub fn write_player_ui(value: &PlayerUi) -> SaveJson {
    let mut members: Vec<(&str, SaveJson)> = Vec::new();
    if let Some(selected) = value.selected_arsenal {
        members.push(("selectedArsenal", boolean(selected)));
    }
    if let Some(inventory) = value.native_inventory.as_ref() {
        let mut native = vec![
            (
                "items",
                arr(inventory
                    .items
                    .iter()
                    .map(|item| {
                        obj(vec![
                            ("item", json_str(&item.item)),
                            ("label", json_str(&item.label)),
                            ("count", num(item.count)),
                        ])
                    })
                    .collect()),
            ),
            (
                "selected",
                inventory
                    .selected
                    .as_ref()
                    .map_or(SaveJson::Null, |selected| json_str(selected)),
            ),
        ];
        if let Some(presentation) = inventory.presentation.as_ref() {
            native.push(("presentation", {
                let mut members = vec![
                    ("source", write_provider(&presentation.source)),
                    (
                        "kind",
                        json_str(match presentation.kind {
                            NativeInventoryKind::Weapon => "weapon",
                            NativeInventoryKind::Ammunition => "ammunition",
                            NativeInventoryKind::Item => "item",
                        }),
                    ),
                ];
                match presentation.kind {
                    NativeInventoryKind::Item => {
                        members.push((
                            "icon",
                            presentation.icon.as_ref().map_or(SaveJson::Null, write_item_icon),
                        ));
                    }
                    _ => {
                        if let Some(weapon) = &presentation.weapon {
                            members.push(("weapon", json_str(weapon)));
                        }
                    }
                }
                obj(members)
            }));
        }
        members.push(("nativeInventory", obj(native)));
    }
    members.push(("health", num(value.health)));
    members.extend([
        ("armor", write_contract_armor(&value.armor)),
        (
            "activeWeapon",
            value
                .active_weapon
                .as_ref()
                .map_or(SaveJson::Null, |weapon| json_str(weapon)),
        ),
        (
            "ammo",
            value.ammo.as_ref().map_or(SaveJson::Null, |ammo| {
                obj(vec![("item", json_str(&ammo.item)), ("count", num(ammo.count))])
            }),
        ),
        (
            "inventory",
            arr(value.inventory.iter().map(write_contract_inventory_entry).collect()),
        ),
        (
            "arsenalWarning",
            json_str(match value.arsenal_warning {
                ArsenalWarning::Low => "low",
                ArsenalWarning::Empty => "empty",
                ArsenalWarning::None => "none",
            }),
        ),
        (
            "powerups",
            arr(value
                .powerups
                .iter()
                .map(|powerup| {
                    obj(vec![
                        ("item", json_str(&powerup.item)),
                        ("label", json_str(&powerup.label)),
                        ("remainingSeconds", num(powerup.remaining_seconds)),
                    ])
                })
                .collect()),
        ),
        (
            "items",
            arr(value
                .items
                .iter()
                .map(|item| {
                    obj(vec![
                        ("id", json_str(&item.id)),
                        ("label", json_str(&item.label)),
                        (
                            "kind",
                            json_str(match item.kind {
                                PlayerArsenalKind::Weapon => "weapon",
                                PlayerArsenalKind::Powerup => "powerup",
                            }),
                        ),
                        ("sourceOrdinal", num(item.source_ordinal)),
                        ("owned", boolean(item.owned)),
                        ("hasAmmo", boolean(item.has_ammo)),
                        ("count", item.count.map_or(SaveJson::Null, num)),
                        ("warningCount", num(item.warning_count)),
                    ])
                })
                .collect()),
        ),
        (
            "weaponStatus",
            value.weapon_status.as_ref().map_or(SaveJson::Null, |status| {
                obj(vec![
                    ("source", write_provider(&status.source)),
                    ("item", json_str(&status.item)),
                    ("label", json_str(&status.label)),
                    (
                        "ammo",
                        match &status.ammo {
                            WeaponAmmoStatus::Unmetered => obj(vec![("kind", json_str("unmetered"))]),
                            WeaponAmmoStatus::Finite { .. } => {
                                let (item, count, has_ammo_to_start, low) =
                                    status.ammo.finite_parts().expect("finite ammo has parts");
                                obj(vec![
                                    ("kind", json_str("finite")),
                                    ("item", json_str(item)),
                                    ("count", num(count)),
                                    ("hasAmmoToStart", boolean(has_ammo_to_start)),
                                    ("low", boolean(low)),
                                ])
                            }
                        },
                    ),
                ])
            }),
        ),
    ]);
    obj(members)
}

/// Write a presentation model for the wire (mirrors `read_model`).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn write_presentation_model(value: &PresentationModel) -> SaveJson {
    let mut members = vec![
        ("actor", wire_actor(&value.actor)),
        ("content", json_str(value.content.as_str())),
        (
            "family",
            json_str(match value.family {
                PresentationFamily::Q1 => "q1",
                PresentationFamily::Q2 => "q2",
                PresentationFamily::Q3 => "q3",
            }),
        ),
        ("path", json_str(&value.path)),
        ("frame", int(value.frame)),
        ("oldFrame", int(value.old_frame)),
        ("skin", int(value.skin)),
        ("effects", int(value.effects)),
        ("renderFlags", int(value.render_flags)),
        ("origin", write_vector(value.origin)),
        ("angles", write_vector(value.angles)),
        ("scale", num(value.scale)),
        ("visible", boolean(value.visible)),
        ("viewWeapon", boolean(value.view_weapon)),
    ];
    if let Some(replaces) = value.replaces_body {
        members.push(("replacesBody", boolean(replaces)));
    }
    if value.render_owner.is_some() {
        members.push(("renderOwner", json_str("source-client")));
    }
    if let Some(held) = &value.held_weapon {
        members.push(("heldWeapon", write_held_weapon(held)));
    }
    if let Some(native) = value.native_held_weapon {
        members.push(("nativeHeldWeapon", boolean(native)));
    }
    if let Some(item) = &value.weapon_item {
        members.push(("weaponItem", json_str(item)));
    }
    if let Some(flare) = &value.flare {
        members.push((
            "flare",
            obj(vec![
                ("image", json_str(&flare.image)),
                ("fadeStart", num(flare.fade_start)),
                ("fadeEnd", num(flare.fade_end)),
                ("scale", num(flare.scale)),
                ("color", write_vector(flare.color)),
                ("rimColor", flare.rim_color.map_or(SaveJson::Null, write_vector)),
                ("lockAngle", boolean(flare.lock_angle)),
            ]),
        ));
    }
    if let Some(back_lerp) = value.back_lerp {
        members.push(("backLerp", num(back_lerp)));
    }
    if let Some(skin_path) = &value.skin_path {
        members.push((
            "skinPath",
            skin_path.as_ref().map_or(SaveJson::Null, |path| json_str(path)),
        ));
    }
    if let Some(skin) = &value.indexed_skin {
        members.push((
            "indexedSkin",
            obj(vec![
                ("name", json_str(&skin.name)),
                ("width", int(skin.width)),
                ("height", int(skin.height)),
                ("pixels", SaveJson::Bytes(skin.pixels.clone())),
            ]),
        ));
    }
    if let Some(colors) = &value.player_colors {
        members.push((
            "playerColors",
            obj(vec![("top", int(colors.top)), ("bottom", int(colors.bottom))]),
        ));
    }
    if let Some(origin) = value.previous_origin {
        members.push(("previousOrigin", write_vector(origin)));
    }
    if let Some(beam) = &value.model_beam {
        members.push(("modelBeam", obj(vec![("segmentLength", num(beam.segment_length))])));
    }
    if let Some(beam) = &value.shader_beam {
        members.push((
            "shaderBeam",
            obj(vec![
                ("path", json_str(&beam.path)),
                ("end", write_vector(beam.end)),
                ("width", int(beam.width)),
            ]),
        ));
    }
    if let Some(attachments) = &value.model_attachments {
        members.push((
            "modelAttachments",
            arr(attachments
                .iter()
                .map(|entry| obj(vec![("path", json_str(&entry.path)), ("tag", json_str(&entry.tag))]))
                .collect()),
        ));
    }
    if let Some(anchor) = &value.model_anchor {
        members.push((
            "modelAnchor",
            obj(vec![
                ("path", json_str(&anchor.path)),
                ("tag", json_str(&anchor.tag)),
                ("offset", write_vector(anchor.offset)),
                (
                    "fovOffset",
                    obj(vec![
                        ("above", int(anchor.fov_offset.above)),
                        ("scale", num(anchor.fov_offset.scale)),
                    ]),
                ),
            ]),
        ));
    }
    if let Some(cable) = &value.q3_grapple_cable {
        members.push((
            "q3GrappleCable",
            obj(vec![
                ("owner", wire_actor(&cable.owner)),
                ("ownerOrigin", write_vector(cable.owner_origin)),
                ("ownerAngles", write_vector(cable.owner_angles)),
                ("viewHeight", num(cable.view_height)),
                ("offhand", boolean(cable.offhand)),
                ("attached", boolean(cable.attached)),
                ("flight", json_str(&cable.flight)),
                ("pull", json_str(&cable.pull)),
                ("hold", json_str(&cable.hold)),
                ("segmentLength", int(cable.segment_length)),
            ]),
        ));
    }
    if let Some(alpha) = value.alpha {
        members.push(("alpha", num(alpha)));
    }
    if let Some(weapon) = &value.q3_weapon {
        members.push((
            "q3Weapon",
            obj(vec![
                ("timeMilliseconds", num(weapon.time_milliseconds)),
                ("torsoAnimation", int(weapon.torso_animation)),
                (
                    "lastFireMilliseconds",
                    weapon.last_fire_milliseconds.map_or(SaveJson::Null, num),
                ),
                ("firing", boolean(weapon.firing)),
                ("horizontalSpeed", num(weapon.horizontal_speed)),
                ("bobCycle", num(weapon.bob_cycle)),
                ("weapon", int(weapon.weapon)),
            ]),
        ));
    }
    obj(members)
}

/// Write a character view for the wire (mirrors `read_character_view`).
#[must_use]
pub fn write_character_view(value: &UnifiedCharacterView) -> SaveJson {
    let mut members = vec![
        ("actor", wire_actor(&value.actor)),
        ("origin", write_vector(value.origin)),
        ("angles", write_vector(value.angles)),
        ("velocity", write_vector(value.velocity)),
        ("movementDirection", int(value.movement_direction)),
        (
            "animation",
            obj(vec![
                ("kind", json_str("q3")),
                ("legs", num(value.animation.legs)),
                ("torso", num(value.animation.torso)),
                ("legsTimerMilliseconds", num(value.animation.legs_timer_milliseconds)),
                ("torsoTimerMilliseconds", num(value.animation.torso_timer_milliseconds)),
            ]),
        ),
        ("sourceFlags", int(value.source_flags)),
        ("powerups", int(value.powerups)),
        (
            "team",
            value.team.map_or(SaveJson::Null, |team| {
                json_str(match team {
                    UnifiedCharacterTeam::Red => "red",
                    UnifiedCharacterTeam::Blue => "blue",
                })
            }),
        ),
        ("color", write_color(value.color)),
    ];
    if let Some(scale) = value.scale {
        members.push(("scale", num(scale)));
    }
    if let Some(opacity) = value.opacity {
        members.push(("opacity", num(opacity)));
    }
    obj(members)
}

/// Write world text for the wire (mirrors `read_world_text`).
#[must_use]
pub fn write_world_text(value: &WorldText) -> SaveJson {
    let mut members = vec![
        ("content", json_str(value.content.as_str())),
        ("text", json_str(&value.text)),
        ("origin", write_vector(value.origin)),
        ("color", write_color(value.color)),
        ("cellSize", num(value.cell_size)),
        (
            "orientation",
            match value.orientation {
                WorldTextOrientation::Billboard => obj(vec![("kind", json_str("billboard"))]),
                WorldTextOrientation::Fixed { angles } => {
                    obj(vec![("kind", json_str("fixed")), ("angles", write_vector(angles))])
                }
            },
        ),
        ("depthTest", boolean(value.depth_test)),
        (
            "font",
            json_str(match value.font {
                WorldTextFont::Classic => "classic",
                WorldTextFont::Selected => "selected",
            }),
        ),
    ];
    if let Some(factor) = value.distance_cull_factor {
        members.push(("distanceCullFactor", num(factor)));
    }
    obj(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_world::save::shared::validate_content_id;
    use qa_world::save::value::{arr, boolean, num};

    fn test_player_view_json(origin: (f64, f64, f64)) -> SaveJson {
        obj(vec![
            (
                "origin",
                obj(vec![("x", num(origin.0)), ("y", num(origin.1)), ("z", num(origin.2))]),
            ),
            ("angles", obj(vec![("x", num(0.0)), ("y", num(0.0)), ("z", num(0.0))])),
            ("viewHeight", num(22.0)),
        ])
    }

    use super::super::unified_types::UnifiedIdentityDecoder as Decoder;
    use qa_core::identity::{ClientId, SeatId, SessionId};

    struct Ledger {
        owner: IdentityOwner,
    }

    impl Decoder for Ledger {
        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
        fn actor(&self, slot: u32, generation: u32) -> ActorId {
            self.owner.actor(slot, generation)
        }
        fn client(&self, slot: u32, generation: u32) -> ClientId {
            self.owner.client(slot, generation)
        }
        fn seat(&self, index: u32) -> SeatId {
            self.owner.seat(index)
        }
        fn resource_id(&self, id: &str) -> String {
            id.to_string()
        }
    }

    fn ledger() -> Ledger {
        Ledger {
            owner: IdentityOwner::create("frame-values").unwrap(),
        }
    }

    fn vec_json(x: f64, y: f64, z: f64) -> SaveJson {
        obj(vec![("x", num(x)), ("y", num(y)), ("z", num(z))])
    }

    fn actor_json(slot: i64, generation: i64) -> SaveJson {
        obj(vec![("slot", num(slot as f64)), ("generation", num(generation as f64))])
    }

    #[test]
    fn wire_actor_round_trips() {
        let ledger = ledger();
        let actor = ledger.actor(5, 9);
        let encoded = wire_actor(&actor);
        let reader = SaveReader::new(&encoded);
        assert_eq!(read_actor(reader, &ledger).unwrap(), actor);
    }

    #[test]
    fn actor_rejects_out_of_range_slot() {
        let ledger = ledger();
        let encoded = actor_json(9_007_199_254_740_992, 0);
        let reader = SaveReader::new(&encoded);
        assert!(read_actor(reader, &ledger).is_err());
    }

    #[test]
    fn axis_requires_three_vectors() {
        let encoded = arr(vec![vec_json(1.0, 0.0, 0.0), vec_json(0.0, 1.0, 0.0)]);
        let reader = SaveReader::new(&encoded);
        assert!(read_axis(reader).is_err());
        let encoded = arr(vec![
            vec_json(1.0, 0.0, 0.0),
            vec_json(0.0, 1.0, 0.0),
            vec_json(0.0, 0.0, 1.0),
        ]);
        let reader = SaveReader::new(&encoded);
        let axis = read_axis(reader).unwrap();
        assert_eq!(axis[2].z, 1.0);
    }

    #[test]
    fn player_view_reads_optional_fields() {
        let mut view = test_player_view_json((1.0, 2.0, 3.0));
        if let SaveJson::Object(members) = &mut view {
            members.push(("fieldOfView".to_string(), num(90.0)));
            members.push((
                "pitchDrift".to_string(),
                obj(vec![
                    ("grounded", boolean(true)),
                    ("idealPitch", num(0.0)),
                    ("disabled", boolean(false)),
                ]),
            ));
        }
        let reader = SaveReader::new(&view);
        let view = read_player_view(reader).unwrap();
        assert_eq!(view.origin.x, 1.0);
        assert_eq!(view.field_of_view, Some(90.0));
        assert!(view.pitch_drift.unwrap().grounded);
    }

    #[test]
    fn classic_camera_rejects_damage_blend() {
        let mut view = test_player_view_json((0.0, 0.0, 0.0));
        if let SaveJson::Object(members) = &mut view {
            members.push((
                "damageBlend".to_string(),
                obj(vec![("x", num(1.0)), ("y", num(0.0)), ("z", num(0.0)), ("w", num(1.0))]),
            ));
            members.push((
                "native".to_string(),
                obj(vec![
                    ("edition", json_str("classic")),
                    ("movementOrigin", vec_json(0.0, 0.0, 0.0)),
                    ("renderFlags", num(0.0)),
                    ("positionPrediction", boolean(true)),
                    ("angularPrediction", boolean(true)),
                    ("weaponVisible", boolean(true)),
                ]),
            ));
        }
        let reader = SaveReader::new(&view);
        assert!(read_native_camera_view(reader).is_err());
    }

    #[test]
    fn world_text_reads_fixed_orientation() {
        let encoded = obj(vec![
            ("content", json_str("q1:classic:base:1")),
            ("text", json_str("hello")),
            ("origin", vec_json(1.0, 2.0, 3.0)),
            (
                "color",
                obj(vec![("x", num(1.0)), ("y", num(1.0)), ("z", num(1.0)), ("w", num(1.0))]),
            ),
            ("cellSize", num(8.0)),
            (
                "orientation",
                obj(vec![("kind", json_str("fixed")), ("angles", vec_json(0.0, 90.0, 0.0))]),
            ),
            ("depthTest", boolean(true)),
            ("font", json_str("selected")),
        ]);
        let reader = SaveReader::new(&encoded);
        let text = read_world_text(reader).unwrap();
        assert!(matches!(text.orientation, WorldTextOrientation::Fixed { .. }));
        assert_eq!(text.font, WorldTextFont::Selected);
        assert_eq!(text.distance_cull_factor, None);
    }

    #[test]
    fn character_view_reads_team_and_animation() {
        let ledger = ledger();
        let encoded = obj(vec![
            ("actor", actor_json(1, 2)),
            ("origin", vec_json(0.0, 0.0, 0.0)),
            ("angles", vec_json(0.0, 0.0, 0.0)),
            ("velocity", vec_json(0.0, 0.0, 0.0)),
            ("movementDirection", num(3.0)),
            (
                "animation",
                obj(vec![
                    ("kind", json_str("q3")),
                    ("legs", num(1.0)),
                    ("torso", num(2.0)),
                    ("legsTimerMilliseconds", num(100.0)),
                    ("torsoTimerMilliseconds", num(200.0)),
                ]),
            ),
            ("sourceFlags", num(0.0)),
            ("powerups", num(0.0)),
            ("team", json_str("red")),
            (
                "color",
                obj(vec![("x", num(1.0)), ("y", num(1.0)), ("z", num(1.0)), ("w", num(1.0))]),
            ),
        ]);
        let reader = SaveReader::new(&encoded);
        let view = read_character_view(reader, &ledger).unwrap();
        assert_eq!(view.team, Some(UnifiedCharacterTeam::Red));
        assert_eq!(view.animation.legs, 1.0);
        assert_eq!(view.scale, None);
    }

    #[test]
    fn model_rejects_bad_indexed_skin_length() {
        let ledger = ledger();
        let mut model = obj(vec![
            ("actor", actor_json(1, 0)),
            ("content", json_str("q2:classic:base:1")),
            ("family", json_str("q2")),
            ("path", json_str("models/player.md2")),
            ("frame", num(0.0)),
            ("oldFrame", num(0.0)),
            ("skin", num(0.0)),
            ("effects", num(0.0)),
            ("renderFlags", num(0.0)),
            ("origin", vec_json(0.0, 0.0, 0.0)),
            ("angles", vec_json(0.0, 0.0, 0.0)),
            ("scale", num(1.0)),
            ("visible", boolean(true)),
            ("viewWeapon", boolean(false)),
        ]);
        if let SaveJson::Object(members) = &mut model {
            members.push((
                "indexedSkin".to_string(),
                obj(vec![
                    ("name", json_str("skin")),
                    ("width", num(2.0)),
                    ("height", num(2.0)),
                    ("pixels", SaveJson::Bytes(vec![0, 1, 2])),
                ]),
            ));
        }
        let reader = SaveReader::new(&model);
        assert!(read_model(reader, &ledger).is_err());
    }

    #[test]
    fn held_weapon_none_reads() {
        let encoded = obj(vec![("kind", json_str("none"))]);
        let reader = SaveReader::new(&encoded);
        assert_eq!(read_held_weapon(reader).unwrap(), HeldWeaponDeclaration::None);
    }

    #[test]
    fn item_icon_rejects_foreign_content() {
        let content = ContentId("q1:classic:base:1".to_string());
        let encoded = obj(vec![
            ("kind", json_str("image")),
            (
                "resource",
                obj(vec![
                    ("content", json_str("q2:classic:base:1")),
                    ("path", json_str("icons/quad.png")),
                ]),
            ),
        ]);
        let reader = SaveReader::new(&encoded);
        assert!(read_item_icon(reader, &content).is_err());
    }

    #[test]
    fn validate_content_id_rejects_bad_shape() {
        assert!(validate_content_id("nope").is_err());
    }
}
