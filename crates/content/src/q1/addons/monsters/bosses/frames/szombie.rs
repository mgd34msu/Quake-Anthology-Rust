//! Q1 Shub zombie frames (`src/content/q1/addons/monsters/bosses/frames/szombie.ts`).
//!
//! mg3_shub_zombie.qc source frame declarations. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_SZOMBIE_STAND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_STAND15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_stand1",
}];
static OPS_SZOMBIE_WALK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk2",
}];
static OPS_SZOMBIE_WALK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk3",
}];
static OPS_SZOMBIE_WALK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk2",
}];
static OPS_SZOMBIE_WALK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk5",
}];
static OPS_SZOMBIE_WALK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk2",
}];
static OPS_SZOMBIE_WALK12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk2",
}];
static OPS_SZOMBIE_WALK13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk5",
}];
static OPS_SZOMBIE_WALK14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk1",
}];
static OPS_SZOMBIE_WALK19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_walk19",
}];
static OPS_SZOMBIE_RUN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run1",
}];
static OPS_SZOMBIE_RUN2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run2",
}];
static OPS_SZOMBIE_RUN3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run3",
}];
static OPS_SZOMBIE_RUN4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run2",
}];
static OPS_SZOMBIE_RUN5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run5",
}];
static OPS_SZOMBIE_RUN6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run6",
}];
static OPS_SZOMBIE_RUN7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run7",
}];
static OPS_SZOMBIE_RUN8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run7",
}];
static OPS_SZOMBIE_RUN9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run5",
}];
static OPS_SZOMBIE_RUN10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run3",
}];
static OPS_SZOMBIE_RUN11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run3",
}];
static OPS_SZOMBIE_RUN12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run3",
}];
static OPS_SZOMBIE_RUN13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run5",
}];
static OPS_SZOMBIE_RUN14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run7",
}];
static OPS_SZOMBIE_RUN15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run15",
}];
static OPS_SZOMBIE_RUN16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run16",
}];
static OPS_SZOMBIE_RUN17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run6",
}];
static OPS_SZOMBIE_RUN18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_run18",
}];
static OPS_SZOMBIE_ATTA1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTA13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta13",
}];
static OPS_SZOMBIE_ATTB1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTB14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_attb14",
}];
static OPS_SZOMBIE_ATTC1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_atta1",
}];
static OPS_SZOMBIE_ATTC12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_attc12",
}];
static OPS_SZOMBIE_PAINA1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina1",
}];
static OPS_SZOMBIE_PAINA2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina2",
}];
static OPS_SZOMBIE_PAINA3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina3",
}];
static OPS_SZOMBIE_PAINA4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINA5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina5",
}];
static OPS_SZOMBIE_PAINA6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINB1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb1",
}];
static OPS_SZOMBIE_PAINB2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb2",
}];
static OPS_SZOMBIE_PAINB3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb3",
}];
static OPS_SZOMBIE_PAINB4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb4",
}];
static OPS_SZOMBIE_PAINB5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb2",
}];
static OPS_SZOMBIE_PAINB9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb9",
}];
static OPS_SZOMBIE_PAINB25: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina3",
}];
static OPS_SZOMBIE_PAINC1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb1",
}];
static OPS_SZOMBIE_PAINC3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina5",
}];
static OPS_SZOMBIE_PAINC4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINC11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina3",
}];
static OPS_SZOMBIE_PAINC12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina3",
}];
static OPS_SZOMBIE_PAIND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina1",
}];
static OPS_SZOMBIE_PAIND9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paine1",
}];
static OPS_SZOMBIE_PAINE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb3",
}];
static OPS_SZOMBIE_PAINE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paine3",
}];
static OPS_SZOMBIE_PAINE4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina5",
}];
static OPS_SZOMBIE_PAINE5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINE6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb2",
}];
static OPS_SZOMBIE_PAINE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINE8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];
static OPS_SZOMBIE_PAINE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_painb2",
}];
static OPS_SZOMBIE_PAINE10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paine10",
}];
static OPS_SZOMBIE_PAINE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paine11",
}];
static OPS_SZOMBIE_PAINE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paine12",
}];
static OPS_SZOMBIE_PAINE25: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paine25",
}];
static OPS_SZOMBIE_PAINE26: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina2",
}];
static OPS_SZOMBIE_PAINE27: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina3",
}];
static OPS_SZOMBIE_PAINE28: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "szombie:szombie_paina4",
}];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("szombie_stand1"),
                MonsterFrame {
                    frame: 0,
                    next: "szombie_stand2",
                    operations: OPS_SZOMBIE_STAND1,
                },
            ),
            (
                String::from("szombie_stand2"),
                MonsterFrame {
                    frame: 1,
                    next: "szombie_stand3",
                    operations: OPS_SZOMBIE_STAND2,
                },
            ),
            (
                String::from("szombie_stand3"),
                MonsterFrame {
                    frame: 2,
                    next: "szombie_stand4",
                    operations: OPS_SZOMBIE_STAND3,
                },
            ),
            (
                String::from("szombie_stand4"),
                MonsterFrame {
                    frame: 3,
                    next: "szombie_stand5",
                    operations: OPS_SZOMBIE_STAND4,
                },
            ),
            (
                String::from("szombie_stand5"),
                MonsterFrame {
                    frame: 4,
                    next: "szombie_stand6",
                    operations: OPS_SZOMBIE_STAND5,
                },
            ),
            (
                String::from("szombie_stand6"),
                MonsterFrame {
                    frame: 5,
                    next: "szombie_stand7",
                    operations: OPS_SZOMBIE_STAND6,
                },
            ),
            (
                String::from("szombie_stand7"),
                MonsterFrame {
                    frame: 6,
                    next: "szombie_stand8",
                    operations: OPS_SZOMBIE_STAND7,
                },
            ),
            (
                String::from("szombie_stand8"),
                MonsterFrame {
                    frame: 7,
                    next: "szombie_stand9",
                    operations: OPS_SZOMBIE_STAND8,
                },
            ),
            (
                String::from("szombie_stand9"),
                MonsterFrame {
                    frame: 8,
                    next: "szombie_stand10",
                    operations: OPS_SZOMBIE_STAND9,
                },
            ),
            (
                String::from("szombie_stand10"),
                MonsterFrame {
                    frame: 9,
                    next: "szombie_stand11",
                    operations: OPS_SZOMBIE_STAND10,
                },
            ),
            (
                String::from("szombie_stand11"),
                MonsterFrame {
                    frame: 10,
                    next: "szombie_stand12",
                    operations: OPS_SZOMBIE_STAND11,
                },
            ),
            (
                String::from("szombie_stand12"),
                MonsterFrame {
                    frame: 11,
                    next: "szombie_stand13",
                    operations: OPS_SZOMBIE_STAND12,
                },
            ),
            (
                String::from("szombie_stand13"),
                MonsterFrame {
                    frame: 12,
                    next: "szombie_stand14",
                    operations: OPS_SZOMBIE_STAND13,
                },
            ),
            (
                String::from("szombie_stand14"),
                MonsterFrame {
                    frame: 13,
                    next: "szombie_stand15",
                    operations: OPS_SZOMBIE_STAND14,
                },
            ),
            (
                String::from("szombie_stand15"),
                MonsterFrame {
                    frame: 14,
                    next: "szombie_stand1",
                    operations: OPS_SZOMBIE_STAND15,
                },
            ),
            (
                String::from("szombie_hang1"),
                MonsterFrame {
                    frame: 162,
                    next: "szombie_hang1",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_walk1"),
                MonsterFrame {
                    frame: 15,
                    next: "szombie_walk2",
                    operations: OPS_SZOMBIE_WALK1,
                },
            ),
            (
                String::from("szombie_walk2"),
                MonsterFrame {
                    frame: 16,
                    next: "szombie_walk3",
                    operations: OPS_SZOMBIE_WALK2,
                },
            ),
            (
                String::from("szombie_walk3"),
                MonsterFrame {
                    frame: 17,
                    next: "szombie_walk4",
                    operations: OPS_SZOMBIE_WALK3,
                },
            ),
            (
                String::from("szombie_walk4"),
                MonsterFrame {
                    frame: 18,
                    next: "szombie_walk5",
                    operations: OPS_SZOMBIE_WALK4,
                },
            ),
            (
                String::from("szombie_walk5"),
                MonsterFrame {
                    frame: 19,
                    next: "szombie_walk6",
                    operations: OPS_SZOMBIE_WALK5,
                },
            ),
            (
                String::from("szombie_walk6"),
                MonsterFrame {
                    frame: 20,
                    next: "szombie_walk7",
                    operations: OPS_SZOMBIE_WALK6,
                },
            ),
            (
                String::from("szombie_walk7"),
                MonsterFrame {
                    frame: 21,
                    next: "szombie_walk8",
                    operations: OPS_SZOMBIE_WALK7,
                },
            ),
            (
                String::from("szombie_walk8"),
                MonsterFrame {
                    frame: 22,
                    next: "szombie_walk9",
                    operations: OPS_SZOMBIE_WALK8,
                },
            ),
            (
                String::from("szombie_walk9"),
                MonsterFrame {
                    frame: 23,
                    next: "szombie_walk10",
                    operations: OPS_SZOMBIE_WALK9,
                },
            ),
            (
                String::from("szombie_walk10"),
                MonsterFrame {
                    frame: 24,
                    next: "szombie_walk11",
                    operations: OPS_SZOMBIE_WALK10,
                },
            ),
            (
                String::from("szombie_walk11"),
                MonsterFrame {
                    frame: 25,
                    next: "szombie_walk12",
                    operations: OPS_SZOMBIE_WALK11,
                },
            ),
            (
                String::from("szombie_walk12"),
                MonsterFrame {
                    frame: 26,
                    next: "szombie_walk13",
                    operations: OPS_SZOMBIE_WALK12,
                },
            ),
            (
                String::from("szombie_walk13"),
                MonsterFrame {
                    frame: 27,
                    next: "szombie_walk14",
                    operations: OPS_SZOMBIE_WALK13,
                },
            ),
            (
                String::from("szombie_walk14"),
                MonsterFrame {
                    frame: 28,
                    next: "szombie_walk15",
                    operations: OPS_SZOMBIE_WALK14,
                },
            ),
            (
                String::from("szombie_walk15"),
                MonsterFrame {
                    frame: 29,
                    next: "szombie_walk16",
                    operations: OPS_SZOMBIE_WALK15,
                },
            ),
            (
                String::from("szombie_walk16"),
                MonsterFrame {
                    frame: 30,
                    next: "szombie_walk17",
                    operations: OPS_SZOMBIE_WALK16,
                },
            ),
            (
                String::from("szombie_walk17"),
                MonsterFrame {
                    frame: 31,
                    next: "szombie_walk18",
                    operations: OPS_SZOMBIE_WALK17,
                },
            ),
            (
                String::from("szombie_walk18"),
                MonsterFrame {
                    frame: 32,
                    next: "szombie_walk19",
                    operations: OPS_SZOMBIE_WALK18,
                },
            ),
            (
                String::from("szombie_walk19"),
                MonsterFrame {
                    frame: 33,
                    next: "szombie_walk1",
                    operations: OPS_SZOMBIE_WALK19,
                },
            ),
            (
                String::from("szombie_run1"),
                MonsterFrame {
                    frame: 34,
                    next: "szombie_run2",
                    operations: OPS_SZOMBIE_RUN1,
                },
            ),
            (
                String::from("szombie_run2"),
                MonsterFrame {
                    frame: 35,
                    next: "szombie_run3",
                    operations: OPS_SZOMBIE_RUN2,
                },
            ),
            (
                String::from("szombie_run3"),
                MonsterFrame {
                    frame: 36,
                    next: "szombie_run4",
                    operations: OPS_SZOMBIE_RUN3,
                },
            ),
            (
                String::from("szombie_run4"),
                MonsterFrame {
                    frame: 37,
                    next: "szombie_run5",
                    operations: OPS_SZOMBIE_RUN4,
                },
            ),
            (
                String::from("szombie_run5"),
                MonsterFrame {
                    frame: 38,
                    next: "szombie_run6",
                    operations: OPS_SZOMBIE_RUN5,
                },
            ),
            (
                String::from("szombie_run6"),
                MonsterFrame {
                    frame: 39,
                    next: "szombie_run7",
                    operations: OPS_SZOMBIE_RUN6,
                },
            ),
            (
                String::from("szombie_run7"),
                MonsterFrame {
                    frame: 40,
                    next: "szombie_run8",
                    operations: OPS_SZOMBIE_RUN7,
                },
            ),
            (
                String::from("szombie_run8"),
                MonsterFrame {
                    frame: 41,
                    next: "szombie_run9",
                    operations: OPS_SZOMBIE_RUN8,
                },
            ),
            (
                String::from("szombie_run9"),
                MonsterFrame {
                    frame: 42,
                    next: "szombie_run10",
                    operations: OPS_SZOMBIE_RUN9,
                },
            ),
            (
                String::from("szombie_run10"),
                MonsterFrame {
                    frame: 43,
                    next: "szombie_run11",
                    operations: OPS_SZOMBIE_RUN10,
                },
            ),
            (
                String::from("szombie_run11"),
                MonsterFrame {
                    frame: 44,
                    next: "szombie_run12",
                    operations: OPS_SZOMBIE_RUN11,
                },
            ),
            (
                String::from("szombie_run12"),
                MonsterFrame {
                    frame: 45,
                    next: "szombie_run13",
                    operations: OPS_SZOMBIE_RUN12,
                },
            ),
            (
                String::from("szombie_run13"),
                MonsterFrame {
                    frame: 46,
                    next: "szombie_run14",
                    operations: OPS_SZOMBIE_RUN13,
                },
            ),
            (
                String::from("szombie_run14"),
                MonsterFrame {
                    frame: 47,
                    next: "szombie_run15",
                    operations: OPS_SZOMBIE_RUN14,
                },
            ),
            (
                String::from("szombie_run15"),
                MonsterFrame {
                    frame: 48,
                    next: "szombie_run16",
                    operations: OPS_SZOMBIE_RUN15,
                },
            ),
            (
                String::from("szombie_run16"),
                MonsterFrame {
                    frame: 49,
                    next: "szombie_run17",
                    operations: OPS_SZOMBIE_RUN16,
                },
            ),
            (
                String::from("szombie_run17"),
                MonsterFrame {
                    frame: 50,
                    next: "szombie_run18",
                    operations: OPS_SZOMBIE_RUN17,
                },
            ),
            (
                String::from("szombie_run18"),
                MonsterFrame {
                    frame: 51,
                    next: "szombie_run1",
                    operations: OPS_SZOMBIE_RUN18,
                },
            ),
            (
                String::from("szombie_atta1"),
                MonsterFrame {
                    frame: 52,
                    next: "szombie_atta2",
                    operations: OPS_SZOMBIE_ATTA1,
                },
            ),
            (
                String::from("szombie_atta2"),
                MonsterFrame {
                    frame: 53,
                    next: "szombie_atta3",
                    operations: OPS_SZOMBIE_ATTA2,
                },
            ),
            (
                String::from("szombie_atta3"),
                MonsterFrame {
                    frame: 54,
                    next: "szombie_atta4",
                    operations: OPS_SZOMBIE_ATTA3,
                },
            ),
            (
                String::from("szombie_atta4"),
                MonsterFrame {
                    frame: 55,
                    next: "szombie_atta5",
                    operations: OPS_SZOMBIE_ATTA4,
                },
            ),
            (
                String::from("szombie_atta5"),
                MonsterFrame {
                    frame: 56,
                    next: "szombie_atta6",
                    operations: OPS_SZOMBIE_ATTA5,
                },
            ),
            (
                String::from("szombie_atta6"),
                MonsterFrame {
                    frame: 57,
                    next: "szombie_atta7",
                    operations: OPS_SZOMBIE_ATTA6,
                },
            ),
            (
                String::from("szombie_atta7"),
                MonsterFrame {
                    frame: 58,
                    next: "szombie_atta8",
                    operations: OPS_SZOMBIE_ATTA7,
                },
            ),
            (
                String::from("szombie_atta8"),
                MonsterFrame {
                    frame: 59,
                    next: "szombie_atta9",
                    operations: OPS_SZOMBIE_ATTA8,
                },
            ),
            (
                String::from("szombie_atta9"),
                MonsterFrame {
                    frame: 60,
                    next: "szombie_atta10",
                    operations: OPS_SZOMBIE_ATTA9,
                },
            ),
            (
                String::from("szombie_atta10"),
                MonsterFrame {
                    frame: 61,
                    next: "szombie_atta11",
                    operations: OPS_SZOMBIE_ATTA10,
                },
            ),
            (
                String::from("szombie_atta11"),
                MonsterFrame {
                    frame: 62,
                    next: "szombie_atta12",
                    operations: OPS_SZOMBIE_ATTA11,
                },
            ),
            (
                String::from("szombie_atta12"),
                MonsterFrame {
                    frame: 63,
                    next: "szombie_atta13",
                    operations: OPS_SZOMBIE_ATTA12,
                },
            ),
            (
                String::from("szombie_atta13"),
                MonsterFrame {
                    frame: 64,
                    next: "szombie_run1",
                    operations: OPS_SZOMBIE_ATTA13,
                },
            ),
            (
                String::from("szombie_attb1"),
                MonsterFrame {
                    frame: 65,
                    next: "szombie_attb2",
                    operations: OPS_SZOMBIE_ATTB1,
                },
            ),
            (
                String::from("szombie_attb2"),
                MonsterFrame {
                    frame: 66,
                    next: "szombie_attb3",
                    operations: OPS_SZOMBIE_ATTB2,
                },
            ),
            (
                String::from("szombie_attb3"),
                MonsterFrame {
                    frame: 67,
                    next: "szombie_attb4",
                    operations: OPS_SZOMBIE_ATTB3,
                },
            ),
            (
                String::from("szombie_attb4"),
                MonsterFrame {
                    frame: 68,
                    next: "szombie_attb5",
                    operations: OPS_SZOMBIE_ATTB4,
                },
            ),
            (
                String::from("szombie_attb5"),
                MonsterFrame {
                    frame: 69,
                    next: "szombie_attb6",
                    operations: OPS_SZOMBIE_ATTB5,
                },
            ),
            (
                String::from("szombie_attb6"),
                MonsterFrame {
                    frame: 70,
                    next: "szombie_attb7",
                    operations: OPS_SZOMBIE_ATTB6,
                },
            ),
            (
                String::from("szombie_attb7"),
                MonsterFrame {
                    frame: 71,
                    next: "szombie_attb8",
                    operations: OPS_SZOMBIE_ATTB7,
                },
            ),
            (
                String::from("szombie_attb8"),
                MonsterFrame {
                    frame: 72,
                    next: "szombie_attb9",
                    operations: OPS_SZOMBIE_ATTB8,
                },
            ),
            (
                String::from("szombie_attb9"),
                MonsterFrame {
                    frame: 73,
                    next: "szombie_attb10",
                    operations: OPS_SZOMBIE_ATTB9,
                },
            ),
            (
                String::from("szombie_attb10"),
                MonsterFrame {
                    frame: 74,
                    next: "szombie_attb11",
                    operations: OPS_SZOMBIE_ATTB10,
                },
            ),
            (
                String::from("szombie_attb11"),
                MonsterFrame {
                    frame: 75,
                    next: "szombie_attb12",
                    operations: OPS_SZOMBIE_ATTB11,
                },
            ),
            (
                String::from("szombie_attb12"),
                MonsterFrame {
                    frame: 76,
                    next: "szombie_attb13",
                    operations: OPS_SZOMBIE_ATTB12,
                },
            ),
            (
                String::from("szombie_attb13"),
                MonsterFrame {
                    frame: 77,
                    next: "szombie_attb14",
                    operations: OPS_SZOMBIE_ATTB13,
                },
            ),
            (
                String::from("szombie_attb14"),
                MonsterFrame {
                    frame: 77,
                    next: "szombie_run1",
                    operations: OPS_SZOMBIE_ATTB14,
                },
            ),
            (
                String::from("szombie_attc1"),
                MonsterFrame {
                    frame: 79,
                    next: "szombie_attc2",
                    operations: OPS_SZOMBIE_ATTC1,
                },
            ),
            (
                String::from("szombie_attc2"),
                MonsterFrame {
                    frame: 80,
                    next: "szombie_attc3",
                    operations: OPS_SZOMBIE_ATTC2,
                },
            ),
            (
                String::from("szombie_attc3"),
                MonsterFrame {
                    frame: 81,
                    next: "szombie_attc4",
                    operations: OPS_SZOMBIE_ATTC3,
                },
            ),
            (
                String::from("szombie_attc4"),
                MonsterFrame {
                    frame: 82,
                    next: "szombie_attc5",
                    operations: OPS_SZOMBIE_ATTC4,
                },
            ),
            (
                String::from("szombie_attc5"),
                MonsterFrame {
                    frame: 83,
                    next: "szombie_attc6",
                    operations: OPS_SZOMBIE_ATTC5,
                },
            ),
            (
                String::from("szombie_attc6"),
                MonsterFrame {
                    frame: 84,
                    next: "szombie_attc7",
                    operations: OPS_SZOMBIE_ATTC6,
                },
            ),
            (
                String::from("szombie_attc7"),
                MonsterFrame {
                    frame: 85,
                    next: "szombie_attc8",
                    operations: OPS_SZOMBIE_ATTC7,
                },
            ),
            (
                String::from("szombie_attc8"),
                MonsterFrame {
                    frame: 86,
                    next: "szombie_attc9",
                    operations: OPS_SZOMBIE_ATTC8,
                },
            ),
            (
                String::from("szombie_attc9"),
                MonsterFrame {
                    frame: 87,
                    next: "szombie_attc10",
                    operations: OPS_SZOMBIE_ATTC9,
                },
            ),
            (
                String::from("szombie_attc10"),
                MonsterFrame {
                    frame: 88,
                    next: "szombie_attc11",
                    operations: OPS_SZOMBIE_ATTC10,
                },
            ),
            (
                String::from("szombie_attc11"),
                MonsterFrame {
                    frame: 89,
                    next: "szombie_attc12",
                    operations: OPS_SZOMBIE_ATTC11,
                },
            ),
            (
                String::from("szombie_attc12"),
                MonsterFrame {
                    frame: 90,
                    next: "szombie_run1",
                    operations: OPS_SZOMBIE_ATTC12,
                },
            ),
            (
                String::from("szombie_paina1"),
                MonsterFrame {
                    frame: 91,
                    next: "szombie_paina2",
                    operations: OPS_SZOMBIE_PAINA1,
                },
            ),
            (
                String::from("szombie_paina2"),
                MonsterFrame {
                    frame: 92,
                    next: "szombie_paina3",
                    operations: OPS_SZOMBIE_PAINA2,
                },
            ),
            (
                String::from("szombie_paina3"),
                MonsterFrame {
                    frame: 93,
                    next: "szombie_paina4",
                    operations: OPS_SZOMBIE_PAINA3,
                },
            ),
            (
                String::from("szombie_paina4"),
                MonsterFrame {
                    frame: 94,
                    next: "szombie_paina5",
                    operations: OPS_SZOMBIE_PAINA4,
                },
            ),
            (
                String::from("szombie_paina5"),
                MonsterFrame {
                    frame: 95,
                    next: "szombie_paina6",
                    operations: OPS_SZOMBIE_PAINA5,
                },
            ),
            (
                String::from("szombie_paina6"),
                MonsterFrame {
                    frame: 96,
                    next: "szombie_paina7",
                    operations: OPS_SZOMBIE_PAINA6,
                },
            ),
            (
                String::from("szombie_paina7"),
                MonsterFrame {
                    frame: 97,
                    next: "szombie_paina8",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paina8"),
                MonsterFrame {
                    frame: 98,
                    next: "szombie_paina9",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paina9"),
                MonsterFrame {
                    frame: 99,
                    next: "szombie_paina10",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paina10"),
                MonsterFrame {
                    frame: 100,
                    next: "szombie_paina11",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paina11"),
                MonsterFrame {
                    frame: 101,
                    next: "szombie_paina12",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paina12"),
                MonsterFrame {
                    frame: 102,
                    next: "szombie_run1",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb1"),
                MonsterFrame {
                    frame: 103,
                    next: "szombie_painb2",
                    operations: OPS_SZOMBIE_PAINB1,
                },
            ),
            (
                String::from("szombie_painb2"),
                MonsterFrame {
                    frame: 104,
                    next: "szombie_painb3",
                    operations: OPS_SZOMBIE_PAINB2,
                },
            ),
            (
                String::from("szombie_painb3"),
                MonsterFrame {
                    frame: 105,
                    next: "szombie_painb4",
                    operations: OPS_SZOMBIE_PAINB3,
                },
            ),
            (
                String::from("szombie_painb4"),
                MonsterFrame {
                    frame: 106,
                    next: "szombie_painb5",
                    operations: OPS_SZOMBIE_PAINB4,
                },
            ),
            (
                String::from("szombie_painb5"),
                MonsterFrame {
                    frame: 107,
                    next: "szombie_painb6",
                    operations: OPS_SZOMBIE_PAINB5,
                },
            ),
            (
                String::from("szombie_painb6"),
                MonsterFrame {
                    frame: 108,
                    next: "szombie_painb7",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb7"),
                MonsterFrame {
                    frame: 109,
                    next: "szombie_painb8",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb8"),
                MonsterFrame {
                    frame: 110,
                    next: "szombie_painb9",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb9"),
                MonsterFrame {
                    frame: 111,
                    next: "szombie_painb10",
                    operations: OPS_SZOMBIE_PAINB9,
                },
            ),
            (
                String::from("szombie_painb10"),
                MonsterFrame {
                    frame: 112,
                    next: "szombie_painb11",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb11"),
                MonsterFrame {
                    frame: 113,
                    next: "szombie_painb12",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb12"),
                MonsterFrame {
                    frame: 114,
                    next: "szombie_painb13",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb13"),
                MonsterFrame {
                    frame: 115,
                    next: "szombie_painb14",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb14"),
                MonsterFrame {
                    frame: 116,
                    next: "szombie_painb15",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb15"),
                MonsterFrame {
                    frame: 117,
                    next: "szombie_painb16",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb16"),
                MonsterFrame {
                    frame: 118,
                    next: "szombie_painb17",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb17"),
                MonsterFrame {
                    frame: 119,
                    next: "szombie_painb18",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb18"),
                MonsterFrame {
                    frame: 120,
                    next: "szombie_painb19",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb19"),
                MonsterFrame {
                    frame: 121,
                    next: "szombie_painb20",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb20"),
                MonsterFrame {
                    frame: 122,
                    next: "szombie_painb21",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb21"),
                MonsterFrame {
                    frame: 123,
                    next: "szombie_painb22",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb22"),
                MonsterFrame {
                    frame: 124,
                    next: "szombie_painb23",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb23"),
                MonsterFrame {
                    frame: 125,
                    next: "szombie_painb24",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb24"),
                MonsterFrame {
                    frame: 126,
                    next: "szombie_painb25",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb25"),
                MonsterFrame {
                    frame: 127,
                    next: "szombie_painb26",
                    operations: OPS_SZOMBIE_PAINB25,
                },
            ),
            (
                String::from("szombie_painb26"),
                MonsterFrame {
                    frame: 128,
                    next: "szombie_painb27",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb27"),
                MonsterFrame {
                    frame: 129,
                    next: "szombie_painb28",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painb28"),
                MonsterFrame {
                    frame: 130,
                    next: "szombie_run1",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc1"),
                MonsterFrame {
                    frame: 131,
                    next: "szombie_painc2",
                    operations: OPS_SZOMBIE_PAINC1,
                },
            ),
            (
                String::from("szombie_painc2"),
                MonsterFrame {
                    frame: 132,
                    next: "szombie_painc3",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc3"),
                MonsterFrame {
                    frame: 133,
                    next: "szombie_painc4",
                    operations: OPS_SZOMBIE_PAINC3,
                },
            ),
            (
                String::from("szombie_painc4"),
                MonsterFrame {
                    frame: 134,
                    next: "szombie_painc5",
                    operations: OPS_SZOMBIE_PAINC4,
                },
            ),
            (
                String::from("szombie_painc5"),
                MonsterFrame {
                    frame: 135,
                    next: "szombie_painc6",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc6"),
                MonsterFrame {
                    frame: 136,
                    next: "szombie_painc7",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc7"),
                MonsterFrame {
                    frame: 137,
                    next: "szombie_painc8",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc8"),
                MonsterFrame {
                    frame: 138,
                    next: "szombie_painc9",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc9"),
                MonsterFrame {
                    frame: 139,
                    next: "szombie_painc10",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc10"),
                MonsterFrame {
                    frame: 140,
                    next: "szombie_painc11",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc11"),
                MonsterFrame {
                    frame: 141,
                    next: "szombie_painc12",
                    operations: OPS_SZOMBIE_PAINC11,
                },
            ),
            (
                String::from("szombie_painc12"),
                MonsterFrame {
                    frame: 142,
                    next: "szombie_painc13",
                    operations: OPS_SZOMBIE_PAINC12,
                },
            ),
            (
                String::from("szombie_painc13"),
                MonsterFrame {
                    frame: 143,
                    next: "szombie_painc14",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc14"),
                MonsterFrame {
                    frame: 144,
                    next: "szombie_painc15",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc15"),
                MonsterFrame {
                    frame: 145,
                    next: "szombie_painc16",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc16"),
                MonsterFrame {
                    frame: 146,
                    next: "szombie_painc17",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc17"),
                MonsterFrame {
                    frame: 147,
                    next: "szombie_painc18",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_painc18"),
                MonsterFrame {
                    frame: 148,
                    next: "szombie_run1",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind1"),
                MonsterFrame {
                    frame: 149,
                    next: "szombie_paind2",
                    operations: OPS_SZOMBIE_PAIND1,
                },
            ),
            (
                String::from("szombie_paind2"),
                MonsterFrame {
                    frame: 150,
                    next: "szombie_paind3",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind3"),
                MonsterFrame {
                    frame: 151,
                    next: "szombie_paind4",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind4"),
                MonsterFrame {
                    frame: 152,
                    next: "szombie_paind5",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind5"),
                MonsterFrame {
                    frame: 153,
                    next: "szombie_paind6",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind6"),
                MonsterFrame {
                    frame: 154,
                    next: "szombie_paind7",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind7"),
                MonsterFrame {
                    frame: 155,
                    next: "szombie_paind8",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind8"),
                MonsterFrame {
                    frame: 156,
                    next: "szombie_paind9",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind9"),
                MonsterFrame {
                    frame: 157,
                    next: "szombie_paind10",
                    operations: OPS_SZOMBIE_PAIND9,
                },
            ),
            (
                String::from("szombie_paind10"),
                MonsterFrame {
                    frame: 158,
                    next: "szombie_paind11",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind11"),
                MonsterFrame {
                    frame: 159,
                    next: "szombie_paind12",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind12"),
                MonsterFrame {
                    frame: 160,
                    next: "szombie_paind13",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paind13"),
                MonsterFrame {
                    frame: 161,
                    next: "szombie_run1",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine1"),
                MonsterFrame {
                    frame: 162,
                    next: "szombie_paine2",
                    operations: OPS_SZOMBIE_PAINE1,
                },
            ),
            (
                String::from("szombie_paine2"),
                MonsterFrame {
                    frame: 163,
                    next: "szombie_paine3",
                    operations: OPS_SZOMBIE_PAINE2,
                },
            ),
            (
                String::from("szombie_paine3"),
                MonsterFrame {
                    frame: 164,
                    next: "szombie_paine4",
                    operations: OPS_SZOMBIE_PAINE3,
                },
            ),
            (
                String::from("szombie_paine4"),
                MonsterFrame {
                    frame: 165,
                    next: "szombie_paine5",
                    operations: OPS_SZOMBIE_PAINE4,
                },
            ),
            (
                String::from("szombie_paine5"),
                MonsterFrame {
                    frame: 166,
                    next: "szombie_paine6",
                    operations: OPS_SZOMBIE_PAINE5,
                },
            ),
            (
                String::from("szombie_paine6"),
                MonsterFrame {
                    frame: 167,
                    next: "szombie_paine7",
                    operations: OPS_SZOMBIE_PAINE6,
                },
            ),
            (
                String::from("szombie_paine7"),
                MonsterFrame {
                    frame: 168,
                    next: "szombie_paine8",
                    operations: OPS_SZOMBIE_PAINE7,
                },
            ),
            (
                String::from("szombie_paine8"),
                MonsterFrame {
                    frame: 169,
                    next: "szombie_paine9",
                    operations: OPS_SZOMBIE_PAINE8,
                },
            ),
            (
                String::from("szombie_paine9"),
                MonsterFrame {
                    frame: 170,
                    next: "szombie_paine10",
                    operations: OPS_SZOMBIE_PAINE9,
                },
            ),
            (
                String::from("szombie_paine10"),
                MonsterFrame {
                    frame: 171,
                    next: "szombie_paine11",
                    operations: OPS_SZOMBIE_PAINE10,
                },
            ),
            (
                String::from("szombie_paine11"),
                MonsterFrame {
                    frame: 172,
                    next: "szombie_paine12",
                    operations: OPS_SZOMBIE_PAINE11,
                },
            ),
            (
                String::from("szombie_paine12"),
                MonsterFrame {
                    frame: 173,
                    next: "szombie_paine13",
                    operations: OPS_SZOMBIE_PAINE12,
                },
            ),
            (
                String::from("szombie_paine13"),
                MonsterFrame {
                    frame: 174,
                    next: "szombie_paine14",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine14"),
                MonsterFrame {
                    frame: 175,
                    next: "szombie_paine15",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine15"),
                MonsterFrame {
                    frame: 176,
                    next: "szombie_paine16",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine16"),
                MonsterFrame {
                    frame: 177,
                    next: "szombie_paine17",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine17"),
                MonsterFrame {
                    frame: 178,
                    next: "szombie_paine18",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine18"),
                MonsterFrame {
                    frame: 179,
                    next: "szombie_paine19",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine19"),
                MonsterFrame {
                    frame: 180,
                    next: "szombie_paine20",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine20"),
                MonsterFrame {
                    frame: 181,
                    next: "szombie_paine21",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine21"),
                MonsterFrame {
                    frame: 182,
                    next: "szombie_paine22",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine22"),
                MonsterFrame {
                    frame: 183,
                    next: "szombie_paine23",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine23"),
                MonsterFrame {
                    frame: 184,
                    next: "szombie_paine24",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine24"),
                MonsterFrame {
                    frame: 185,
                    next: "szombie_paine25",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine25"),
                MonsterFrame {
                    frame: 186,
                    next: "szombie_paine26",
                    operations: OPS_SZOMBIE_PAINE25,
                },
            ),
            (
                String::from("szombie_paine26"),
                MonsterFrame {
                    frame: 187,
                    next: "szombie_paine27",
                    operations: OPS_SZOMBIE_PAINE26,
                },
            ),
            (
                String::from("szombie_paine27"),
                MonsterFrame {
                    frame: 188,
                    next: "szombie_paine28",
                    operations: OPS_SZOMBIE_PAINE27,
                },
            ),
            (
                String::from("szombie_paine28"),
                MonsterFrame {
                    frame: 189,
                    next: "szombie_paine29",
                    operations: OPS_SZOMBIE_PAINE28,
                },
            ),
            (
                String::from("szombie_paine29"),
                MonsterFrame {
                    frame: 190,
                    next: "szombie_paine30",
                    operations: &[],
                },
            ),
            (
                String::from("szombie_paine30"),
                MonsterFrame {
                    frame: 191,
                    next: "szombie_run1",
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
        assert_eq!(frames().len(), 193);
        for (name, frame) in frames() {
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
