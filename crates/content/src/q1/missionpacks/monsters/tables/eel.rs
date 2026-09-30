//! Rogue eel frames (src/content/q1/missionpacks/monsters/tables/eel.ts).
//!
//! quakec_rogue/eel.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

static OPS_EEL_ATTACK1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "eel:eel_attack10",
}];
static OPS_EEL_ATTACK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "eel:eel_attack11",
}];
static OPS_EEL_ATTACK12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "eel:eel_attack12",
}];
static OPS_EEL_ATTACK2: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_ATTACK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "eel:eel_attack8",
}];
static OPS_EEL_ATTACK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "eel:eel_attack9",
}];
static OPS_EEL_DEATH1: &[MonsterOperation] = &[MonsterOperation::Action { name: "eel:eel_death1" }];
static OPS_EEL_DEATH10: &[MonsterOperation] = &[];
static OPS_EEL_DEATH11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "eel:eel_death11",
}];
static OPS_EEL_DEATH12: &[MonsterOperation] = &[];
static OPS_EEL_DEATH13: &[MonsterOperation] = &[];
static OPS_EEL_DEATH14: &[MonsterOperation] = &[];
static OPS_EEL_DEATH15: &[MonsterOperation] = &[MonsterOperation::Action { name: "droptofloor" }];
static OPS_EEL_DEATH16: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_EEL_DEATH2: &[MonsterOperation] = &[];
static OPS_EEL_DEATH3: &[MonsterOperation] = &[];
static OPS_EEL_DEATH4: &[MonsterOperation] = &[];
static OPS_EEL_DEATH5: &[MonsterOperation] = &[];
static OPS_EEL_DEATH6: &[MonsterOperation] = &[];
static OPS_EEL_DEATH7: &[MonsterOperation] = &[];
static OPS_EEL_DEATH8: &[MonsterOperation] = &[];
static OPS_EEL_DEATH9: &[MonsterOperation] = &[];
static OPS_EEL_PAIN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "eel:eel_pain1" }];
static OPS_EEL_PAIN2: &[MonsterOperation] = &[];
static OPS_EEL_PAIN3: &[MonsterOperation] = &[];
static OPS_EEL_PAIN4: &[MonsterOperation] = &[];
static OPS_EEL_PAIN5: &[MonsterOperation] = &[];
static OPS_EEL_PAIN6: &[MonsterOperation] = &[];
static OPS_EEL_PAIN7: &[MonsterOperation] = &[];
static OPS_EEL_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "eel/eactive1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.4),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 10.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_RUN2: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 10.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_RUN3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 10.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_RUN4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 10.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_RUN5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 10.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_RUN6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 10.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_STAND1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_STAND2: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_STAND3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_STAND4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_STAND5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_STAND6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "eel/eactive1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_WALK2: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_WALK3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_WALK4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_WALK5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];
static OPS_EEL_WALK6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
    MonsterOperation::Action {
        name: "eel_pitch_change",
    },
];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "eel_attack1",
        MonsterFrame {
            frame: 0,
            next: "eel_attack2",
            operations: OPS_EEL_ATTACK1,
        },
    ),
    (
        "eel_attack10",
        MonsterFrame {
            frame: 3,
            next: "eel_attack11",
            operations: OPS_EEL_ATTACK10,
        },
    ),
    (
        "eel_attack11",
        MonsterFrame {
            frame: 4,
            next: "eel_attack12",
            operations: OPS_EEL_ATTACK11,
        },
    ),
    (
        "eel_attack12",
        MonsterFrame {
            frame: 5,
            next: "eel_run1",
            operations: OPS_EEL_ATTACK12,
        },
    ),
    (
        "eel_attack2",
        MonsterFrame {
            frame: 1,
            next: "eel_attack3",
            operations: OPS_EEL_ATTACK2,
        },
    ),
    (
        "eel_attack3",
        MonsterFrame {
            frame: 2,
            next: "eel_attack4",
            operations: OPS_EEL_ATTACK3,
        },
    ),
    (
        "eel_attack4",
        MonsterFrame {
            frame: 3,
            next: "eel_attack5",
            operations: OPS_EEL_ATTACK4,
        },
    ),
    (
        "eel_attack5",
        MonsterFrame {
            frame: 4,
            next: "eel_attack6",
            operations: OPS_EEL_ATTACK5,
        },
    ),
    (
        "eel_attack6",
        MonsterFrame {
            frame: 5,
            next: "eel_attack7",
            operations: OPS_EEL_ATTACK6,
        },
    ),
    (
        "eel_attack7",
        MonsterFrame {
            frame: 0,
            next: "eel_attack8",
            operations: OPS_EEL_ATTACK7,
        },
    ),
    (
        "eel_attack8",
        MonsterFrame {
            frame: 1,
            next: "eel_attack9",
            operations: OPS_EEL_ATTACK8,
        },
    ),
    (
        "eel_attack9",
        MonsterFrame {
            frame: 2,
            next: "eel_attack10",
            operations: OPS_EEL_ATTACK9,
        },
    ),
    (
        "eel_death1",
        MonsterFrame {
            frame: 6,
            next: "eel_death2",
            operations: OPS_EEL_DEATH1,
        },
    ),
    (
        "eel_death10",
        MonsterFrame {
            frame: 9,
            next: "eel_death11",
            operations: OPS_EEL_DEATH10,
        },
    ),
    (
        "eel_death11",
        MonsterFrame {
            frame: 10,
            next: "eel_death12",
            operations: OPS_EEL_DEATH11,
        },
    ),
    (
        "eel_death12",
        MonsterFrame {
            frame: 11,
            next: "eel_death13",
            operations: OPS_EEL_DEATH12,
        },
    ),
    (
        "eel_death13",
        MonsterFrame {
            frame: 12,
            next: "eel_death14",
            operations: OPS_EEL_DEATH13,
        },
    ),
    (
        "eel_death14",
        MonsterFrame {
            frame: 13,
            next: "eel_death15",
            operations: OPS_EEL_DEATH14,
        },
    ),
    (
        "eel_death15",
        MonsterFrame {
            frame: 14,
            next: "eel_death16",
            operations: OPS_EEL_DEATH15,
        },
    ),
    (
        "eel_death16",
        MonsterFrame {
            frame: 15,
            next: "eel_death16",
            operations: OPS_EEL_DEATH16,
        },
    ),
    (
        "eel_death2",
        MonsterFrame {
            frame: 7,
            next: "eel_death3",
            operations: OPS_EEL_DEATH2,
        },
    ),
    (
        "eel_death3",
        MonsterFrame {
            frame: 8,
            next: "eel_death4",
            operations: OPS_EEL_DEATH3,
        },
    ),
    (
        "eel_death4",
        MonsterFrame {
            frame: 9,
            next: "eel_death5",
            operations: OPS_EEL_DEATH4,
        },
    ),
    (
        "eel_death5",
        MonsterFrame {
            frame: 8,
            next: "eel_death6",
            operations: OPS_EEL_DEATH5,
        },
    ),
    (
        "eel_death6",
        MonsterFrame {
            frame: 7,
            next: "eel_death7",
            operations: OPS_EEL_DEATH6,
        },
    ),
    (
        "eel_death7",
        MonsterFrame {
            frame: 6,
            next: "eel_death8",
            operations: OPS_EEL_DEATH7,
        },
    ),
    (
        "eel_death8",
        MonsterFrame {
            frame: 7,
            next: "eel_death9",
            operations: OPS_EEL_DEATH8,
        },
    ),
    (
        "eel_death9",
        MonsterFrame {
            frame: 8,
            next: "eel_death10",
            operations: OPS_EEL_DEATH9,
        },
    ),
    (
        "eel_pain1",
        MonsterFrame {
            frame: 6,
            next: "eel_pain2",
            operations: OPS_EEL_PAIN1,
        },
    ),
    (
        "eel_pain2",
        MonsterFrame {
            frame: 7,
            next: "eel_pain3",
            operations: OPS_EEL_PAIN2,
        },
    ),
    (
        "eel_pain3",
        MonsterFrame {
            frame: 8,
            next: "eel_pain4",
            operations: OPS_EEL_PAIN3,
        },
    ),
    (
        "eel_pain4",
        MonsterFrame {
            frame: 9,
            next: "eel_pain5",
            operations: OPS_EEL_PAIN4,
        },
    ),
    (
        "eel_pain5",
        MonsterFrame {
            frame: 8,
            next: "eel_pain6",
            operations: OPS_EEL_PAIN5,
        },
    ),
    (
        "eel_pain6",
        MonsterFrame {
            frame: 7,
            next: "eel_pain7",
            operations: OPS_EEL_PAIN6,
        },
    ),
    (
        "eel_pain7",
        MonsterFrame {
            frame: 6,
            next: "eel_run1",
            operations: OPS_EEL_PAIN7,
        },
    ),
    (
        "eel_run1",
        MonsterFrame {
            frame: 0,
            next: "eel_run2",
            operations: OPS_EEL_RUN1,
        },
    ),
    (
        "eel_run2",
        MonsterFrame {
            frame: 1,
            next: "eel_run3",
            operations: OPS_EEL_RUN2,
        },
    ),
    (
        "eel_run3",
        MonsterFrame {
            frame: 2,
            next: "eel_run4",
            operations: OPS_EEL_RUN3,
        },
    ),
    (
        "eel_run4",
        MonsterFrame {
            frame: 3,
            next: "eel_run5",
            operations: OPS_EEL_RUN4,
        },
    ),
    (
        "eel_run5",
        MonsterFrame {
            frame: 4,
            next: "eel_run6",
            operations: OPS_EEL_RUN5,
        },
    ),
    (
        "eel_run6",
        MonsterFrame {
            frame: 5,
            next: "eel_run1",
            operations: OPS_EEL_RUN6,
        },
    ),
    (
        "eel_stand1",
        MonsterFrame {
            frame: 0,
            next: "eel_stand2",
            operations: OPS_EEL_STAND1,
        },
    ),
    (
        "eel_stand2",
        MonsterFrame {
            frame: 1,
            next: "eel_stand3",
            operations: OPS_EEL_STAND2,
        },
    ),
    (
        "eel_stand3",
        MonsterFrame {
            frame: 2,
            next: "eel_stand4",
            operations: OPS_EEL_STAND3,
        },
    ),
    (
        "eel_stand4",
        MonsterFrame {
            frame: 3,
            next: "eel_stand5",
            operations: OPS_EEL_STAND4,
        },
    ),
    (
        "eel_stand5",
        MonsterFrame {
            frame: 4,
            next: "eel_stand6",
            operations: OPS_EEL_STAND5,
        },
    ),
    (
        "eel_stand6",
        MonsterFrame {
            frame: 5,
            next: "eel_stand1",
            operations: OPS_EEL_STAND6,
        },
    ),
    (
        "eel_walk1",
        MonsterFrame {
            frame: 0,
            next: "eel_walk2",
            operations: OPS_EEL_WALK1,
        },
    ),
    (
        "eel_walk2",
        MonsterFrame {
            frame: 1,
            next: "eel_walk3",
            operations: OPS_EEL_WALK2,
        },
    ),
    (
        "eel_walk3",
        MonsterFrame {
            frame: 2,
            next: "eel_walk4",
            operations: OPS_EEL_WALK3,
        },
    ),
    (
        "eel_walk4",
        MonsterFrame {
            frame: 3,
            next: "eel_walk5",
            operations: OPS_EEL_WALK4,
        },
    ),
    (
        "eel_walk5",
        MonsterFrame {
            frame: 4,
            next: "eel_walk6",
            operations: OPS_EEL_WALK5,
        },
    ),
    (
        "eel_walk6",
        MonsterFrame {
            frame: 5,
            next: "eel_walk1",
            operations: OPS_EEL_WALK6,
        },
    ),
];

/// Look up a eel frame by name.
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
        assert_eq!(FRAMES.len(), 53);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("eel_attack1").expect("first frame");
        assert_eq!((head.frame, head.next), (0, "eel_attack2"));
        let tail = frame("eel_walk6").expect("last frame");
        assert_eq!((tail.frame, tail.next), (5, "eel_walk1"));
        assert!(frame("no_such_frame").is_none());
    }
}
