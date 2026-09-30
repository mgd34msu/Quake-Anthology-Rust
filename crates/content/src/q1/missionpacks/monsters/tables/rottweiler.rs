//! Hipnotic rottweiler frames (src/content/q1/missionpacks/monsters/tables/rottweiler.ts).
//!
//! quakec_hipnotic/monsters/rottweiler.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::Q1SoundChannel;

static OPS_DOG_ATTA1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_ATTA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_ATTA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_ATTA4: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "dog/dattack1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Action { name: "dog_bite" },
];
static OPS_DOG_ATTA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_ATTA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_ATTA7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_ATTA8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_DOG_DIE1: &[MonsterOperation] = &[];
static OPS_DOG_DIE2: &[MonsterOperation] = &[];
static OPS_DOG_DIE3: &[MonsterOperation] = &[];
static OPS_DOG_DIE4: &[MonsterOperation] = &[];
static OPS_DOG_DIE5: &[MonsterOperation] = &[];
static OPS_DOG_DIE6: &[MonsterOperation] = &[];
static OPS_DOG_DIE7: &[MonsterOperation] = &[];
static OPS_DOG_DIE8: &[MonsterOperation] = &[];
static OPS_DOG_DIE9: &[MonsterOperation] = &[];
static OPS_DOG_DIEB1: &[MonsterOperation] = &[];
static OPS_DOG_DIEB2: &[MonsterOperation] = &[];
static OPS_DOG_DIEB3: &[MonsterOperation] = &[];
static OPS_DOG_DIEB4: &[MonsterOperation] = &[];
static OPS_DOG_DIEB5: &[MonsterOperation] = &[];
static OPS_DOG_DIEB6: &[MonsterOperation] = &[];
static OPS_DOG_DIEB7: &[MonsterOperation] = &[];
static OPS_DOG_DIEB8: &[MonsterOperation] = &[];
static OPS_DOG_DIEB9: &[MonsterOperation] = &[];
static OPS_DOG_LEAP1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_DOG_LEAP2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "rottweiler:dog_leap2",
}];
static OPS_DOG_LEAP3: &[MonsterOperation] = &[];
static OPS_DOG_LEAP4: &[MonsterOperation] = &[];
static OPS_DOG_LEAP5: &[MonsterOperation] = &[];
static OPS_DOG_LEAP6: &[MonsterOperation] = &[];
static OPS_DOG_LEAP7: &[MonsterOperation] = &[];
static OPS_DOG_LEAP8: &[MonsterOperation] = &[];
static OPS_DOG_LEAP9: &[MonsterOperation] = &[];
static OPS_DOG_PAIN1: &[MonsterOperation] = &[];
static OPS_DOG_PAIN2: &[MonsterOperation] = &[];
static OPS_DOG_PAIN3: &[MonsterOperation] = &[];
static OPS_DOG_PAIN4: &[MonsterOperation] = &[];
static OPS_DOG_PAIN5: &[MonsterOperation] = &[];
static OPS_DOG_PAIN6: &[MonsterOperation] = &[];
static OPS_DOG_PAINB1: &[MonsterOperation] = &[];
static OPS_DOG_PAINB10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 10.0,
}];
static OPS_DOG_PAINB11: &[MonsterOperation] = &[];
static OPS_DOG_PAINB12: &[MonsterOperation] = &[];
static OPS_DOG_PAINB13: &[MonsterOperation] = &[];
static OPS_DOG_PAINB14: &[MonsterOperation] = &[];
static OPS_DOG_PAINB15: &[MonsterOperation] = &[];
static OPS_DOG_PAINB16: &[MonsterOperation] = &[];
static OPS_DOG_PAINB2: &[MonsterOperation] = &[];
static OPS_DOG_PAINB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 4.0,
}];
static OPS_DOG_PAINB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 12.0,
}];
static OPS_DOG_PAINB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 12.0,
}];
static OPS_DOG_PAINB6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_DOG_PAINB7: &[MonsterOperation] = &[];
static OPS_DOG_PAINB8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 4.0,
}];
static OPS_DOG_PAINB9: &[MonsterOperation] = &[];
static OPS_DOG_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "dog/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 16.0,
    },
];
static OPS_DOG_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_DOG_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 64.0,
}];
static OPS_DOG_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 32.0,
}];
static OPS_DOG_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 32.0,
}];
static OPS_DOG_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 32.0,
}];
static OPS_DOG_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_DOG_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 64.0,
}];
static OPS_DOG_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 32.0,
}];
static OPS_DOG_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_DOG_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 32.0,
}];
static OPS_DOG_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 32.0,
}];
static OPS_DOG_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DOG_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "dog/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 8.0,
    },
];
static OPS_DOG_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_DOG_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_DOG_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_DOG_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_DOG_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_DOG_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_DOG_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "dog_atta1",
        MonsterFrame {
            frame: 0,
            next: "dog_atta2",
            operations: OPS_DOG_ATTA1,
        },
    ),
    (
        "dog_atta2",
        MonsterFrame {
            frame: 1,
            next: "dog_atta3",
            operations: OPS_DOG_ATTA2,
        },
    ),
    (
        "dog_atta3",
        MonsterFrame {
            frame: 2,
            next: "dog_atta4",
            operations: OPS_DOG_ATTA3,
        },
    ),
    (
        "dog_atta4",
        MonsterFrame {
            frame: 3,
            next: "dog_atta5",
            operations: OPS_DOG_ATTA4,
        },
    ),
    (
        "dog_atta5",
        MonsterFrame {
            frame: 4,
            next: "dog_atta6",
            operations: OPS_DOG_ATTA5,
        },
    ),
    (
        "dog_atta6",
        MonsterFrame {
            frame: 5,
            next: "dog_atta7",
            operations: OPS_DOG_ATTA6,
        },
    ),
    (
        "dog_atta7",
        MonsterFrame {
            frame: 6,
            next: "dog_atta8",
            operations: OPS_DOG_ATTA7,
        },
    ),
    (
        "dog_atta8",
        MonsterFrame {
            frame: 7,
            next: "dog_run1",
            operations: OPS_DOG_ATTA8,
        },
    ),
    (
        "dog_die1",
        MonsterFrame {
            frame: 8,
            next: "dog_die2",
            operations: OPS_DOG_DIE1,
        },
    ),
    (
        "dog_die2",
        MonsterFrame {
            frame: 9,
            next: "dog_die3",
            operations: OPS_DOG_DIE2,
        },
    ),
    (
        "dog_die3",
        MonsterFrame {
            frame: 10,
            next: "dog_die4",
            operations: OPS_DOG_DIE3,
        },
    ),
    (
        "dog_die4",
        MonsterFrame {
            frame: 11,
            next: "dog_die5",
            operations: OPS_DOG_DIE4,
        },
    ),
    (
        "dog_die5",
        MonsterFrame {
            frame: 12,
            next: "dog_die6",
            operations: OPS_DOG_DIE5,
        },
    ),
    (
        "dog_die6",
        MonsterFrame {
            frame: 13,
            next: "dog_die7",
            operations: OPS_DOG_DIE6,
        },
    ),
    (
        "dog_die7",
        MonsterFrame {
            frame: 14,
            next: "dog_die8",
            operations: OPS_DOG_DIE7,
        },
    ),
    (
        "dog_die8",
        MonsterFrame {
            frame: 15,
            next: "dog_die9",
            operations: OPS_DOG_DIE8,
        },
    ),
    (
        "dog_die9",
        MonsterFrame {
            frame: 16,
            next: "dog_die9",
            operations: OPS_DOG_DIE9,
        },
    ),
    (
        "dog_dieb1",
        MonsterFrame {
            frame: 17,
            next: "dog_dieb2",
            operations: OPS_DOG_DIEB1,
        },
    ),
    (
        "dog_dieb2",
        MonsterFrame {
            frame: 18,
            next: "dog_dieb3",
            operations: OPS_DOG_DIEB2,
        },
    ),
    (
        "dog_dieb3",
        MonsterFrame {
            frame: 19,
            next: "dog_dieb4",
            operations: OPS_DOG_DIEB3,
        },
    ),
    (
        "dog_dieb4",
        MonsterFrame {
            frame: 20,
            next: "dog_dieb5",
            operations: OPS_DOG_DIEB4,
        },
    ),
    (
        "dog_dieb5",
        MonsterFrame {
            frame: 21,
            next: "dog_dieb6",
            operations: OPS_DOG_DIEB5,
        },
    ),
    (
        "dog_dieb6",
        MonsterFrame {
            frame: 22,
            next: "dog_dieb7",
            operations: OPS_DOG_DIEB6,
        },
    ),
    (
        "dog_dieb7",
        MonsterFrame {
            frame: 23,
            next: "dog_dieb8",
            operations: OPS_DOG_DIEB7,
        },
    ),
    (
        "dog_dieb8",
        MonsterFrame {
            frame: 24,
            next: "dog_dieb9",
            operations: OPS_DOG_DIEB8,
        },
    ),
    (
        "dog_dieb9",
        MonsterFrame {
            frame: 25,
            next: "dog_dieb9",
            operations: OPS_DOG_DIEB9,
        },
    ),
    (
        "dog_leap1",
        MonsterFrame {
            frame: 60,
            next: "dog_leap2",
            operations: OPS_DOG_LEAP1,
        },
    ),
    (
        "dog_leap2",
        MonsterFrame {
            frame: 61,
            next: "dog_leap3",
            operations: OPS_DOG_LEAP2,
        },
    ),
    (
        "dog_leap3",
        MonsterFrame {
            frame: 62,
            next: "dog_leap4",
            operations: OPS_DOG_LEAP3,
        },
    ),
    (
        "dog_leap4",
        MonsterFrame {
            frame: 63,
            next: "dog_leap5",
            operations: OPS_DOG_LEAP4,
        },
    ),
    (
        "dog_leap5",
        MonsterFrame {
            frame: 64,
            next: "dog_leap6",
            operations: OPS_DOG_LEAP5,
        },
    ),
    (
        "dog_leap6",
        MonsterFrame {
            frame: 65,
            next: "dog_leap7",
            operations: OPS_DOG_LEAP6,
        },
    ),
    (
        "dog_leap7",
        MonsterFrame {
            frame: 66,
            next: "dog_leap8",
            operations: OPS_DOG_LEAP7,
        },
    ),
    (
        "dog_leap8",
        MonsterFrame {
            frame: 67,
            next: "dog_leap9",
            operations: OPS_DOG_LEAP8,
        },
    ),
    (
        "dog_leap9",
        MonsterFrame {
            frame: 68,
            next: "dog_leap9",
            operations: OPS_DOG_LEAP9,
        },
    ),
    (
        "dog_pain1",
        MonsterFrame {
            frame: 26,
            next: "dog_pain2",
            operations: OPS_DOG_PAIN1,
        },
    ),
    (
        "dog_pain2",
        MonsterFrame {
            frame: 27,
            next: "dog_pain3",
            operations: OPS_DOG_PAIN2,
        },
    ),
    (
        "dog_pain3",
        MonsterFrame {
            frame: 28,
            next: "dog_pain4",
            operations: OPS_DOG_PAIN3,
        },
    ),
    (
        "dog_pain4",
        MonsterFrame {
            frame: 29,
            next: "dog_pain5",
            operations: OPS_DOG_PAIN4,
        },
    ),
    (
        "dog_pain5",
        MonsterFrame {
            frame: 30,
            next: "dog_pain6",
            operations: OPS_DOG_PAIN5,
        },
    ),
    (
        "dog_pain6",
        MonsterFrame {
            frame: 31,
            next: "dog_run1",
            operations: OPS_DOG_PAIN6,
        },
    ),
    (
        "dog_painb1",
        MonsterFrame {
            frame: 32,
            next: "dog_painb2",
            operations: OPS_DOG_PAINB1,
        },
    ),
    (
        "dog_painb10",
        MonsterFrame {
            frame: 41,
            next: "dog_painb11",
            operations: OPS_DOG_PAINB10,
        },
    ),
    (
        "dog_painb11",
        MonsterFrame {
            frame: 42,
            next: "dog_painb12",
            operations: OPS_DOG_PAINB11,
        },
    ),
    (
        "dog_painb12",
        MonsterFrame {
            frame: 43,
            next: "dog_painb13",
            operations: OPS_DOG_PAINB12,
        },
    ),
    (
        "dog_painb13",
        MonsterFrame {
            frame: 44,
            next: "dog_painb14",
            operations: OPS_DOG_PAINB13,
        },
    ),
    (
        "dog_painb14",
        MonsterFrame {
            frame: 45,
            next: "dog_painb15",
            operations: OPS_DOG_PAINB14,
        },
    ),
    (
        "dog_painb15",
        MonsterFrame {
            frame: 46,
            next: "dog_painb16",
            operations: OPS_DOG_PAINB15,
        },
    ),
    (
        "dog_painb16",
        MonsterFrame {
            frame: 47,
            next: "dog_run1",
            operations: OPS_DOG_PAINB16,
        },
    ),
    (
        "dog_painb2",
        MonsterFrame {
            frame: 33,
            next: "dog_painb3",
            operations: OPS_DOG_PAINB2,
        },
    ),
    (
        "dog_painb3",
        MonsterFrame {
            frame: 34,
            next: "dog_painb4",
            operations: OPS_DOG_PAINB3,
        },
    ),
    (
        "dog_painb4",
        MonsterFrame {
            frame: 35,
            next: "dog_painb5",
            operations: OPS_DOG_PAINB4,
        },
    ),
    (
        "dog_painb5",
        MonsterFrame {
            frame: 36,
            next: "dog_painb6",
            operations: OPS_DOG_PAINB5,
        },
    ),
    (
        "dog_painb6",
        MonsterFrame {
            frame: 37,
            next: "dog_painb7",
            operations: OPS_DOG_PAINB6,
        },
    ),
    (
        "dog_painb7",
        MonsterFrame {
            frame: 38,
            next: "dog_painb8",
            operations: OPS_DOG_PAINB7,
        },
    ),
    (
        "dog_painb8",
        MonsterFrame {
            frame: 39,
            next: "dog_painb9",
            operations: OPS_DOG_PAINB8,
        },
    ),
    (
        "dog_painb9",
        MonsterFrame {
            frame: 40,
            next: "dog_painb10",
            operations: OPS_DOG_PAINB9,
        },
    ),
    (
        "dog_run1",
        MonsterFrame {
            frame: 48,
            next: "dog_run2",
            operations: OPS_DOG_RUN1,
        },
    ),
    (
        "dog_run10",
        MonsterFrame {
            frame: 57,
            next: "dog_run11",
            operations: OPS_DOG_RUN10,
        },
    ),
    (
        "dog_run11",
        MonsterFrame {
            frame: 58,
            next: "dog_run12",
            operations: OPS_DOG_RUN11,
        },
    ),
    (
        "dog_run12",
        MonsterFrame {
            frame: 59,
            next: "dog_run1",
            operations: OPS_DOG_RUN12,
        },
    ),
    (
        "dog_run2",
        MonsterFrame {
            frame: 49,
            next: "dog_run3",
            operations: OPS_DOG_RUN2,
        },
    ),
    (
        "dog_run3",
        MonsterFrame {
            frame: 50,
            next: "dog_run4",
            operations: OPS_DOG_RUN3,
        },
    ),
    (
        "dog_run4",
        MonsterFrame {
            frame: 51,
            next: "dog_run5",
            operations: OPS_DOG_RUN4,
        },
    ),
    (
        "dog_run5",
        MonsterFrame {
            frame: 52,
            next: "dog_run6",
            operations: OPS_DOG_RUN5,
        },
    ),
    (
        "dog_run6",
        MonsterFrame {
            frame: 53,
            next: "dog_run7",
            operations: OPS_DOG_RUN6,
        },
    ),
    (
        "dog_run7",
        MonsterFrame {
            frame: 54,
            next: "dog_run8",
            operations: OPS_DOG_RUN7,
        },
    ),
    (
        "dog_run8",
        MonsterFrame {
            frame: 55,
            next: "dog_run9",
            operations: OPS_DOG_RUN8,
        },
    ),
    (
        "dog_run9",
        MonsterFrame {
            frame: 56,
            next: "dog_run10",
            operations: OPS_DOG_RUN9,
        },
    ),
    (
        "dog_stand1",
        MonsterFrame {
            frame: 69,
            next: "dog_stand2",
            operations: OPS_DOG_STAND1,
        },
    ),
    (
        "dog_stand2",
        MonsterFrame {
            frame: 70,
            next: "dog_stand3",
            operations: OPS_DOG_STAND2,
        },
    ),
    (
        "dog_stand3",
        MonsterFrame {
            frame: 71,
            next: "dog_stand4",
            operations: OPS_DOG_STAND3,
        },
    ),
    (
        "dog_stand4",
        MonsterFrame {
            frame: 72,
            next: "dog_stand5",
            operations: OPS_DOG_STAND4,
        },
    ),
    (
        "dog_stand5",
        MonsterFrame {
            frame: 73,
            next: "dog_stand6",
            operations: OPS_DOG_STAND5,
        },
    ),
    (
        "dog_stand6",
        MonsterFrame {
            frame: 74,
            next: "dog_stand7",
            operations: OPS_DOG_STAND6,
        },
    ),
    (
        "dog_stand7",
        MonsterFrame {
            frame: 75,
            next: "dog_stand8",
            operations: OPS_DOG_STAND7,
        },
    ),
    (
        "dog_stand8",
        MonsterFrame {
            frame: 76,
            next: "dog_stand9",
            operations: OPS_DOG_STAND8,
        },
    ),
    (
        "dog_stand9",
        MonsterFrame {
            frame: 77,
            next: "dog_stand1",
            operations: OPS_DOG_STAND9,
        },
    ),
    (
        "dog_walk1",
        MonsterFrame {
            frame: 78,
            next: "dog_walk2",
            operations: OPS_DOG_WALK1,
        },
    ),
    (
        "dog_walk2",
        MonsterFrame {
            frame: 79,
            next: "dog_walk3",
            operations: OPS_DOG_WALK2,
        },
    ),
    (
        "dog_walk3",
        MonsterFrame {
            frame: 80,
            next: "dog_walk4",
            operations: OPS_DOG_WALK3,
        },
    ),
    (
        "dog_walk4",
        MonsterFrame {
            frame: 81,
            next: "dog_walk5",
            operations: OPS_DOG_WALK4,
        },
    ),
    (
        "dog_walk5",
        MonsterFrame {
            frame: 82,
            next: "dog_walk6",
            operations: OPS_DOG_WALK5,
        },
    ),
    (
        "dog_walk6",
        MonsterFrame {
            frame: 83,
            next: "dog_walk7",
            operations: OPS_DOG_WALK6,
        },
    ),
    (
        "dog_walk7",
        MonsterFrame {
            frame: 84,
            next: "dog_walk8",
            operations: OPS_DOG_WALK7,
        },
    ),
    (
        "dog_walk8",
        MonsterFrame {
            frame: 85,
            next: "dog_walk1",
            operations: OPS_DOG_WALK8,
        },
    ),
];

/// Look up a rottweiler frame by name.
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
        assert_eq!(FRAMES.len(), 86);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("dog_atta1").expect("first frame");
        assert_eq!((head.frame, head.next), (0, "dog_atta2"));
        let tail = frame("dog_walk8").expect("last frame");
        assert_eq!((tail.frame, tail.next), (85, "dog_walk1"));
        assert!(frame("no_such_frame").is_none());
    }
}
