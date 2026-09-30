//! Q2 oracle cases (data donor `verification/reference-cases/q2/cases.ts`).
//!
//! The thirty-two pinned input/expected pairs with contracts, source spans,
//! and preconditions. The case data lives outside `tools/` in the donor
//! tree; it is transcribed here so the capture runs, and the TypeScript file
//! remains the data authority: every capture identifies it in
//! `modelIdentities`. Inputs and expectations are stored as JSON text in
//! donor key order and parsed once by [`q2_cases`].

use crate::error::ToolsError;
use crate::json::{parse_json, Json};
use crate::reference::q2::sources::{SourceId, SourceLocation};

/// A pinned Q2 oracle case.
#[derive(Debug, Clone)]
pub struct Q2Case {
    /// Stable case identifier.
    pub id: &'static str,
    /// Contract the case pins.
    pub contract: &'static str,
    /// Backing original-source spans.
    pub sources: Vec<SourceLocation>,
    /// Case preconditions.
    pub preconditions: Vec<String>,
    /// Oracle input document.
    pub input: Json,
    /// Expected oracle output.
    pub expected: Json,
}

impl Q2Case {
    /// Render with donor field order plus capture results.
    #[must_use]
    pub fn to_json(&self, actual: &Json, passed: bool) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(self.id)),
            ("contract".to_owned(), Json::string(self.contract)),
            (
                "sources".to_owned(),
                Json::array(self.sources.iter().map(SourceLocation::to_json).collect()),
            ),
            (
                "preconditions".to_owned(),
                Json::array(self.preconditions.iter().map(Json::string).collect()),
            ),
            ("input".to_owned(), self.input.clone()),
            ("expected".to_owned(), self.expected.clone()),
            ("oracleKind".to_owned(), Json::string("source-derived")),
            (
                "rng".to_owned(),
                Json::object(vec![
                    ("draws".to_owned(), Json::int(0)),
                    ("owner".to_owned(), Json::Null),
                ]),
            ),
            ("actual".to_owned(), actual.clone()),
            ("passed".to_owned(), Json::boolean(passed)),
        ])
    }
}

