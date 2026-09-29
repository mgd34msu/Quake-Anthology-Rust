//! Port of `src/compat/q2/native-primary-profiles.ts`.
//! Bridges profile documents into typed primary service profiles.

use qa_guest::core::contracts::NativeAbi;

use super::native_primary_commands::{
    AmmoGrant, CommandClient, CommandItems, DropCommand, GiveProfile, GrantKind, ItemAmmo,
    NativePrimaryCommandProfile,
};
use super::native_primary_drop::{
    DropClient, InventoryDrop, NativePrimaryDropProfile,
};
use super::native_primary_inventory::{
    InventoryPrototypes, NamedUseProfile, NativePrimaryInventoryProfile, NextProfile,
    PreviousProfile, SelectionWrite, UseProfile, ValidateProfile,
};
use super::native_primary_pickups::{
    AmmoSupply, NativePickupGrant, NativePickupProfile, PickupConsumer, PickupEntity, PickupItems,
    PickupResource, PickupSupply, PickupSupplyProfile, PickupTime, ProtectionChannel, TimeStorage,
};
use super::native_primary_player::{
    NativePrimaryPlayerProfile, PlayerObjectives, SourcePrimaryMatch, SourceTeam,
};
use super::native_primary_reader::{
    Reader, RecordKind, native_field, native_offset, native_register, native_scalar,
    native_signature, native_test, namespaced,
};
use super::native_primary_weapons::{
    AttackAnimation, DamageResult, DecisionField, DelayEvaluate, EquipmentContext,
    NativePrimaryWeaponProfile, ProjectionWrite, SpawnGate, WeaponAnimation, WeaponClient,
    WeaponDamage, WeaponDecision, WeaponDelay, WeaponDispatcher, WeaponEntity, WeaponTime,
};

/// Read a score-storage match declaration bounded by `u32`.
pub fn read_source_primary_match(reader: &Reader) -> SourcePrimaryMatch {
    SourcePrimaryMatch {
        score: reader.field("score").integer_u32(0),
        teams: reader.field("teams").list(|team| SourceTeam {
            source: namespaced(&team.field("source")),
            team: namespaced(&team.field("team")),
            arguments: team.field("arguments").list(|argument| argument.string()),
        }),
    }
}

