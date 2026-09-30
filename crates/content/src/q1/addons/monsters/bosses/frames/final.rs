//! Q1 Mg1 final-boss frames (`src/content/q1/addons/monsters/bosses/frames/final.ts`).
//!
//! boss_final.qc source frame declarations. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_BOSS_FINAL_RISE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_rise1",
}];
static OPS_BOSS_FINAL_RISE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_rise2",
}];
static OPS_BOSS_FINAL_IDLE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle2",
}];
static OPS_BOSS_FINAL_IDLE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle3",
}];
static OPS_BOSS_FINAL_IDLE4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle4",
}];
static OPS_BOSS_FINAL_IDLE5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle5",
}];
static OPS_BOSS_FINAL_IDLE6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle6",
}];
static OPS_BOSS_FINAL_IDLE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle7",
}];
static OPS_BOSS_FINAL_IDLE8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle8",
}];
static OPS_BOSS_FINAL_IDLE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle9",
}];
static OPS_BOSS_FINAL_IDLE10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle10",
}];
static OPS_BOSS_FINAL_IDLE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle11",
}];
static OPS_BOSS_FINAL_IDLE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle12",
}];
static OPS_BOSS_FINAL_IDLE13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle13",
}];
static OPS_BOSS_FINAL_IDLE14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle14",
}];
static OPS_BOSS_FINAL_IDLE15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle15",
}];
static OPS_BOSS_FINAL_IDLE16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle16",
}];
static OPS_BOSS_FINAL_IDLE17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle17",
}];
static OPS_BOSS_FINAL_IDLE18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle18",
}];
static OPS_BOSS_FINAL_IDLE19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle19",
}];
static OPS_BOSS_FINAL_IDLE20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle20",
}];
static OPS_BOSS_FINAL_IDLE21: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle21",
}];
static OPS_BOSS_FINAL_IDLE22: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle22",
}];
static OPS_BOSS_FINAL_IDLE23: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle23",
}];
static OPS_BOSS_FINAL_IDLE24: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle24",
}];
static OPS_BOSS_FINAL_IDLE25: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle25",
}];
static OPS_BOSS_FINAL_IDLE26: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle26",
}];
static OPS_BOSS_FINAL_IDLE27: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle27",
}];
static OPS_BOSS_FINAL_IDLE28: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle28",
}];
static OPS_BOSS_FINAL_IDLE29: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle29",
}];
static OPS_BOSS_FINAL_IDLE30: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle30",
}];
static OPS_BOSS_FINAL_IDLE31: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_idle31",
}];
static OPS_BOSS_FINAL_MG1: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg1" }];
static OPS_BOSS_FINAL_MG2: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg2" }];
static OPS_BOSS_FINAL_MG3: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg3" }];
static OPS_BOSS_FINAL_MG4: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg4" }];
static OPS_BOSS_FINAL_MG5: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg5" }];
static OPS_BOSS_FINAL_MG6: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg6" }];
static OPS_BOSS_FINAL_MG7: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg7" }];
static OPS_BOSS_FINAL_MG8: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_final_mg8" }];
static OPS_BOSS_FINAL_MISSILE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile1",
}];
static OPS_BOSS_FINAL_MISSILE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile2",
}];
static OPS_BOSS_FINAL_MISSILE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile3",
}];
static OPS_BOSS_FINAL_MISSILE4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile4",
}];
static OPS_BOSS_FINAL_MISSILE5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile5",
}];
static OPS_BOSS_FINAL_MISSILE6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile6",
}];
static OPS_BOSS_FINAL_MISSILE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile7",
}];
static OPS_BOSS_FINAL_MISSILE8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile8",
}];
static OPS_BOSS_FINAL_MISSILE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile9",
}];
static OPS_BOSS_FINAL_MISSILE10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile10",
}];
static OPS_BOSS_FINAL_MISSILE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile11",
}];
static OPS_BOSS_FINAL_MISSILE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile12",
}];
static OPS_BOSS_FINAL_MISSILE13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile13",
}];
static OPS_BOSS_FINAL_MISSILE14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile14",
}];
static OPS_BOSS_FINAL_MISSILE15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile15",
}];
static OPS_BOSS_FINAL_MISSILE16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile16",
}];
static OPS_BOSS_FINAL_MISSILE17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile17",
}];
static OPS_BOSS_FINAL_MISSILE18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile18",
}];
static OPS_BOSS_FINAL_MISSILE19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile19",
}];
static OPS_BOSS_FINAL_MISSILE20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile20",
}];
static OPS_BOSS_FINAL_MISSILE21: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile21",
}];
static OPS_BOSS_FINAL_MISSILE22: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile22",
}];
static OPS_BOSS_FINAL_MISSILE23: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_missile23",
}];
static OPS_BOSS_FINAL_SHOCKA2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shocka2",
}];
static OPS_BOSS_FINAL_SHOCKA5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shocka5",
}];
static OPS_BOSS_FINAL_SHOCKA8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shocka8",
}];
static OPS_BOSS_FINAL_SHOCKA10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shocka10",
}];
static OPS_BOSS_FINAL_SHOCKB2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockb2",
}];
static OPS_BOSS_FINAL_SHOCKB5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockb5",
}];
static OPS_BOSS_FINAL_SHOCKB8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockb8",
}];
static OPS_BOSS_FINAL_SHOCKB10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockb10",
}];
static OPS_BOSS_FINAL_SHOCKC2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockc2",
}];
static OPS_BOSS_FINAL_SHOCKC5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockc5",
}];
static OPS_BOSS_FINAL_SHOCKC8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_shockc8",
}];
static OPS_BOSS_FINAL_DEATH1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_death1",
}];
static OPS_BOSS_FINAL_DEATH9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_death9",
}];
static OPS_BOSS_FINAL_DEATH10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "boss_final_death10",
}];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("boss_final_rise1"),
                MonsterFrame {
                    frame: 0,
                    next: "boss_final_rise2",
                    operations: OPS_BOSS_FINAL_RISE1,
                },
            ),
            (
                String::from("boss_final_rise2"),
                MonsterFrame {
                    frame: 1,
                    next: "boss_final_rise3",
                    operations: OPS_BOSS_FINAL_RISE2,
                },
            ),
            (
                String::from("boss_final_rise3"),
                MonsterFrame {
                    frame: 2,
                    next: "boss_final_rise4",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise4"),
                MonsterFrame {
                    frame: 3,
                    next: "boss_final_rise5",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise5"),
                MonsterFrame {
                    frame: 4,
                    next: "boss_final_rise6",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise6"),
                MonsterFrame {
                    frame: 5,
                    next: "boss_final_rise7",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise7"),
                MonsterFrame {
                    frame: 6,
                    next: "boss_final_rise8",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise8"),
                MonsterFrame {
                    frame: 7,
                    next: "boss_final_rise9",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise9"),
                MonsterFrame {
                    frame: 8,
                    next: "boss_final_rise10",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise10"),
                MonsterFrame {
                    frame: 9,
                    next: "boss_final_rise11",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise11"),
                MonsterFrame {
                    frame: 10,
                    next: "boss_final_rise12",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise12"),
                MonsterFrame {
                    frame: 11,
                    next: "boss_final_rise13",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise13"),
                MonsterFrame {
                    frame: 12,
                    next: "boss_final_rise14",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise14"),
                MonsterFrame {
                    frame: 13,
                    next: "boss_final_rise15",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise15"),
                MonsterFrame {
                    frame: 14,
                    next: "boss_final_rise16",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise16"),
                MonsterFrame {
                    frame: 15,
                    next: "boss_final_rise17",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_rise17"),
                MonsterFrame {
                    frame: 16,
                    next: "boss_final_missile1",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_idle1"),
                MonsterFrame {
                    frame: 17,
                    next: "boss_final_idle2",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_idle2"),
                MonsterFrame {
                    frame: 18,
                    next: "boss_final_idle3",
                    operations: OPS_BOSS_FINAL_IDLE2,
                },
            ),
            (
                String::from("boss_final_idle3"),
                MonsterFrame {
                    frame: 19,
                    next: "boss_final_idle4",
                    operations: OPS_BOSS_FINAL_IDLE3,
                },
            ),
            (
                String::from("boss_final_idle4"),
                MonsterFrame {
                    frame: 20,
                    next: "boss_final_idle5",
                    operations: OPS_BOSS_FINAL_IDLE4,
                },
            ),
            (
                String::from("boss_final_idle5"),
                MonsterFrame {
                    frame: 21,
                    next: "boss_final_idle6",
                    operations: OPS_BOSS_FINAL_IDLE5,
                },
            ),
            (
                String::from("boss_final_idle6"),
                MonsterFrame {
                    frame: 22,
                    next: "boss_final_idle7",
                    operations: OPS_BOSS_FINAL_IDLE6,
                },
            ),
            (
                String::from("boss_final_idle7"),
                MonsterFrame {
                    frame: 23,
                    next: "boss_final_idle8",
                    operations: OPS_BOSS_FINAL_IDLE7,
                },
            ),
            (
                String::from("boss_final_idle8"),
                MonsterFrame {
                    frame: 24,
                    next: "boss_final_idle9",
                    operations: OPS_BOSS_FINAL_IDLE8,
                },
            ),
            (
                String::from("boss_final_idle9"),
                MonsterFrame {
                    frame: 25,
                    next: "boss_final_idle10",
                    operations: OPS_BOSS_FINAL_IDLE9,
                },
            ),
            (
                String::from("boss_final_idle10"),
                MonsterFrame {
                    frame: 26,
                    next: "boss_final_idle11",
                    operations: OPS_BOSS_FINAL_IDLE10,
                },
            ),
            (
                String::from("boss_final_idle11"),
                MonsterFrame {
                    frame: 27,
                    next: "boss_final_idle12",
                    operations: OPS_BOSS_FINAL_IDLE11,
                },
            ),
            (
                String::from("boss_final_idle12"),
                MonsterFrame {
                    frame: 28,
                    next: "boss_final_idle13",
                    operations: OPS_BOSS_FINAL_IDLE12,
                },
            ),
            (
                String::from("boss_final_idle13"),
                MonsterFrame {
                    frame: 29,
                    next: "boss_final_idle14",
                    operations: OPS_BOSS_FINAL_IDLE13,
                },
            ),
            (
                String::from("boss_final_idle14"),
                MonsterFrame {
                    frame: 30,
                    next: "boss_final_idle15",
                    operations: OPS_BOSS_FINAL_IDLE14,
                },
            ),
            (
                String::from("boss_final_idle15"),
                MonsterFrame {
                    frame: 31,
                    next: "boss_final_idle16",
                    operations: OPS_BOSS_FINAL_IDLE15,
                },
            ),
            (
                String::from("boss_final_idle16"),
                MonsterFrame {
                    frame: 32,
                    next: "boss_final_idle17",
                    operations: OPS_BOSS_FINAL_IDLE16,
                },
            ),
            (
                String::from("boss_final_idle17"),
                MonsterFrame {
                    frame: 33,
                    next: "boss_final_idle18",
                    operations: OPS_BOSS_FINAL_IDLE17,
                },
            ),
            (
                String::from("boss_final_idle18"),
                MonsterFrame {
                    frame: 34,
                    next: "boss_final_idle19",
                    operations: OPS_BOSS_FINAL_IDLE18,
                },
            ),
            (
                String::from("boss_final_idle19"),
                MonsterFrame {
                    frame: 35,
                    next: "boss_final_idle20",
                    operations: OPS_BOSS_FINAL_IDLE19,
                },
            ),
            (
                String::from("boss_final_idle20"),
                MonsterFrame {
                    frame: 36,
                    next: "boss_final_idle21",
                    operations: OPS_BOSS_FINAL_IDLE20,
                },
            ),
            (
                String::from("boss_final_idle21"),
                MonsterFrame {
                    frame: 37,
                    next: "boss_final_idle22",
                    operations: OPS_BOSS_FINAL_IDLE21,
                },
            ),
            (
                String::from("boss_final_idle22"),
                MonsterFrame {
                    frame: 38,
                    next: "boss_final_idle23",
                    operations: OPS_BOSS_FINAL_IDLE22,
                },
            ),
            (
                String::from("boss_final_idle23"),
                MonsterFrame {
                    frame: 39,
                    next: "boss_final_idle24",
                    operations: OPS_BOSS_FINAL_IDLE23,
                },
            ),
            (
                String::from("boss_final_idle24"),
                MonsterFrame {
                    frame: 40,
                    next: "boss_final_idle25",
                    operations: OPS_BOSS_FINAL_IDLE24,
                },
            ),
            (
                String::from("boss_final_idle25"),
                MonsterFrame {
                    frame: 41,
                    next: "boss_final_idle26",
                    operations: OPS_BOSS_FINAL_IDLE25,
                },
            ),
            (
                String::from("boss_final_idle26"),
                MonsterFrame {
                    frame: 42,
                    next: "boss_final_idle27",
                    operations: OPS_BOSS_FINAL_IDLE26,
                },
            ),
            (
                String::from("boss_final_idle27"),
                MonsterFrame {
                    frame: 43,
                    next: "boss_final_idle28",
                    operations: OPS_BOSS_FINAL_IDLE27,
                },
            ),
            (
                String::from("boss_final_idle28"),
                MonsterFrame {
                    frame: 44,
                    next: "boss_final_idle29",
                    operations: OPS_BOSS_FINAL_IDLE28,
                },
            ),
            (
                String::from("boss_final_idle29"),
                MonsterFrame {
                    frame: 45,
                    next: "boss_final_idle30",
                    operations: OPS_BOSS_FINAL_IDLE29,
                },
            ),
            (
                String::from("boss_final_idle30"),
                MonsterFrame {
                    frame: 46,
                    next: "boss_final_idle31",
                    operations: OPS_BOSS_FINAL_IDLE30,
                },
            ),
            (
                String::from("boss_final_idle31"),
                MonsterFrame {
                    frame: 47,
                    next: "boss_final_idle1",
                    operations: OPS_BOSS_FINAL_IDLE31,
                },
            ),
            (
                String::from("boss_final_mg1"),
                MonsterFrame {
                    frame: 57,
                    next: "boss_final_mg2",
                    operations: OPS_BOSS_FINAL_MG1,
                },
            ),
            (
                String::from("boss_final_mg2"),
                MonsterFrame {
                    frame: 58,
                    next: "boss_final_mg3",
                    operations: OPS_BOSS_FINAL_MG2,
                },
            ),
            (
                String::from("boss_final_mg3"),
                MonsterFrame {
                    frame: 59,
                    next: "boss_final_mg4",
                    operations: OPS_BOSS_FINAL_MG3,
                },
            ),
            (
                String::from("boss_final_mg4"),
                MonsterFrame {
                    frame: 60,
                    next: "boss_final_mg5",
                    operations: OPS_BOSS_FINAL_MG4,
                },
            ),
            (
                String::from("boss_final_mg5"),
                MonsterFrame {
                    frame: 61,
                    next: "boss_final_mg6",
                    operations: OPS_BOSS_FINAL_MG5,
                },
            ),
            (
                String::from("boss_final_mg6"),
                MonsterFrame {
                    frame: 62,
                    next: "boss_final_mg7",
                    operations: OPS_BOSS_FINAL_MG6,
                },
            ),
            (
                String::from("boss_final_mg7"),
                MonsterFrame {
                    frame: 63,
                    next: "boss_final_mg8",
                    operations: OPS_BOSS_FINAL_MG7,
                },
            ),
            (
                String::from("boss_final_mg8"),
                MonsterFrame {
                    frame: 64,
                    next: "boss_final_missile1",
                    operations: OPS_BOSS_FINAL_MG8,
                },
            ),
            (
                String::from("boss_final_missile1"),
                MonsterFrame {
                    frame: 57,
                    next: "boss_final_missile2",
                    operations: OPS_BOSS_FINAL_MISSILE1,
                },
            ),
            (
                String::from("boss_final_missile2"),
                MonsterFrame {
                    frame: 58,
                    next: "boss_final_missile3",
                    operations: OPS_BOSS_FINAL_MISSILE2,
                },
            ),
            (
                String::from("boss_final_missile3"),
                MonsterFrame {
                    frame: 59,
                    next: "boss_final_missile4",
                    operations: OPS_BOSS_FINAL_MISSILE3,
                },
            ),
            (
                String::from("boss_final_missile4"),
                MonsterFrame {
                    frame: 60,
                    next: "boss_final_missile5",
                    operations: OPS_BOSS_FINAL_MISSILE4,
                },
            ),
            (
                String::from("boss_final_missile5"),
                MonsterFrame {
                    frame: 61,
                    next: "boss_final_missile6",
                    operations: OPS_BOSS_FINAL_MISSILE5,
                },
            ),
            (
                String::from("boss_final_missile6"),
                MonsterFrame {
                    frame: 62,
                    next: "boss_final_missile7",
                    operations: OPS_BOSS_FINAL_MISSILE6,
                },
            ),
            (
                String::from("boss_final_missile7"),
                MonsterFrame {
                    frame: 63,
                    next: "boss_final_missile8",
                    operations: OPS_BOSS_FINAL_MISSILE7,
                },
            ),
            (
                String::from("boss_final_missile8"),
                MonsterFrame {
                    frame: 64,
                    next: "boss_final_missile9",
                    operations: OPS_BOSS_FINAL_MISSILE8,
                },
            ),
            (
                String::from("boss_final_missile9"),
                MonsterFrame {
                    frame: 65,
                    next: "boss_final_missile10",
                    operations: OPS_BOSS_FINAL_MISSILE9,
                },
            ),
            (
                String::from("boss_final_missile10"),
                MonsterFrame {
                    frame: 66,
                    next: "boss_final_missile11",
                    operations: OPS_BOSS_FINAL_MISSILE10,
                },
            ),
            (
                String::from("boss_final_missile11"),
                MonsterFrame {
                    frame: 67,
                    next: "boss_final_missile12",
                    operations: OPS_BOSS_FINAL_MISSILE11,
                },
            ),
            (
                String::from("boss_final_missile12"),
                MonsterFrame {
                    frame: 68,
                    next: "boss_final_missile13",
                    operations: OPS_BOSS_FINAL_MISSILE12,
                },
            ),
            (
                String::from("boss_final_missile13"),
                MonsterFrame {
                    frame: 69,
                    next: "boss_final_missile14",
                    operations: OPS_BOSS_FINAL_MISSILE13,
                },
            ),
            (
                String::from("boss_final_missile14"),
                MonsterFrame {
                    frame: 70,
                    next: "boss_final_missile15",
                    operations: OPS_BOSS_FINAL_MISSILE14,
                },
            ),
            (
                String::from("boss_final_missile15"),
                MonsterFrame {
                    frame: 71,
                    next: "boss_final_missile16",
                    operations: OPS_BOSS_FINAL_MISSILE15,
                },
            ),
            (
                String::from("boss_final_missile16"),
                MonsterFrame {
                    frame: 72,
                    next: "boss_final_missile17",
                    operations: OPS_BOSS_FINAL_MISSILE16,
                },
            ),
            (
                String::from("boss_final_missile17"),
                MonsterFrame {
                    frame: 73,
                    next: "boss_final_missile18",
                    operations: OPS_BOSS_FINAL_MISSILE17,
                },
            ),
            (
                String::from("boss_final_missile18"),
                MonsterFrame {
                    frame: 74,
                    next: "boss_final_missile19",
                    operations: OPS_BOSS_FINAL_MISSILE18,
                },
            ),
            (
                String::from("boss_final_missile19"),
                MonsterFrame {
                    frame: 75,
                    next: "boss_final_missile20",
                    operations: OPS_BOSS_FINAL_MISSILE19,
                },
            ),
            (
                String::from("boss_final_missile20"),
                MonsterFrame {
                    frame: 76,
                    next: "boss_final_missile21",
                    operations: OPS_BOSS_FINAL_MISSILE20,
                },
            ),
            (
                String::from("boss_final_missile21"),
                MonsterFrame {
                    frame: 77,
                    next: "boss_final_missile22",
                    operations: OPS_BOSS_FINAL_MISSILE21,
                },
            ),
            (
                String::from("boss_final_missile22"),
                MonsterFrame {
                    frame: 78,
                    next: "boss_final_missile23",
                    operations: OPS_BOSS_FINAL_MISSILE22,
                },
            ),
            (
                String::from("boss_final_missile23"),
                MonsterFrame {
                    frame: 79,
                    next: "boss_final_decide",
                    operations: OPS_BOSS_FINAL_MISSILE23,
                },
            ),
            (
                String::from("boss_final_shocka1"),
                MonsterFrame {
                    frame: 80,
                    next: "boss_final_shocka2",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shocka2"),
                MonsterFrame {
                    frame: 81,
                    next: "boss_final_shocka3",
                    operations: OPS_BOSS_FINAL_SHOCKA2,
                },
            ),
            (
                String::from("boss_final_shocka3"),
                MonsterFrame {
                    frame: 82,
                    next: "boss_final_shocka4",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shocka4"),
                MonsterFrame {
                    frame: 83,
                    next: "boss_final_shocka5",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shocka5"),
                MonsterFrame {
                    frame: 84,
                    next: "boss_final_shocka6",
                    operations: OPS_BOSS_FINAL_SHOCKA5,
                },
            ),
            (
                String::from("boss_final_shocka6"),
                MonsterFrame {
                    frame: 85,
                    next: "boss_final_shocka7",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shocka7"),
                MonsterFrame {
                    frame: 86,
                    next: "boss_final_shocka8",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shocka8"),
                MonsterFrame {
                    frame: 87,
                    next: "boss_final_shocka9",
                    operations: OPS_BOSS_FINAL_SHOCKA8,
                },
            ),
            (
                String::from("boss_final_shocka9"),
                MonsterFrame {
                    frame: 88,
                    next: "boss_final_shocka10",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shocka10"),
                MonsterFrame {
                    frame: 89,
                    next: "boss_final_missile1",
                    operations: OPS_BOSS_FINAL_SHOCKA10,
                },
            ),
            (
                String::from("boss_final_shockb1"),
                MonsterFrame {
                    frame: 90,
                    next: "boss_final_shockb2",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockb2"),
                MonsterFrame {
                    frame: 91,
                    next: "boss_final_shockb3",
                    operations: OPS_BOSS_FINAL_SHOCKB2,
                },
            ),
            (
                String::from("boss_final_shockb3"),
                MonsterFrame {
                    frame: 92,
                    next: "boss_final_shockb4",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockb4"),
                MonsterFrame {
                    frame: 93,
                    next: "boss_final_shockb5",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockb5"),
                MonsterFrame {
                    frame: 94,
                    next: "boss_final_shockb6",
                    operations: OPS_BOSS_FINAL_SHOCKB5,
                },
            ),
            (
                String::from("boss_final_shockb6"),
                MonsterFrame {
                    frame: 95,
                    next: "boss_final_shockb7",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockb7"),
                MonsterFrame {
                    frame: 90,
                    next: "boss_final_shockb8",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockb8"),
                MonsterFrame {
                    frame: 91,
                    next: "boss_final_shockb9",
                    operations: OPS_BOSS_FINAL_SHOCKB8,
                },
            ),
            (
                String::from("boss_final_shockb9"),
                MonsterFrame {
                    frame: 92,
                    next: "boss_final_shockb10",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockb10"),
                MonsterFrame {
                    frame: 93,
                    next: "boss_final_missile1",
                    operations: OPS_BOSS_FINAL_SHOCKB10,
                },
            ),
            (
                String::from("boss_final_shockc1"),
                MonsterFrame {
                    frame: 96,
                    next: "boss_final_shockc2",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockc2"),
                MonsterFrame {
                    frame: 97,
                    next: "boss_final_shockc3",
                    operations: OPS_BOSS_FINAL_SHOCKC2,
                },
            ),
            (
                String::from("boss_final_shockc3"),
                MonsterFrame {
                    frame: 98,
                    next: "boss_final_shockc4",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockc4"),
                MonsterFrame {
                    frame: 99,
                    next: "boss_final_shockc5",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockc5"),
                MonsterFrame {
                    frame: 100,
                    next: "boss_final_shockc6",
                    operations: OPS_BOSS_FINAL_SHOCKC5,
                },
            ),
            (
                String::from("boss_final_shockc6"),
                MonsterFrame {
                    frame: 101,
                    next: "boss_final_shockc7",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockc7"),
                MonsterFrame {
                    frame: 102,
                    next: "boss_final_shockc8",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockc8"),
                MonsterFrame {
                    frame: 103,
                    next: "boss_final_shockc9",
                    operations: OPS_BOSS_FINAL_SHOCKC8,
                },
            ),
            (
                String::from("boss_final_shockc9"),
                MonsterFrame {
                    frame: 104,
                    next: "boss_final_shockc10",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_shockc10"),
                MonsterFrame {
                    frame: 105,
                    next: "boss_final_death1",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death1"),
                MonsterFrame {
                    frame: 48,
                    next: "boss_final_death2",
                    operations: OPS_BOSS_FINAL_DEATH1,
                },
            ),
            (
                String::from("boss_final_death2"),
                MonsterFrame {
                    frame: 49,
                    next: "boss_final_death3",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death3"),
                MonsterFrame {
                    frame: 50,
                    next: "boss_final_death4",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death4"),
                MonsterFrame {
                    frame: 51,
                    next: "boss_final_death5",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death5"),
                MonsterFrame {
                    frame: 52,
                    next: "boss_final_death6",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death6"),
                MonsterFrame {
                    frame: 53,
                    next: "boss_final_death7",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death7"),
                MonsterFrame {
                    frame: 54,
                    next: "boss_final_death8",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death8"),
                MonsterFrame {
                    frame: 55,
                    next: "boss_final_death9",
                    operations: &[],
                },
            ),
            (
                String::from("boss_final_death9"),
                MonsterFrame {
                    frame: 56,
                    next: "boss_final_death10",
                    operations: OPS_BOSS_FINAL_DEATH9,
                },
            ),
            (
                String::from("boss_final_death10"),
                MonsterFrame {
                    frame: 56,
                    next: "boss_final_death10",
                    operations: OPS_BOSS_FINAL_DEATH10,
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
        assert_eq!(frames().len(), 119);
        for (name, frame) in frames() {
            // `boss_final_decide` is action-routed (see `super::super::r#final`).
            if frame.next == "boss_final_decide" {
                continue;
            }
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
