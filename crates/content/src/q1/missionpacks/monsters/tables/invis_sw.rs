//! Rogue invis_sw frames (src/content/q1/missionpacks/monsters/tables/invis_sw.ts).
//!
//! quakec_rogue/invis_sw.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation};
use crate::q1::foundation::types::Q1Solid;

static OPS_SWORD_ATK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "invis_sw:sword_atk1",
}];
static OPS_SWORD_ATK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SWORD_ATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SWORD_ATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SWORD_ATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SWORD_ATK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_SWORD_ATK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_SWORD_ATK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_SWORD_ATK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SWORD_ATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SWORD_DIE1: &[MonsterOperation] = &[];
static OPS_SWORD_DIE10: &[MonsterOperation] = &[];
static OPS_SWORD_DIE2: &[MonsterOperation] = &[];
static OPS_SWORD_DIE3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_SWORD_DIE4: &[MonsterOperation] = &[];
static OPS_SWORD_DIE5: &[MonsterOperation] = &[];
static OPS_SWORD_DIE6: &[MonsterOperation] = &[];
static OPS_SWORD_DIE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "invis_sw:sword_die7",
}];
static OPS_SWORD_DIE8: &[MonsterOperation] = &[];
static OPS_SWORD_DIE9: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB1: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB10: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB11: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB2: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_SWORD_DIEB4: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB5: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB6: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "invis_sw:sword_die7",
}];
static OPS_SWORD_DIEB8: &[MonsterOperation] = &[];
static OPS_SWORD_DIEB9: &[MonsterOperation] = &[];
static OPS_SWORD_RUN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "invis_sw:sword_run1",
}];
static OPS_SWORD_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SWORD_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "sword_atk1",
        MonsterFrame {
            frame: 9,
            next: "sword_atk2",
            operations: OPS_SWORD_ATK1,
        },
    ),
    (
        "sword_atk10",
        MonsterFrame {
            frame: 18,
            next: "sword_run1",
            operations: OPS_SWORD_ATK10,
        },
    ),
    (
        "sword_atk2",
        MonsterFrame {
            frame: 10,
            next: "sword_atk3",
            operations: OPS_SWORD_ATK2,
        },
    ),
    (
        "sword_atk3",
        MonsterFrame {
            frame: 11,
            next: "sword_atk4",
            operations: OPS_SWORD_ATK3,
        },
    ),
    (
        "sword_atk4",
        MonsterFrame {
            frame: 12,
            next: "sword_atk5",
            operations: OPS_SWORD_ATK4,
        },
    ),
    (
        "sword_atk5",
        MonsterFrame {
            frame: 13,
            next: "sword_atk6",
            operations: OPS_SWORD_ATK5,
        },
    ),
    (
        "sword_atk6",
        MonsterFrame {
            frame: 14,
            next: "sword_atk7",
            operations: OPS_SWORD_ATK6,
        },
    ),
    (
        "sword_atk7",
        MonsterFrame {
            frame: 15,
            next: "sword_atk8",
            operations: OPS_SWORD_ATK7,
        },
    ),
    (
        "sword_atk8",
        MonsterFrame {
            frame: 16,
            next: "sword_atk9",
            operations: OPS_SWORD_ATK8,
        },
    ),
    (
        "sword_atk9",
        MonsterFrame {
            frame: 17,
            next: "sword_atk10",
            operations: OPS_SWORD_ATK9,
        },
    ),
    (
        "sword_die1",
        MonsterFrame {
            frame: 19,
            next: "sword_die2",
            operations: OPS_SWORD_DIE1,
        },
    ),
    (
        "sword_die10",
        MonsterFrame {
            frame: 28,
            next: "sword_die10",
            operations: OPS_SWORD_DIE10,
        },
    ),
    (
        "sword_die2",
        MonsterFrame {
            frame: 20,
            next: "sword_die3",
            operations: OPS_SWORD_DIE2,
        },
    ),
    (
        "sword_die3",
        MonsterFrame {
            frame: 21,
            next: "sword_die4",
            operations: OPS_SWORD_DIE3,
        },
    ),
    (
        "sword_die4",
        MonsterFrame {
            frame: 22,
            next: "sword_die5",
            operations: OPS_SWORD_DIE4,
        },
    ),
    (
        "sword_die5",
        MonsterFrame {
            frame: 23,
            next: "sword_die6",
            operations: OPS_SWORD_DIE5,
        },
    ),
    (
        "sword_die6",
        MonsterFrame {
            frame: 24,
            next: "sword_die7",
            operations: OPS_SWORD_DIE6,
        },
    ),
    (
        "sword_die7",
        MonsterFrame {
            frame: 25,
            next: "sword_die8",
            operations: OPS_SWORD_DIE7,
        },
    ),
    (
        "sword_die8",
        MonsterFrame {
            frame: 26,
            next: "sword_die9",
            operations: OPS_SWORD_DIE8,
        },
    ),
    (
        "sword_die9",
        MonsterFrame {
            frame: 27,
            next: "sword_die10",
            operations: OPS_SWORD_DIE9,
        },
    ),
    (
        "sword_dieb1",
        MonsterFrame {
            frame: 29,
            next: "sword_dieb2",
            operations: OPS_SWORD_DIEB1,
        },
    ),
    (
        "sword_dieb10",
        MonsterFrame {
            frame: 38,
            next: "sword_dieb11",
            operations: OPS_SWORD_DIEB10,
        },
    ),
    (
        "sword_dieb11",
        MonsterFrame {
            frame: 39,
            next: "sword_dieb11",
            operations: OPS_SWORD_DIEB11,
        },
    ),
    (
        "sword_dieb2",
        MonsterFrame {
            frame: 30,
            next: "sword_dieb3",
            operations: OPS_SWORD_DIEB2,
        },
    ),
    (
        "sword_dieb3",
        MonsterFrame {
            frame: 31,
            next: "sword_dieb4",
            operations: OPS_SWORD_DIEB3,
        },
    ),
    (
        "sword_dieb4",
        MonsterFrame {
            frame: 32,
            next: "sword_dieb5",
            operations: OPS_SWORD_DIEB4,
        },
    ),
    (
        "sword_dieb5",
        MonsterFrame {
            frame: 33,
            next: "sword_dieb6",
            operations: OPS_SWORD_DIEB5,
        },
    ),
    (
        "sword_dieb6",
        MonsterFrame {
            frame: 34,
            next: "sword_dieb7",
            operations: OPS_SWORD_DIEB6,
        },
    ),
    (
        "sword_dieb7",
        MonsterFrame {
            frame: 35,
            next: "sword_dieb8",
            operations: OPS_SWORD_DIEB7,
        },
    ),
    (
        "sword_dieb8",
        MonsterFrame {
            frame: 36,
            next: "sword_dieb9",
            operations: OPS_SWORD_DIEB8,
        },
    ),
    (
        "sword_dieb9",
        MonsterFrame {
            frame: 37,
            next: "sword_dieb10",
            operations: OPS_SWORD_DIEB9,
        },
    ),
    (
        "sword_run1",
        MonsterFrame {
            frame: 1,
            next: "sword_run2",
            operations: OPS_SWORD_RUN1,
        },
    ),
    (
        "sword_run2",
        MonsterFrame {
            frame: 2,
            next: "sword_run3",
            operations: OPS_SWORD_RUN2,
        },
    ),
    (
        "sword_run3",
        MonsterFrame {
            frame: 3,
            next: "sword_run4",
            operations: OPS_SWORD_RUN3,
        },
    ),
    (
        "sword_run4",
        MonsterFrame {
            frame: 4,
            next: "sword_run5",
            operations: OPS_SWORD_RUN4,
        },
    ),
    (
        "sword_run5",
        MonsterFrame {
            frame: 5,
            next: "sword_run6",
            operations: OPS_SWORD_RUN5,
        },
    ),
    (
        "sword_run6",
        MonsterFrame {
            frame: 6,
            next: "sword_run7",
            operations: OPS_SWORD_RUN6,
        },
    ),
    (
        "sword_run7",
        MonsterFrame {
            frame: 7,
            next: "sword_run8",
            operations: OPS_SWORD_RUN7,
        },
    ),
    (
        "sword_run8",
        MonsterFrame {
            frame: 8,
            next: "sword_run1",
            operations: OPS_SWORD_RUN8,
        },
    ),
    (
        "sword_stand1",
        MonsterFrame {
            frame: 0,
            next: "sword_stand1",
            operations: OPS_SWORD_STAND1,
        },
    ),
];

/// Look up a invis_sw frame by name.
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
        assert_eq!(FRAMES.len(), 40);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("sword_atk1").expect("first frame");
        assert_eq!((head.frame, head.next), (9, "sword_atk2"));
        let tail = frame("sword_stand1").expect("last frame");
        assert_eq!((tail.frame, tail.next), (0, "sword_stand1"));
        assert!(frame("no_such_frame").is_none());
    }
}
