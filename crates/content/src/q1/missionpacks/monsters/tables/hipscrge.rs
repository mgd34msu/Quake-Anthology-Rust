//! Hipnotic hipscrge frames (src/content/q1/missionpacks/monsters/tables/hipscrge.ts).
//!
//! quakec_hipnotic/hipscrge.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

static OPS_SCOURGE_ATK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk1",
}];
static OPS_SCOURGE_ATK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk2",
}];
static OPS_SCOURGE_ATK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk3",
}];
static OPS_SCOURGE_ATK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk4",
}];
static OPS_SCOURGE_ATK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk5",
}];
static OPS_SCOURGE_ATK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk2",
}];
static OPS_SCOURGE_ATK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk3",
}];
static OPS_SCOURGE_ATK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_atk8",
}];
static OPS_SCOURGE_DIE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_pain1",
}];
static OPS_SCOURGE_DIE2: &[MonsterOperation] = &[];
static OPS_SCOURGE_DIE3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_SCOURGE_DIE4: &[MonsterOperation] = &[];
static OPS_SCOURGE_DIE5: &[MonsterOperation] = &[];
static OPS_SCOURGE_MELEE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_melee1",
}];
static OPS_SCOURGE_MELEE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SCOURGE_MELEE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_melee11",
}];
static OPS_SCOURGE_MELEE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SCOURGE_MELEE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 2.0,
}];
static OPS_SCOURGE_MELEE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 2.0,
}];
static OPS_SCOURGE_MELEE5: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "scourge/tailswng.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
];
static OPS_SCOURGE_MELEE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_SCOURGE_MELEE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "Attack_With_Tail",
}];
static OPS_SCOURGE_MELEE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SCOURGE_MELEE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SCOURGE_PAIN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_pain1",
}];
static OPS_SCOURGE_PAIN2: &[MonsterOperation] = &[];
static OPS_SCOURGE_PAIN3: &[MonsterOperation] = &[];
static OPS_SCOURGE_PAIN4: &[MonsterOperation] = &[];
static OPS_SCOURGE_PAIN5: &[MonsterOperation] = &[];
static OPS_SCOURGE_RUN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_run1",
}];
static OPS_SCOURGE_RUN2: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "scourge_think" },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 14.0,
    },
];
static OPS_SCOURGE_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SCOURGE_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SCOURGE_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SCOURGE_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_SCOURGE_STAND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STAND9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_stand1",
}];
static OPS_SCOURGE_STRAFELEFT1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_strafeleft1",
}];
static OPS_SCOURGE_STRAFELEFT2: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_left(20)" }];
static OPS_SCOURGE_STRAFELEFT3: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_left(20)" }];
static OPS_SCOURGE_STRAFELEFT4: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_left(14)" }];
static OPS_SCOURGE_STRAFELEFT5: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_left(14)" }];
static OPS_SCOURGE_STRAFELEFT6: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_left(14)" }];
static OPS_SCOURGE_STRAFERIGHT1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_straferight1",
}];
static OPS_SCOURGE_STRAFERIGHT2: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_right(20)" }];
static OPS_SCOURGE_STRAFERIGHT3: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_right(20)" }];
static OPS_SCOURGE_STRAFERIGHT4: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_right(14)" }];
static OPS_SCOURGE_STRAFERIGHT5: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_right(14)" }];
static OPS_SCOURGE_STRAFERIGHT6: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_right(14)" }];
static OPS_SCOURGE_TURN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_turn1",
}];
static OPS_SCOURGE_TURN2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ai_turn_in_place",
}];
static OPS_SCOURGE_TURN3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ai_turn_in_place",
}];
static OPS_SCOURGE_TURN4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ai_turn_in_place",
}];
static OPS_SCOURGE_TURN5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ai_turn_in_place",
}];
static OPS_SCOURGE_TURN6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ai_turn_in_place",
}];
static OPS_SCOURGE_WALK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipscrge:scourge_walk1",
}];
static OPS_SCOURGE_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_SCOURGE_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_SCOURGE_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_SCOURGE_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_SCOURGE_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "scourge_atk1",
        MonsterFrame {
            frame: 18,
            next: "scourge_atk2",
            operations: OPS_SCOURGE_ATK1,
        },
    ),
    (
        "scourge_atk2",
        MonsterFrame {
            frame: 19,
            next: "scourge_atk3",
            operations: OPS_SCOURGE_ATK2,
        },
    ),
    (
        "scourge_atk3",
        MonsterFrame {
            frame: 18,
            next: "scourge_atk4",
            operations: OPS_SCOURGE_ATK3,
        },
    ),
    (
        "scourge_atk4",
        MonsterFrame {
            frame: 19,
            next: "scourge_atk5",
            operations: OPS_SCOURGE_ATK4,
        },
    ),
    (
        "scourge_atk5",
        MonsterFrame {
            frame: 18,
            next: "scourge_atk6",
            operations: OPS_SCOURGE_ATK5,
        },
    ),
    (
        "scourge_atk6",
        MonsterFrame {
            frame: 19,
            next: "scourge_atk7",
            operations: OPS_SCOURGE_ATK6,
        },
    ),
    (
        "scourge_atk7",
        MonsterFrame {
            frame: 18,
            next: "scourge_atk8",
            operations: OPS_SCOURGE_ATK7,
        },
    ),
    (
        "scourge_atk8",
        MonsterFrame {
            frame: 19,
            next: "scourge_run1",
            operations: OPS_SCOURGE_ATK8,
        },
    ),
    (
        "scourge_die1",
        MonsterFrame {
            frame: 36,
            next: "scourge_die2",
            operations: OPS_SCOURGE_DIE1,
        },
    ),
    (
        "scourge_die2",
        MonsterFrame {
            frame: 37,
            next: "scourge_die3",
            operations: OPS_SCOURGE_DIE2,
        },
    ),
    (
        "scourge_die3",
        MonsterFrame {
            frame: 38,
            next: "scourge_die4",
            operations: OPS_SCOURGE_DIE3,
        },
    ),
    (
        "scourge_die4",
        MonsterFrame {
            frame: 39,
            next: "scourge_die5",
            operations: OPS_SCOURGE_DIE4,
        },
    ),
    (
        "scourge_die5",
        MonsterFrame {
            frame: 40,
            next: "scourge_die5",
            operations: OPS_SCOURGE_DIE5,
        },
    ),
    (
        "scourge_melee1",
        MonsterFrame {
            frame: 20,
            next: "scourge_melee2",
            operations: OPS_SCOURGE_MELEE1,
        },
    ),
    (
        "scourge_melee10",
        MonsterFrame {
            frame: 29,
            next: "scourge_melee11",
            operations: OPS_SCOURGE_MELEE10,
        },
    ),
    (
        "scourge_melee11",
        MonsterFrame {
            frame: 30,
            next: "scourge_run1",
            operations: OPS_SCOURGE_MELEE11,
        },
    ),
    (
        "scourge_melee2",
        MonsterFrame {
            frame: 21,
            next: "scourge_melee3",
            operations: OPS_SCOURGE_MELEE2,
        },
    ),
    (
        "scourge_melee3",
        MonsterFrame {
            frame: 22,
            next: "scourge_melee4",
            operations: OPS_SCOURGE_MELEE3,
        },
    ),
    (
        "scourge_melee4",
        MonsterFrame {
            frame: 23,
            next: "scourge_melee5",
            operations: OPS_SCOURGE_MELEE4,
        },
    ),
    (
        "scourge_melee5",
        MonsterFrame {
            frame: 24,
            next: "scourge_melee6",
            operations: OPS_SCOURGE_MELEE5,
        },
    ),
    (
        "scourge_melee6",
        MonsterFrame {
            frame: 25,
            next: "scourge_melee7",
            operations: OPS_SCOURGE_MELEE6,
        },
    ),
    (
        "scourge_melee7",
        MonsterFrame {
            frame: 26,
            next: "scourge_melee8",
            operations: OPS_SCOURGE_MELEE7,
        },
    ),
    (
        "scourge_melee8",
        MonsterFrame {
            frame: 27,
            next: "scourge_melee9",
            operations: OPS_SCOURGE_MELEE8,
        },
    ),
    (
        "scourge_melee9",
        MonsterFrame {
            frame: 28,
            next: "scourge_melee10",
            operations: OPS_SCOURGE_MELEE9,
        },
    ),
    (
        "scourge_pain1",
        MonsterFrame {
            frame: 31,
            next: "scourge_pain2",
            operations: OPS_SCOURGE_PAIN1,
        },
    ),
    (
        "scourge_pain2",
        MonsterFrame {
            frame: 32,
            next: "scourge_pain3",
            operations: OPS_SCOURGE_PAIN2,
        },
    ),
    (
        "scourge_pain3",
        MonsterFrame {
            frame: 33,
            next: "scourge_pain4",
            operations: OPS_SCOURGE_PAIN3,
        },
    ),
    (
        "scourge_pain4",
        MonsterFrame {
            frame: 34,
            next: "scourge_pain5",
            operations: OPS_SCOURGE_PAIN4,
        },
    ),
    (
        "scourge_pain5",
        MonsterFrame {
            frame: 35,
            next: "scourge_run1",
            operations: OPS_SCOURGE_PAIN5,
        },
    ),
    (
        "scourge_run1",
        MonsterFrame {
            frame: 12,
            next: "scourge_run2",
            operations: OPS_SCOURGE_RUN1,
        },
    ),
    (
        "scourge_run2",
        MonsterFrame {
            frame: 13,
            next: "scourge_run3",
            operations: OPS_SCOURGE_RUN2,
        },
    ),
    (
        "scourge_run3",
        MonsterFrame {
            frame: 14,
            next: "scourge_run4",
            operations: OPS_SCOURGE_RUN3,
        },
    ),
    (
        "scourge_run4",
        MonsterFrame {
            frame: 15,
            next: "scourge_run5",
            operations: OPS_SCOURGE_RUN4,
        },
    ),
    (
        "scourge_run5",
        MonsterFrame {
            frame: 16,
            next: "scourge_run6",
            operations: OPS_SCOURGE_RUN5,
        },
    ),
    (
        "scourge_run6",
        MonsterFrame {
            frame: 17,
            next: "scourge_run1",
            operations: OPS_SCOURGE_RUN6,
        },
    ),
    (
        "scourge_stand1",
        MonsterFrame {
            frame: 0,
            next: "scourge_stand2",
            operations: OPS_SCOURGE_STAND1,
        },
    ),
    (
        "scourge_stand10",
        MonsterFrame {
            frame: 9,
            next: "scourge_stand11",
            operations: OPS_SCOURGE_STAND10,
        },
    ),
    (
        "scourge_stand11",
        MonsterFrame {
            frame: 10,
            next: "scourge_stand12",
            operations: OPS_SCOURGE_STAND11,
        },
    ),
    (
        "scourge_stand12",
        MonsterFrame {
            frame: 11,
            next: "scourge_stand1",
            operations: OPS_SCOURGE_STAND12,
        },
    ),
    (
        "scourge_stand2",
        MonsterFrame {
            frame: 1,
            next: "scourge_stand3",
            operations: OPS_SCOURGE_STAND2,
        },
    ),
    (
        "scourge_stand3",
        MonsterFrame {
            frame: 2,
            next: "scourge_stand4",
            operations: OPS_SCOURGE_STAND3,
        },
    ),
    (
        "scourge_stand4",
        MonsterFrame {
            frame: 3,
            next: "scourge_stand5",
            operations: OPS_SCOURGE_STAND4,
        },
    ),
    (
        "scourge_stand5",
        MonsterFrame {
            frame: 4,
            next: "scourge_stand6",
            operations: OPS_SCOURGE_STAND5,
        },
    ),
    (
        "scourge_stand6",
        MonsterFrame {
            frame: 5,
            next: "scourge_stand7",
            operations: OPS_SCOURGE_STAND6,
        },
    ),
    (
        "scourge_stand7",
        MonsterFrame {
            frame: 6,
            next: "scourge_stand8",
            operations: OPS_SCOURGE_STAND7,
        },
    ),
    (
        "scourge_stand8",
        MonsterFrame {
            frame: 7,
            next: "scourge_stand9",
            operations: OPS_SCOURGE_STAND8,
        },
    ),
    (
        "scourge_stand9",
        MonsterFrame {
            frame: 8,
            next: "scourge_stand10",
            operations: OPS_SCOURGE_STAND9,
        },
    ),
    (
        "scourge_strafeleft1",
        MonsterFrame {
            frame: 12,
            next: "scourge_strafeleft2",
            operations: OPS_SCOURGE_STRAFELEFT1,
        },
    ),
    (
        "scourge_strafeleft2",
        MonsterFrame {
            frame: 13,
            next: "scourge_strafeleft3",
            operations: OPS_SCOURGE_STRAFELEFT2,
        },
    ),
    (
        "scourge_strafeleft3",
        MonsterFrame {
            frame: 14,
            next: "scourge_strafeleft4",
            operations: OPS_SCOURGE_STRAFELEFT3,
        },
    ),
    (
        "scourge_strafeleft4",
        MonsterFrame {
            frame: 15,
            next: "scourge_strafeleft5",
            operations: OPS_SCOURGE_STRAFELEFT4,
        },
    ),
    (
        "scourge_strafeleft5",
        MonsterFrame {
            frame: 16,
            next: "scourge_strafeleft6",
            operations: OPS_SCOURGE_STRAFELEFT5,
        },
    ),
    (
        "scourge_strafeleft6",
        MonsterFrame {
            frame: 17,
            next: "scourge_run1",
            operations: OPS_SCOURGE_STRAFELEFT6,
        },
    ),
    (
        "scourge_straferight1",
        MonsterFrame {
            frame: 12,
            next: "scourge_straferight2",
            operations: OPS_SCOURGE_STRAFERIGHT1,
        },
    ),
    (
        "scourge_straferight2",
        MonsterFrame {
            frame: 13,
            next: "scourge_straferight3",
            operations: OPS_SCOURGE_STRAFERIGHT2,
        },
    ),
    (
        "scourge_straferight3",
        MonsterFrame {
            frame: 14,
            next: "scourge_straferight4",
            operations: OPS_SCOURGE_STRAFERIGHT3,
        },
    ),
    (
        "scourge_straferight4",
        MonsterFrame {
            frame: 15,
            next: "scourge_straferight5",
            operations: OPS_SCOURGE_STRAFERIGHT4,
        },
    ),
    (
        "scourge_straferight5",
        MonsterFrame {
            frame: 16,
            next: "scourge_straferight6",
            operations: OPS_SCOURGE_STRAFERIGHT5,
        },
    ),
    (
        "scourge_straferight6",
        MonsterFrame {
            frame: 17,
            next: "scourge_run1",
            operations: OPS_SCOURGE_STRAFERIGHT6,
        },
    ),
    (
        "scourge_turn1",
        MonsterFrame {
            frame: 12,
            next: "scourge_turn2",
            operations: OPS_SCOURGE_TURN1,
        },
    ),
    (
        "scourge_turn2",
        MonsterFrame {
            frame: 13,
            next: "scourge_turn3",
            operations: OPS_SCOURGE_TURN2,
        },
    ),
    (
        "scourge_turn3",
        MonsterFrame {
            frame: 14,
            next: "scourge_turn4",
            operations: OPS_SCOURGE_TURN3,
        },
    ),
    (
        "scourge_turn4",
        MonsterFrame {
            frame: 15,
            next: "scourge_turn5",
            operations: OPS_SCOURGE_TURN4,
        },
    ),
    (
        "scourge_turn5",
        MonsterFrame {
            frame: 16,
            next: "scourge_turn6",
            operations: OPS_SCOURGE_TURN5,
        },
    ),
    (
        "scourge_turn6",
        MonsterFrame {
            frame: 17,
            next: "scourge_turn1",
            operations: OPS_SCOURGE_TURN6,
        },
    ),
    (
        "scourge_walk1",
        MonsterFrame {
            frame: 12,
            next: "scourge_walk2",
            operations: OPS_SCOURGE_WALK1,
        },
    ),
    (
        "scourge_walk2",
        MonsterFrame {
            frame: 13,
            next: "scourge_walk3",
            operations: OPS_SCOURGE_WALK2,
        },
    ),
    (
        "scourge_walk3",
        MonsterFrame {
            frame: 14,
            next: "scourge_walk4",
            operations: OPS_SCOURGE_WALK3,
        },
    ),
    (
        "scourge_walk4",
        MonsterFrame {
            frame: 15,
            next: "scourge_walk5",
            operations: OPS_SCOURGE_WALK4,
        },
    ),
    (
        "scourge_walk5",
        MonsterFrame {
            frame: 16,
            next: "scourge_walk6",
            operations: OPS_SCOURGE_WALK5,
        },
    ),
    (
        "scourge_walk6",
        MonsterFrame {
            frame: 17,
            next: "scourge_walk1",
            operations: OPS_SCOURGE_WALK6,
        },
    ),
];

/// Look up a hipscrge frame by name.
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
        assert_eq!(FRAMES.len(), 71);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("scourge_atk1").expect("first frame");
        assert_eq!((head.frame, head.next), (18, "scourge_atk2"));
        let tail = frame("scourge_walk6").expect("last frame");
        assert_eq!((tail.frame, tail.next), (17, "scourge_walk1"));
        assert!(frame("no_such_frame").is_none());
    }
}
