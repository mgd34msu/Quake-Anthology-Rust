//! Q1 Old One reborn frames (`src/content/q1/addons/monsters/bosses/frames/oldnew.ts`).
//!
//! mg3_oldone_new.qc source frame declarations. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_OLDNEW_IDLE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE21: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE22: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE23: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE24: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE25: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE26: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE27: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE28: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE29: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE30: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE31: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE32: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE33: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE34: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE35: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE36: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE37: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE38: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE39: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE40: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE41: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE42: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE43: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE44: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE45: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_IDLE46: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_idle1",
}];
static OPS_OLDNEW_WALK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk1",
}];
static OPS_OLDNEW_WALK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk14",
}];
static OPS_OLDNEW_WALK15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK21: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk21",
}];
static OPS_OLDNEW_WALK22: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk22",
}];
static OPS_OLDNEW_WALK23: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk23",
}];
static OPS_OLDNEW_WALK24: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk24",
}];
static OPS_OLDNEW_WALK25: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk25",
}];
static OPS_OLDNEW_WALK26: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK27: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK28: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK29: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk14",
}];
static OPS_OLDNEW_WALK30: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK31: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK32: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK33: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK34: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK35: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK36: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk36",
}];
static OPS_OLDNEW_WALK37: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk37",
}];
static OPS_OLDNEW_WALK38: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk38",
}];
static OPS_OLDNEW_WALK39: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk39",
}];
static OPS_OLDNEW_WALK40: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk40",
}];
static OPS_OLDNEW_WALK41: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK42: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK43: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_WALK44: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk14",
}];
static OPS_OLDNEW_WALK45: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk45",
}];
static OPS_OLDNEW_WALK46: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_walk2",
}];
static OPS_OLDNEW_THRASH4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash4",
}];
static OPS_OLDNEW_THRASH7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash4",
}];
static OPS_OLDNEW_THRASH11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash4",
}];
static OPS_OLDNEW_THRASH14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash14",
}];
static OPS_OLDNEW_THRASH15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash15",
}];
static OPS_OLDNEW_DEATH1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death1",
}];
static OPS_OLDNEW_DEATH4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash4",
}];
static OPS_OLDNEW_DEATH7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash4",
}];
static OPS_OLDNEW_DEATH10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_thrash4",
}];
static OPS_OLDNEW_DEATH15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death15",
}];
static OPS_OLDNEW_DEATH16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death16",
}];
static OPS_OLDNEW_DEATH17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death17",
}];
static OPS_OLDNEW_DEATH18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death18",
}];
static OPS_OLDNEW_DEATH19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death19",
}];
static OPS_OLDNEW_DEATH20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "oldnew:oldnew_death20",
}];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("oldnew_idle1"),
                MonsterFrame {
                    frame: 0,
                    next: "oldnew_idle2",
                    operations: OPS_OLDNEW_IDLE1,
                },
            ),
            (
                String::from("oldnew_idle2"),
                MonsterFrame {
                    frame: 1,
                    next: "oldnew_idle3",
                    operations: OPS_OLDNEW_IDLE2,
                },
            ),
            (
                String::from("oldnew_idle3"),
                MonsterFrame {
                    frame: 2,
                    next: "oldnew_idle4",
                    operations: OPS_OLDNEW_IDLE3,
                },
            ),
            (
                String::from("oldnew_idle4"),
                MonsterFrame {
                    frame: 3,
                    next: "oldnew_idle5",
                    operations: OPS_OLDNEW_IDLE4,
                },
            ),
            (
                String::from("oldnew_idle5"),
                MonsterFrame {
                    frame: 4,
                    next: "oldnew_idle6",
                    operations: OPS_OLDNEW_IDLE5,
                },
            ),
            (
                String::from("oldnew_idle6"),
                MonsterFrame {
                    frame: 5,
                    next: "oldnew_idle7",
                    operations: OPS_OLDNEW_IDLE6,
                },
            ),
            (
                String::from("oldnew_idle7"),
                MonsterFrame {
                    frame: 6,
                    next: "oldnew_idle8",
                    operations: OPS_OLDNEW_IDLE7,
                },
            ),
            (
                String::from("oldnew_idle8"),
                MonsterFrame {
                    frame: 7,
                    next: "oldnew_idle9",
                    operations: OPS_OLDNEW_IDLE8,
                },
            ),
            (
                String::from("oldnew_idle9"),
                MonsterFrame {
                    frame: 8,
                    next: "oldnew_idle10",
                    operations: OPS_OLDNEW_IDLE9,
                },
            ),
            (
                String::from("oldnew_idle10"),
                MonsterFrame {
                    frame: 9,
                    next: "oldnew_idle11",
                    operations: OPS_OLDNEW_IDLE10,
                },
            ),
            (
                String::from("oldnew_idle11"),
                MonsterFrame {
                    frame: 10,
                    next: "oldnew_idle12",
                    operations: OPS_OLDNEW_IDLE11,
                },
            ),
            (
                String::from("oldnew_idle12"),
                MonsterFrame {
                    frame: 11,
                    next: "oldnew_idle13",
                    operations: OPS_OLDNEW_IDLE12,
                },
            ),
            (
                String::from("oldnew_idle13"),
                MonsterFrame {
                    frame: 12,
                    next: "oldnew_idle14",
                    operations: OPS_OLDNEW_IDLE13,
                },
            ),
            (
                String::from("oldnew_idle14"),
                MonsterFrame {
                    frame: 13,
                    next: "oldnew_idle15",
                    operations: OPS_OLDNEW_IDLE14,
                },
            ),
            (
                String::from("oldnew_idle15"),
                MonsterFrame {
                    frame: 14,
                    next: "oldnew_idle16",
                    operations: OPS_OLDNEW_IDLE15,
                },
            ),
            (
                String::from("oldnew_idle16"),
                MonsterFrame {
                    frame: 15,
                    next: "oldnew_idle17",
                    operations: OPS_OLDNEW_IDLE16,
                },
            ),
            (
                String::from("oldnew_idle17"),
                MonsterFrame {
                    frame: 16,
                    next: "oldnew_idle18",
                    operations: OPS_OLDNEW_IDLE17,
                },
            ),
            (
                String::from("oldnew_idle18"),
                MonsterFrame {
                    frame: 17,
                    next: "oldnew_idle19",
                    operations: OPS_OLDNEW_IDLE18,
                },
            ),
            (
                String::from("oldnew_idle19"),
                MonsterFrame {
                    frame: 18,
                    next: "oldnew_idle20",
                    operations: OPS_OLDNEW_IDLE19,
                },
            ),
            (
                String::from("oldnew_idle20"),
                MonsterFrame {
                    frame: 19,
                    next: "oldnew_idle21",
                    operations: OPS_OLDNEW_IDLE20,
                },
            ),
            (
                String::from("oldnew_idle21"),
                MonsterFrame {
                    frame: 20,
                    next: "oldnew_idle22",
                    operations: OPS_OLDNEW_IDLE21,
                },
            ),
            (
                String::from("oldnew_idle22"),
                MonsterFrame {
                    frame: 21,
                    next: "oldnew_idle23",
                    operations: OPS_OLDNEW_IDLE22,
                },
            ),
            (
                String::from("oldnew_idle23"),
                MonsterFrame {
                    frame: 22,
                    next: "oldnew_idle24",
                    operations: OPS_OLDNEW_IDLE23,
                },
            ),
            (
                String::from("oldnew_idle24"),
                MonsterFrame {
                    frame: 23,
                    next: "oldnew_idle25",
                    operations: OPS_OLDNEW_IDLE24,
                },
            ),
            (
                String::from("oldnew_idle25"),
                MonsterFrame {
                    frame: 24,
                    next: "oldnew_idle26",
                    operations: OPS_OLDNEW_IDLE25,
                },
            ),
            (
                String::from("oldnew_idle26"),
                MonsterFrame {
                    frame: 25,
                    next: "oldnew_idle27",
                    operations: OPS_OLDNEW_IDLE26,
                },
            ),
            (
                String::from("oldnew_idle27"),
                MonsterFrame {
                    frame: 26,
                    next: "oldnew_idle28",
                    operations: OPS_OLDNEW_IDLE27,
                },
            ),
            (
                String::from("oldnew_idle28"),
                MonsterFrame {
                    frame: 27,
                    next: "oldnew_idle29",
                    operations: OPS_OLDNEW_IDLE28,
                },
            ),
            (
                String::from("oldnew_idle29"),
                MonsterFrame {
                    frame: 28,
                    next: "oldnew_idle30",
                    operations: OPS_OLDNEW_IDLE29,
                },
            ),
            (
                String::from("oldnew_idle30"),
                MonsterFrame {
                    frame: 29,
                    next: "oldnew_idle31",
                    operations: OPS_OLDNEW_IDLE30,
                },
            ),
            (
                String::from("oldnew_idle31"),
                MonsterFrame {
                    frame: 30,
                    next: "oldnew_idle32",
                    operations: OPS_OLDNEW_IDLE31,
                },
            ),
            (
                String::from("oldnew_idle32"),
                MonsterFrame {
                    frame: 31,
                    next: "oldnew_idle33",
                    operations: OPS_OLDNEW_IDLE32,
                },
            ),
            (
                String::from("oldnew_idle33"),
                MonsterFrame {
                    frame: 32,
                    next: "oldnew_idle34",
                    operations: OPS_OLDNEW_IDLE33,
                },
            ),
            (
                String::from("oldnew_idle34"),
                MonsterFrame {
                    frame: 33,
                    next: "oldnew_idle35",
                    operations: OPS_OLDNEW_IDLE34,
                },
            ),
            (
                String::from("oldnew_idle35"),
                MonsterFrame {
                    frame: 34,
                    next: "oldnew_idle36",
                    operations: OPS_OLDNEW_IDLE35,
                },
            ),
            (
                String::from("oldnew_idle36"),
                MonsterFrame {
                    frame: 35,
                    next: "oldnew_idle37",
                    operations: OPS_OLDNEW_IDLE36,
                },
            ),
            (
                String::from("oldnew_idle37"),
                MonsterFrame {
                    frame: 36,
                    next: "oldnew_idle38",
                    operations: OPS_OLDNEW_IDLE37,
                },
            ),
            (
                String::from("oldnew_idle38"),
                MonsterFrame {
                    frame: 37,
                    next: "oldnew_idle39",
                    operations: OPS_OLDNEW_IDLE38,
                },
            ),
            (
                String::from("oldnew_idle39"),
                MonsterFrame {
                    frame: 38,
                    next: "oldnew_idle40",
                    operations: OPS_OLDNEW_IDLE39,
                },
            ),
            (
                String::from("oldnew_idle40"),
                MonsterFrame {
                    frame: 39,
                    next: "oldnew_idle41",
                    operations: OPS_OLDNEW_IDLE40,
                },
            ),
            (
                String::from("oldnew_idle41"),
                MonsterFrame {
                    frame: 40,
                    next: "oldnew_idle42",
                    operations: OPS_OLDNEW_IDLE41,
                },
            ),
            (
                String::from("oldnew_idle42"),
                MonsterFrame {
                    frame: 41,
                    next: "oldnew_idle43",
                    operations: OPS_OLDNEW_IDLE42,
                },
            ),
            (
                String::from("oldnew_idle43"),
                MonsterFrame {
                    frame: 42,
                    next: "oldnew_idle44",
                    operations: OPS_OLDNEW_IDLE43,
                },
            ),
            (
                String::from("oldnew_idle44"),
                MonsterFrame {
                    frame: 43,
                    next: "oldnew_idle45",
                    operations: OPS_OLDNEW_IDLE44,
                },
            ),
            (
                String::from("oldnew_idle45"),
                MonsterFrame {
                    frame: 44,
                    next: "oldnew_idle46",
                    operations: OPS_OLDNEW_IDLE45,
                },
            ),
            (
                String::from("oldnew_idle46"),
                MonsterFrame {
                    frame: 45,
                    next: "oldnew_idle1",
                    operations: OPS_OLDNEW_IDLE46,
                },
            ),
            (
                String::from("oldnew_walk1"),
                MonsterFrame {
                    frame: 0,
                    next: "oldnew_walk2",
                    operations: OPS_OLDNEW_WALK1,
                },
            ),
            (
                String::from("oldnew_walk2"),
                MonsterFrame {
                    frame: 1,
                    next: "oldnew_walk3",
                    operations: OPS_OLDNEW_WALK2,
                },
            ),
            (
                String::from("oldnew_walk3"),
                MonsterFrame {
                    frame: 2,
                    next: "oldnew_walk4",
                    operations: OPS_OLDNEW_WALK3,
                },
            ),
            (
                String::from("oldnew_walk4"),
                MonsterFrame {
                    frame: 3,
                    next: "oldnew_walk5",
                    operations: OPS_OLDNEW_WALK4,
                },
            ),
            (
                String::from("oldnew_walk5"),
                MonsterFrame {
                    frame: 4,
                    next: "oldnew_walk6",
                    operations: OPS_OLDNEW_WALK5,
                },
            ),
            (
                String::from("oldnew_walk6"),
                MonsterFrame {
                    frame: 5,
                    next: "oldnew_walk7",
                    operations: OPS_OLDNEW_WALK6,
                },
            ),
            (
                String::from("oldnew_walk7"),
                MonsterFrame {
                    frame: 6,
                    next: "oldnew_walk8",
                    operations: OPS_OLDNEW_WALK7,
                },
            ),
            (
                String::from("oldnew_walk8"),
                MonsterFrame {
                    frame: 7,
                    next: "oldnew_walk9",
                    operations: OPS_OLDNEW_WALK8,
                },
            ),
            (
                String::from("oldnew_walk9"),
                MonsterFrame {
                    frame: 8,
                    next: "oldnew_walk10",
                    operations: OPS_OLDNEW_WALK9,
                },
            ),
            (
                String::from("oldnew_walk10"),
                MonsterFrame {
                    frame: 9,
                    next: "oldnew_walk11",
                    operations: OPS_OLDNEW_WALK10,
                },
            ),
            (
                String::from("oldnew_walk11"),
                MonsterFrame {
                    frame: 10,
                    next: "oldnew_walk12",
                    operations: OPS_OLDNEW_WALK11,
                },
            ),
            (
                String::from("oldnew_walk12"),
                MonsterFrame {
                    frame: 11,
                    next: "oldnew_walk13",
                    operations: OPS_OLDNEW_WALK12,
                },
            ),
            (
                String::from("oldnew_walk13"),
                MonsterFrame {
                    frame: 12,
                    next: "oldnew_walk14",
                    operations: OPS_OLDNEW_WALK13,
                },
            ),
            (
                String::from("oldnew_walk14"),
                MonsterFrame {
                    frame: 13,
                    next: "oldnew_walk15",
                    operations: OPS_OLDNEW_WALK14,
                },
            ),
            (
                String::from("oldnew_walk15"),
                MonsterFrame {
                    frame: 14,
                    next: "oldnew_walk16",
                    operations: OPS_OLDNEW_WALK15,
                },
            ),
            (
                String::from("oldnew_walk16"),
                MonsterFrame {
                    frame: 15,
                    next: "oldnew_walk17",
                    operations: OPS_OLDNEW_WALK16,
                },
            ),
            (
                String::from("oldnew_walk17"),
                MonsterFrame {
                    frame: 16,
                    next: "oldnew_walk18",
                    operations: OPS_OLDNEW_WALK17,
                },
            ),
            (
                String::from("oldnew_walk18"),
                MonsterFrame {
                    frame: 17,
                    next: "oldnew_walk19",
                    operations: OPS_OLDNEW_WALK18,
                },
            ),
            (
                String::from("oldnew_walk19"),
                MonsterFrame {
                    frame: 18,
                    next: "oldnew_walk20",
                    operations: OPS_OLDNEW_WALK19,
                },
            ),
            (
                String::from("oldnew_walk20"),
                MonsterFrame {
                    frame: 19,
                    next: "oldnew_walk21",
                    operations: OPS_OLDNEW_WALK20,
                },
            ),
            (
                String::from("oldnew_walk21"),
                MonsterFrame {
                    frame: 20,
                    next: "oldnew_walk22",
                    operations: OPS_OLDNEW_WALK21,
                },
            ),
            (
                String::from("oldnew_walk22"),
                MonsterFrame {
                    frame: 21,
                    next: "oldnew_walk23",
                    operations: OPS_OLDNEW_WALK22,
                },
            ),
            (
                String::from("oldnew_walk23"),
                MonsterFrame {
                    frame: 22,
                    next: "oldnew_walk24",
                    operations: OPS_OLDNEW_WALK23,
                },
            ),
            (
                String::from("oldnew_walk24"),
                MonsterFrame {
                    frame: 23,
                    next: "oldnew_walk25",
                    operations: OPS_OLDNEW_WALK24,
                },
            ),
            (
                String::from("oldnew_walk25"),
                MonsterFrame {
                    frame: 24,
                    next: "oldnew_walk26",
                    operations: OPS_OLDNEW_WALK25,
                },
            ),
            (
                String::from("oldnew_walk26"),
                MonsterFrame {
                    frame: 25,
                    next: "oldnew_walk27",
                    operations: OPS_OLDNEW_WALK26,
                },
            ),
            (
                String::from("oldnew_walk27"),
                MonsterFrame {
                    frame: 26,
                    next: "oldnew_walk28",
                    operations: OPS_OLDNEW_WALK27,
                },
            ),
            (
                String::from("oldnew_walk28"),
                MonsterFrame {
                    frame: 27,
                    next: "oldnew_walk29",
                    operations: OPS_OLDNEW_WALK28,
                },
            ),
            (
                String::from("oldnew_walk29"),
                MonsterFrame {
                    frame: 28,
                    next: "oldnew_walk30",
                    operations: OPS_OLDNEW_WALK29,
                },
            ),
            (
                String::from("oldnew_walk30"),
                MonsterFrame {
                    frame: 29,
                    next: "oldnew_walk31",
                    operations: OPS_OLDNEW_WALK30,
                },
            ),
            (
                String::from("oldnew_walk31"),
                MonsterFrame {
                    frame: 30,
                    next: "oldnew_walk32",
                    operations: OPS_OLDNEW_WALK31,
                },
            ),
            (
                String::from("oldnew_walk32"),
                MonsterFrame {
                    frame: 31,
                    next: "oldnew_walk33",
                    operations: OPS_OLDNEW_WALK32,
                },
            ),
            (
                String::from("oldnew_walk33"),
                MonsterFrame {
                    frame: 32,
                    next: "oldnew_walk34",
                    operations: OPS_OLDNEW_WALK33,
                },
            ),
            (
                String::from("oldnew_walk34"),
                MonsterFrame {
                    frame: 33,
                    next: "oldnew_walk35",
                    operations: OPS_OLDNEW_WALK34,
                },
            ),
            (
                String::from("oldnew_walk35"),
                MonsterFrame {
                    frame: 34,
                    next: "oldnew_walk36",
                    operations: OPS_OLDNEW_WALK35,
                },
            ),
            (
                String::from("oldnew_walk36"),
                MonsterFrame {
                    frame: 35,
                    next: "oldnew_walk37",
                    operations: OPS_OLDNEW_WALK36,
                },
            ),
            (
                String::from("oldnew_walk37"),
                MonsterFrame {
                    frame: 36,
                    next: "oldnew_walk38",
                    operations: OPS_OLDNEW_WALK37,
                },
            ),
            (
                String::from("oldnew_walk38"),
                MonsterFrame {
                    frame: 37,
                    next: "oldnew_walk39",
                    operations: OPS_OLDNEW_WALK38,
                },
            ),
            (
                String::from("oldnew_walk39"),
                MonsterFrame {
                    frame: 38,
                    next: "oldnew_walk40",
                    operations: OPS_OLDNEW_WALK39,
                },
            ),
            (
                String::from("oldnew_walk40"),
                MonsterFrame {
                    frame: 39,
                    next: "oldnew_walk41",
                    operations: OPS_OLDNEW_WALK40,
                },
            ),
            (
                String::from("oldnew_walk41"),
                MonsterFrame {
                    frame: 40,
                    next: "oldnew_walk42",
                    operations: OPS_OLDNEW_WALK41,
                },
            ),
            (
                String::from("oldnew_walk42"),
                MonsterFrame {
                    frame: 41,
                    next: "oldnew_walk43",
                    operations: OPS_OLDNEW_WALK42,
                },
            ),
            (
                String::from("oldnew_walk43"),
                MonsterFrame {
                    frame: 42,
                    next: "oldnew_walk44",
                    operations: OPS_OLDNEW_WALK43,
                },
            ),
            (
                String::from("oldnew_walk44"),
                MonsterFrame {
                    frame: 43,
                    next: "oldnew_walk45",
                    operations: OPS_OLDNEW_WALK44,
                },
            ),
            (
                String::from("oldnew_walk45"),
                MonsterFrame {
                    frame: 44,
                    next: "oldnew_walk46",
                    operations: OPS_OLDNEW_WALK45,
                },
            ),
            (
                String::from("oldnew_walk46"),
                MonsterFrame {
                    frame: 45,
                    next: "oldnew_walk1",
                    operations: OPS_OLDNEW_WALK46,
                },
            ),
            (
                String::from("oldnew_thrash1"),
                MonsterFrame {
                    frame: 46,
                    next: "oldnew_thrash2",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash2"),
                MonsterFrame {
                    frame: 47,
                    next: "oldnew_thrash3",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash3"),
                MonsterFrame {
                    frame: 48,
                    next: "oldnew_thrash4",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash4"),
                MonsterFrame {
                    frame: 49,
                    next: "oldnew_thrash5",
                    operations: OPS_OLDNEW_THRASH4,
                },
            ),
            (
                String::from("oldnew_thrash5"),
                MonsterFrame {
                    frame: 50,
                    next: "oldnew_thrash6",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash6"),
                MonsterFrame {
                    frame: 51,
                    next: "oldnew_thrash7",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash7"),
                MonsterFrame {
                    frame: 52,
                    next: "oldnew_thrash8",
                    operations: OPS_OLDNEW_THRASH7,
                },
            ),
            (
                String::from("oldnew_thrash8"),
                MonsterFrame {
                    frame: 53,
                    next: "oldnew_thrash9",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash9"),
                MonsterFrame {
                    frame: 54,
                    next: "oldnew_thrash10",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash10"),
                MonsterFrame {
                    frame: 55,
                    next: "oldnew_thrash11",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash11"),
                MonsterFrame {
                    frame: 56,
                    next: "oldnew_thrash12",
                    operations: OPS_OLDNEW_THRASH11,
                },
            ),
            (
                String::from("oldnew_thrash12"),
                MonsterFrame {
                    frame: 57,
                    next: "oldnew_thrash13",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash13"),
                MonsterFrame {
                    frame: 58,
                    next: "oldnew_thrash14",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_thrash14"),
                MonsterFrame {
                    frame: 59,
                    next: "oldnew_thrash15",
                    operations: OPS_OLDNEW_THRASH14,
                },
            ),
            (
                String::from("oldnew_thrash15"),
                MonsterFrame {
                    frame: 60,
                    next: "oldnew_walk1",
                    operations: OPS_OLDNEW_THRASH15,
                },
            ),
            (
                String::from("oldnew_death1"),
                MonsterFrame {
                    frame: 46,
                    next: "oldnew_death2",
                    operations: OPS_OLDNEW_DEATH1,
                },
            ),
            (
                String::from("oldnew_death2"),
                MonsterFrame {
                    frame: 47,
                    next: "oldnew_death3",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death3"),
                MonsterFrame {
                    frame: 48,
                    next: "oldnew_death4",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death4"),
                MonsterFrame {
                    frame: 49,
                    next: "oldnew_death5",
                    operations: OPS_OLDNEW_DEATH4,
                },
            ),
            (
                String::from("oldnew_death5"),
                MonsterFrame {
                    frame: 50,
                    next: "oldnew_death6",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death6"),
                MonsterFrame {
                    frame: 51,
                    next: "oldnew_death7",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death7"),
                MonsterFrame {
                    frame: 52,
                    next: "oldnew_death8",
                    operations: OPS_OLDNEW_DEATH7,
                },
            ),
            (
                String::from("oldnew_death8"),
                MonsterFrame {
                    frame: 53,
                    next: "oldnew_death9",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death9"),
                MonsterFrame {
                    frame: 54,
                    next: "oldnew_death10",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death10"),
                MonsterFrame {
                    frame: 55,
                    next: "oldnew_death11",
                    operations: OPS_OLDNEW_DEATH10,
                },
            ),
            (
                String::from("oldnew_death11"),
                MonsterFrame {
                    frame: 56,
                    next: "oldnew_death12",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death12"),
                MonsterFrame {
                    frame: 57,
                    next: "oldnew_death13",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death13"),
                MonsterFrame {
                    frame: 58,
                    next: "oldnew_death14",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death14"),
                MonsterFrame {
                    frame: 59,
                    next: "oldnew_death15",
                    operations: &[],
                },
            ),
            (
                String::from("oldnew_death15"),
                MonsterFrame {
                    frame: 60,
                    next: "oldnew_death16",
                    operations: OPS_OLDNEW_DEATH15,
                },
            ),
            (
                String::from("oldnew_death16"),
                MonsterFrame {
                    frame: 61,
                    next: "oldnew_death17",
                    operations: OPS_OLDNEW_DEATH16,
                },
            ),
            (
                String::from("oldnew_death17"),
                MonsterFrame {
                    frame: 62,
                    next: "oldnew_death18",
                    operations: OPS_OLDNEW_DEATH17,
                },
            ),
            (
                String::from("oldnew_death18"),
                MonsterFrame {
                    frame: 63,
                    next: "oldnew_death19",
                    operations: OPS_OLDNEW_DEATH18,
                },
            ),
            (
                String::from("oldnew_death19"),
                MonsterFrame {
                    frame: 64,
                    next: "oldnew_death20",
                    operations: OPS_OLDNEW_DEATH19,
                },
            ),
            (
                String::from("oldnew_death20"),
                MonsterFrame {
                    frame: 65,
                    next: "oldnew_death20",
                    operations: OPS_OLDNEW_DEATH20,
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
        assert_eq!(frames().len(), 127);
        for (name, frame) in frames() {
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
