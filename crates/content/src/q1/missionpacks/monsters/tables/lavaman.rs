//! Rogue lavaman frames (src/content/q1/missionpacks/monsters/tables/lavaman.ts).
//!
//! quakec_rogue/lavaman.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::Q1SoundChannel;

static OPS_LAVAMAN_DEATH1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/death.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_LAVAMAN_DEATH10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "lavaman:lavaman_death10",
}];
static OPS_LAVAMAN_DEATH2: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH3: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH4: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH5: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH6: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH7: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH8: &[MonsterOperation] = &[];
static OPS_LAVAMAN_DEATH9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "lavaman:lavaman_death9",
}];
static OPS_LAVAMAN_FIRE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "lavaman_missile(2)",
}];
static OPS_LAVAMAN_FIRE19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE22: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE23: &[MonsterOperation] = &[];
static OPS_LAVAMAN_FIRE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_LAVAMAN_FIRE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "lavaman_missile(1)",
}];
static OPS_LAVAMAN_FIRE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_IDLE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/sight1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: Some(0.2),
}];
static OPS_LAVAMAN_IDLE2: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE3: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE4: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE5: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE6: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE7: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE8: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_IDLE9: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_stand" }];
static OPS_LAVAMAN_RISE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/out1.wav",
    channel: Q1SoundChannel::Weapon,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_LAVAMAN_RISE10: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE11: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE12: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE13: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE14: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE15: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE16: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE17: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE2: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/sight1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_LAVAMAN_RISE3: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE4: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE5: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE6: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE7: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE8: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RISE9: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RUN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN10: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN11: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN12: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN13: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN14: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN15: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN16: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN17: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN18: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN19: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN2: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN20: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN21: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN22: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN23: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN24: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN25: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN26: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN27: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN28: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN29: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN3: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN30: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN31: &[MonsterOperation] = &[];