const CLASSIC_THINK: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::ClassicPhys,
    first_line: 95,
    last_line: 111,
    symbol: "SV_RunThink",
}];
const RERELEASE_THINK: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::RereleasePhys,
    first_line: 99,
    last_line: 113,
    symbol: "SV_RunThink",
}];
const PICKUP_SOURCES: [SourceLocation; 3] = [
    SourceLocation {
        source: SourceId::ClassicLocal,
        first_line: 234,
        last_line: 240,
        symbol: "gitem_t.pickup",
    },
    SourceLocation {
        source: SourceId::ClassicItems,
        first_line: 447,
        last_line: 510,
        symbol: "Add_Ammo/Pickup_Ammo",
    },
    SourceLocation {
        source: SourceId::ClassicItems,
        first_line: 761,
        last_line: 821,
        symbol: "Touch_Item",
    },
];
const ARMOR_SOURCES: [SourceLocation; 4] = [
    SourceLocation {
        source: SourceId::ClassicLocal,
        first_line: 202,
        last_line: 210,
        symbol: "gitem_armor_t",
    },
    SourceLocation {
        source: SourceId::ClassicItems,
        first_line: 39,
        last_line: 41,
        symbol: "armor_info",
    },
    SourceLocation {
        source: SourceId::ClassicCombat,
        first_line: 255,
        last_line: 292,
        symbol: "CheckArmor",
    },
    SourceLocation {
        source: SourceId::ClassicCombat,
        first_line: 474,
        last_line: 477,
        symbol: "T_Damage remaining take",
    },
];
const CROSS_SOURCES: [SourceLocation; 3] = [
    SourceLocation {
        source: SourceId::RereleaseLocal,
        first_line: 251,
        last_line: 262,
        symbol: "SPAWNFLAG_EDITOR_MASK",
    },
    SourceLocation {
        source: SourceId::RereleaseLocal,
        first_line: 733,
        last_line: 733,
        symbol: "SFL_CROSS_TRIGGER_MASK",
    },
    SourceLocation {
        source: SourceId::RereleaseTarget,
        first_line: 1985,
        last_line: 2031,
        symbol: "trigger_crossunit_trigger_use/target_crossunit_target_think",
    },
];
const COMMAND_SOURCES: [SourceLocation; 5] = [
    SourceLocation {
        source: SourceId::ClassicMove,
        first_line: 779,
        last_line: 822,
        symbol: "PM_CheckJump",
    },
    SourceLocation {
        source: SourceId::ClassicMove,
        first_line: 994,
        last_line: 1003,
        symbol: "PM_CheckDuck",
    },
    SourceLocation {
        source: SourceId::RereleaseGame,
        first_line: 417,
        last_line: 425,
        symbol: "button_t",
    },
    SourceLocation {
        source: SourceId::RereleaseMove,
        first_line: 1065,
        last_line: 1102,
        symbol: "PM_CheckJump",
    },
    SourceLocation {
        source: SourceId::RereleaseMove,
        first_line: 1372,
        last_line: 1391,
        symbol: "PM_CheckDuck",
    },
];
const CONFIG_SOURCES: [SourceLocation; 2] = [
    SourceLocation {
        source: SourceId::RereleaseSpawn,
        first_line: 1517,
        last_line: 1529,
        symbol: "SP_worldspawn Q64 and air acceleration",
    },
    SourceLocation {
        source: SourceId::RereleaseCgame,
        first_line: 19,
        last_line: 26,
        symbol: "InitCGame configuration consumption",
    },
];
const CLOCK_SOURCES: [SourceLocation; 7] = [
    SourceLocation {
        source: SourceId::ClassicLocal,
        first_line: 73,
        last_line: 73,
        symbol: "FRAMETIME",
    },
    SourceLocation {
        source: SourceId::ClassicLocal,
        first_line: 301,
        last_line: 304,
        symbol: "level_locals_t.time",
    },
    SourceLocation {
        source: SourceId::ClassicMain,
        first_line: 353,
        last_line: 359,
        symbol: "G_RunFrame",
    },
    SourceLocation {
        source: SourceId::RereleaseLocal,
        first_line: 288,
        last_line: 309,
        symbol: "gtime_t",
    },
    SourceLocation {
        source: SourceId::RereleaseMain,
        first_line: 415,
        last_line: 415,
        symbol: "FRAME_TIME_MS initialization",
    },
    SourceLocation {
        source: SourceId::RereleaseMain,
        first_line: 823,
        last_line: 831,
        symbol: "G_RunFrame_",
    },
    SourceLocation {
        source: SourceId::RereleaseReadme,
        first_line: 46,
        last_line: 48,
        symbol: "40hz Tickrate Support",
    },
];
const FRAME_SOURCES: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::ClassicMain,
    first_line: 353,
    last_line: 410,
    symbol: "G_RunFrame",
}];
const FRAME_LOOP_SOURCES: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::ClassicMain,
    first_line: 376,
    last_line: 410,
    symbol: "G_RunFrame live edict loop",
}];
const SAVE_POI_SOURCES: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::RereleaseSave,
    first_line: 728,
    last_line: 740,
    symbol: "level_locals_t save fields",
}];
const SAVE_FOG_SOURCES: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::RereleaseSave,
    first_line: 1262,
    last_line: 1301,
    symbol: "edict_t fog/bmodel_anim fields",
}];
const SAVE_AMMO_SOURCES: [SourceLocation; 3] = [
    SourceLocation {
        source: SourceId::RereleaseSave,
        first_line: 484,
        last_line: 497,
        symbol: "std::array save type",
    },
    SourceLocation {
        source: SourceId::RereleaseSave,
        first_line: 788,
        last_line: 802,
        symbol: "client_persistant_t.max_ammo",
    },
    SourceLocation {
        source: SourceId::RereleaseShared,
        first_line: 79,
        last_line: 98,
        symbol: "ammo_t",
    },
];
const SAVE_CROSS_SOURCES: [SourceLocation; 1] = [SourceLocation {
    source: SourceId::RereleaseSave,
    first_line: 661,
    last_line: 680,
    symbol: "game_locals_t save fields",
}];
const FLECHETTE_SOURCES: [SourceLocation; 2] = [
    SourceLocation {
        source: SourceId::RereleaseShared,
        first_line: 79,
        last_line: 98,
        symbol: "ammo_t",
    },
    SourceLocation {
        source: SourceId::RereleaseClient,
        first_line: 844,
        last_line: 858,
        symbol: "InitClientPersistant default ammo capacities",
    },
];

