//! Q2Eaks v0.21 weapon entries and trajectory declaration.
//!
//! Donor: `src/compat/q2/rerelease/q2eaks-weapon-profile.ts` — bridges the
//! exact v0.21 artifact entries and rocket-trajectory declaration.

use qa_guest::core::contracts::{
    GuestAddress, GuestCallSignature, GuestFieldLayout, GuestLayout, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

/// Q2Eaks v0.21 artifact digest.
pub const Q2EAKS_WEAPON_DIGEST: &str = "sha256:b60b79f7fb6f115218681a9cbab8765267e34f72466975526df05ad288925dde";

/// Weapon profile failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WeaponProfileError {
    /// Profile requires the exact v0.21 Windows x64 artifact.
    #[error("Q2Eaks weapon profile requires the exact v0.21 Windows x64 artifact")]
    ForeignArtifact,
    /// Typed source callback registration differs.
    #[error("Q2Eaks typed source callback registration differs: {0}")]
    BadRegistration(String),
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

fn scalar(storage: GuestStorage) -> GuestValueLayout {
    GuestValueLayout::Scalar(storage)
}

/// Weapon think signature: one entity pointer.
#[must_use]
pub fn think_signature() -> GuestCallSignature {
    GuestCallSignature {
        abi: NativeCallAbi::MicrosoftX64,
        parameters: vec![scalar(GuestStorage::Pointer)],
        result: None,
        variadic: false,
    }
}

/// `fireRocket` signature: owner, start, direction, damage, speed, radius
/// factor and radius damage, returning the projectile.
#[must_use]
pub fn fire_rocket_signature() -> GuestCallSignature {
    GuestCallSignature {
        abi: NativeCallAbi::MicrosoftX64,
        parameters: vec![
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Int32),
            scalar(GuestStorage::Int32),
            scalar(GuestStorage::Float32),
            scalar(GuestStorage::Int32),
        ],
        result: Some(scalar(GuestStorage::Pointer)),
        variadic: false,
    }
}

fn sparse_field(name: &str, byte_offset: usize, storage: GuestStorage, count: usize) -> GuestFieldLayout {
    GuestFieldLayout {
        name: name.to_string(),
        byte_offset,
        storage,
        count,
    }
}

/// Sparse fields present in the v0.21 launch/project-source instructions.
#[must_use]
pub fn projectile_layout() -> GuestLayout {
    GuestLayout::new(
        "q2eaks-v0.21:observed-projectile-fields",
        0x7a8,
        8,
        8,
        vec![
            sparse_field("s.origin", 4, GuestStorage::Float32, 3),
            sparse_field("s.angles", 16, GuestStorage::Float32, 3),
            sparse_field("client", 0x78, GuestStorage::Pointer, 1),
            sparse_field("owner", 0x5b8, GuestStorage::Pointer, 1),
            sparse_field("velocity", 0x694, GuestStorage::Float32, 3),
            sparse_field("nextthink", 0x6d8, GuestStorage::Int64, 1),
            sparse_field("think.value", 0x700, GuestStorage::Pointer, 1),
            sparse_field("think.list", 0x708, GuestStorage::Pointer, 1),
            sparse_field("touch.value", 0x710, GuestStorage::Pointer, 1),
            sparse_field("touch.list", 0x718, GuestStorage::Pointer, 1),
            sparse_field("viewheight", 0x7a0, GuestStorage::Int32, 1),
        ],
    )
}

/// Sparse weapon-client fields.
#[must_use]
pub fn weapon_client_layout() -> GuestLayout {
    GuestLayout::new(
        "q2eaks-v0.21:observed-weapon-client-fields",
        0x19b0,
        8,
        8,
        vec![
            sparse_field("pers.hand", 0xa50, GuestStorage::Int32, 1),
            sparse_field("pers.weapon", 0xbe8, GuestStorage::Pointer, 1),
            sparse_field("v_angle", 0x1998, GuestStorage::Float32, 3),
            sparse_field("v_forward", 0x19a4, GuestStorage::Float32, 3),
        ],
    )
}

