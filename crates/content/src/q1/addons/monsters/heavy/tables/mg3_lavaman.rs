//! Q1 Mg3 lava-man frames (`src/content/q1/addons/monsters/heavy/tables/mg3_lavaman.ts`).
//!
//! quakec_mg3/monsters/mg3_lavaman.qc source frame order. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::Q1SoundChannel;

static OPS_LAVAMAN_RISE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/out1.wav",
    channel: Q1SoundChannel::Weapon,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_LAVAMAN_RISE2: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/sight1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
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
static OPS_LAVAMAN_WALK1: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK2: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK3: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK4: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK5: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK6: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK7: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK8: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_WALK9: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
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
static OPS_LAVAMAN_WALK30: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_walk" }];
static OPS_LAVAMAN_RUN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN2: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN3: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN4: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN5: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN6: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN7: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN8: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_RUN9: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
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
static OPS_LAVAMAN_RUN30: &[MonsterOperation] = &[MonsterOperation::Action { name: "lavaman_run" }];
static OPS_LAVAMAN_FIRE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
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
static OPS_LAVAMAN_FIRE11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
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
static OPS_LAVAMAN_FIRE20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_FIRE21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_LAVAMAN_SHOCKA1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/pain.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_LAVAMAN_DEATH1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/death.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_LAVAMAN_DEATH9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_lavaman:lavaman_death9",
}];
static OPS_LAVAMAN_DEATH10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_lavaman:lavaman_death10",
}];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("lavaman_rise1"),
                MonsterFrame {
                    frame: 0,
                    next: "lavaman_rise2",
                    operations: OPS_LAVAMAN_RISE1,
                },
            ),
            (
                String::from("lavaman_rise2"),
                MonsterFrame {
                    frame: 1,
                    next: "lavaman_rise3",
                    operations: OPS_LAVAMAN_RISE2,
                },
            ),
            (
                String::from("lavaman_rise3"),
                MonsterFrame {
                    frame: 2,
                    next: "lavaman_rise4",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise4"),
                MonsterFrame {
                    frame: 3,
                    next: "lavaman_rise5",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise5"),
                MonsterFrame {
                    frame: 4,
                    next: "lavaman_rise6",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise6"),
                MonsterFrame {
                    frame: 5,
                    next: "lavaman_rise7",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise7"),
                MonsterFrame {
                    frame: 6,
                    next: "lavaman_rise8",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise8"),
                MonsterFrame {
                    frame: 7,
                    next: "lavaman_rise9",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise9"),
                MonsterFrame {
                    frame: 8,
                    next: "lavaman_rise10",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise10"),
                MonsterFrame {
                    frame: 9,
                    next: "lavaman_rise11",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise11"),
                MonsterFrame {
                    frame: 10,
                    next: "lavaman_rise12",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise12"),
                MonsterFrame {
                    frame: 11,
                    next: "lavaman_rise13",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise13"),
                MonsterFrame {
                    frame: 12,
                    next: "lavaman_rise14",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise14"),
                MonsterFrame {
                    frame: 13,
                    next: "lavaman_rise15",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise15"),
                MonsterFrame {
                    frame: 14,
                    next: "lavaman_rise16",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise16"),
                MonsterFrame {
                    frame: 15,
                    next: "lavaman_rise17",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_rise17"),
                MonsterFrame {
                    frame: 16,
                    next: "lavaman_fire1",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_idle1"),
                MonsterFrame {
                    frame: 17,
                    next: "lavaman_idle2",
                    operations: OPS_LAVAMAN_IDLE1,
                },
            ),
            (
                String::from("lavaman_idle2"),
                MonsterFrame {
                    frame: 18,
                    next: "lavaman_idle3",
                    operations: OPS_LAVAMAN_IDLE2,
                },
            ),
            (
                String::from("lavaman_idle3"),
                MonsterFrame {
                    frame: 19,
                    next: "lavaman_idle4",
                    operations: OPS_LAVAMAN_IDLE3,
                },
            ),
            (
                String::from("lavaman_idle4"),
                MonsterFrame {
                    frame: 20,
                    next: "lavaman_idle5",
                    operations: OPS_LAVAMAN_IDLE4,
                },
            ),
            (
                String::from("lavaman_idle5"),
                MonsterFrame {
                    frame: 21,
                    next: "lavaman_idle6",
                    operations: OPS_LAVAMAN_IDLE5,
                },
            ),
            (
                String::from("lavaman_idle6"),
                MonsterFrame {
                    frame: 20,
                    next: "lavaman_idle7",
                    operations: OPS_LAVAMAN_IDLE6,
                },
            ),
            (
                String::from("lavaman_idle7"),
                MonsterFrame {
                    frame: 19,
                    next: "lavaman_idle8",
                    operations: OPS_LAVAMAN_IDLE7,
                },
            ),
            (
                String::from("lavaman_idle8"),
                MonsterFrame {
                    frame: 18,
                    next: "lavaman_idle9",
                    operations: OPS_LAVAMAN_IDLE8,
                },
            ),
            (
                String::from("lavaman_idle9"),
                MonsterFrame {
                    frame: 17,
                    next: "lavaman_idle1",
                    operations: OPS_LAVAMAN_IDLE9,
                },
            ),
            (
                String::from("lavaman_walk1"),
                MonsterFrame {
                    frame: 17,
                    next: "lavaman_walk2",
                    operations: OPS_LAVAMAN_WALK1,
                },
            ),
            (
                String::from("lavaman_walk2"),
                MonsterFrame {
                    frame: 18,
                    next: "lavaman_walk3",
                    operations: OPS_LAVAMAN_WALK2,
                },
            ),
            (
                String::from("lavaman_walk3"),
                MonsterFrame {
                    frame: 19,
                    next: "lavaman_walk4",
                    operations: OPS_LAVAMAN_WALK3,
                },
            ),
            (
                String::from("lavaman_walk4"),
                MonsterFrame {
                    frame: 20,
                    next: "lavaman_walk5",
                    operations: OPS_LAVAMAN_WALK4,
                },
            ),
            (
                String::from("lavaman_walk5"),
                MonsterFrame {
                    frame: 21,
                    next: "lavaman_walk6",
                    operations: OPS_LAVAMAN_WALK5,
                },
            ),
            (
                String::from("lavaman_walk6"),
                MonsterFrame {
                    frame: 22,
                    next: "lavaman_walk7",
                    operations: OPS_LAVAMAN_WALK6,
                },
            ),
            (
                String::from("lavaman_walk7"),
                MonsterFrame {
                    frame: 23,
                    next: "lavaman_walk8",
                    operations: OPS_LAVAMAN_WALK7,
                },
            ),
            (
                String::from("lavaman_walk8"),
                MonsterFrame {
                    frame: 24,
                    next: "lavaman_walk9",
                    operations: OPS_LAVAMAN_WALK8,
                },
            ),
            (
                String::from("lavaman_walk9"),
                MonsterFrame {
                    frame: 25,
                    next: "lavaman_walk10",
                    operations: OPS_LAVAMAN_WALK9,
                },
            ),
            (
                String::from("lavaman_walk10"),
                MonsterFrame {
                    frame: 26,
                    next: "lavaman_walk11",
                    operations: OPS_LAVAMAN_WALK10,
                },
            ),
            (
                String::from("lavaman_walk11"),
                MonsterFrame {
                    frame: 27,
                    next: "lavaman_walk12",
                    operations: OPS_LAVAMAN_WALK11,
                },
            ),
            (
                String::from("lavaman_walk12"),
                MonsterFrame {
                    frame: 28,
                    next: "lavaman_walk13",
                    operations: OPS_LAVAMAN_WALK12,
                },
            ),
            (
                String::from("lavaman_walk13"),
                MonsterFrame {
                    frame: 29,
                    next: "lavaman_walk14",
                    operations: OPS_LAVAMAN_WALK13,
                },
            ),
            (
                String::from("lavaman_walk14"),
                MonsterFrame {
                    frame: 30,
                    next: "lavaman_walk15",
                    operations: OPS_LAVAMAN_WALK14,
                },
            ),
            (
                String::from("lavaman_walk15"),
                MonsterFrame {
                    frame: 31,
                    next: "lavaman_walk16",
                    operations: OPS_LAVAMAN_WALK15,
                },
            ),
            (
                String::from("lavaman_walk16"),
                MonsterFrame {
                    frame: 32,
                    next: "lavaman_walk17",
                    operations: OPS_LAVAMAN_WALK16,
                },
            ),
            (
                String::from("lavaman_walk17"),
                MonsterFrame {
                    frame: 33,
                    next: "lavaman_walk18",
                    operations: OPS_LAVAMAN_WALK17,
                },
            ),
            (
                String::from("lavaman_walk18"),
                MonsterFrame {
                    frame: 34,
                    next: "lavaman_walk19",
                    operations: OPS_LAVAMAN_WALK18,
                },
            ),
            (
                String::from("lavaman_walk19"),
                MonsterFrame {
                    frame: 35,
                    next: "lavaman_walk20",
                    operations: OPS_LAVAMAN_WALK19,
                },
            ),
            (
                String::from("lavaman_walk20"),
                MonsterFrame {
                    frame: 36,
                    next: "lavaman_walk21",
                    operations: OPS_LAVAMAN_WALK20,
                },
            ),
            (
                String::from("lavaman_walk21"),
                MonsterFrame {
                    frame: 37,
                    next: "lavaman_walk22",
                    operations: OPS_LAVAMAN_WALK21,
                },
            ),
            (
                String::from("lavaman_walk22"),
                MonsterFrame {
                    frame: 38,
                    next: "lavaman_walk23",
                    operations: OPS_LAVAMAN_WALK22,
                },
            ),
            (
                String::from("lavaman_walk23"),
                MonsterFrame {
                    frame: 39,
                    next: "lavaman_walk24",
                    operations: OPS_LAVAMAN_WALK23,
                },
            ),
            (
                String::from("lavaman_walk24"),
                MonsterFrame {
                    frame: 40,
                    next: "lavaman_walk25",
                    operations: OPS_LAVAMAN_WALK24,
                },
            ),
            (
                String::from("lavaman_walk25"),
                MonsterFrame {
                    frame: 41,
                    next: "lavaman_walk26",
                    operations: OPS_LAVAMAN_WALK25,
                },
            ),
            (
                String::from("lavaman_walk26"),
                MonsterFrame {
                    frame: 42,
                    next: "lavaman_walk27",
                    operations: OPS_LAVAMAN_WALK26,
                },
            ),
            (
                String::from("lavaman_walk27"),
                MonsterFrame {
                    frame: 43,
                    next: "lavaman_walk28",
                    operations: OPS_LAVAMAN_WALK27,
                },
            ),
            (
                String::from("lavaman_walk28"),
                MonsterFrame {
                    frame: 44,
                    next: "lavaman_walk29",
                    operations: OPS_LAVAMAN_WALK28,
                },
            ),
            (
                String::from("lavaman_walk29"),
                MonsterFrame {
                    frame: 45,
                    next: "lavaman_walk30",
                    operations: OPS_LAVAMAN_WALK29,
                },
            ),
            (
                String::from("lavaman_walk30"),
                MonsterFrame {
                    frame: 46,
                    next: "lavaman_walk31",
                    operations: OPS_LAVAMAN_WALK30,
                },
            ),
            (
                String::from("lavaman_walk31"),
                MonsterFrame {
                    frame: 47,
                    next: "lavaman_walk1",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_run1"),
                MonsterFrame {
                    frame: 17,
                    next: "lavaman_run2",
                    operations: OPS_LAVAMAN_RUN1,
                },
            ),
            (
                String::from("lavaman_run2"),
                MonsterFrame {
                    frame: 18,
                    next: "lavaman_run3",
                    operations: OPS_LAVAMAN_RUN2,
                },
            ),
            (
                String::from("lavaman_run3"),
                MonsterFrame {
                    frame: 19,
                    next: "lavaman_run4",
                    operations: OPS_LAVAMAN_RUN3,
                },
            ),
            (
                String::from("lavaman_run4"),
                MonsterFrame {
                    frame: 20,
                    next: "lavaman_run5",
                    operations: OPS_LAVAMAN_RUN4,
                },
            ),
            (
                String::from("lavaman_run5"),
                MonsterFrame {
                    frame: 21,
                    next: "lavaman_run6",
                    operations: OPS_LAVAMAN_RUN5,
                },
            ),
            (
                String::from("lavaman_run6"),
                MonsterFrame {
                    frame: 22,
                    next: "lavaman_run7",
                    operations: OPS_LAVAMAN_RUN6,
                },
            ),
            (
                String::from("lavaman_run7"),
                MonsterFrame {
                    frame: 23,
                    next: "lavaman_run8",
                    operations: OPS_LAVAMAN_RUN7,
                },
            ),
            (
                String::from("lavaman_run8"),
                MonsterFrame {
                    frame: 24,
                    next: "lavaman_run9",
                    operations: OPS_LAVAMAN_RUN8,
                },
            ),
            (
                String::from("lavaman_run9"),
                MonsterFrame {
                    frame: 25,
                    next: "lavaman_run10",
                    operations: OPS_LAVAMAN_RUN9,
                },
            ),
            (
                String::from("lavaman_run10"),
                MonsterFrame {
                    frame: 26,
                    next: "lavaman_run11",
                    operations: OPS_LAVAMAN_RUN10,
                },
            ),
            (
                String::from("lavaman_run11"),
                MonsterFrame {
                    frame: 27,
                    next: "lavaman_run12",
                    operations: OPS_LAVAMAN_RUN11,
                },
            ),
            (
                String::from("lavaman_run12"),
                MonsterFrame {
                    frame: 28,
                    next: "lavaman_run13",
                    operations: OPS_LAVAMAN_RUN12,
                },
            ),
            (
                String::from("lavaman_run13"),
                MonsterFrame {
                    frame: 29,
                    next: "lavaman_run14",
                    operations: OPS_LAVAMAN_RUN13,
                },
            ),
            (
                String::from("lavaman_run14"),
                MonsterFrame {
                    frame: 30,
                    next: "lavaman_run15",
                    operations: OPS_LAVAMAN_RUN14,
                },
            ),
            (
                String::from("lavaman_run15"),
                MonsterFrame {
                    frame: 31,
                    next: "lavaman_run16",
                    operations: OPS_LAVAMAN_RUN15,
                },
            ),
            (
                String::from("lavaman_run16"),
                MonsterFrame {
                    frame: 32,
                    next: "lavaman_run17",
                    operations: OPS_LAVAMAN_RUN16,
                },
            ),
            (
                String::from("lavaman_run17"),
                MonsterFrame {
                    frame: 33,
                    next: "lavaman_run18",
                    operations: OPS_LAVAMAN_RUN17,
                },
            ),
            (
                String::from("lavaman_run18"),
                MonsterFrame {
                    frame: 34,
                    next: "lavaman_run19",
                    operations: OPS_LAVAMAN_RUN18,
                },
            ),
            (
                String::from("lavaman_run19"),
                MonsterFrame {
                    frame: 35,
                    next: "lavaman_run20",
                    operations: OPS_LAVAMAN_RUN19,
                },
            ),
            (
                String::from("lavaman_run20"),
                MonsterFrame {
                    frame: 36,
                    next: "lavaman_run21",
                    operations: OPS_LAVAMAN_RUN20,
                },
            ),
            (
                String::from("lavaman_run21"),
                MonsterFrame {
                    frame: 37,
                    next: "lavaman_run22",
                    operations: OPS_LAVAMAN_RUN21,
                },
            ),
            (
                String::from("lavaman_run22"),
                MonsterFrame {
                    frame: 38,
                    next: "lavaman_run23",
                    operations: OPS_LAVAMAN_RUN22,
                },
            ),
            (
                String::from("lavaman_run23"),
                MonsterFrame {
                    frame: 39,
                    next: "lavaman_run24",
                    operations: OPS_LAVAMAN_RUN23,
                },
            ),
            (
                String::from("lavaman_run24"),
                MonsterFrame {
                    frame: 40,
                    next: "lavaman_run25",
                    operations: OPS_LAVAMAN_RUN24,
                },
            ),
            (
                String::from("lavaman_run25"),
                MonsterFrame {
                    frame: 41,
                    next: "lavaman_run26",
                    operations: OPS_LAVAMAN_RUN25,
                },
            ),
            (
                String::from("lavaman_run26"),
                MonsterFrame {
                    frame: 42,
                    next: "lavaman_run27",
                    operations: OPS_LAVAMAN_RUN26,
                },
            ),
            (
                String::from("lavaman_run27"),
                MonsterFrame {
                    frame: 43,
                    next: "lavaman_run28",
                    operations: OPS_LAVAMAN_RUN27,
                },
            ),
            (
                String::from("lavaman_run28"),
                MonsterFrame {
                    frame: 44,
                    next: "lavaman_run29",
                    operations: OPS_LAVAMAN_RUN28,
                },
            ),
            (
                String::from("lavaman_run29"),
                MonsterFrame {
                    frame: 45,
                    next: "lavaman_run30",
                    operations: OPS_LAVAMAN_RUN29,
                },
            ),
            (
                String::from("lavaman_run30"),
                MonsterFrame {
                    frame: 46,
                    next: "lavaman_run31",
                    operations: OPS_LAVAMAN_RUN30,
                },
            ),
            (
                String::from("lavaman_run31"),
                MonsterFrame {
                    frame: 47,
                    next: "lavaman_run1",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire1"),
                MonsterFrame {
                    frame: 57,
                    next: "lavaman_fire2",
                    operations: OPS_LAVAMAN_FIRE1,
                },
            ),
            (
                String::from("lavaman_fire2"),
                MonsterFrame {
                    frame: 58,
                    next: "lavaman_fire3",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire3"),
                MonsterFrame {
                    frame: 59,
                    next: "lavaman_fire4",
                    operations: OPS_LAVAMAN_FIRE3,
                },
            ),
            (
                String::from("lavaman_fire4"),
                MonsterFrame {
                    frame: 60,
                    next: "lavaman_fire5",
                    operations: OPS_LAVAMAN_FIRE4,
                },
            ),
            (
                String::from("lavaman_fire5"),
                MonsterFrame {
                    frame: 61,
                    next: "lavaman_fire6",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire6"),
                MonsterFrame {
                    frame: 62,
                    next: "lavaman_fire7",
                    operations: OPS_LAVAMAN_FIRE6,
                },
            ),
            (
                String::from("lavaman_fire7"),
                MonsterFrame {
                    frame: 63,
                    next: "lavaman_fire8",
                    operations: OPS_LAVAMAN_FIRE7,
                },
            ),
            (
                String::from("lavaman_fire8"),
                MonsterFrame {
                    frame: 64,
                    next: "lavaman_fire9",
                    operations: OPS_LAVAMAN_FIRE8,
                },
            ),
            (
                String::from("lavaman_fire9"),
                MonsterFrame {
                    frame: 65,
                    next: "lavaman_fire10",
                    operations: OPS_LAVAMAN_FIRE9,
                },
            ),
            (
                String::from("lavaman_fire10"),
                MonsterFrame {
                    frame: 66,
                    next: "lavaman_fire11",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire11"),
                MonsterFrame {
                    frame: 67,
                    next: "lavaman_fire12",
                    operations: OPS_LAVAMAN_FIRE11,
                },
            ),
            (
                String::from("lavaman_fire12"),
                MonsterFrame {
                    frame: 68,
                    next: "lavaman_fire13",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire13"),
                MonsterFrame {
                    frame: 69,
                    next: "lavaman_fire14",
                    operations: OPS_LAVAMAN_FIRE13,
                },
            ),
            (
                String::from("lavaman_fire14"),
                MonsterFrame {
                    frame: 70,
                    next: "lavaman_fire15",
                    operations: OPS_LAVAMAN_FIRE14,
                },
            ),
            (
                String::from("lavaman_fire15"),
                MonsterFrame {
                    frame: 71,
                    next: "lavaman_fire16",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire16"),
                MonsterFrame {
                    frame: 72,
                    next: "lavaman_fire17",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire17"),
                MonsterFrame {
                    frame: 73,
                    next: "lavaman_fire18",
                    operations: OPS_LAVAMAN_FIRE17,
                },
            ),
            (
                String::from("lavaman_fire18"),
                MonsterFrame {
                    frame: 74,
                    next: "lavaman_fire19",
                    operations: OPS_LAVAMAN_FIRE18,
                },
            ),
            (
                String::from("lavaman_fire19"),
                MonsterFrame {
                    frame: 75,
                    next: "lavaman_fire20",
                    operations: OPS_LAVAMAN_FIRE19,
                },
            ),
            (
                String::from("lavaman_fire20"),
                MonsterFrame {
                    frame: 76,
                    next: "lavaman_fire21",
                    operations: OPS_LAVAMAN_FIRE20,
                },
            ),
            (
                String::from("lavaman_fire21"),
                MonsterFrame {
                    frame: 77,
                    next: "lavaman_fire22",
                    operations: OPS_LAVAMAN_FIRE21,
                },
            ),
            (
                String::from("lavaman_fire22"),
                MonsterFrame {
                    frame: 78,
                    next: "lavaman_fire23",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_fire23"),
                MonsterFrame {
                    frame: 79,
                    next: "lavaman_run1",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka1"),
                MonsterFrame {
                    frame: 80,
                    next: "lavaman_shocka2",
                    operations: OPS_LAVAMAN_SHOCKA1,
                },
            ),
            (
                String::from("lavaman_shocka2"),
                MonsterFrame {
                    frame: 81,
                    next: "lavaman_shocka3",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka3"),
                MonsterFrame {
                    frame: 82,
                    next: "lavaman_shocka4",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka4"),
                MonsterFrame {
                    frame: 83,
                    next: "lavaman_shocka5",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka5"),
                MonsterFrame {
                    frame: 84,
                    next: "lavaman_shocka6",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka6"),
                MonsterFrame {
                    frame: 85,
                    next: "lavaman_shocka7",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka7"),
                MonsterFrame {
                    frame: 86,
                    next: "lavaman_shocka8",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka8"),
                MonsterFrame {
                    frame: 87,
                    next: "lavaman_shocka9",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka9"),
                MonsterFrame {
                    frame: 88,
                    next: "lavaman_shocka10",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_shocka10"),
                MonsterFrame {
                    frame: 89,
                    next: "lavaman_run1",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death1"),
                MonsterFrame {
                    frame: 48,
                    next: "lavaman_death2",
                    operations: OPS_LAVAMAN_DEATH1,
                },
            ),
            (
                String::from("lavaman_death2"),
                MonsterFrame {
                    frame: 49,
                    next: "lavaman_death3",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death3"),
                MonsterFrame {
                    frame: 50,
                    next: "lavaman_death4",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death4"),
                MonsterFrame {
                    frame: 51,
                    next: "lavaman_death5",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death5"),
                MonsterFrame {
                    frame: 52,
                    next: "lavaman_death6",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death6"),
                MonsterFrame {
                    frame: 53,
                    next: "lavaman_death7",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death7"),
                MonsterFrame {
                    frame: 54,
                    next: "lavaman_death8",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death8"),
                MonsterFrame {
                    frame: 55,
                    next: "lavaman_death9",
                    operations: &[],
                },
            ),
            (
                String::from("lavaman_death9"),
                MonsterFrame {
                    frame: 56,
                    next: "lavaman_death10",
                    operations: OPS_LAVAMAN_DEATH9,
                },
            ),
            (
                String::from("lavaman_death10"),
                MonsterFrame {
                    frame: 56,
                    next: "lavaman_death10",
                    operations: OPS_LAVAMAN_DEATH10,
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
        assert_eq!(frames().len(), 131);
        for (name, frame) in frames() {
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
