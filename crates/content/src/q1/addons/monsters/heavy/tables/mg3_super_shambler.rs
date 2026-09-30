//! Q1 Mg3 super-shambler frames (`src/content/q1/addons/monsters/heavy/tables/mg3_super_shambler.ts`).
//!
//! quakec_mg3/monsters/mg3_super_shambler.qc source frame order. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

static OPS_SUPSHAM_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_STAND17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SUPSHAM_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 10.0,
}];
static OPS_SUPSHAM_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 9.0,
}];
static OPS_SUPSHAM_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 9.0,
}];
static OPS_SUPSHAM_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_SUPSHAM_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_SUPSHAM_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 12.0,
}];
static OPS_SUPSHAM_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_SUPSHAM_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_SUPSHAM_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 13.0,
}];
static OPS_SUPSHAM_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 9.0,
}];
static OPS_SUPSHAM_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 7.0,
}];
static OPS_SUPSHAM_WALK12: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 7.0,
    },
    MonsterOperation::Sound {
        path: "shambler/sidle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Greater,
        chance: Some(0.8),
    },
];
static OPS_SUPSHAM_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "supsham_removechild",
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 20.0,
    },
];
static OPS_SUPSHAM_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 24.0,
}];
static OPS_SUPSHAM_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_SUPSHAM_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_SUPSHAM_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 24.0,
}];
static OPS_SUPSHAM_RUN6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 20.0,
    },
    MonsterOperation::Sound {
        path: "shambler/sidle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Greater,
        chance: Some(0.8),
    },
];
static OPS_SUPSHAM_SMASH1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shambler/melee1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
];
static OPS_SUPSHAM_SMASH2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_SUPSHAM_SMASH3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_SUPSHAM_SMASH4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_SUPSHAM_SMASH5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_SUPSHAM_SMASH6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_SUPSHAM_SMASH7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_SUPSHAM_SMASH8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_SUPSHAM_SMASH9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_SUPSHAM_SMASH10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_smash10",
}];
static OPS_SUPSHAM_SMASH11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_SUPSHAM_SMASH12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_smash12",
}];
static OPS_SUPSHAM_SWINGL1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shambler/melee2.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 5.0,
    },
];
static OPS_SUPSHAM_SWINGL2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SUPSHAM_SWINGL3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_SUPSHAM_SWINGL4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SUPSHAM_SWINGL5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_SUPSHAM_SWINGL6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_SUPSHAM_SWINGL7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_swingl7",
}];
static OPS_SUPSHAM_SWINGL8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_SUPSHAM_SWINGL9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_swingl9",
}];
static OPS_SUPSHAM_SWINGR1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shambler/melee1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 1.0,
    },
];
static OPS_SUPSHAM_SWINGR2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 8.0,
}];
static OPS_SUPSHAM_SWINGR3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SUPSHAM_SWINGR4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_SUPSHAM_SWINGR5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SUPSHAM_SWINGR6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_SUPSHAM_SWINGR7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_swingr7",
}];
static OPS_SUPSHAM_SWINGR8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SUPSHAM_SWINGR9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_swingr9",
}];
static OPS_SUPSHAM_MAGIC1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic1",
}];
static OPS_SUPSHAM_MAGIC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SUPSHAM_MAGIC3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic3",
}];
static OPS_SUPSHAM_MAGIC4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic4",
}];
static OPS_SUPSHAM_MAGIC5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic5",
}];
static OPS_SUPSHAM_MAGIC4B: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic4b",
}];
static OPS_SUPSHAM_MAGIC5B: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic5",
}];
static OPS_SUPSHAM_MAGIC4C: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic4",
}];
static OPS_SUPSHAM_MAGIC5C: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic5",
}];
static OPS_SUPSHAM_MAGIC6: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "supsham_removechild",
    },
    MonsterOperation::Action {
        name: "SupCastLightning",
    },
    MonsterOperation::Sound {
        path: "shambler/sboom.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
];
static OPS_SUPSHAM_MAGIC9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "SupCastLightning",
}];
static OPS_SUPSHAM_MAGIC10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "SupCastLightning",
}];
static OPS_SUPSHAM_MAGIC_B1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic1",
}];
static OPS_SUPSHAM_MAGIC_B2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SUPSHAM_MAGIC_B3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic_b3",
}];
static OPS_SUPSHAM_MAGIC_B4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic4",
}];
static OPS_SUPSHAM_MAGIC_B5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_super_shambler:supsham_magic5",
}];
static OPS_SUPSHAM_MAGIC_B6: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "supsham_removechild",
    },
    MonsterOperation::Action {
        name: "SupCastLightning",
    },
    MonsterOperation::Sound {
        path: "shambler/sboom.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
];
static OPS_SUPSHAM_MAGIC_B9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "SupCastLightning",
}];
static OPS_SUPSHAM_MAGIC_B10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "SupCastLightning",
}];
static OPS_SUPSHAM_DEATH3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Action { name: "cleanup_orbs" },
];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("supsham_stand1"),
                MonsterFrame {
                    frame: 0,
                    next: "supsham_stand2",
                    operations: OPS_SUPSHAM_STAND1,
                },
            ),
            (
                String::from("supsham_stand2"),
                MonsterFrame {
                    frame: 1,
                    next: "supsham_stand3",
                    operations: OPS_SUPSHAM_STAND2,
                },
            ),
            (
                String::from("supsham_stand3"),
                MonsterFrame {
                    frame: 2,
                    next: "supsham_stand4",
                    operations: OPS_SUPSHAM_STAND3,
                },
            ),
            (
                String::from("supsham_stand4"),
                MonsterFrame {
                    frame: 3,
                    next: "supsham_stand5",
                    operations: OPS_SUPSHAM_STAND4,
                },
            ),
            (
                String::from("supsham_stand5"),
                MonsterFrame {
                    frame: 4,
                    next: "supsham_stand6",
                    operations: OPS_SUPSHAM_STAND5,
                },
            ),
            (
                String::from("supsham_stand6"),
                MonsterFrame {
                    frame: 5,
                    next: "supsham_stand7",
                    operations: OPS_SUPSHAM_STAND6,
                },
            ),
            (
                String::from("supsham_stand7"),
                MonsterFrame {
                    frame: 6,
                    next: "supsham_stand8",
                    operations: OPS_SUPSHAM_STAND7,
                },
            ),
            (
                String::from("supsham_stand8"),
                MonsterFrame {
                    frame: 7,
                    next: "supsham_stand9",
                    operations: OPS_SUPSHAM_STAND8,
                },
            ),
            (
                String::from("supsham_stand9"),
                MonsterFrame {
                    frame: 8,
                    next: "supsham_stand10",
                    operations: OPS_SUPSHAM_STAND9,
                },
            ),
            (
                String::from("supsham_stand10"),
                MonsterFrame {
                    frame: 9,
                    next: "supsham_stand11",
                    operations: OPS_SUPSHAM_STAND10,
                },
            ),
            (
                String::from("supsham_stand11"),
                MonsterFrame {
                    frame: 10,
                    next: "supsham_stand12",
                    operations: OPS_SUPSHAM_STAND11,
                },
            ),
            (
                String::from("supsham_stand12"),
                MonsterFrame {
                    frame: 11,
                    next: "supsham_stand13",
                    operations: OPS_SUPSHAM_STAND12,
                },
            ),
            (
                String::from("supsham_stand13"),
                MonsterFrame {
                    frame: 12,
                    next: "supsham_stand14",
                    operations: OPS_SUPSHAM_STAND13,
                },
            ),
            (
                String::from("supsham_stand14"),
                MonsterFrame {
                    frame: 13,
                    next: "supsham_stand15",
                    operations: OPS_SUPSHAM_STAND14,
                },
            ),
            (
                String::from("supsham_stand15"),
                MonsterFrame {
                    frame: 14,
                    next: "supsham_stand16",
                    operations: OPS_SUPSHAM_STAND15,
                },
            ),
            (
                String::from("supsham_stand16"),
                MonsterFrame {
                    frame: 15,
                    next: "supsham_stand17",
                    operations: OPS_SUPSHAM_STAND16,
                },
            ),
            (
                String::from("supsham_stand17"),
                MonsterFrame {
                    frame: 16,
                    next: "supsham_stand1",
                    operations: OPS_SUPSHAM_STAND17,
                },
            ),
            (
                String::from("supsham_walk1"),
                MonsterFrame {
                    frame: 17,
                    next: "supsham_walk2",
                    operations: OPS_SUPSHAM_WALK1,
                },
            ),
            (
                String::from("supsham_walk2"),
                MonsterFrame {
                    frame: 18,
                    next: "supsham_walk3",
                    operations: OPS_SUPSHAM_WALK2,
                },
            ),
            (
                String::from("supsham_walk3"),
                MonsterFrame {
                    frame: 19,
                    next: "supsham_walk4",
                    operations: OPS_SUPSHAM_WALK3,
                },
            ),
            (
                String::from("supsham_walk4"),
                MonsterFrame {
                    frame: 20,
                    next: "supsham_walk5",
                    operations: OPS_SUPSHAM_WALK4,
                },
            ),
            (
                String::from("supsham_walk5"),
                MonsterFrame {
                    frame: 21,
                    next: "supsham_walk6",
                    operations: OPS_SUPSHAM_WALK5,
                },
            ),
            (
                String::from("supsham_walk6"),
                MonsterFrame {
                    frame: 22,
                    next: "supsham_walk7",
                    operations: OPS_SUPSHAM_WALK6,
                },
            ),
            (
                String::from("supsham_walk7"),
                MonsterFrame {
                    frame: 23,
                    next: "supsham_walk8",
                    operations: OPS_SUPSHAM_WALK7,
                },
            ),
            (
                String::from("supsham_walk8"),
                MonsterFrame {
                    frame: 24,
                    next: "supsham_walk9",
                    operations: OPS_SUPSHAM_WALK8,
                },
            ),
            (
                String::from("supsham_walk9"),
                MonsterFrame {
                    frame: 25,
                    next: "supsham_walk10",
                    operations: OPS_SUPSHAM_WALK9,
                },
            ),
            (
                String::from("supsham_walk10"),
                MonsterFrame {
                    frame: 26,
                    next: "supsham_walk11",
                    operations: OPS_SUPSHAM_WALK10,
                },
            ),
            (
                String::from("supsham_walk11"),
                MonsterFrame {
                    frame: 27,
                    next: "supsham_walk12",
                    operations: OPS_SUPSHAM_WALK11,
                },
            ),
            (
                String::from("supsham_walk12"),
                MonsterFrame {
                    frame: 28,
                    next: "supsham_walk1",
                    operations: OPS_SUPSHAM_WALK12,
                },
            ),
            (
                String::from("supsham_run1"),
                MonsterFrame {
                    frame: 29,
                    next: "supsham_run2",
                    operations: OPS_SUPSHAM_RUN1,
                },
            ),
            (
                String::from("supsham_run2"),
                MonsterFrame {
                    frame: 30,
                    next: "supsham_run3",
                    operations: OPS_SUPSHAM_RUN2,
                },
            ),
            (
                String::from("supsham_run3"),
                MonsterFrame {
                    frame: 31,
                    next: "supsham_run4",
                    operations: OPS_SUPSHAM_RUN3,
                },
            ),
            (
                String::from("supsham_run4"),
                MonsterFrame {
                    frame: 32,
                    next: "supsham_run5",
                    operations: OPS_SUPSHAM_RUN4,
                },
            ),
            (
                String::from("supsham_run5"),
                MonsterFrame {
                    frame: 33,
                    next: "supsham_run6",
                    operations: OPS_SUPSHAM_RUN5,
                },
            ),
            (
                String::from("supsham_run6"),
                MonsterFrame {
                    frame: 34,
                    next: "supsham_run1",
                    operations: OPS_SUPSHAM_RUN6,
                },
            ),
            (
                String::from("supsham_smash1"),
                MonsterFrame {
                    frame: 35,
                    next: "supsham_smash2",
                    operations: OPS_SUPSHAM_SMASH1,
                },
            ),
            (
                String::from("supsham_smash2"),
                MonsterFrame {
                    frame: 36,
                    next: "supsham_smash3",
                    operations: OPS_SUPSHAM_SMASH2,
                },
            ),
            (
                String::from("supsham_smash3"),
                MonsterFrame {
                    frame: 37,
                    next: "supsham_smash4",
                    operations: OPS_SUPSHAM_SMASH3,
                },
            ),
            (
                String::from("supsham_smash4"),
                MonsterFrame {
                    frame: 38,
                    next: "supsham_smash5",
                    operations: OPS_SUPSHAM_SMASH4,
                },
            ),
            (
                String::from("supsham_smash5"),
                MonsterFrame {
                    frame: 39,
                    next: "supsham_smash6",
                    operations: OPS_SUPSHAM_SMASH5,
                },
            ),
            (
                String::from("supsham_smash6"),
                MonsterFrame {
                    frame: 40,
                    next: "supsham_smash7",
                    operations: OPS_SUPSHAM_SMASH6,
                },
            ),
            (
                String::from("supsham_smash7"),
                MonsterFrame {
                    frame: 41,
                    next: "supsham_smash8",
                    operations: OPS_SUPSHAM_SMASH7,
                },
            ),
            (
                String::from("supsham_smash8"),
                MonsterFrame {
                    frame: 42,
                    next: "supsham_smash9",
                    operations: OPS_SUPSHAM_SMASH8,
                },
            ),
            (
                String::from("supsham_smash9"),
                MonsterFrame {
                    frame: 43,
                    next: "supsham_smash10",
                    operations: OPS_SUPSHAM_SMASH9,
                },
            ),
            (
                String::from("supsham_smash10"),
                MonsterFrame {
                    frame: 44,
                    next: "supsham_smash11",
                    operations: OPS_SUPSHAM_SMASH10,
                },
            ),
            (
                String::from("supsham_smash11"),
                MonsterFrame {
                    frame: 45,
                    next: "supsham_smash12",
                    operations: OPS_SUPSHAM_SMASH11,
                },
            ),
            (
                String::from("supsham_smash12"),
                MonsterFrame {
                    frame: 46,
                    next: "supsham_run1",
                    operations: OPS_SUPSHAM_SMASH12,
                },
            ),
            (
                String::from("supsham_swingl1"),
                MonsterFrame {
                    frame: 56,
                    next: "supsham_swingl2",
                    operations: OPS_SUPSHAM_SWINGL1,
                },
            ),
            (
                String::from("supsham_swingl2"),
                MonsterFrame {
                    frame: 57,
                    next: "supsham_swingl3",
                    operations: OPS_SUPSHAM_SWINGL2,
                },
            ),
            (
                String::from("supsham_swingl3"),
                MonsterFrame {
                    frame: 58,
                    next: "supsham_swingl4",
                    operations: OPS_SUPSHAM_SWINGL3,
                },
            ),
            (
                String::from("supsham_swingl4"),
                MonsterFrame {
                    frame: 59,
                    next: "supsham_swingl5",
                    operations: OPS_SUPSHAM_SWINGL4,
                },
            ),
            (
                String::from("supsham_swingl5"),
                MonsterFrame {
                    frame: 60,
                    next: "supsham_swingl6",
                    operations: OPS_SUPSHAM_SWINGL5,
                },
            ),
            (
                String::from("supsham_swingl6"),
                MonsterFrame {
                    frame: 61,
                    next: "supsham_swingl7",
                    operations: OPS_SUPSHAM_SWINGL6,
                },
            ),
            (
                String::from("supsham_swingl7"),
                MonsterFrame {
                    frame: 62,
                    next: "supsham_swingl8",
                    operations: OPS_SUPSHAM_SWINGL7,
                },
            ),
            (
                String::from("supsham_swingl8"),
                MonsterFrame {
                    frame: 63,
                    next: "supsham_swingl9",
                    operations: OPS_SUPSHAM_SWINGL8,
                },
            ),
            (
                String::from("supsham_swingl9"),
                MonsterFrame {
                    frame: 64,
                    next: "supsham_run1",
                    operations: OPS_SUPSHAM_SWINGL9,
                },
            ),
            (
                String::from("supsham_swingr1"),
                MonsterFrame {
                    frame: 47,
                    next: "supsham_swingr2",
                    operations: OPS_SUPSHAM_SWINGR1,
                },
            ),
            (
                String::from("supsham_swingr2"),
                MonsterFrame {
                    frame: 48,
                    next: "supsham_swingr3",
                    operations: OPS_SUPSHAM_SWINGR2,
                },
            ),
            (
                String::from("supsham_swingr3"),
                MonsterFrame {
                    frame: 49,
                    next: "supsham_swingr4",
                    operations: OPS_SUPSHAM_SWINGR3,
                },
            ),
            (
                String::from("supsham_swingr4"),
                MonsterFrame {
                    frame: 50,
                    next: "supsham_swingr5",
                    operations: OPS_SUPSHAM_SWINGR4,
                },
            ),
            (
                String::from("supsham_swingr5"),
                MonsterFrame {
                    frame: 51,
                    next: "supsham_swingr6",
                    operations: OPS_SUPSHAM_SWINGR5,
                },
            ),
            (
                String::from("supsham_swingr6"),
                MonsterFrame {
                    frame: 52,
                    next: "supsham_swingr7",
                    operations: OPS_SUPSHAM_SWINGR6,
                },
            ),
            (
                String::from("supsham_swingr7"),
                MonsterFrame {
                    frame: 53,
                    next: "supsham_swingr8",
                    operations: OPS_SUPSHAM_SWINGR7,
                },
            ),
            (
                String::from("supsham_swingr8"),
                MonsterFrame {
                    frame: 54,
                    next: "supsham_swingr9",
                    operations: OPS_SUPSHAM_SWINGR8,
                },
            ),
            (
                String::from("supsham_swingr9"),
                MonsterFrame {
                    frame: 55,
                    next: "supsham_run1",
                    operations: OPS_SUPSHAM_SWINGR9,
                },
            ),
            (
                String::from("supsham_magic1"),
                MonsterFrame {
                    frame: 65,
                    next: "supsham_magic2",
                    operations: OPS_SUPSHAM_MAGIC1,
                },
            ),
            (
                String::from("supsham_magic2"),
                MonsterFrame {
                    frame: 66,
                    next: "supsham_magic3",
                    operations: OPS_SUPSHAM_MAGIC2,
                },
            ),
            (
                String::from("supsham_magic3"),
                MonsterFrame {
                    frame: 67,
                    next: "supsham_magic4",
                    operations: OPS_SUPSHAM_MAGIC3,
                },
            ),
            (
                String::from("supsham_magic4"),
                MonsterFrame {
                    frame: 68,
                    next: "supsham_magic5",
                    operations: OPS_SUPSHAM_MAGIC4,
                },
            ),
            (
                String::from("supsham_magic5"),
                MonsterFrame {
                    frame: 69,
                    next: "supsham_magic4b",
                    operations: OPS_SUPSHAM_MAGIC5,
                },
            ),
            (
                String::from("supsham_magic4b"),
                MonsterFrame {
                    frame: 68,
                    next: "supsham_magic5b",
                    operations: OPS_SUPSHAM_MAGIC4B,
                },
            ),
            (
                String::from("supsham_magic5b"),
                MonsterFrame {
                    frame: 69,
                    next: "supsham_magic4c",
                    operations: OPS_SUPSHAM_MAGIC5B,
                },
            ),
            (
                String::from("supsham_magic4c"),
                MonsterFrame {
                    frame: 68,
                    next: "supsham_magic5c",
                    operations: OPS_SUPSHAM_MAGIC4C,
                },
            ),
            (
                String::from("supsham_magic5c"),
                MonsterFrame {
                    frame: 69,
                    next: "supsham_magic6",
                    operations: OPS_SUPSHAM_MAGIC5C,
                },
            ),
            (
                String::from("supsham_magic6"),
                MonsterFrame {
                    frame: 70,
                    next: "supsham_magic9",
                    operations: OPS_SUPSHAM_MAGIC6,
                },
            ),
            (
                String::from("supsham_magic9"),
                MonsterFrame {
                    frame: 73,
                    next: "supsham_magic10",
                    operations: OPS_SUPSHAM_MAGIC9,
                },
            ),
            (
                String::from("supsham_magic10"),
                MonsterFrame {
                    frame: 74,
                    next: "supsham_magic11",
                    operations: OPS_SUPSHAM_MAGIC10,
                },
            ),
            (
                String::from("supsham_magic11"),
                MonsterFrame {
                    frame: 75,
                    next: "supsham_magic12",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_magic12"),
                MonsterFrame {
                    frame: 76,
                    next: "supsham_run1",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_magic_b1"),
                MonsterFrame {
                    frame: 65,
                    next: "supsham_magic_b2",
                    operations: OPS_SUPSHAM_MAGIC_B1,
                },
            ),
            (
                String::from("supsham_magic_b2"),
                MonsterFrame {
                    frame: 66,
                    next: "supsham_magic_b3",
                    operations: OPS_SUPSHAM_MAGIC_B2,
                },
            ),
            (
                String::from("supsham_magic_b3"),
                MonsterFrame {
                    frame: 67,
                    next: "supsham_magic_b4",
                    operations: OPS_SUPSHAM_MAGIC_B3,
                },
            ),
            (
                String::from("supsham_magic_b4"),
                MonsterFrame {
                    frame: 68,
                    next: "supsham_magic_b5",
                    operations: OPS_SUPSHAM_MAGIC_B4,
                },
            ),
            (
                String::from("supsham_magic_b5"),
                MonsterFrame {
                    frame: 69,
                    next: "supsham_magic_b6",
                    operations: OPS_SUPSHAM_MAGIC_B5,
                },
            ),
            (
                String::from("supsham_magic_b6"),
                MonsterFrame {
                    frame: 70,
                    next: "supsham_magic_b9",
                    operations: OPS_SUPSHAM_MAGIC_B6,
                },
            ),
            (
                String::from("supsham_magic_b9"),
                MonsterFrame {
                    frame: 73,
                    next: "supsham_magic_b10",
                    operations: OPS_SUPSHAM_MAGIC_B9,
                },
            ),
            (
                String::from("supsham_magic_b10"),
                MonsterFrame {
                    frame: 74,
                    next: "supsham_magic_b11",
                    operations: OPS_SUPSHAM_MAGIC_B10,
                },
            ),
            (
                String::from("supsham_magic_b11"),
                MonsterFrame {
                    frame: 75,
                    next: "supsham_magic_b12",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_magic_b12"),
                MonsterFrame {
                    frame: 76,
                    next: "supsham_run1",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_pain1"),
                MonsterFrame {
                    frame: 77,
                    next: "supsham_pain2",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_pain2"),
                MonsterFrame {
                    frame: 78,
                    next: "supsham_pain3",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_pain3"),
                MonsterFrame {
                    frame: 79,
                    next: "supsham_pain4",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_pain4"),
                MonsterFrame {
                    frame: 80,
                    next: "supsham_pain5",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_pain5"),
                MonsterFrame {
                    frame: 81,
                    next: "supsham_pain6",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_pain6"),
                MonsterFrame {
                    frame: 82,
                    next: "supsham_run1",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death1"),
                MonsterFrame {
                    frame: 83,
                    next: "supsham_death2",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death2"),
                MonsterFrame {
                    frame: 84,
                    next: "supsham_death3",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death3"),
                MonsterFrame {
                    frame: 85,
                    next: "supsham_death4",
                    operations: OPS_SUPSHAM_DEATH3,
                },
            ),
            (
                String::from("supsham_death4"),
                MonsterFrame {
                    frame: 86,
                    next: "supsham_death5",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death5"),
                MonsterFrame {
                    frame: 87,
                    next: "supsham_death6",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death6"),
                MonsterFrame {
                    frame: 88,
                    next: "supsham_death7",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death7"),
                MonsterFrame {
                    frame: 89,
                    next: "supsham_death8",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death8"),
                MonsterFrame {
                    frame: 90,
                    next: "supsham_death9",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death9"),
                MonsterFrame {
                    frame: 91,
                    next: "supsham_death10",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death10"),
                MonsterFrame {
                    frame: 92,
                    next: "supsham_death11",
                    operations: &[],
                },
            ),
            (
                String::from("supsham_death11"),
                MonsterFrame {
                    frame: 93,
                    next: "supsham_death11",
                    operations: &[],
                },
            ),
        ])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuations_resolve() {
        assert_eq!(frames().len(), 106);
        for (name, frame) in frames() {
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