/// Exact PE entries; calling them retains all native side effects and
/// requires an initialized source shooter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2EaksWeaponEntries {
    /// Transient damage/silencer setup before the weapon dispatcher.
    pub weapon_run_think: GuestAddress,
    /// Launcher fire with the actual cvar policy.
    pub rocket_launcher_fire: GuestAddress,
    /// Projectile spawner.
    pub fire_rocket: GuestAddress,
    /// Entity allocator.
    pub spawn: GuestAddress,
    /// Entity release.
    pub free: GuestAddress,
    /// Rocket touch callback.
    pub rocket_touch: GuestAddress,
    /// Level time.
    pub level_time: GuestAddress,
}

/// Read a guest string with an exact maximum.
fn read_guest_string(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    maximum: usize,
) -> Result<String, WeaponProfileError> {
    let found = memory.find_zero(address, maximum)?;
    if found < 0 {
        return Err(WeaponProfileError::BadRegistration("<unterminated>".to_string()));
    }
    Ok(String::from_utf8_lossy(&memory.copy(address, found as usize)?).into_owned())
}

/// Resolve the exact v0.21 entries, checking typed save registrations.
pub fn q2eaks_weapon_entries(
    memory: &mut SparseGuestMemory,
    module_digest: &str,
    image_base: GuestAddress,
) -> Result<Q2EaksWeaponEntries, WeaponProfileError> {
    if module_digest != Q2EAKS_WEAPON_DIGEST || memory.pointer_bytes() != 8 {
        return Err(WeaponProfileError::ForeignArtifact);
    }
    let entry = |memory: &SparseGuestMemory, rva: u64| memory.offset(image_base, rva as i64);
    let registered = |memory: &mut SparseGuestMemory,
                      rva: u64,
                      name: &str,
                      tag: u32,
                      expected_rva: u64|
     -> Result<GuestAddress, WeaponProfileError> {
        let record = memory.offset(image_base, rva as i64)?;
        let text = memory.read_pointer(record)?;
        let callback = memory.read_pointer(memory.offset(record, 16)?)?;
        let (Some(text), Some(callback)) = (text, callback) else {
            return Err(WeaponProfileError::BadRegistration(name.to_string()));
        };
        let expected = entry(memory, expected_rva)?;
        if read_guest_string(memory, text, name.len() + 1)? != name
            || memory.read_u32(memory.offset(record, 8)?)? != tag
            || callback != expected
        {
            return Err(WeaponProfileError::BadRegistration(name.to_string()));
        }
        Ok(callback)
    };
    let level_time = memory.offset(image_base, 0x2999c8)?;
    Ok(Q2EaksWeaponEntries {
        weapon_run_think: entry(memory, 0xed420)?,
        rocket_launcher_fire: entry(memory, 0xef900)?,
        fire_rocket: entry(memory, 0x98310)?,
        spawn: entry(memory, 0x95010)?,
        free: registered(memory, 0x21e0c8, "G_FreeEdict", 20, 0x95140)?,
        rocket_touch: registered(memory, 0x21e1e8, "rocket_touch", 21, 0x98060)?,
        level_time,
    })
}

/// Native weapon registration record layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponRegistrationLayout {
    /// Record byte length.
    pub byte_length: usize,
    /// Name pointer offset.
    pub name: usize,
    /// Tag offset.
    pub tag: usize,
    /// Callback offset.
    pub callback: usize,
}

/// Native weapon entry with optional typed registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponEntry {
    /// Entry RVA.
    pub rva: u64,
    /// Typed registration, if any.
    pub registration: Option<WeaponRegistration>,
}

/// Typed save registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponRegistration {
    /// Record RVA.
    pub rva: u64,
    /// Callback name.
    pub name: String,
    /// Save tag.
    pub tag: u32,
    /// Record layout.
    pub layout: WeaponRegistrationLayout,
}

/// Native weapon console command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponCommand {
    /// Arguments.
    pub arguments: Vec<String>,
    /// Tail text.
    pub tail: String,
}

/// Native weapon cvar override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponCvar {
    /// Cvar name.
    pub name: String,
    /// Value.
    pub value: String,
}

/// Entity field contract for trajectory behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponEntityFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Origin offset.
    pub origin: usize,
    /// Angles offset.
    pub angles: usize,
    /// Velocity offset.
    pub velocity: usize,
    /// Client offset.
    pub client: usize,
    /// Owner offset.
    pub owner: usize,
    /// View height offset.
    pub view_height: usize,
    /// Generation offset.
    pub generation: usize,
    /// Next think offset.
    pub next_think: usize,
    /// Think callback offset.
    pub think_callback: usize,
    /// Think registration offset.
    pub think_registration: usize,
    /// Touch callback offset.
    pub touch_callback: usize,
}

