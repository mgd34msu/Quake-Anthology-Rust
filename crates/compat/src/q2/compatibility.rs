//! Port of `src/compat/q2/compatibility.ts`.
//! Bridges native-compatibility documents into declared primary profiles.

use std::collections::HashSet;

use qa_guest::core::contracts::NativeAbi;

use super::native_primary::{
    read_classic_world_profile, read_rerelease_world_profile, NativePrimaryDeclaration, NativePrimaryProfile,
};
use super::native_primary_profiles::{
    read_native_primary_commands, read_native_primary_drop, read_native_primary_inventory, read_native_primary_pickups,
    read_native_primary_player, read_native_primary_weapons,
};
use super::native_primary_reader::{namespaced, parse_json, JsonValue, Reader};

/// Native execution module selecting one compatibility declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeExecution {
    /// Requested artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: String,
    /// Native API version: 3 for classic, 2023 for rerelease.
    pub api_version: u32,
    /// Executable ABI.
    pub profile: NativeAbi,
    /// Owning content identity.
    pub owner_content: String,
}

/// Opened compatibility resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityResource {
    /// Document bytes.
    pub bytes: Vec<u8>,
    /// Resource reference.
    pub reference: String,
}

/// Synthetic content mounts serving the compatibility document.
pub trait CompatibilityMounts {
    /// Open `native-compatibility.json` for the owning content, if present.
    fn open_native_compatibility(&self, owner_content: &str) -> Option<CompatibilityResource>;
}

/// Normalize a resource path for case-insensitive comparison.
#[must_use]
pub fn normalize_resource_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    loop {
        if let Some(stripped) = normalized.strip_prefix("./") {
            normalized = stripped.to_string();
            continue;
        }
        if let Some(stripped) = normalized.strip_prefix('/') {
            normalized = stripped.to_string();
            continue;
        }
        return normalized;
    }
}

/// Read a content digest reference.
pub fn read_digest(reader: &Reader) -> String {
    namespaced(reader)
}

/// Parse a compatibility document, selecting the execution module entry.
/// Schema violations panic with the document path, mirroring the donor throw;
/// a document without the selected module yields `None`.
pub fn parse_native_compatibility(value: &JsonValue, execution: &NativeExecution) -> Option<NativePrimaryProfile> {
    let root = Reader::root(value);
    root.field("version").literal_int(1);
    let wanted = normalize_resource_path(&execution.artifact_path).to_lowercase();
    let mut seen = HashSet::new();
    let modules = root.field("modules");
    let parsed: Vec<Option<NativePrimaryProfile>> = modules.list(|entry| {
        let path = normalize_resource_path(&entry.field("artifactPath").string());
        let key = path.to_lowercase();
        if !seen.insert(key.clone()) {
            entry.fail("duplicate native module declaration");
        }
        let digest = read_digest(&entry.field("artifactDigest"));
        let api = entry.field("apiVersion").choice_int(&[3, 2023]) as u32;
        if key != wanted {
            return None;
        }
        if digest != execution.digest || api != execution.api_version {
            entry.fail("native primary declaration differs from the selected artifact or API");
        }
        let reader = entry.field("primary");
        let abi = execution.profile;
        let weapons = read_native_primary_weapons(&reader.field("weapons"), digest.clone(), abi);
        let player = read_native_primary_player(&reader.field("player"), digest.clone());
        let commands = read_native_primary_commands(&reader.field("commands"), digest.clone(), abi);
        let inventory = read_native_primary_inventory(&reader.field("inventory"), digest.clone(), abi);
        let drop = read_native_primary_drop(&reader.field("drop"), digest.clone(), abi);
        let pickups = read_native_primary_pickups(&reader.field("pickups"), digest.clone(), abi);
        if weapons.entity.client != inventory.client
            || commands.client.pointer != inventory.client
            || drop.client.pointer != inventory.client
            || pickups.supply.client != inventory.client
            || commands.client.inventory != inventory.inventory
            || drop.client.inventory != inventory.inventory
            || pickups.supply.inventory != inventory.inventory
            || commands.items.table != pickups.items.table
            || commands.items.stride != pickups.items.stride
            || commands.items.count != pickups.items.count
            || commands.items.count > inventory.count
        {
            reader.fail("primary services disagree about their source client or item storage");
        }
        Some(if api == 3 {
            NativePrimaryProfile::Classic {
                weapons,
                player,
                commands,
                inventory,
                drop,
                pickups,
                world: Box::new(read_classic_world_profile(&reader.field("world"), digest)),
            }
        } else {
            NativePrimaryProfile::Rerelease {
                weapons,
                player,
                commands,
                inventory,
                drop,
                pickups,
                world: Box::new(read_rerelease_world_profile(&reader.field("world"), digest)),
            }
        })
    });
    parsed.into_iter().flatten().next_back()
}