/// Read a weapon profile subtree.
pub fn read_native_primary_weapons(
    reader: &Reader,
    digest: String,
    abi: NativeAbi,
) -> NativePrimaryWeaponProfile {
    let dispatcher = reader.field("dispatcher");
    let spawn = reader.field("spawn");
    let time = reader.field("time");
    let entity = reader.field("entity");
    let client = reader.field("client");
    let animation = reader.field("animation");
    let attack = reader.field("attackAnimation");
    let delay = reader.field("delay");
    let evaluate = delay.field("evaluate");
    let damage = reader.field("damage");
    dispatcher.field("entry").field("kind").literal_str("rva");
    let argument = dispatcher.field("argument").integer_u32(0);
    let arguments = dispatcher.field("arguments").integer_u32(1);
    if argument >= arguments {
        dispatcher.fail("dispatcher actor argument is outside its call");
    }
    dispatcher.field("record").literal_str("entity");
    let milliseconds = time.field("milliseconds").finite();
    if milliseconds <= 0.0 {
        time.fail("clock scale must be positive");
    }
    let equipment_contexts = reader.field("equipmentContexts").list(|value| EquipmentContext {
        provider: namespaced(&value.field("provider")),
        item: value.field("item").nullable(namespaced),
    });
    let mut providers = std::collections::HashSet::new();
    for context in &equipment_contexts {
        if !providers.insert(context.provider.clone()) {
            reader.field("equipmentContexts").fail("duplicate equipment source context");
        }
    }
    NativePrimaryWeaponProfile {
        digest,
        abi,
        equipment_contexts,
        dispatcher: WeaponDispatcher {
            entry_rva: native_offset(&dispatcher.field("entry").field("rva")),
            record: RecordKind::Entity,
            argument,
            arguments,
        },
        decisions: reader.field("decisions").list(|value| {
            let region = super::native_primary_reader::native_region(value);
            WeaponDecision {
                entry: region.entry,
                join: region.join,
                fields: value.field("fields").list(|value| DecisionField {
                    field: native_field(&value.field("field")),
                    clear_mask: value.field("clearMask").integer_u32(1),
                }),
            }
        }),
        spawn: SpawnGate {
            entry: native_offset(&spawn.field("entry")),
            accepted: spawn.field("accepted").list(native_test),
        },
        active: reader.field("active").list(native_test),
        committed_input: reader
            .field("committedInput")
            .list(|value| value.list(native_test)),
        continuations: reader
            .field("continuations")
            .list(|value| value.list(native_test)),
        time: WeaponTime {
            address: native_offset(&time.field("address")),
            encoding: native_scalar(&time.field("encoding")),
            milliseconds,
        },
        entity: WeaponEntity {
            client: native_offset(&entity.field("client")),
            water_level: native_field(&entity.field("waterLevel")),
            view_height: native_field(&entity.field("viewHeight")),
            max_health: native_field(&entity.field("maxHealth")),
        },
        client: WeaponClient {
            byte_length: client.field("byteLength").integer_u32(1),
            view_angles: native_offset(&client.field("viewAngles")),
            buttons: native_field(&client.field("buttons")),
            latched_buttons: native_field(&client.field("latchedButtons")),
        },
        attack_animation: AttackAnimation {
            entry: native_offset(&attack.field("entry")),
            skip: attack
                .field("skip")
                .list(super::native_primary_reader::native_region),
        },
        animation: WeaponAnimation {
            frame: native_field(&animation.field("frame")),
            end: native_field(&animation.field("end")),
            priority: native_field(&animation.field("priority")),
            duck: native_field(&animation.field("duck")),
            run: native_field(&animation.field("run")),
        },
        delay: WeaponDelay {
            flag: native_field(&delay.field("flag")),
            region: super::native_primary_reader::native_region(&delay.field("region")),
            evaluate: if evaluate.field("kind").choice_index(&["source-flag", "source-animation"]) == 0 {
                DelayEvaluate::SourceFlag {
                    factors: evaluate.field("factors").list(|value| value.finite()),
                }
            } else {
                DelayEvaluate::SourceAnimation {
                    entry: native_offset(&evaluate.field("entry")),
                    baseline_milliseconds: evaluate.field("baselineMilliseconds").finite(),
                    projection: evaluate.field("projection").list(|value| ProjectionWrite {
                        field: native_field(&value.field("field")),
                        value: value.field("value").finite(),
                    }),
                    writes: evaluate.field("writes").list(native_field),
                }
            },
        },
        damage: if damage.field("kind").choice_index(&["source-result", "source-flag"]) == 0 {
            WeaponDamage::SourceResult {
                entry: native_offset(&damage.field("entry")),
                result: match damage.field("result").choice_index(&["uint8", "int32"]) {
                    0 => DamageResult::Uint8,
                    _ => DamageResult::Int32,
                },
            }
        } else {
            WeaponDamage::SourceFlag {
                address: native_offset(&damage.field("address")),
                encoding: native_scalar(&damage.field("encoding")),
                factors: damage.field("factors").list(|value| value.finite()),
                region: super::native_primary_reader::native_region(&damage.field("region")),
            }
        },
    }
}

/// Read a player profile subtree.
pub fn read_native_primary_player(reader: &Reader, digest: String) -> NativePrimaryPlayerProfile {
    let objectives = reader.field("objectives");
    NativePrimaryPlayerProfile {
        digest,
        match_profile: if reader.has("match") && !reader.field("match").is_null() {
            Some(read_source_primary_match(&reader.field("match")))
        } else {
            None
        },
        spawn: native_offset(&reader.field("spawn")),
        objectives: if objectives.field("kind").choice_index(&["none", "entry"]) == 0 {
            PlayerObjectives::None
        } else {
            PlayerObjectives::Entry(native_offset(&objectives.field("entry")))
        },
        command_angles: native_offset(&reader.field("commandAngles")),
        velocity: native_offset(&reader.field("velocity")),
        forward: reader.field("forward").nullable(native_offset),
    }
}