/// Client field contract for trajectory behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponClientFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Weapon offset.
    pub weapon: usize,
    /// View angles offset.
    pub view_angles: usize,
    /// Forward offset.
    pub forward: usize,
}

/// Equipped weapon contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquippedWeaponFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Callback offset.
    pub callback: usize,
    /// Expected entry.
    pub expected: WeaponEntry,
}

/// Built-in evidence expressed through the same declaration contract as
/// mounted profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorDeclaration {
    /// Declaration version.
    pub version: u32,
    /// Kind tag.
    pub kind: String,
    /// ABI tag.
    pub abi: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub artifact_digest: String,
    /// Behavior id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Weapon role.
    pub role: String,
    /// Aspect.
    pub aspect: String,
    /// Entity fields.
    pub entity: WeaponEntityFields,
    /// Client fields.
    pub client: WeaponClientFields,
    /// Equipped weapon fields.
    pub equipped_weapon: EquippedWeaponFields,
    /// Time storage tag.
    pub time_storage: String,
    /// Time RVA.
    pub time_rva: u64,
    /// Think signature tag.
    pub think_signature: String,
    /// Think save tag.
    pub think_tag: u32,
    /// Think registration layout.
    pub think_registration: WeaponRegistrationLayout,
    /// Allocate signature tag.
    pub allocate_signature: String,
    /// Allocate entry.
    pub allocate: WeaponEntry,
    /// Free signature tag.
    pub free_signature: String,
    /// Free entry.
    pub free: WeaponEntry,
    /// Projectile touch entry.
    pub projectile_touch: WeaponEntry,
    /// Equip call chain.
    pub equip: Vec<WeaponEntry>,
    /// Launch call chain.
    pub launch: Vec<WeaponEntry>,
    /// Activation RVA, if any.
    pub activate_rva: Option<u64>,
    /// Fire RVA.
    pub fire_rva: u64,
    /// Initialization classes.
    pub initialization_classes: Vec<String>,
    /// Equipment commands.
    pub equipment: Vec<WeaponCommand>,
    /// Ammunition command.
    pub ammunition: WeaponCommand,
    /// Initial cvars.
    pub initial_cvars: Vec<WeaponCvar>,
    /// Provisioning cvars.
    pub provisioning_cvars: Vec<WeaponCvar>,
}