static OPS_LAVAMAN_RUN4: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN5: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN6: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN7: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN8: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN9: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_SHOCKA1: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA10: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA2: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA3: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA4: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA5: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA6: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA7: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA8: &[MonsterOperation] = &[];
static OPS_LAVAMAN_SHOCKA9: &[MonsterOperation] = &[];
static OPS_LAVAMAN_WALK1: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK10: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK11: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK12: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK13: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK14: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK15: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK16: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK17: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK18: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK19: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK2: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK20: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK21: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK22: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK23: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK24: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK25: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK26: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK27: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK28: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK29: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK3: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK30: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK31: &[MonsterOperation] = &[];
static OPS_LAVAMAN_WALK4: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK5: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK6: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK7: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK8: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK9: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "lavaman_death1",
        MonsterFrame {
            frame: 48,
            next: "lavaman_death2",
            operations: OPS_LAVAMAN_DEATH1,
        },
    ),
    (
        "lavaman_death10",
        MonsterFrame {
            frame: 56,
            next: "lavaman_death10",
            operations: OPS_LAVAMAN_DEATH10,
        },
    ),
    (
        "lavaman_death2",
        MonsterFrame {
            frame: 49,
            next: "lavaman_death3",
            operations: OPS_LAVAMAN_DEATH2,
        },
    ),
    (
        "lavaman_death3",
        MonsterFrame {
            frame: 50,
            next: "lavaman_death4",
            operations: OPS_LAVAMAN_DEATH3,
        },
    ),
    (
        "lavaman_death4",
        MonsterFrame {
            frame: 51,
            next: "lavaman_death5",
            operations: OPS_LAVAMAN_DEATH4,
        },
    ),
    (
        "lavaman_death5",
        MonsterFrame {
            frame: 52,
            next: "lavaman_death6",
            operations: OPS_LAVAMAN_DEATH5,
        },
    ),
    (
        "lavaman_death6",
        MonsterFrame {
            frame: 53,
            next: "lavaman_death7",
            operations: OPS_LAVAMAN_DEATH6,
        },
    ),
    (
        "lavaman_death7",
        MonsterFrame {
            frame: 54,
            next: "lavaman_death8",
            operations: OPS_LAVAMAN_DEATH7,
        },
    ),
    (
        "lavaman_death8",
        MonsterFrame {
            frame: 55,
            next: "lavaman_death9",
            operations: OPS_LAVAMAN_DEATH8,
        },
    ),
    (
        "lavaman_death9",
        MonsterFrame {
            frame: 56,
            next: "lavaman_death10",
            operations: OPS_LAVAMAN_DEATH9,
        },
    ),
    (
        "lavaman_fire1",
        MonsterFrame {
            frame: 57,
            next: "lavaman_fire2",
            operations: OPS_LAVAMAN_FIRE1,
        },
    ),
    (
        "lavaman_fire10",
        MonsterFrame {
            frame: 66,
            next: "lavaman_fire11",
            operations: OPS_LAVAMAN_FIRE10,
        },
    ),
    (
        "lavaman_fire11",
        MonsterFrame {
            frame: 67,
            next: "lavaman_fire12",
            operations: OPS_LAVAMAN_FIRE11,
        },
    ),
    (
        "lavaman_fire12",
        MonsterFrame {
            frame: 68,
            next: "lavaman_fire13",
            operations: OPS_LAVAMAN_FIRE12,
        },
    ),
    (
        "lavaman_fire13",
        MonsterFrame {
            frame: 69,
            next: "lavaman_fire14",
            operations: OPS_LAVAMAN_FIRE13,
        },
    ),
    (
        "lavaman_fire14",
        MonsterFrame {
            frame: 70,
            next: "lavaman_fire15",
            operations: OPS_LAVAMAN_FIRE14,
        },
    ),
    (
        "lavaman_fire15",
        MonsterFrame {
            frame: 71,
            next: "lavaman_fire16",
            operations: OPS_LAVAMAN_FIRE15,
        },
    ),
    (
        "lavaman_fire16",
        MonsterFrame {
            frame: 72,
            next: "lavaman_fire17",
            operations: OPS_LAVAMAN_FIRE16,
        },
    ),
    (
        "lavaman_fire17",
        MonsterFrame {
            frame: 73,
            next: "lavaman_fire18",
            operations: OPS_LAVAMAN_FIRE17,
        },
    ),
    (
        "lavaman_fire18",
        MonsterFrame {
            frame: 74,
            next: "lavaman_fire19",
            operations: OPS_LAVAMAN_FIRE18,
        },
    ),
    (
        "lavaman_fire19",
        MonsterFrame {
            frame: 75,
            next: "lavaman_fire20",
            operations: OPS_LAVAMAN_FIRE19,
        },
    ),
    (
        "lavaman_fire2",
        MonsterFrame {
            frame: 58,
            next: "lavaman_fire3",
            operations: OPS_LAVAMAN_FIRE2,
        },
    ),
    (
        "lavaman_fire20",
        MonsterFrame {
            frame: 76,
            next: "lavaman_fire21",
            operations: OPS_LAVAMAN_FIRE20,
        },
    ),
    (
        "lavaman_fire21",
        MonsterFrame {
            frame: 77,
            next: "lavaman_fire22",
            operations: OPS_LAVAMAN_FIRE21,
        },
    ),
    (
        "lavaman_fire22",
        MonsterFrame {
            frame: 78,
            next: "lavaman_fire23",
            operations: OPS_LAVAMAN_FIRE22,
        },
    ),
    (
        "lavaman_fire23",
        MonsterFrame {
            frame: 79,
            next: "lavaman_run1",
            operations: OPS_LAVAMAN_FIRE23,
        },
    ),
    (
        "lavaman_fire3",
        MonsterFrame {
            frame: 59,
            next: "lavaman_fire4",
            operations: OPS_LAVAMAN_FIRE3,
        },
    ),
    (
        "lavaman_fire4",
        MonsterFrame {
            frame: 60,
            next: "lavaman_fire5",
            operations: OPS_LAVAMAN_FIRE4,
        },
    ),
    (
        "lavaman_fire5",
        MonsterFrame {
            frame: 61,
            next: "lavaman_fire6",
            operations: OPS_LAVAMAN_FIRE5,
        },
    ),
    (
        "lavaman_fire6",
        MonsterFrame {
            frame: 62,
            next: "lavaman_fire7",
            operations: OPS_LAVAMAN_FIRE6,
        },
    ),
    (
        "lavaman_fire7",
        MonsterFrame {
            frame: 63,
            next: "lavaman_fire8",
            operations: OPS_LAVAMAN_FIRE7,
        },
    ),
    (
        "lavaman_fire8",
        MonsterFrame {
            frame: 64,
            next: "lavaman_fire9",
            operations: OPS_LAVAMAN_FIRE8,
        },
    ),
    (
        "lavaman_fire9",
        MonsterFrame {
            frame: 65,
            next: "lavaman_fire10",
            operations: OPS_LAVAMAN_FIRE9,
        },
    ),
    (
        "lavaman_idle1",
        MonsterFrame {
            frame: 17,
            next: "lavaman_idle2",
            operations: OPS_LAVAMAN_IDLE1,
        },
    ),
    (
        "lavaman_idle2",
        MonsterFrame {
            frame: 18,
            next: "lavaman_idle3",
            operations: OPS_LAVAMAN_IDLE2,
        },
    ),
    (
        "lavaman_idle3",
        MonsterFrame {
            frame: 19,
            next: "lavaman_idle4",
            operations: OPS_LAVAMAN_IDLE3,
        },
    ),
    (
        "lavaman_idle4",
        MonsterFrame {
            frame: 20,
            next: "lavaman_idle5",
            operations: OPS_LAVAMAN_IDLE4,
        },
    ),
    (
        "lavaman_idle5",
        MonsterFrame {
            frame: 21,
            next: "lavaman_idle6",
            operations: OPS_LAVAMAN_IDLE5,
        },
    ),
    (
        "lavaman_idle6",
        MonsterFrame {
            frame: 20,
            next: "lavaman_idle7",
            operations: OPS_LAVAMAN_IDLE6,
        },
    ),
    (
        "lavaman_idle7",
        MonsterFrame {
            frame: 19,
            next: "lavaman_idle8",
            operations: OPS_LAVAMAN_IDLE7,
        },
    ),
    (
        "lavaman_idle8",
        MonsterFrame {
            frame: 18,
            next: "lavaman_idle9",
            operations: OPS_LAVAMAN_IDLE8,
        },
    ),
    (
        "lavaman_idle9",
        MonsterFrame {
            frame: 17,
            next: "lavaman_idle1",
            operations: OPS_LAVAMAN_IDLE9,
        },
    ),
    (
        "lavaman_rise1",
        MonsterFrame {
            frame: 0,
            next: "lavaman_rise2",
            operations: OPS_LAVAMAN_RISE1,
        },
    ),
    (
        "lavaman_rise10",
        MonsterFrame {
            frame: 9,
            next: "lavaman_rise11",
            operations: OPS_LAVAMAN_RISE10,
        },
    ),
    (
        "lavaman_rise11",
        MonsterFrame {
            frame: 10,
            next: "lavaman_rise12",
            operations: OPS_LAVAMAN_RISE11,
        },
    ),
    (
        "lavaman_rise12",
        MonsterFrame {
            frame: 11,
            next: "lavaman_rise13",
            operations: OPS_LAVAMAN_RISE12,
        },
    ),
    (
        "lavaman_rise13",
        MonsterFrame {
            frame: 12,
            next: "lavaman_rise14",
            operations: OPS_LAVAMAN_RISE13,
        },
    ),
    (
        "lavaman_rise14",
        MonsterFrame {
            frame: 13,
            next: "lavaman_rise15",
            operations: OPS_LAVAMAN_RISE14,
        },
    ),
    (
        "lavaman_rise15",
        MonsterFrame {
            frame: 14,
            next: "lavaman_rise16",
            operations: OPS_LAVAMAN_RISE15,
        },
    ),
    (
        "lavaman_rise16",
        MonsterFrame {
            frame: 15,
            next: "lavaman_rise17",
            operations: OPS_LAVAMAN_RISE16,
        },
    ),
    (
        "lavaman_rise17",
        MonsterFrame {
            frame: 16,
            next: "lavaman_fire1",
            operations: OPS_LAVAMAN_RISE17,
        },
    ),
    (
        "lavaman_rise2",
        MonsterFrame {
            frame: 1,
            next: "lavaman_rise3",
            operations: OPS_LAVAMAN_RISE2,
        },
    ),
    (
        "lavaman_rise3",
        MonsterFrame {
            frame: 2,
            next: "lavaman_rise4",
            operations: OPS_LAVAMAN_RISE3,
        },
    ),
    (
        "lavaman_rise4",
        MonsterFrame {
            frame: 3,
            next: "lavaman_rise5",
            operations: OPS_LAVAMAN_RISE4,
        },
    ),
    (
        "lavaman_rise5",
        MonsterFrame {
            frame: 4,
            next: "lavaman_rise6",
            operations: OPS_LAVAMAN_RISE5,
        },
    ),
    (
        "lavaman_rise6",
        MonsterFrame {
            frame: 5,
            next: "lavaman_rise7",
            operations: OPS_LAVAMAN_RISE6,
        },
    ),
    (
        "lavaman_rise7",
        MonsterFrame {
            frame: 6,
            next: "lavaman_rise8",
            operations: OPS_LAVAMAN_RISE7,
        },
    ),
    (
        "lavaman_rise8",
        MonsterFrame {
            frame: 7,
            next: "lavaman_rise9",
            operations: OPS_LAVAMAN_RISE8,
        },
    ),
    (
        "lavaman_rise9",
        MonsterFrame {
            frame: 8,
            next: "lavaman_rise10",
            operations: OPS_LAVAMAN_RISE9,
        },
    ),
    (
        "lavaman_run1",
        MonsterFrame {
            frame: 17,
            next: "lavaman_run2",
            operations: OPS_LAVAMAN_RUN1,
        },
    ),
    (
        "lavaman_run10",
        MonsterFrame {
            frame: 26,
            next: "lavaman_run11",
            operations: OPS_LAVAMAN_RUN10,
        },
    ),
    (
        "lavaman_run11",
        MonsterFrame {
            frame: 27,
            next: "lavaman_run12",
            operations: OPS_LAVAMAN_RUN11,
        },
    ),
    (
        "lavaman_run12",
        MonsterFrame {
            frame: 28,
            next: "lavaman_run13",
            operations: OPS_LAVAMAN_RUN12,
        },
    ),
    (
        "lavaman_run13",
        MonsterFrame {
            frame: 29,
            next: "lavaman_run14",
            operations: OPS_LAVAMAN_RUN13,
        },
    ),
    (
        "lavaman_run14",
        MonsterFrame {
            frame: 30,
            next: "lavaman_run15",
            operations: OPS_LAVAMAN_RUN14,
        },
    ),
    (
        "lavaman_run15",
        MonsterFrame {
            frame: 31,
            next: "lavaman_run16",
            operations: OPS_LAVAMAN_RUN15,
        },
    ),
    (
        "lavaman_run16",
        MonsterFrame {
            frame: 32,
            next: "lavaman_run17",
            operations: OPS_LAVAMAN_RUN16,
        },
    ),
    (
        "lavaman_run17",
        MonsterFrame {
            frame: 33,
            next: "lavaman_run18",
            operations: OPS_LAVAMAN_RUN17,
        },
    ),
    (
        "lavaman_run18",
        MonsterFrame {
            frame: 34,
            next: "lavaman_run19",
            operations: OPS_LAVAMAN_RUN18,
        },
    ),
    (
        "lavaman_run19",
        MonsterFrame {
            frame: 35,
            next: "lavaman_run20",
            operations: OPS_LAVAMAN_RUN19,
        },
    ),
    (
        "lavaman_run2",
        MonsterFrame {
            frame: 18,
            next: "lavaman_run3",
            operations: OPS_LAVAMAN_RUN2,
        },
    ),
    (
        "lavaman_run20",
        MonsterFrame {
            frame: 36,
            next: "lavaman_run21",
            operations: OPS_LAVAMAN_RUN20,
        },
    ),
    (
        "lavaman_run21",
        MonsterFrame {
            frame: 37,
            next: "lavaman_run22",
            operations: OPS_LAVAMAN_RUN21,
        },
    ),
    (
        "lavaman_run22",
        MonsterFrame {
            frame: 38,
            next: "lavaman_run23",
            operations: OPS_LAVAMAN_RUN22,
        },
    ),
    (
        "lavaman_run23",
        MonsterFrame {
            frame: 39,
            next: "lavaman_run24",
            operations: OPS_LAVAMAN_RUN23,
        },
    ),
    (
        "lavaman_run24",
        MonsterFrame {
            frame: 40,
            next: "lavaman_run25",
            operations: OPS_LAVAMAN_RUN24,
        },
    ),
    (
        "lavaman_run25",
        MonsterFrame {
            frame: 41,
            next: "lavaman_run26",
            operations: OPS_LAVAMAN_RUN25,
        },
    ),
    (
        "lavaman_run26",
        MonsterFrame {
            frame: 42,
            next: "lavaman_run27",
            operations: OPS_LAVAMAN_RUN26,
        },
    ),
    (
        "lavaman_run27",
        MonsterFrame {
            frame: 43,
            next: "lavaman_run28",
            operations: OPS_LAVAMAN_RUN27,
        },
    ),
    (
        "lavaman_run28",
        MonsterFrame {
            frame: 44,
            next: "lavaman_run29",
            operations: OPS_LAVAMAN_RUN28,
        },
    ),
    (
        "lavaman_run29",
        MonsterFrame {
            frame: 45,
            next: "lavaman_run30",
            operations: OPS_LAVAMAN_RUN29,
        },
    ),
    (
        "lavaman_run3",
        MonsterFrame {
            frame: 19,
            next: "lavaman_run4",
            operations: OPS_LAVAMAN_RUN3,
        },
    ),
    (
        "lavaman_run30",
        MonsterFrame {
            frame: 46,
            next: "lavaman_run31",
            operations: OPS_LAVAMAN_RUN30,
        },
    ),
    (
        "lavaman_run31",
        MonsterFrame {
            frame: 47,
            next: "lavaman_run1",
            operations: OPS_LAVAMAN_RUN31,
        },
    ),
    (
        "lavaman_run4",
        MonsterFrame {
            frame: 20,
            next: "lavaman_run5",
            operations: OPS_LAVAMAN_RUN4,
        },
    ),
    (
        "lavaman_run5",
        MonsterFrame {
            frame: 21,
            next: "lavaman_run6",
            operations: OPS_LAVAMAN_RUN5,
        },
    ),
    (
        "lavaman_run6",
        MonsterFrame {
            frame: 22,
            next: "lavaman_run7",
            operations: OPS_LAVAMAN_RUN6,
        },
    ),
    (
        "lavaman_run7",
        MonsterFrame {
            frame: 23,
            next: "lavaman_run8",
            operations: OPS_LAVAMAN_RUN7,
        },
    ),
    (
        "lavaman_run8",
        MonsterFrame {
            frame: 24,
            next: "lavaman_run9",
            operations: OPS_LAVAMAN_RUN8,
        },
    ),
    (
        "lavaman_run9",
        MonsterFrame {
            frame: 25,
            next: "lavaman_run10",
            operations: OPS_LAVAMAN_RUN9,
        },
    ),
    (
        "lavaman_shocka1",
        MonsterFrame {
            frame: 80,
            next: "lavaman_shocka2",
            operations: OPS_LAVAMAN_SHOCKA1,
        },
    ),
    (
        "lavaman_shocka10",
        MonsterFrame {
            frame: 89,
            next: "lavaman_run1",
            operations: OPS_LAVAMAN_SHOCKA10,
        },
    ),
    (
        "lavaman_shocka2",
        MonsterFrame {
            frame: 81,
            next: "lavaman_shocka3",
            operations: OPS_LAVAMAN_SHOCKA2,
        },
    ),
    (
        "lavaman_shocka3",
        MonsterFrame {
            frame: 82,
            next: "lavaman_shocka4",
            operations: OPS_LAVAMAN_SHOCKA3,
        },
    ),
    (
        "lavaman_shocka4",
        MonsterFrame {
            frame: 83,
            next: "lavaman_shocka5",
            operations: OPS_LAVAMAN_SHOCKA4,
        },
    ),
    (
        "lavaman_shocka5",
        MonsterFrame {
            frame: 84,
            next: "lavaman_shocka6",
            operations: OPS_LAVAMAN_SHOCKA5,
        },
    ),
    (
        "lavaman_shocka6",
        MonsterFrame {
            frame: 85,
            next: "lavaman_shocka7",
            operations: OPS_LAVAMAN_SHOCKA6,
        },
    ),
    (
        "lavaman_shocka7",
        MonsterFrame {
            frame: 86,
            next: "lavaman_shocka8",
            operations: OPS_LAVAMAN_SHOCKA7,
        },
    ),
    (
        "lavaman_shocka8",
        MonsterFrame {
            frame: 87,
            next: "lavaman_shocka9",
            operations: OPS_LAVAMAN_SHOCKA8,
        },
    ),
    (
        "lavaman_shocka9",
        MonsterFrame {
            frame: 88,
            next: "lavaman_shocka10",
            operations: OPS_LAVAMAN_SHOCKA9,
        },
    ),
    (
        "lavaman_walk1",
        MonsterFrame {
            frame: 17,
            next: "lavaman_walk2",
            operations: OPS_LAVAMAN_WALK1,
        },
    ),
    (
        "lavaman_walk10",
        MonsterFrame {
            frame: 26,
            next: "lavaman_walk11",
            operations: OPS_LAVAMAN_WALK10,
        },
    ),
    (
        "lavaman_walk11",
        MonsterFrame {
            frame: 27,
            next: "lavaman_walk12",
            operations: OPS_LAVAMAN_WALK11,
        },
    ),
    (
        "lavaman_walk12",
        MonsterFrame {
            frame: 28,
            next: "lavaman_walk13",
            operations: OPS_LAVAMAN_WALK12,
        },
    ),
    (
        "lavaman_walk13",
        MonsterFrame {
            frame: 29,
            next: "lavaman_walk14",
            operations: OPS_LAVAMAN_WALK13,
        },
    ),
    (
        "lavaman_walk14",
        MonsterFrame {
            frame: 30,
            next: "lavaman_walk15",
            operations: OPS_LAVAMAN_WALK14,
        },
    ),
    (
        "lavaman_walk15",
        MonsterFrame {
            frame: 31,
            next: "lavaman_walk16",
            operations: OPS_LAVAMAN_WALK15,
        },
    ),
    (
        "lavaman_walk16",
        MonsterFrame {
            frame: 32,
            next: "lavaman_walk17",
            operations: OPS_LAVAMAN_WALK16,
        },
    ),
    (
        "lavaman_walk17",
        MonsterFrame {
            frame: 33,
            next: "lavaman_walk18",
            operations: OPS_LAVAMAN_WALK17,
        },
    ),
    (
        "lavaman_walk18",
        MonsterFrame {
            frame: 34,
            next: "lavaman_walk19",
            operations: OPS_LAVAMAN_WALK18,
        },
    ),
    (
        "lavaman_walk19",
        MonsterFrame {
            frame: 35,
            next: "lavaman_walk20",
            operations: OPS_LAVAMAN_WALK19,
        },
    ),
    (
        "lavaman_walk2",
        MonsterFrame {
            frame: 18,
            next: "lavaman_walk3",
            operations: OPS_LAVAMAN_WALK2,
        },
    ),
    (
        "lavaman_walk20",
        MonsterFrame {
            frame: 36,
            next: "lavaman_walk21",
            operations: OPS_LAVAMAN_WALK20,
        },
    ),
    (
        "lavaman_walk21",
        MonsterFrame {
            frame: 37,
            next: "lavaman_walk22",
            operations: OPS_LAVAMAN_WALK21,
        },
    ),
    (
        "lavaman_walk22",
        MonsterFrame {
            frame: 38,
            next: "lavaman_walk23",
            operations: OPS_LAVAMAN_WALK22,
        },
    ),
    (
        "lavaman_walk23",
        MonsterFrame {
            frame: 39,
            next: "lavaman_walk24",
            operations: OPS_LAVAMAN_WALK23,
        },
    ),
    (
        "lavaman_walk24",
        MonsterFrame {
            frame: 40,
            next: "lavaman_walk25",
            operations: OPS_LAVAMAN_WALK24,
        },
    ),
    (
        "lavaman_walk25",
        MonsterFrame {
            frame: 41,
            next: "lavaman_walk26",
            operations: OPS_LAVAMAN_WALK25,
        },
    ),
    (
        "lavaman_walk26",
        MonsterFrame {
            frame: 42,
            next: "lavaman_walk27",
            operations: OPS_LAVAMAN_WALK26,
        },
    ),
    (
        "lavaman_walk27",
        MonsterFrame {
            frame: 43,
            next: "lavaman_walk28",
            operations: OPS_LAVAMAN_WALK27,
        },
    ),
    (
        "lavaman_walk28",
        MonsterFrame {
            frame: 44,
            next: "lavaman_walk29",
            operations: OPS_LAVAMAN_WALK28,
        },
    ),
    (
        "lavaman_walk29",
        MonsterFrame {
            frame: 45,
            next: "lavaman_walk30",
            operations: OPS_LAVAMAN_WALK29,
        },
    ),
    (
        "lavaman_walk3",
        MonsterFrame {
            frame: 19,
            next: "lavaman_walk4",
            operations: OPS_LAVAMAN_WALK3,
        },
    ),
    (
        "lavaman_walk30",
        MonsterFrame {
            frame: 46,
            next: "lavaman_walk31",
            operations: OPS_LAVAMAN_WALK30,
        },
    ),
    (
        "lavaman_walk31",
        MonsterFrame {
            frame: 47,
            next: "lavaman_walk1",
            operations: OPS_LAVAMAN_WALK31,
        },
    ),
    (
        "lavaman_walk4",
        MonsterFrame {
            frame: 20,
            next: "lavaman_walk5",
            operations: OPS_LAVAMAN_WALK4,
        },
    ),
    (
        "lavaman_walk5",
        MonsterFrame {
            frame: 21,
            next: "lavaman_walk6",
            operations: OPS_LAVAMAN_WALK5,
        },
    ),
    (
        "lavaman_walk6",
        MonsterFrame {
            frame: 22,
            next: "lavaman_walk7",
            operations: OPS_LAVAMAN_WALK6,
        },
    ),
    (
        "lavaman_walk7",
        MonsterFrame {
            frame: 23,
            next: "lavaman_walk8",
            operations: OPS_LAVAMAN_WALK7,
        },
    ),
    (
        "lavaman_walk8",
        MonsterFrame {
            frame: 24,
            next: "lavaman_walk9",
            operations: OPS_LAVAMAN_WALK8,
        },
    ),
    (
        "lavaman_walk9",
        MonsterFrame {
            frame: 25,
            next: "lavaman_walk10",
            operations: OPS_LAVAMAN_WALK9,
        },
    ),
];

/// Look up a lavaman frame by name.
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
        assert_eq!(FRAMES.len(), 131);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("lavaman_death1").expect("first frame");
        assert_eq!((head.frame, head.next), (48, "lavaman_death2"));
        let tail = frame("lavaman_walk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (25, "lavaman_walk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