/// Read a command profile subtree.
pub fn read_native_primary_commands(
    reader: &Reader,
    digest: String,
    abi: NativeAbi,
) -> NativePrimaryCommandProfile {
    let give = reader.field("give");
    let drop = reader.field("drop");
    let client = reader.field("client");
    let items = reader.field("items");
    let ammo = items.field("ammo");
    NativePrimaryCommandProfile {
        digest,
        abi,
        give: GiveProfile {
            entry: native_offset(&give.field("entry")),
            weapons: native_offset(&give.field("weapons")),
            ammo: native_offset(&give.field("ammo")),
            unknown: super::native_primary_reader::native_region(&give.field("unknown")),
            ammo_grants: give.field("ammoGrants").list(|value| {
                let region = super::native_primary_reader::native_region(value);
                AmmoGrant {
                    entry: region.entry,
                    join: region.join,
                    descriptor: native_register(&value.field("descriptor")),
                    kind: match value.field("kind").choice_index(&["set", "add"]) {
                        0 => GrantKind::Set,
                        _ => GrantKind::Add,
                    },
                }
            }),
            argc: native_offset(&give.field("argc")),
            argv: native_offset(&give.field("argv")),
        },
        drop: DropCommand {
            entry: native_offset(&drop.field("entry")),
            eligibility: super::native_primary_reader::native_region(&drop.field("eligibility")),
        },
        client: CommandClient {
            pointer: native_offset(&client.field("pointer")),
            weapon: native_offset(&client.field("weapon")),
            ammo_index: client.field("ammoIndex").nullable(native_offset),
            inventory: native_offset(&client.field("inventory")),
        },
        items: CommandItems {
            table: native_offset(&items.field("table")),
            stride: items.field("stride").integer_u32(1),
            count: items.field("count").integer_u32(1),
            classname: native_offset(&items.field("classname")),
            flags: native_offset(&items.field("flags")),
            weapon_flag: items.field("weaponFlag").integer_u32(1),
            ammunition_flag: items.field("ammunitionFlag").integer_u32(1),
            icon: native_offset(&items.field("icon")),
            ammo: if ammo.field("kind").choice_index(&["name", "index"]) == 0 {
                ItemAmmo::Name {
                    offset: native_offset(&ammo.field("offset")),
                    label: native_offset(&ammo.field("label")),
                }
            } else {
                ItemAmmo::Index {
                    offset: native_offset(&ammo.field("offset")),
                }
            },
        },
    }
}

fn integer_i32(reader: &Reader, min: i64) -> i32 {
    let value = reader.integer(min);
    i32::try_from(value).unwrap_or_else(|_| reader.fail("expected int32"))
}

/// Read an inventory profile subtree.
pub fn read_native_primary_inventory(
    reader: &Reader,
    digest: String,
    abi: NativeAbi,
) -> NativePrimaryInventoryProfile {
    let prototypes = reader.field("prototypes");
    let next = reader.field("next");
    let previous = reader.field("previous");
    let validate = reader.field("validate");
    let use_profile = reader.field("use");
    let named_use = reader.field("namedUse");
    let next_region = super::native_primary_reader::native_region(&next);
    let previous_region = super::native_primary_reader::native_region(&previous);
    let use_region = super::native_primary_reader::native_region(&use_profile);
    let named_region = super::native_primary_reader::native_region(&named_use);
    NativePrimaryInventoryProfile {
        digest,
        abi,
        client: native_offset(&reader.field("client")),
        inventory: native_offset(&reader.field("inventory")),
        count: reader.field("count").integer_u32(1),
        cursor: native_offset(&reader.field("cursor")),
        empty: integer_i32(&reader.field("empty"), i64::MIN),
        prototypes: InventoryPrototypes {
            weapon: namespaced(&prototypes.field("weapon")),
            ammunition: namespaced(&prototypes.field("ammunition")),
            usable: namespaced(&prototypes.field("usable")),
            passive: namespaced(&prototypes.field("passive")),
            droppable: namespaced(&prototypes.field("droppable")),
            undroppable: namespaced(&prototypes.field("undroppable")),
        },
        selection_writes: reader.field("selectionWrites").list(|value| SelectionWrite {
            offset: native_offset(&value.field("offset")),
            bytes: value.field("bytes").integer_u32(1),
        }),
        next: NextProfile {
            entry: next_region.entry,
            join: next_region.join,
            scan: native_offset(&next.field("scan")),
            menu_argument: next.field("menuArgument").boolean(),
        },
        previous: PreviousProfile {
            entry: previous_region.entry,
            join: previous_region.join,
            scan: native_offset(&previous.field("scan")),
        },
        validate: ValidateProfile {
            entry: native_offset(&validate.field("entry")),
            scan: validate
                .field("scan")
                .nullable(super::native_primary_reader::native_region),
        },
        use_profile: UseProfile {
            entry: use_region.entry,
            join: use_region.join,
            call: native_offset(&use_profile.field("call")),
        },
        named_use: NamedUseProfile {
            entry: named_region.entry,
            join: named_region.join,
            call: native_offset(&named_use.field("call")),
            lookup_call: native_offset(&named_use.field("lookupCall")),
            lookup_return: native_offset(&named_use.field("lookupReturn")),
        },
    }
}