/// Read the compatibility declaration for an execution module: `None` when
/// the mounts have no document or the document has no matching module.
pub fn read_native_compatibility(
    mounts: &dyn CompatibilityMounts,
    execution: &NativeExecution,
) -> Option<NativePrimaryDeclaration> {
    let opened = mounts.open_native_compatibility(&execution.owner_content)?;
    let text =
        String::from_utf8(opened.bytes).unwrap_or_else(|_| panic!("native-compatibility.json is not valid UTF-8"));
    let value: JsonValue = parse_json(&text).unwrap_or_else(|error| panic!("native-compatibility.json: {error}"));
    parse_native_compatibility(&value, execution).map(|profile| NativePrimaryDeclaration {
        declaration: opened.reference,
        profile,
    })
}

#[cfg(test)]
mod tests {
    use super::super::native_primary::PrimaryEdition;
    use super::super::native_primary_reader::CLASSIC_DIGEST;
    use super::*;

    const ABI: &str = r#"{"kind":"windows-i386","image":"pe32","call":"cdecl","pointerBytes":4}"#;

    fn field(record: &str, offset: u32, encoding: &str) -> String {
        format!(r#"{{"record":"{record}","offset":{offset},"encoding":"{encoding}"}}"#)
    }

    fn signature() -> String {
        format!(
            r#"{{"abi":{ABI},"parameters":[{{"kind":"scalar","storage":"pointer"}}],"result":"void","variadic":false}}"#
        )
    }

    fn weapons_json() -> String {
        format!(
            r#"{{"dispatcher":{{"entry":{{"kind":"rva","rva":222000}},"record":"entity","argument":0,"arguments":1}},
            "decisions":[],"spawn":{{"entry":203216,"accepted":[]}},"active":[],"committedInput":[],"continuations":[],
            "time":{{"address":485632,"encoding":"float32","milliseconds":1000}},
            "entity":{{"client":84,"waterLevel":{},"viewHeight":{},"maxHealth":{}}},
            "client":{{"byteLength":3832,"viewAngles":3652,"buttons":{},"latchedButtons":{}}},
            "attackAnimation":{{"entry":224096,"skip":[]}},
            "animation":{{"frame":{},"end":{},"priority":{},"duck":{},"run":{}}},
            "equipmentContexts":[],
            "delay":{{"flag":{},"region":{{"entry":223343,"join":223379}},"evaluate":{{"kind":"source-flag","factors":[1]}}}},
            "damage":{{"kind":"source-result","entry":223304,"result":"uint8"}}}}"#,
            field("entity", 612, "int32"),
            field("entity", 508, "int32"),
            field("entity", 484, "int32"),
            field("client", 3532, "int32"),
            field("client", 3540, "int32"),
            field("entity", 56, "int32"),
            field("client", 3708, "int32"),
            field("client", 3712, "int32"),
            field("client", 3716, "int32"),
            field("client", 3720, "int32"),
            field("image", 448372, "int32"),
        )
    }

    fn player_json() -> String {
        r#"{"spawn":201120,"objectives":{"kind":"none"},"commandAngles":3476,"velocity":376,"forward":null}"#
            .to_string()
    }

    fn commands_json() -> String {
        r#"{"give":{"entry":12608,"weapons":12886,"ammo":12975,"unknown":{"entry":13429,"join":13446},
        "ammoGrants":[],"argc":485500,"argv":485504},
        "drop":{"entry":198848,"eligibility":{"entry":198887,"join":198971}},
        "client":{"pointer":84,"weapon":1796,"ammoIndex":3528,"inventory":740},
        "items":{"table":16384,"stride":76,"count":48,"classname":0,"flags":56,"weaponFlag":1,
        "ammunitionFlag":2,"icon":36,"ammo":{"kind":"name","offset":52,"label":40}}}"#
            .to_string()
    }

    fn inventory_json() -> String {
        r#"{"client":84,"inventory":740,"count":256,"cursor":736,"empty":-1,
        "prototypes":{"weapon":"q2:weapon_blaster","ammunition":"q2:ammo_shells","usable":"q2:item_quad",
        "passive":"q2:key_data_cd","droppable":"q2:item_quad","undroppable":"q2:weapon_blaster"},
        "selectionWrites":[],"next":{"entry":12256,"scan":12291,"join":12394,"menuArgument":false},
        "previous":{"entry":12400,"scan":12435,"join":12543},
        "validate":{"entry":12560,"scan":null},
        "use":{"entry":14944,"call":15036,"join":15038},
        "namedUse":{"entry":14032,"lookupCall":14045,"lookupReturn":14050,"call":14412,"join":14415}}"#
            .to_string()
    }

    fn drop_json() -> String {
        r#"{"client":{"pointer":84,"inventory":740,"cursor":736,"weapon":1796,"pending":3548},
        "named":14432,"inventory":{"entry":15488,"admitted":15499},"find":38288,"lookupReturn":14450,
        "allocate":44288,"free":102976,"callbacks":[],"debits":[],"consumer":null}"#
            .to_string()
    }

    fn pickups_json() -> String {
        let signature = signature();
        format!(
            r#"{{"touch":256,"grantReturn":272,"targetsReturn":288,
            "touchSignature":{signature},"grantSignature":{signature},"grants":[],
            "items":{{"table":16384,"stride":76,"count":48,"classname":0,"pickup":8}},
            "entity":{{"item":64,"count":68,"spawnflags":72,"inuse":76,"inuseBytes":4,"generation":null}},
            "time":{{"address":1280,"storage":"float32-seconds"}},
            "supply":{{"client":84,"inventory":740,"flags":56,"weaponFlag":1,
            "ammo":{{"entry":1536,"signature":{signature},"stop":null,"tag":68,"capacities":[],"capacityBytes":4}}}}}}"#
        )
    }

    fn world_json() -> String {
        let pain = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"attacker"},{"kind":"field","field":"kick"},{"kind":"field","field":"amount"}]}"#;
        let death = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"inflictor"},{"kind":"field","field":"attacker"},{"kind":"field","field":"amount"},{"kind":"field","field":"point"}]}"#;
        let damage = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"inflictor"},{"kind":"field","field":"attacker"},{"kind":"field","field":"direction"},{"kind":"field","field":"point"},{"kind":"field","field":"normal"},{"kind":"field","field":"amount"},{"kind":"field","field":"knockback"},{"kind":"field","field":"flags"},{"kind":"field","field":"cause"}]}"#;
        let regular = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"point"},{"kind":"field","field":"normal"},{"kind":"field","field":"amount"},{"kind":"field","field":"sparks"},{"kind":"field","field":"flags"}]}"#;
        let power = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"point"},{"kind":"field","field":"normal"},{"kind":"field","field":"amount"},{"kind":"field","field":"flags"}]}"#;
        format!(
            r#"{{"calls":{{"pain":{pain},"death":{death},"damage":{damage},"regularArmor":{regular},"powerArmor":{power}}},
            "game":"xatrix","entityBytes":896,
            "fields":{{"health":480,"damageable":512,"flags":264,"mass":400,"velocity":376,"pain":452,"die":456}},
            "client":{{"inventory":740,"inventoryCount":256,"maxGrenades":1776,"invincibleFrame":3728,"userinfo":188,"userinfoBytes":512,"viewAngles":3652}},
            "entries":{{"damage":20560,"powerArmor":21888,"regularArmor":22368,"spawn":102544,"free":102976}},
            "globals":{{"levelFrame":485376,"itemList":16384,"itemBytes":76}},
            "items":{{"jacket":3,"combat":2,"body":1,"screen":5,"shield":6,"cells":23,"grenades":12}},
            "itemFields":{{"className":0,"armorInfo":64}},"armorInfo":{{"normalProtection":8,"energyProtection":12}},
            "flags":{{"invulnerable":16,"notarget":32,"noKnockback":2048,"powerArmor":4096}},
            "teams":{{"model":64,"skin":128}},"armor":{{"regular":[3,2,1],"empty":1}},
            "inventoryTable":{{"count":48,"className":0,"label":40,"flags":56,"ammoFlag":2,"tag":68,
            "capacities":[],"unnamed":[],"emptyIndex":0,"sentinel":true}}}}"#
        )
    }

    fn document_json() -> String {
        format!(
            r#"{{"version":1,"modules":[{{"artifactPath":"GAME\\xatrix.dll",
            "artifactDigest":"{CLASSIC_DIGEST}","apiVersion":3,
            "primary":{{"weapons":{},"player":{},"commands":{},"inventory":{},"drop":{},"pickups":{},"world":{}}}}}]}}"#,
            weapons_json(),
            player_json(),
            commands_json(),
            inventory_json(),
            drop_json(),
            pickups_json(),
            world_json(),
        )
    }

    fn execution() -> NativeExecution {
        NativeExecution {
            artifact_path: "game/xatrix.dll".to_string(),
            digest: CLASSIC_DIGEST.to_string(),
            api_version: 3,
            profile: NativeAbi::WindowsI386,
            owner_content: "q2:xatrix".to_string(),
        }
    }

    struct StubMounts {
        resource: Option<CompatibilityResource>,
    }

    impl CompatibilityMounts for StubMounts {
        fn open_native_compatibility(&self, owner_content: &str) -> Option<CompatibilityResource> {
            if owner_content == "q2:xatrix" {
                self.resource.clone()
            } else {
                None
            }
        }
    }

    #[test]
    fn parses_matching_modules() {
        let value = parse_json(&document_json()).expect("valid json");
        let profile = parse_native_compatibility(&value, &execution()).expect("profile");
        assert_eq!(profile.edition(), PrimaryEdition::Classic);
        assert_eq!(profile.inventory().count, 256);
        assert_eq!(profile.commands().items.count, 48);
        match &profile {
            NativePrimaryProfile::Classic { world, .. } => {
                assert_eq!(world.entity_bytes, 896);
            }
            NativePrimaryProfile::Rerelease { .. } => panic!("expected classic"),
        }
    }

    #[test]
    fn reads_declarations_through_mounts() {
        let mounts = StubMounts {
            resource: Some(CompatibilityResource {
                bytes: document_json().into_bytes(),
                reference: "q2-compat:json".to_string(),
            }),
        };
        let declaration = read_native_compatibility(&mounts, &execution()).expect("declaration");
        assert_eq!(declaration.declaration, "q2-compat:json");
        assert_eq!(declaration.profile.edition(), PrimaryEdition::Classic);
        let empty = StubMounts { resource: None };
        assert!(read_native_compatibility(&empty, &execution()).is_none());
        let foreign = execution();
        let other = NativeExecution {
            owner_content: "q2:other".to_string(),
            ..foreign
        };
        assert!(read_native_compatibility(&mounts, &other).is_none());
    }

    #[test]
    fn skips_non_matching_modules() {
        let value = parse_json(&document_json()).expect("valid json");
        let execution = NativeExecution {
            artifact_path: "game/rogue.dll".to_string(),
            ..execution()
        };
        assert!(parse_native_compatibility(&value, &execution).is_none());
    }

    #[test]
    #[should_panic(expected = "duplicate native module declaration")]
    fn rejects_duplicate_modules() {
        let text = format!(
            r#"{{"version":1,"modules":[{{"artifactPath":"a.dll","artifactDigest":"{CLASSIC_DIGEST}","apiVersion":3,
            "primary":{{"weapons":{},"player":{},"commands":{},"inventory":{},"drop":{},"pickups":{},"world":{}}}}},
            {{"artifactPath":"A.DLL","artifactDigest":"{CLASSIC_DIGEST}","apiVersion":3,
            "primary":{{"weapons":{},"player":{},"commands":{},"inventory":{},"drop":{},"pickups":{},"world":{}}}}}]}}"#,
            weapons_json(),
            player_json(),
            commands_json(),
            inventory_json(),
            drop_json(),
            pickups_json(),
            world_json(),
            weapons_json(),
            player_json(),
            commands_json(),
            inventory_json(),
            drop_json(),
            pickups_json(),
            world_json(),
        );
        let value = parse_json(&text).expect("valid json");
        parse_native_compatibility(&value, &execution());
    }
}