const PICKUP_PRECONDITIONS: [&str; 1] = ["Alive client; ordinary supported ammo; positive quantity; no weapon flag; singleplayer; no stay-coop or respawn flags; target callback observes without deleting the item."];
const COMMAND_PRECONDITIONS: [&str; 1] = ["Only command predicates are evaluated. Alive grounded client; landing timer and jump-held clear; no ladder; standing transition trace clear. Full movement is outside this case."];

// Each entry: id, contract, sources, preconditions, input JSON, expected JSON.
type RawCase = (
    &'static str,
    &'static str,
    &'static [SourceLocation],
    &'static [&'static str],
    &'static str,
    &'static str,
);

#[allow(clippy::too_many_lines)]
fn raw_cases() -> Vec<RawCase> {
    vec![
        (
            "q2.clock.classic10hz-rerelease40hz",
            "Classic stores frame*double(0.1) in binary32 seconds; rerelease accumulates the supplied integer millisecond frame time. The documented 40 Hz setting supplies 25 ms.",
            &CLOCK_SOURCES,
            &["Normal frame path, level starts at zero; binary32 storage with binary64 evaluation of the unsuffixed classic literal; no claim about x87 excess precision; wire codec is outside this clock."],
            r#"{"kind": "clock", "classicFrames": [1, 2, 3, 10], "rereleaseStepMs": 25, "rereleaseFrames": 4}"#,
            r#"{"classicTimesSeconds": [0.10000000149011612, 0.20000000298023224, 0.30000001192092896, 1], "rereleaseTimesMs": [25, 50, 75, 100]}"#,
        ),
        (
            "q2.think.classic-lookahead-due",
            "A nextthink less than level.time+0.001 fires early; it is zero inside the callback, callback rescheduling survives, and SV_RunThink returns false.",
            &CLASSIC_THINK,
            &["Valid synchronous think callback; values use seconds."],
            r#"{"kind": "think", "family": "classic", "now": 0.1, "nextthink": 0.1009, "reschedule": 0.2}"#,
            r#"{"now": 0.10000000149011612, "nextthink": 0.20000000298023224, "mayContinuePhysics": false, "trace": [{"event": "think.enter", "nextthink": 0}, {"event": "think.return", "nextthink": 0.20000000298023224}]}"#,
        ),
        (
            "q2.think.classic-lookahead-future",
            "A deadline beyond the classic epsilon remains pending and returns true.",
            &CLASSIC_THINK,
            &["Values use seconds."],
            r#"{"kind": "think", "family": "classic", "now": 0.1, "nextthink": 0.102, "reschedule": 0.2}"#,
            r#"{"now": 0.10000000149011612, "nextthink": 0.10199999809265137, "mayContinuePhysics": true, "trace": []}"#,
        ),
        (
            "q2.think.classic-disabled",
            "A zero deadline disables think even after level time has advanced.",
            &CLASSIC_THINK,
            &["Values use seconds."],
            r#"{"kind": "think", "family": "classic", "now": 1, "nextthink": 0, "reschedule": 2}"#,
            r#"{"now": 1, "nextthink": 0, "mayContinuePhysics": true, "trace": []}"#,
        ),
        (
            "q2.think.rerelease-one-ms-future",
            "Rerelease does not apply the classic 1 ms lookahead.",
            &RERELEASE_THINK,
            &["Values use integer milliseconds."],
            r#"{"kind": "think", "family": "rerelease", "now": 100, "nextthink": 101, "reschedule": 200}"#,
            r#"{"now": 100, "nextthink": 101, "mayContinuePhysics": true, "trace": []}"#,
        ),
        (
            "q2.think.rerelease-equal-deadline",
            "An equal rerelease deadline runs synchronously, clears nextthink first, preserves rescheduling, and returns false.",
            &RERELEASE_THINK,
            &["Valid synchronous think callback; integer milliseconds."],
            r#"{"kind": "think", "family": "rerelease", "now": 100, "nextthink": 100, "reschedule": 125}"#,
            r#"{"now": 100, "nextthink": 125, "mayContinuePhysics": false, "trace": [{"event": "think.enter", "nextthink": 0}, {"event": "think.return", "nextthink": 125}]}"#,
        ),
        (
            "q2.frame.classic-world-client-order",
            "The world runs first, the client takes ClientBeginServerFrame, later edicts run in slot order, then rules and final client frames run.",
            &FRAME_SOURCES,
            &["No intermission, ground changes, or entity callback mutations. Slots 0,1,2,3 in use; maxclients=1; entity callback order is recorded, not complete physics."],
            r#"{"kind": "frame-order", "deleteLaterActor": false, "spawnActor": false}"#,
            r#"{"trace": ["level.framenum++", "level.time=frame*0.1", "AI_SetSightClient", {"event": "current_entity+old_origin", "actor": 0}, {"event": "G_RunEntity.enter", "actor": 0}, {"event": "G_RunEntity.return", "actor": 0}, {"event": "current_entity+old_origin", "actor": 1}, {"event": "ClientBeginServerFrame", "actor": 1}, {"event": "current_entity+old_origin", "actor": 2}, {"event": "G_RunEntity.enter", "actor": 2}, {"event": "G_RunEntity.return", "actor": 2}, {"event": "current_entity+old_origin", "actor": 3}, {"event": "G_RunEntity.enter", "actor": 3}, {"event": "G_RunEntity.return", "actor": 3}, "CheckDMRules", "ClientEndServerFrames"]}"#,
        ),
        (
            "q2.frame.classic-live-edict-loop",
            "The loop reads current num_edicts and inuse each iteration: an authored append is visible in the same frame and a deleted later slot is skipped.",
            &FRAME_LOOP_SOURCES,
            &["No intermission or ground changes; maxclients=1. Callback at slot 2 explicitly clears slot 3 inuse, appends active slot 4, and sets num_edicts=5. This fixture does not model G_Spawn allocator policy."],
            r#"{"kind": "frame-order", "deleteLaterActor": true, "spawnActor": true}"#,
            r#"{"trace": ["level.framenum++", "level.time=frame*0.1", "AI_SetSightClient", {"event": "current_entity+old_origin", "actor": 0}, {"event": "G_RunEntity.enter", "actor": 0}, {"event": "G_RunEntity.return", "actor": 0}, {"event": "current_entity+old_origin", "actor": 1}, {"event": "ClientBeginServerFrame", "actor": 1}, {"event": "current_entity+old_origin", "actor": 2}, {"event": "G_RunEntity.enter", "actor": 2}, {"event": "authored.delete", "actor": 3}, {"event": "authored.append", "actor": 4}, {"event": "G_RunEntity.return", "actor": 2}, {"event": "current_entity+old_origin", "actor": 4}, {"event": "G_RunEntity.enter", "actor": 4}, {"event": "G_RunEntity.return", "actor": 4}, "CheckDMRules", "ClientEndServerFrames"]}"#,
        ),
        (
            "q2.pickup.full-ammo-still-uses-targets",
            "False pickup return suppresses feedback/freeing, but targets fire before the failed-pickup early return and before ITEM_TARGETS_USED is set.",
            &PICKUP_SOURCES,
            &PICKUP_PRECONDITIONS,
            r#"{"kind": "pickup", "inventory": 200, "capacity": 200, "quantity": 50, "targetsUsed": false}"#,
            r#"{"taken": false, "targetsUsed": true, "freed": false, "inventory": 200, "trace": [{"event": "Pickup_Ammo.enter", "inventory": 200}, {"event": "Pickup_Ammo.return", "taken": false, "inventory": 200}, {"event": "Touch_Item.observes-return", "taken": false}, {"event": "G_UseTargets.enter", "inventory": 200, "targetsUsed": false}, "G_UseTargets.return", "ITEM_TARGETS_USED=set"]}"#,
        ),
        (
            "q2.pickup.targets-once",
            "ITEM_TARGETS_USED suppresses a repeat target callback when the pickup still fails.",
            &PICKUP_SOURCES,
            &PICKUP_PRECONDITIONS,
            r#"{"kind": "pickup", "inventory": 200, "capacity": 200, "quantity": 50, "targetsUsed": true}"#,
            r#"{"taken": false, "targetsUsed": true, "freed": false, "inventory": 200, "trace": [{"event": "Pickup_Ammo.enter", "inventory": 200}, {"event": "Pickup_Ammo.return", "taken": false, "inventory": 200}, {"event": "Touch_Item.observes-return", "taken": false}]}"#,
        ),
        (
            "q2.pickup.partial-ammo-synchronous-return",
            "Clamped partial pickup returns true; targets synchronously observe the updated inventory before the item is freed.",
            &PICKUP_SOURCES,
            &PICKUP_PRECONDITIONS,
            r#"{"kind": "pickup", "inventory": 199, "capacity": 200, "quantity": 50, "targetsUsed": false}"#,
            r#"{"taken": true, "targetsUsed": true, "freed": true, "inventory": 200, "trace": [{"event": "Pickup_Ammo.enter", "inventory": 199}, {"event": "Pickup_Ammo.return", "taken": true, "inventory": 200}, {"event": "Touch_Item.observes-return", "taken": true}, "pickup.feedback", {"event": "G_UseTargets.enter", "inventory": 200, "targetsUsed": false}, "G_UseTargets.return", "ITEM_TARGETS_USED=set", "G_FreeEdict"]}"#,
        ),
        (
            "q2.armor.round-up-one-damage",
            "Armor saves ceil(binary32(protection*damage)), so jacket armor absorbs one normal damage entirely.",
            &ARMOR_SOURCES,
            &["Client with selected armor, positive damage entering CheckArmor; no preceding damage modifiers modeled."],
            r#"{"kind": "armor", "damage": 1, "inventory": 50, "armor": "jacket", "energy": false, "bypass": false}"#,
            r#"{"absorbed": 1, "remainingArmor": 49, "remainingDamage": 0, "effect": "SpawnDamage"}"#,
        ),
        (
            "q2.armor.binary32-before-ceil",
            "The binary32 product of stored 0.3f and 10 is 3, so ceil saves 3. Promoting stored 0.3f directly to binary64 multiplication would incorrectly save 4 in this model.",
            &ARMOR_SOURCES,
            &["Binary32 multiplication without x87 excess precision; client with jacket armor; normal damage."],
            r#"{"kind": "armor", "damage": 10, "inventory": 50, "armor": "jacket", "energy": false, "bypass": false}"#,
            r#"{"absorbed": 3, "remainingArmor": 47, "remainingDamage": 7, "effect": "SpawnDamage"}"#,
        ),
        (
            "q2.armor.energy-jacket",
            "Jacket energy protection is zero and emits no armor spark event.",
            &ARMOR_SOURCES,
            &["Client with jacket armor; no preceding damage modifiers modeled."],
            r#"{"kind": "armor", "damage": 10, "inventory": 50, "armor": "jacket", "energy": true, "bypass": false}"#,
            r#"{"absorbed": 0, "remainingArmor": 50, "remainingDamage": 10, "effect": null}"#,
        ),
        (
            "q2.armor.clamp-inventory",
            "Absorption cannot exceed remaining armor inventory.",
            &ARMOR_SOURCES,
            &["Client with body armor; no preceding damage modifiers modeled."],
            r#"{"kind": "armor", "damage": 10, "inventory": 2, "armor": "body", "energy": false, "bypass": false}"#,
            r#"{"absorbed": 2, "remainingArmor": 0, "remainingDamage": 8, "effect": "SpawnDamage"}"#,
        ),
        (
            "q2.armor.no-armor-flag",
            "DAMAGE_NO_ARMOR returns zero before inventory mutation or effects.",
            &ARMOR_SOURCES,
            &["Client with body armor; no preceding damage modifiers modeled."],
            r#"{"kind": "armor", "damage": 10, "inventory": 100, "armor": "body", "energy": false, "bypass": true}"#,
            r#"{"absorbed": 0, "remainingArmor": 100, "remainingDamage": 10, "effect": null}"#,
        ),
        (
            "q2.cross-unit.all-required",
            "A trigger ORs its bits into unit flags; the target requires every requested non-editor bit before using targets and freeing itself.",
            &CROSS_SOURCES,
            &["Non-deathmatch target think invoked; synchronous G_UseTargets callback returns without mutation."],
            r#"{"kind": "cross-unit", "flags": 1, "trigger": 2, "required": 3}"#,
            r#"{"flags": 3, "satisfied": true, "trace": ["flags|=trigger", "G_FreeEdict(trigger)", "G_UseTargets(target,target)", "G_FreeEdict(target)"]}"#,
        ),
        (
            "q2.cross-unit.missing-required",
            "One missing required bit prevents progression.",
            &CROSS_SOURCES,
            &["Non-deathmatch target think invoked."],
            r#"{"kind": "cross-unit", "flags": 1, "trigger": 4, "required": 3}"#,
            r#"{"flags": 5, "satisfied": false, "trace": ["flags|=trigger", "G_FreeEdict(trigger)"]}"#,
        ),
        (
            "q2.cross-unit.high-bit-unsigned",
            "The uint32 comparison supports high trigger bits without signed JavaScript comparison loss.",
            &CROSS_SOURCES,
            &["Non-deathmatch target think invoked."],
            r#"{"kind": "cross-unit", "flags": 0, "trigger": 2147483648, "required": 2147483648}"#,
            r#"{"flags": 2147483648, "satisfied": true, "trace": ["flags|=trigger", "G_FreeEdict(trigger)", "G_UseTargets(target,target)", "G_FreeEdict(target)"]}"#,
        ),
        (
            "q2.cross-unit.editor-bits-excluded",
            "The cross-trigger mask excludes bits 8 through 15, including an authored required editor bit.",
            &CROSS_SOURCES,
            &["Direct target-think input; spawn parsing/inhibition is outside this malformed-requirement boundary case."],
            r#"{"kind": "cross-unit", "flags": 0, "trigger": 256, "required": 256}"#,
            r#"{"flags": 256, "satisfied": false, "trace": ["flags|=trigger", "G_FreeEdict(trigger)"]}"#,
        ),
        (
            "q2.save.poi-story-fields",
            "Original FIELD_AUTO declarations retain these POI, health bar, and story fields. This is a field-selection projection, not a save codec round trip.",
            &SAVE_POI_SOURCES,
            &["Entity references are represented symbolically; JSON encoding, pointer remapping, and load hooks are not evaluated."],
            r#"{"kind": "save-fields", "struct": "level_locals_t", "fields": {"current_poi_stage": 9, "valid_poi": true, "current_poi_image": 7, "current_dynamic_poi": "edict:17", "health_bar_entities": ["edict:18"], "story_active": true, "not_a_save_field": 999}}"#,
            r#"{"retained": {"current_poi_stage": 9, "valid_poi": true, "current_poi_image": 7, "current_dynamic_poi": "edict:17", "health_bar_entities": ["edict:18"], "story_active": true}, "omitted": ["not_a_save_field"]}"#,
        ),
        (
            "q2.save.fog-brush-animation-fields",
            "The source declares entity fog and all sampled brush animation fields, including enabled and next_tick.",
            &SAVE_FOG_SOURCES,
            &["Field selection only; supplied density is the binary32 value of authored 0.35; next_tick is represented in integer milliseconds."],
            r#"{"kind": "save-fields", "struct": "edict_t", "fields": {"fog.density": 0.3499999940395355, "fog.sky_factor": 0.5, "bmodel_anim.enabled": true, "bmodel_anim.start": 5, "bmodel_anim.end": 12, "bmodel_anim.alternate": true, "bmodel_anim.currently_alternate": false, "bmodel_anim.next_tick": 125}}"#,
            r#"{"retained": {"fog.density": 0.3499999940395355, "fog.sky_factor": 0.5, "bmodel_anim.enabled": true, "bmodel_anim.start": 5, "bmodel_anim.end": 12, "bmodel_anim.alternate": true, "bmodel_anim.currently_alternate": false, "bmodel_anim.next_tick": 125}, "omitted": []}"#,
        ),
        (
            "q2.save.max-ammo-array",
            "Rerelease saves the max_ammo array, whose index 8 stores flechette capacity. A TS-only max_flechettes field name is not the original representation.",
            &SAVE_AMMO_SOURCES,
            &["Field-selection projection; valid int16 capacities in original enum order; no byte codec claimed."],
            r#"{"kind": "save-fields", "struct": "client_persistant_t", "fields": {"max_ammo": [200, 100, 50, 50, 200, 50, 50, 5, 200, 5, 12, 50], "max_flechettes": 200}}"#,
            r#"{"retained": {"max_ammo": [200, 100, 50, 50, 200, 50, 50, 5, 200, 5, 12, 50]}, "omitted": ["max_flechettes"]}"#,
        ),
        (
            "q2.save.cross-unit-fields",
            "Game-level serialization has separate cross-level and cross-unit flag fields.",
            &SAVE_CROSS_SOURCES,
            &["Field selection only."],
            r#"{"kind": "save-fields", "struct": "game_locals_t", "fields": {"cross_level_flags": 3, "cross_unit_flags": 65537}}"#,
            r#"{"retained": {"cross_level_flags": 3, "cross_unit_flags": 65537}, "omitted": []}"#,
        ),
        (
            "q2.flechette.source-default",
            "Extract the original ammo enum index and initialized flechette capacity directly from raw source.",
            &FLECHETTE_SOURCES,
            &["The !taken_loadout initialization branch is selected; capacity upgrades and custom loadouts are outside this case."],
            r#"{"kind": "flechette-default"}"#,
            r#"{"arrayIndex": 8, "arrayLength": 12, "defaultCapacity": 200, "savedField": "max_ammo"}"#,
        ),
        (
            "q2.q64.publishes-and-sets-server-config",
            "Q64 non-deathmatch worldspawn publishes CONFIG_N64_PHYSICS and immediately sets server pm_config, then publishes and stores air acceleration.",
            &CONFIG_SOURCES,
            &["Start at the displayed worldspawn block with integer cvar air acceleration; this does not run prediction."],
            r#"{"kind": "q64-config", "isN64": true, "deathmatch": false, "initialN64Physics": false, "airacceleration": 1}"#,
            r#"{"serverN64Physics": true, "serverAiracceleration": 1, "trace": [{"event": "configstring", "name": "CONFIG_N64_PHYSICS", "value": "1"}, {"event": "server.pm_config.n64_physics", "value": true}, "G_InitStatusbar", {"event": "configstring", "name": "CS_AIRACCEL", "value": "1"}, {"event": "server.pm_config.airaccel", "value": 1}]}"#,
        ),
        (
            "q2.q64.deathmatch-does-not-enable",
            "Q64 deathmatch skips the N64 physics enable block; air acceleration still propagates.",
            &CONFIG_SOURCES,
            &["Initial server n64_physics=false; surrounding worldspawn reset and cgame update callbacks are outside this block."],
            r#"{"kind": "q64-config", "isN64": true, "deathmatch": true, "initialN64Physics": false, "airacceleration": 0}"#,
            r#"{"serverN64Physics": false, "serverAiracceleration": 0, "trace": ["G_InitStatusbar", {"event": "configstring", "name": "CS_AIRACCEL", "value": "0"}, {"event": "server.pm_config.airaccel", "value": 0}]}"#,
        ),
        (
            "q2.q64.block-does-not-clear-prior-state",
            "This source block contains no else clearing n64_physics; prior state is retained when its condition is false.",
            &CONFIG_SOURCES,
            &["Deliberately supplied pre-block true state; this does not imply that full worldspawn fails to reset configuration."],
            r#"{"kind": "q64-config", "isN64": false, "deathmatch": false, "initialN64Physics": true, "airacceleration": 0}"#,
            r#"{"serverN64Physics": true, "serverAiracceleration": 0, "trace": ["G_InitStatusbar", {"event": "configstring", "name": "CS_AIRACCEL", "value": "0"}, {"event": "server.pm_config.airaccel", "value": 0}]}"#,
        ),
        (
            "q2.commands.classic-jump-threshold-below",
            "upmove=9 does not hold classic jump; the rerelease jump button does hold rerelease jump.",
            &COMMAND_SOURCES,
            &COMMAND_PRECONDITIONS,
            r#"{"kind": "command-predicates", "upmove": 9, "buttons": 8, "n64Physics": false}"#,
            r#"{"classicHoldingJump": false, "classicGroundedDuckBranch": false, "rereleaseHoldingJump": true, "rereleaseGroundedDuckBranch": false}"#,
        ),
        (
            "q2.commands.classic-jump-threshold-equal",
            "upmove=10 holds classic jump independently of rerelease button bits.",
            &COMMAND_SOURCES,
            &COMMAND_PRECONDITIONS,
            r#"{"kind": "command-predicates", "upmove": 10, "buttons": 0, "n64Physics": false}"#,
            r#"{"classicHoldingJump": true, "classicGroundedDuckBranch": false, "rereleaseHoldingJump": false, "rereleaseGroundedDuckBranch": false}"#,
        ),
        (
            "q2.commands.crouch-native-predicates",
            "Negative classic upmove and rerelease BUTTON_CROUCH request their grounded duck branches.",
            &COMMAND_SOURCES,
            &COMMAND_PRECONDITIONS,
            r#"{"kind": "command-predicates", "upmove": -400, "buttons": 16, "n64Physics": false}"#,
            r#"{"classicHoldingJump": false, "classicGroundedDuckBranch": true, "rereleaseHoldingJump": false, "rereleaseGroundedDuckBranch": true}"#,
        ),
        (
            "q2.commands.q64-disables-duck",
            "Q64 configuration prevents the rerelease duck branch even with BUTTON_CROUCH set.",
            &COMMAND_SOURCES,
            &COMMAND_PRECONDITIONS,
            r#"{"kind": "command-predicates", "upmove": -400, "buttons": 16, "n64Physics": true}"#,
            r#"{"classicHoldingJump": false, "classicGroundedDuckBranch": true, "rereleaseHoldingJump": false, "rereleaseGroundedDuckBranch": false}"#,
        ),
    ]
}

/// Parse the pinned Q2 oracle cases.
pub fn q2_cases() -> Result<Vec<Q2Case>, ToolsError> {
    let mut cases = Vec::new();
    for (id, contract, sources, preconditions, input, expected) in raw_cases() {
        cases.push(Q2Case {
            id,
            contract,
            sources: sources.to_vec(),
            preconditions: preconditions.iter().map(|text| (*text).to_owned()).collect(),
            input: parse_json(input)
                .map_err(|error| ToolsError::parse(format!("Case {id} has invalid input: {error}")))?,
            expected: parse_json(expected)
                .map_err(|error| ToolsError::parse(format!("Case {id} has invalid expectation: {error}")))?,
        });
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_cases() {
        let cases = q2_cases().expect("cases parse");
        assert_eq!(cases.len(), 32);
        let mut ids = std::collections::BTreeSet::new();
        for case in &cases {
            assert!(ids.insert(case.id), "duplicate {}", case.id);
            assert!(!case.sources.is_empty(), "{}", case.id);
            assert!(!case.preconditions.is_empty(), "{}", case.id);
        }
    }
}
