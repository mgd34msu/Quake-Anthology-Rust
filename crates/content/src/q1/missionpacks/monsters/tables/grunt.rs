//! Hipnotic grunt frames (src/content/q1/missionpacks/monsters/tables/grunt.ts).
//!
//! quakec_hipnotic/monsters/grunt.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::Q1SoundChannel;

static OPS_ARMY_ATK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_ATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_ATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_ATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_ATK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "grunt:army_atk5",
}];
static OPS_ARMY_ATK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_ATK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "grunt:army_atk7",
}];
static OPS_ARMY_ATK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_ATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ARMY_CDIE1: &[MonsterOperation] = &[];
static OPS_ARMY_CDIE10: &[MonsterOperation] = &[];
static OPS_ARMY_CDIE11: &[MonsterOperation] = &[];
static OPS_ARMY_CDIE2: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(5)" }];
static OPS_ARMY_CDIE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "grunt:army_cdie3",
}];
static OPS_ARMY_CDIE4: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(13)" }];
static OPS_ARMY_CDIE5: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(3)" }];
static OPS_ARMY_CDIE6: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(4)" }];
static OPS_ARMY_CDIE7: &[MonsterOperation] = &[];
static OPS_ARMY_CDIE8: &[MonsterOperation] = &[];
static OPS_ARMY_CDIE9: &[MonsterOperation] = &[];
static OPS_ARMY_DIE1: &[MonsterOperation] = &[];
static OPS_ARMY_DIE10: &[MonsterOperation] = &[];
static OPS_ARMY_DIE2: &[MonsterOperation] = &[];
static OPS_ARMY_DIE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "grunt:army_die3",
}];
static OPS_ARMY_DIE4: &[MonsterOperation] = &[];
static OPS_ARMY_DIE5: &[MonsterOperation] = &[];
static OPS_ARMY_DIE6: &[MonsterOperation] = &[];
static OPS_ARMY_DIE7: &[MonsterOperation] = &[];
static OPS_ARMY_DIE8: &[MonsterOperation] = &[];
static OPS_ARMY_DIE9: &[MonsterOperation] = &[];
static OPS_ARMY_PAIN1: &[MonsterOperation] = &[];
static OPS_ARMY_PAIN2: &[MonsterOperation] = &[];
static OPS_ARMY_PAIN3: &[MonsterOperation] = &[];
static OPS_ARMY_PAIN4: &[MonsterOperation] = &[];
static OPS_ARMY_PAIN5: &[MonsterOperation] = &[];
static OPS_ARMY_PAIN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ARMY_PAINB1: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB10: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB11: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_ARMY_PAINB13: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB14: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 13.0,
}];
static OPS_ARMY_PAINB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 9.0,
}];
static OPS_ARMY_PAINB4: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB5: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB6: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB7: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB8: &[MonsterOperation] = &[];
static OPS_ARMY_PAINB9: &[MonsterOperation] = &[];
static OPS_ARMY_PAINC1: &[MonsterOperation] = &[];
static OPS_ARMY_PAINC10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 3.0,
}];
static OPS_ARMY_PAINC11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 6.0,
}];
static OPS_ARMY_PAINC12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 8.0,
}];
static OPS_ARMY_PAINC13: &[MonsterOperation] = &[];
static OPS_ARMY_PAINC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ARMY_PAINC3: &[MonsterOperation] = &[];
static OPS_ARMY_PAINC4: &[MonsterOperation] = &[];
static OPS_ARMY_PAINC5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ARMY_PAINC6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ARMY_PAINC7: &[MonsterOperation] = &[];
static OPS_ARMY_PAINC8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ARMY_PAINC9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 4.0,
}];
static OPS_ARMY_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "soldier/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 11.0,
    },
];
static OPS_ARMY_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 15.0,
}];
static OPS_ARMY_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 10.0,
}];
static OPS_ARMY_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 10.0,
}];
static OPS_ARMY_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_ARMY_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 15.0,
}];
static OPS_ARMY_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 10.0,
}];
static OPS_ARMY_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_ARMY_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ARMY_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "soldier/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 1.0,
    },
];
static OPS_ARMY_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ARMY_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ARMY_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ARMY_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ARMY_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ARMY_WALK19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ARMY_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ARMY_WALK21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ARMY_WALK22: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK23: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK24: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ARMY_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ARMY_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ARMY_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ARMY_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ARMY_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "army_atk1",
        MonsterFrame {
            frame: 81,
            next: "army_atk2",
            operations: OPS_ARMY_ATK1,
        },
    ),
    (
        "army_atk2",
        MonsterFrame {
            frame: 82,
            next: "army_atk3",
            operations: OPS_ARMY_ATK2,
        },
    ),
    (
        "army_atk3",
        MonsterFrame {
            frame: 83,
            next: "army_atk4",
            operations: OPS_ARMY_ATK3,
        },
    ),
    (
        "army_atk4",
        MonsterFrame {
            frame: 84,
            next: "army_atk5",
            operations: OPS_ARMY_ATK4,
        },
    ),
    (
        "army_atk5",
        MonsterFrame {
            frame: 85,
            next: "army_atk6",
            operations: OPS_ARMY_ATK5,
        },
    ),
    (
        "army_atk6",
        MonsterFrame {
            frame: 86,
            next: "army_atk7",
            operations: OPS_ARMY_ATK6,
        },
    ),
    (
        "army_atk7",
        MonsterFrame {
            frame: 87,
            next: "army_atk8",
            operations: OPS_ARMY_ATK7,
        },
    ),
    (
        "army_atk8",
        MonsterFrame {
            frame: 88,
            next: "army_atk9",
            operations: OPS_ARMY_ATK8,
        },
    ),
    (
        "army_atk9",
        MonsterFrame {
            frame: 89,
            next: "army_run1",
            operations: OPS_ARMY_ATK9,
        },
    ),
    (
        "army_cdie1",
        MonsterFrame {
            frame: 18,
            next: "army_cdie2",
            operations: OPS_ARMY_CDIE1,
        },
    ),
    (
        "army_cdie10",
        MonsterFrame {
            frame: 27,
            next: "army_cdie11",
            operations: OPS_ARMY_CDIE10,
        },
    ),
    (
        "army_cdie11",
        MonsterFrame {
            frame: 28,
            next: "army_cdie11",
            operations: OPS_ARMY_CDIE11,
        },
    ),
    (
        "army_cdie2",
        MonsterFrame {
            frame: 19,
            next: "army_cdie3",
            operations: OPS_ARMY_CDIE2,
        },
    ),
    (
        "army_cdie3",
        MonsterFrame {
            frame: 20,
            next: "army_cdie4",
            operations: OPS_ARMY_CDIE3,
        },
    ),
    (
        "army_cdie4",
        MonsterFrame {
            frame: 21,
            next: "army_cdie5",
            operations: OPS_ARMY_CDIE4,
        },
    ),
    (
        "army_cdie5",
        MonsterFrame {
            frame: 22,
            next: "army_cdie6",
            operations: OPS_ARMY_CDIE5,
        },
    ),
    (
        "army_cdie6",
        MonsterFrame {
            frame: 23,
            next: "army_cdie7",
            operations: OPS_ARMY_CDIE6,
        },
    ),
    (
        "army_cdie7",
        MonsterFrame {
            frame: 24,
            next: "army_cdie8",
            operations: OPS_ARMY_CDIE7,
        },
    ),
    (
        "army_cdie8",
        MonsterFrame {
            frame: 25,
            next: "army_cdie9",
            operations: OPS_ARMY_CDIE8,
        },
    ),
    (
        "army_cdie9",
        MonsterFrame {
            frame: 26,
            next: "army_cdie10",
            operations: OPS_ARMY_CDIE9,
        },
    ),
    (
        "army_die1",
        MonsterFrame {
            frame: 8,
            next: "army_die2",
            operations: OPS_ARMY_DIE1,
        },
    ),
    (
        "army_die10",
        MonsterFrame {
            frame: 17,
            next: "army_die10",
            operations: OPS_ARMY_DIE10,
        },
    ),
    (
        "army_die2",
        MonsterFrame {
            frame: 9,
            next: "army_die3",
            operations: OPS_ARMY_DIE2,
        },
    ),
    (
        "army_die3",
        MonsterFrame {
            frame: 10,
            next: "army_die4",
            operations: OPS_ARMY_DIE3,
        },
    ),
    (
        "army_die4",
        MonsterFrame {
            frame: 11,
            next: "army_die5",
            operations: OPS_ARMY_DIE4,
        },
    ),
    (
        "army_die5",
        MonsterFrame {
            frame: 12,
            next: "army_die6",
            operations: OPS_ARMY_DIE5,
        },
    ),
    (
        "army_die6",
        MonsterFrame {
            frame: 13,
            next: "army_die7",
            operations: OPS_ARMY_DIE6,
        },
    ),
    (
        "army_die7",
        MonsterFrame {
            frame: 14,
            next: "army_die8",
            operations: OPS_ARMY_DIE7,
        },
    ),
    (
        "army_die8",
        MonsterFrame {
            frame: 15,
            next: "army_die9",
            operations: OPS_ARMY_DIE8,
        },
    ),
    (
        "army_die9",
        MonsterFrame {
            frame: 16,
            next: "army_die10",
            operations: OPS_ARMY_DIE9,
        },
    ),
    (
        "army_pain1",
        MonsterFrame {
            frame: 40,
            next: "army_pain2",
            operations: OPS_ARMY_PAIN1,
        },
    ),
    (
        "army_pain2",
        MonsterFrame {
            frame: 41,
            next: "army_pain3",
            operations: OPS_ARMY_PAIN2,
        },
    ),
    (
        "army_pain3",
        MonsterFrame {
            frame: 42,
            next: "army_pain4",
            operations: OPS_ARMY_PAIN3,
        },
    ),
    (
        "army_pain4",
        MonsterFrame {
            frame: 43,
            next: "army_pain5",
            operations: OPS_ARMY_PAIN4,
        },
    ),
    (
        "army_pain5",
        MonsterFrame {
            frame: 44,
            next: "army_pain6",
            operations: OPS_ARMY_PAIN5,
        },
    ),
    (
        "army_pain6",
        MonsterFrame {
            frame: 45,
            next: "army_run1",
            operations: OPS_ARMY_PAIN6,
        },
    ),
    (
        "army_painb1",
        MonsterFrame {
            frame: 46,
            next: "army_painb2",
            operations: OPS_ARMY_PAINB1,
        },
    ),
    (
        "army_painb10",
        MonsterFrame {
            frame: 55,
            next: "army_painb11",
            operations: OPS_ARMY_PAINB10,
        },
    ),
    (
        "army_painb11",
        MonsterFrame {
            frame: 56,
            next: "army_painb12",
            operations: OPS_ARMY_PAINB11,
        },
    ),
    (
        "army_painb12",
        MonsterFrame {
            frame: 57,
            next: "army_painb13",
            operations: OPS_ARMY_PAINB12,
        },
    ),
    (
        "army_painb13",
        MonsterFrame {
            frame: 58,
            next: "army_painb14",
            operations: OPS_ARMY_PAINB13,
        },
    ),
    (
        "army_painb14",
        MonsterFrame {
            frame: 59,
            next: "army_run1",
            operations: OPS_ARMY_PAINB14,
        },
    ),
    (
        "army_painb2",
        MonsterFrame {
            frame: 47,
            next: "army_painb3",
            operations: OPS_ARMY_PAINB2,
        },
    ),
    (
        "army_painb3",
        MonsterFrame {
            frame: 48,
            next: "army_painb4",
            operations: OPS_ARMY_PAINB3,
        },
    ),
    (
        "army_painb4",
        MonsterFrame {
            frame: 49,
            next: "army_painb5",
            operations: OPS_ARMY_PAINB4,
        },
    ),
    (
        "army_painb5",
        MonsterFrame {
            frame: 50,
            next: "army_painb6",
            operations: OPS_ARMY_PAINB5,
        },
    ),
    (
        "army_painb6",
        MonsterFrame {
            frame: 51,
            next: "army_painb7",
            operations: OPS_ARMY_PAINB6,
        },
    ),
    (
        "army_painb7",
        MonsterFrame {
            frame: 52,
            next: "army_painb8",
            operations: OPS_ARMY_PAINB7,
        },
    ),
    (
        "army_painb8",
        MonsterFrame {
            frame: 53,
            next: "army_painb9",
            operations: OPS_ARMY_PAINB8,
        },
    ),
    (
        "army_painb9",
        MonsterFrame {
            frame: 54,
            next: "army_painb10",
            operations: OPS_ARMY_PAINB9,
        },
    ),
    (
        "army_painc1",
        MonsterFrame {
            frame: 60,
            next: "army_painc2",
            operations: OPS_ARMY_PAINC1,
        },
    ),
    (
        "army_painc10",
        MonsterFrame {
            frame: 69,
            next: "army_painc11",
            operations: OPS_ARMY_PAINC10,
        },
    ),
    (
        "army_painc11",
        MonsterFrame {
            frame: 70,
            next: "army_painc12",
            operations: OPS_ARMY_PAINC11,
        },
    ),
    (
        "army_painc12",
        MonsterFrame {
            frame: 71,
            next: "army_painc13",
            operations: OPS_ARMY_PAINC12,
        },
    ),
    (
        "army_painc13",
        MonsterFrame {
            frame: 72,
            next: "army_run1",
            operations: OPS_ARMY_PAINC13,
        },
    ),
    (
        "army_painc2",
        MonsterFrame {
            frame: 61,
            next: "army_painc3",
            operations: OPS_ARMY_PAINC2,
        },
    ),
    (
        "army_painc3",
        MonsterFrame {
            frame: 62,
            next: "army_painc4",
            operations: OPS_ARMY_PAINC3,
        },
    ),
    (
        "army_painc4",
        MonsterFrame {
            frame: 63,
            next: "army_painc5",
            operations: OPS_ARMY_PAINC4,
        },
    ),
    (
        "army_painc5",
        MonsterFrame {
            frame: 64,
            next: "army_painc6",
            operations: OPS_ARMY_PAINC5,
        },
    ),
    (
        "army_painc6",
        MonsterFrame {
            frame: 65,
            next: "army_painc7",
            operations: OPS_ARMY_PAINC6,
        },
    ),
    (
        "army_painc7",
        MonsterFrame {
            frame: 66,
            next: "army_painc8",
            operations: OPS_ARMY_PAINC7,
        },
    ),
    (
        "army_painc8",
        MonsterFrame {
            frame: 67,
            next: "army_painc9",
            operations: OPS_ARMY_PAINC8,
        },
    ),
    (
        "army_painc9",
        MonsterFrame {
            frame: 68,
            next: "army_painc10",
            operations: OPS_ARMY_PAINC9,
        },
    ),
    (
        "army_run1",
        MonsterFrame {
            frame: 73,
            next: "army_run2",
            operations: OPS_ARMY_RUN1,
        },
    ),
    (
        "army_run2",
        MonsterFrame {
            frame: 74,
            next: "army_run3",
            operations: OPS_ARMY_RUN2,
        },
    ),
    (
        "army_run3",
        MonsterFrame {
            frame: 75,
            next: "army_run4",
            operations: OPS_ARMY_RUN3,
        },
    ),
    (
        "army_run4",
        MonsterFrame {
            frame: 76,
            next: "army_run5",
            operations: OPS_ARMY_RUN4,
        },
    ),
    (
        "army_run5",
        MonsterFrame {
            frame: 77,
            next: "army_run6",
            operations: OPS_ARMY_RUN5,
        },
    ),
    (
        "army_run6",
        MonsterFrame {
            frame: 78,
            next: "army_run7",
            operations: OPS_ARMY_RUN6,
        },
    ),
    (
        "army_run7",
        MonsterFrame {
            frame: 79,
            next: "army_run8",
            operations: OPS_ARMY_RUN7,
        },
    ),
    (
        "army_run8",
        MonsterFrame {
            frame: 80,
            next: "army_run1",
            operations: OPS_ARMY_RUN8,
        },
    ),
    (
        "army_stand1",
        MonsterFrame {
            frame: 0,
            next: "army_stand2",
            operations: OPS_ARMY_STAND1,
        },
    ),
    (
        "army_stand2",
        MonsterFrame {
            frame: 1,
            next: "army_stand3",
            operations: OPS_ARMY_STAND2,
        },
    ),
    (
        "army_stand3",
        MonsterFrame {
            frame: 2,
            next: "army_stand4",
            operations: OPS_ARMY_STAND3,
        },
    ),
    (
        "army_stand4",
        MonsterFrame {
            frame: 3,
            next: "army_stand5",
            operations: OPS_ARMY_STAND4,
        },
    ),
    (
        "army_stand5",
        MonsterFrame {
            frame: 4,
            next: "army_stand6",
            operations: OPS_ARMY_STAND5,
        },
    ),
    (
        "army_stand6",
        MonsterFrame {
            frame: 5,
            next: "army_stand7",
            operations: OPS_ARMY_STAND6,
        },
    ),
    (
        "army_stand7",
        MonsterFrame {
            frame: 6,
            next: "army_stand8",
            operations: OPS_ARMY_STAND7,
        },
    ),
    (
        "army_stand8",
        MonsterFrame {
            frame: 7,
            next: "army_stand1",
            operations: OPS_ARMY_STAND8,
        },
    ),
    (
        "army_walk1",
        MonsterFrame {
            frame: 90,
            next: "army_walk2",
            operations: OPS_ARMY_WALK1,
        },
    ),
    (
        "army_walk10",
        MonsterFrame {
            frame: 99,
            next: "army_walk11",
            operations: OPS_ARMY_WALK10,
        },
    ),
    (
        "army_walk11",
        MonsterFrame {
            frame: 100,
            next: "army_walk12",
            operations: OPS_ARMY_WALK11,
        },
    ),
    (
        "army_walk12",
        MonsterFrame {
            frame: 101,
            next: "army_walk13",
            operations: OPS_ARMY_WALK12,
        },
    ),
    (
        "army_walk13",
        MonsterFrame {
            frame: 102,
            next: "army_walk14",
            operations: OPS_ARMY_WALK13,
        },
    ),
    (
        "army_walk14",
        MonsterFrame {
            frame: 103,
            next: "army_walk15",
            operations: OPS_ARMY_WALK14,
        },
    ),
    (
        "army_walk15",
        MonsterFrame {
            frame: 104,
            next: "army_walk16",
            operations: OPS_ARMY_WALK15,
        },
    ),
    (
        "army_walk16",
        MonsterFrame {
            frame: 105,
            next: "army_walk17",
            operations: OPS_ARMY_WALK16,
        },
    ),
    (
        "army_walk17",
        MonsterFrame {
            frame: 106,
            next: "army_walk18",
            operations: OPS_ARMY_WALK17,
        },
    ),
    (
        "army_walk18",
        MonsterFrame {
            frame: 107,
            next: "army_walk19",
            operations: OPS_ARMY_WALK18,
        },
    ),
    (
        "army_walk19",
        MonsterFrame {
            frame: 108,
            next: "army_walk20",
            operations: OPS_ARMY_WALK19,
        },
    ),
    (
        "army_walk2",
        MonsterFrame {
            frame: 91,
            next: "army_walk3",
            operations: OPS_ARMY_WALK2,
        },
    ),
    (
        "army_walk20",
        MonsterFrame {
            frame: 109,
            next: "army_walk21",
            operations: OPS_ARMY_WALK20,
        },
    ),
    (
        "army_walk21",
        MonsterFrame {
            frame: 110,
            next: "army_walk22",
            operations: OPS_ARMY_WALK21,
        },
    ),
    (
        "army_walk22",
        MonsterFrame {
            frame: 111,
            next: "army_walk23",
            operations: OPS_ARMY_WALK22,
        },
    ),
    (
        "army_walk23",
        MonsterFrame {
            frame: 112,
            next: "army_walk24",
            operations: OPS_ARMY_WALK23,
        },
    ),
    (
        "army_walk24",
        MonsterFrame {
            frame: 113,
            next: "army_walk1",
            operations: OPS_ARMY_WALK24,
        },
    ),
    (
        "army_walk3",
        MonsterFrame {
            frame: 92,
            next: "army_walk4",
            operations: OPS_ARMY_WALK3,
        },
    ),
    (
        "army_walk4",
        MonsterFrame {
            frame: 93,
            next: "army_walk5",
            operations: OPS_ARMY_WALK4,
        },
    ),
    (
        "army_walk5",
        MonsterFrame {
            frame: 94,
            next: "army_walk6",
            operations: OPS_ARMY_WALK5,
        },
    ),
    (
        "army_walk6",
        MonsterFrame {
            frame: 95,
            next: "army_walk7",
            operations: OPS_ARMY_WALK6,
        },
    ),
    (
        "army_walk7",
        MonsterFrame {
            frame: 96,
            next: "army_walk8",
            operations: OPS_ARMY_WALK7,
        },
    ),
    (
        "army_walk8",
        MonsterFrame {
            frame: 97,
            next: "army_walk9",
            operations: OPS_ARMY_WALK8,
        },
    ),
    (
        "army_walk9",
        MonsterFrame {
            frame: 98,
            next: "army_walk10",
            operations: OPS_ARMY_WALK9,
        },
    ),
];

/// Look up a grunt frame by name.
#[must_use]
pub fn frame(name: &str) -> Option<&'static MonsterFrame> {
    FRAMES
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| &FRAMES[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_sorted_with_expected_count() {
        assert_eq!(FRAMES.len(), 103);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("army_atk1").expect("first frame");
        assert_eq!((head.frame, head.next), (81, "army_atk2"));
        let tail = frame("army_walk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (98, "army_walk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