/// Read a drop profile subtree.
pub fn read_native_primary_drop(
    reader: &Reader,
    digest: String,
    abi: NativeAbi,
) -> NativePrimaryDropProfile {
    let client = reader.field("client");
    let inventory = reader.field("inventory");
    NativePrimaryDropProfile {
        digest,
        abi,
        client: DropClient {
            pointer: native_offset(&client.field("pointer")),
            inventory: native_offset(&client.field("inventory")),
            cursor: native_offset(&client.field("cursor")),
            weapon: native_offset(&client.field("weapon")),
            pending: native_offset(&client.field("pending")),
        },
        named: native_offset(&reader.field("named")),
        inventory: InventoryDrop {
            entry: native_offset(&inventory.field("entry")),
            admitted: native_offset(&inventory.field("admitted")),
        },
        find: native_offset(&reader.field("find")),
        lookup_return: native_offset(&reader.field("lookupReturn")),
        allocate: native_offset(&reader.field("allocate")),
        free: native_offset(&reader.field("free")),
        callbacks: reader
            .field("callbacks")
            .list(super::native_primary_reader::native_region),
        debits: reader
            .field("debits")
            .list(super::native_primary_reader::native_region),
        consumer: reader
            .field("consumer")
            .nullable(super::native_primary_reader::native_region),
    }
}