/// Built-in faster-rockets declaration for an artifact path.
#[must_use]
pub fn q2eaks_weapon_declaration(artifact_path: &str) -> WeaponBehaviorDeclaration {
    let layout = WeaponRegistrationLayout {
        byte_length: 24,
        name: 0,
        tag: 8,
        callback: 16,
    };
    let entry = |rva: u64| WeaponEntry {
        rva,
        registration: None,
    };
    let command = |arguments: &[&str], tail: &str| WeaponCommand {
        arguments: arguments.iter().map(|value| (*value).to_string()).collect(),
        tail: tail.to_string(),
    };
    WeaponBehaviorDeclaration {
        version: 1,
        kind: "q2-api2023-trajectory".to_string(),
        abi: "windows-x86-64".to_string(),
        artifact_path: artifact_path.to_string(),
        artifact_digest: Q2EAKS_WEAPON_DIGEST.to_string(),
        id: "native:rocket-trajectory".to_string(),
        title: "Faster rockets".to_string(),
        role: "rocket".to_string(),
        aspect: "trajectory".to_string(),
        entity: WeaponEntityFields {
            byte_length: 0x7a8,
            origin: 4,
            angles: 16,
            velocity: 0x694,
            client: 0x78,
            owner: 0x5b8,
            view_height: 0x7a0,
            generation: 0x5c0,
            next_think: 0x6d8,
            think_callback: 0x700,
            think_registration: 0x708,
            touch_callback: 0x710,
        },
        client: WeaponClientFields {
            byte_length: 0x19b0,
            weapon: 0xbe8,
            view_angles: 0x1998,
            forward: 0x19a4,
        },
        equipped_weapon: EquippedWeaponFields {
            byte_length: 0x30,
            callback: 0x28,
            expected: entry(0xefaf0),
        },
        time_storage: "int64-milliseconds".to_string(),
        time_rva: 0x2999c8,
        think_signature: "entity-void".to_string(),
        think_tag: 20,
        think_registration: layout,
        allocate_signature: "void-pointer".to_string(),
        allocate: entry(0x95010),
        free_signature: "entity-void".to_string(),
        free: WeaponEntry {
            rva: 0x95140,
            registration: Some(WeaponRegistration {
                rva: 0x21e0c8,
                name: "G_FreeEdict".to_string(),
                tag: 20,
                layout,
            }),
        },
        projectile_touch: WeaponEntry {
            rva: 0x98060,
            registration: Some(WeaponRegistration {
                rva: 0x21e1e8,
                name: "rocket_touch".to_string(),
                tag: 21,
                layout,
            }),
        },
        equip: vec![entry(0xed4d0)],
        launch: vec![entry(0xed420), entry(0xef900)],
        activate_rva: Some(0xed4d0),
        fire_rva: 0xef900,
        initialization_classes: [
            "worldspawn",
            "info_player_start",
            "info_player_deathmatch",
            "info_player_coop",
            "info_player_team1",
            "info_player_team2",
            "info_player_intermission",
        ]
        .iter()
        .map(|value| (*value).to_string())
        .collect(),
        equipment: vec![
            command(&["give", "Rocket Launcher"], "Rocket Launcher"),
            command(&["give", "Rockets"], "Rockets"),
            command(&["use", "Rocket Launcher"], "Rocket Launcher"),
        ],
        ammunition: command(&["give", "Rockets"], "Rockets"),
        initial_cvars: vec![WeaponCvar {
            name: "g_faster_rockets".to_string(),
            value: "1".to_string(),
        }],
        provisioning_cvars: vec![WeaponCvar {
            name: "cheats".to_string(),
            value: "1".to_string(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "q2eaks-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    #[test]
    fn layouts_and_signatures_match_v021() {
        assert_eq!(projectile_layout().byte_length, 0x7a8);
        assert_eq!(weapon_client_layout().byte_length, 0x19b0);
        assert_eq!(think_signature().parameters.len(), 1);
        let fire = fire_rocket_signature();
        assert_eq!(fire.parameters.len(), 7);
        assert!(fire.result.is_some());
        let declaration = q2eaks_weapon_declaration("q2eaks/game.dll");
        assert_eq!(declaration.entity.byte_length, 0x7a8);
        assert_eq!(declaration.fire_rva, 0xef900);
        assert_eq!(declaration.activate_rva, Some(0xed4d0));
        assert_eq!(declaration.initialization_classes.len(), 7);
        assert!(declaration.free.registration.is_some());
    }

    #[test]
    fn entries_check_registrations_and_digest() {
        let mut memory = test_memory();
        let image_base = memory
            .allocate(&GuestAllocationOptions::bytes(0x2a_0000))
            .expect("alloc");
        let name_free = memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        memory.write(name_free, b"G_FreeEdict\0").expect("write");
        let name_touch = memory.allocate(&GuestAllocationOptions::bytes(13)).expect("alloc");
        memory.write(name_touch, b"rocket_touch\0").expect("write");
        for (rva, name, tag, expected) in [
            (0x21e0c8u64, name_free, 20u32, 0x95140u64),
            (0x21e1e8u64, name_touch, 21u32, 0x98060u64),
        ] {
            let record = memory.offset(image_base, rva as i64).expect("record");
            memory.write_pointer(record, Some(name)).expect("name");
            memory
                .write_u32(memory.offset(record, 8).expect("o"), tag)
                .expect("tag");
            memory
                .write_pointer(
                    memory.offset(record, 16).expect("o"),
                    Some(memory.offset(image_base, expected as i64).expect("cb")),
                )
                .expect("callback");
        }
        let entries = q2eaks_weapon_entries(&mut memory, Q2EAKS_WEAPON_DIGEST, image_base).expect("entries");
        assert_eq!(entries.spawn.offset, image_base.offset + 0x95010);
        assert_eq!(entries.fire_rocket.offset, image_base.offset + 0x98310);
        assert_eq!(entries.rocket_touch.offset, image_base.offset + 0x98060);
        assert_eq!(entries.level_time.offset, image_base.offset + 0x2999c8);
        assert_eq!(
            q2eaks_weapon_entries(&mut memory, "sha256:other", image_base).unwrap_err(),
            WeaponProfileError::ForeignArtifact
        );
    }
}