/// Read a pickup profile subtree.
pub fn read_native_primary_pickups(
    reader: &Reader,
    digest: String,
    abi: NativeAbi,
) -> NativePickupProfile {
    let items = reader.field("items");
    let entity = reader.field("entity");
    let time = reader.field("time");
    let supply = reader.field("supply");
    let ammo = supply.field("ammo");
    NativePickupProfile {
        digest,
        abi,
        touch: native_offset(&reader.field("touch")),
        grant_return: native_offset(&reader.field("grantReturn")),
        targets_return: native_offset(&reader.field("targetsReturn")),
        touch_signature: native_signature(&reader.field("touchSignature"), abi),
        grant_signature: native_signature(&reader.field("grantSignature"), abi),
        grants: reader.field("grants").list(|value| {
            let supply_value = if value.has("supply") && !value.field("supply").is_null() {
                Some(value.field("supply"))
            } else {
                None
            };
            NativePickupGrant {
                entry: native_offset(&value.field("entry")),
                recipient: super::native_primary_reader::native_region(&value.field("recipient")),
                resource: match value.field("resource").choice_index(&["regular", "inventory"]) {
                    0 => PickupResource::Regular,
                    _ => PickupResource::Inventory,
                },
                consumers: if value.has("consumers") && !value.field("consumers").is_null() {
                    value.field("consumers").list(|value| PickupConsumer {
                        entry: native_offset(&value.field("entry")),
                        signature: native_signature(&value.field("signature"), abi),
                        protection: match value.field("protection").choice_index(&["regular", "powered"]) {
                            0 => ProtectionChannel::Regular,
                            _ => ProtectionChannel::Powered,
                        },
                    })
                } else {
                    Vec::new()
                },
                supply: supply_value.map(|supply| {
                    if supply.field("kind").choice_index(&["ammo", "weapon"]) == 0 {
                        PickupSupply::Ammo {
                            entry: native_offset(&supply.field("entry")),
                            amount: native_register(&supply.field("amount")),
                        }
                    } else {
                        PickupSupply::Weapon {
                            ammo_return: native_offset(&supply.field("ammoReturn")),
                            settle: native_offset(&supply.field("settle")),
                            autoswitch: super::native_primary_reader::native_region(
                                &supply.field("autoswitch"),
                            ),
                        }
                    }
                }),
            }
        }),
        items: PickupItems {
            table: native_offset(&items.field("table")),
            stride: items.field("stride").integer_u32(1),
            count: items.field("count").integer_u32(1),
            classname: native_offset(&items.field("classname")),
            pickup: native_offset(&items.field("pickup")),
        },
        entity: PickupEntity {
            item: native_offset(&entity.field("item")),
            count: native_offset(&entity.field("count")),
            spawnflags: native_offset(&entity.field("spawnflags")),
            inuse: native_offset(&entity.field("inuse")),
            inuse_bytes: entity.field("inuseBytes").choice_int(&[1, 4]) as u8,
            generation: entity.field("generation").nullable(native_offset),
        },
        time: PickupTime {
            address: native_offset(&time.field("address")),
            storage: match time.field("storage").choice_index(&["float32-seconds", "int64-milliseconds"]) {
                0 => TimeStorage::FloatSeconds,
                _ => TimeStorage::Int64Milliseconds,
            },
        },
        supply: PickupSupplyProfile {
            client: native_offset(&supply.field("client")),
            inventory: native_offset(&supply.field("inventory")),
            flags: native_offset(&supply.field("flags")),
            weapon_flag: supply.field("weaponFlag").integer_u32(1),
            ammo: AmmoSupply {
                entry: native_offset(&ammo.field("entry")),
                signature: native_signature(&ammo.field("signature"), abi),
                stop: ammo.field("stop").nullable(native_offset),
                tag: native_offset(&ammo.field("tag")),
                capacities: ammo.field("capacities").list(native_offset),
                capacity_bytes: ammo.field("capacityBytes").choice_int(&[2, 4]) as u8,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::{CLASSIC_DIGEST, parse_json};
    use super::*;

    const ABI: &str = r#"{"kind":"windows-i386","image":"pe32","call":"cdecl","pointerBytes":4}"#;

    fn field(record: &str, offset: u32, encoding: &str) -> String {
        format!(r#"{{"record":"{record}","offset":{offset},"encoding":"{encoding}"}}"#)
    }

    #[test]
    fn reads_weapon_profiles() {
        let text = format!(
            r#"{{"dispatcher":{{"entry":{{"kind":"rva","rva":222000}},"record":"entity","argument":0,"arguments":1}},
            "decisions":[],"spawn":{{"entry":203216,"accepted":[]}},"active":[],"committedInput":[],"continuations":[],
            "time":{{"address":485632,"encoding":"float32","milliseconds":1000}},
            "entity":{{"client":84,"waterLevel":{},"viewHeight":{},"maxHealth":{}}},
            "client":{{"byteLength":3832,"viewAngles":3652,"buttons":{},"latchedButtons":{}}},
            "attackAnimation":{{"entry":224096,"skip":[]}},
            "animation":{{"frame":{},"end":{},"priority":{},"duck":{},"run":{}}},
            "equipmentContexts":[],
            "delay":{{"flag":{},"region":{{"entry":223343,"join":223379}},"evaluate":{{"kind":"source-flag","factors":[1,0.5]}}}},
            "damage":{{"kind":"source-flag","address":438672,"encoding":"int32","factors":[1,4],"region":{{"entry":223304,"join":223343}}}}}}"#,
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
        );
        let value = parse_json(&text).expect("valid json");
        let root = Reader::root(&value);
        let profile = read_native_primary_weapons(
            &root,
            CLASSIC_DIGEST.to_string(),
            NativeAbi::WindowsI386,
        );
        assert_eq!(profile.dispatcher.entry_rva, 222000);
        assert_eq!(profile.client.byte_length, 3832);
        assert!(matches!(
            profile.damage,
            super::super::native_primary_weapons::WeaponDamage::SourceFlag { .. }
        ));
    }

    #[test]
    fn reads_player_profiles_with_matches() {
        let value = parse_json(
            r#"{"match":{"score":3464,"teams":[{"source":"q2:1","team":"team:red","arguments":["team","red"]}]},
            "spawn":201120,"objectives":{"kind":"none"},"commandAngles":3476,"velocity":376,"forward":null}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        let profile = read_native_primary_player(&root, CLASSIC_DIGEST.to_string());
        assert_eq!(profile.spawn, 201120);
        assert_eq!(profile.objectives, PlayerObjectives::None);
        assert_eq!(profile.match_profile.expect("match").teams.len(), 1);
    }

    #[test]
    fn reads_inventory_and_drop_profiles() {
        let value = parse_json(
            r#"{"client":84,"inventory":740,"count":256,"cursor":736,"empty":-1,
            "prototypes":{"weapon":"q2:weapon_blaster","ammunition":"q2:ammo_shells","usable":"q2:item_quad",
            "passive":"q2:key_data_cd","droppable":"q2:item_quad","undroppable":"q2:weapon_blaster"},
            "selectionWrites":[{"offset":736,"bytes":4}],
            "next":{"entry":12256,"scan":12291,"join":12394,"menuArgument":false},
            "previous":{"entry":12400,"scan":12435,"join":12543},
            "validate":{"entry":12560,"scan":null},
            "use":{"entry":14944,"call":15036,"join":15038},
            "namedUse":{"entry":14032,"lookupCall":14045,"lookupReturn":14050,"call":14412,"join":14415}}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        let profile = read_native_primary_inventory(
            &root,
            CLASSIC_DIGEST.to_string(),
            NativeAbi::WindowsI386,
        );
        assert_eq!(profile.count, 256);
        assert_eq!(profile.empty, -1);
        assert!(!profile.next.menu_argument);
        let value = parse_json(
            r#"{"client":{"pointer":84,"inventory":740,"cursor":736,"weapon":1796,"pending":3548},
            "named":14432,"inventory":{"entry":15488,"admitted":15499},"find":38288,"lookupReturn":14450,
            "allocate":44288,"free":102976,"callbacks":[],"debits":[],"consumer":null}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        let profile = read_native_primary_drop(
            &root,
            CLASSIC_DIGEST.to_string(),
            NativeAbi::WindowsI386,
        );
        assert_eq!(profile.find, 38288);
        assert_eq!(profile.consumer, None);
    }

    #[test]
    fn reads_pickup_profiles() {
        let signature = format!(
            r#"{{"abi":{ABI},"parameters":[{{"kind":"scalar","storage":"pointer"}}],"result":"void","variadic":false}}"#
        );
        let text = format!(
            r#"{{"abi":{ABI},"touch":256,"grantReturn":272,"targetsReturn":288,
            "touchSignature":{signature},"grantSignature":{signature},
            "grants":[{{"entry":512,"recipient":{{"entry":528,"join":544}},"resource":"regular"}}],
            "items":{{"table":1024,"stride":32,"count":2,"classname":0,"pickup":8}},
            "entity":{{"item":64,"count":68,"spawnflags":72,"inuse":76,"inuseBytes":4,"generation":null}},
            "time":{{"address":1280,"storage":"float32-seconds"}},
            "supply":{{"client":84,"inventory":256,"flags":16,"weaponFlag":1,
            "ammo":{{"entry":1536,"signature":{signature},"stop":null,"tag":20,"capacities":[128],"capacityBytes":4}}}}}}"#
        );
        let value = parse_json(&text).expect("valid json");
        let root = Reader::root(&value);
        let profile = read_native_primary_pickups(
            &root,
            CLASSIC_DIGEST.to_string(),
            NativeAbi::WindowsI386,
        );
        assert_eq!(profile.grants.len(), 1);
        assert_eq!(profile.entity.inuse_bytes, 4);
        assert_eq!(profile.supply.ammo.capacity_bytes, 4);
    }

    #[test]
    #[should_panic(expected = "dispatcher actor argument")]
    fn rejects_dispatcher_arguments_outside_calls() {
        let value = parse_json(
            r#"{"dispatcher":{"entry":{"kind":"rva","rva":1},"record":"entity","argument":2,"arguments":1},
            "spawn":{"entry":1,"accepted":[]},"time":{"address":1,"encoding":"int32","milliseconds":1},
            "entity":{"client":1,"waterLevel":{"record":"entity","offset":1,"encoding":"int32"},
            "viewHeight":{"record":"entity","offset":1,"encoding":"int32"},
            "maxHealth":{"record":"entity","offset":1,"encoding":"int32"}},
            "client":{"byteLength":1,"viewAngles":1,"buttons":{"record":"client","offset":1,"encoding":"int32"},
            "latchedButtons":{"record":"client","offset":1,"encoding":"int32"}},
            "animation":{"frame":{"record":"entity","offset":1,"encoding":"int32"},
            "end":{"record":"client","offset":1,"encoding":"int32"},
            "priority":{"record":"client","offset":1,"encoding":"int32"},
            "duck":{"record":"client","offset":1,"encoding":"int32"},
            "run":{"record":"client","offset":1,"encoding":"int32"}},
            "attackAnimation":{"entry":1,"skip":[]},"decisions":[],"active":[],"committedInput":[],
            "continuations":[],"equipmentContexts":[],
            "delay":{"flag":{"record":"entity","offset":1,"encoding":"int32"},
            "region":{"entry":1,"join":2},"evaluate":{"kind":"source-flag","factors":[]}},
            "damage":{"kind":"source-result","entry":1,"result":"uint8"}}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        read_native_primary_weapons(&root, CLASSIC_DIGEST.to_string(), NativeAbi::WindowsI386);
    }
}
