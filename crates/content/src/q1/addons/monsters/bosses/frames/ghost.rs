//! Q1 Mg3 player-ghost frames (`src/content/q1/addons/monsters/bosses/frames/ghost.ts`).
//!
//! mg3_player_ghost.qc source frame declarations. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_GHOST_STAND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_stand1",
}];
static OPS_GHOST_STAND5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_stand5",
}];
static OPS_GHOST_RUN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_run1",
}];
static OPS_GHOST_RUN6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_run6",
}];
static OPS_GHOST_DIEA1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea1",
}];
static OPS_GHOST_DIEA11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea11",
}];
static OPS_GHOST_DIEB1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea1",
}];
static OPS_GHOST_DIEB9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea11",
}];
static OPS_GHOST_DIEC1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea1",
}];
static OPS_GHOST_DIEC15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea11",
}];
static OPS_GHOST_DIED1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea1",
}];
static OPS_GHOST_DIED9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea11",
}];
static OPS_GHOST_DIEE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea1",
}];
static OPS_GHOST_DIEE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "ghost:ghost_diea11",
}];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("ghost_stand1"),
                MonsterFrame {
                    frame: 12,
                    next: "ghost_stand2",
                    operations: OPS_GHOST_STAND1,
                },
            ),
            (
                String::from("ghost_stand2"),
                MonsterFrame {
                    frame: 13,
                    next: "ghost_stand3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_stand3"),
                MonsterFrame {
                    frame: 14,
                    next: "ghost_stand4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_stand4"),
                MonsterFrame {
                    frame: 15,
                    next: "ghost_stand5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_stand5"),
                MonsterFrame {
                    frame: 16,
                    next: "ghost_stand1",
                    operations: OPS_GHOST_STAND5,
                },
            ),
            (
                String::from("ghost_run1"),
                MonsterFrame {
                    frame: 6,
                    next: "ghost_run2",
                    operations: OPS_GHOST_RUN1,
                },
            ),
            (
                String::from("ghost_run2"),
                MonsterFrame {
                    frame: 7,
                    next: "ghost_run3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_run3"),
                MonsterFrame {
                    frame: 8,
                    next: "ghost_run4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_run4"),
                MonsterFrame {
                    frame: 9,
                    next: "ghost_run5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_run5"),
                MonsterFrame {
                    frame: 10,
                    next: "ghost_run6",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_run6"),
                MonsterFrame {
                    frame: 11,
                    next: "ghost_stand1",
                    operations: OPS_GHOST_RUN6,
                },
            ),
            (
                String::from("ghost_diea1"),
                MonsterFrame {
                    frame: 50,
                    next: "ghost_diea2",
                    operations: OPS_GHOST_DIEA1,
                },
            ),
            (
                String::from("ghost_diea2"),
                MonsterFrame {
                    frame: 51,
                    next: "ghost_diea3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea3"),
                MonsterFrame {
                    frame: 52,
                    next: "ghost_diea4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea4"),
                MonsterFrame {
                    frame: 53,
                    next: "ghost_diea5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea5"),
                MonsterFrame {
                    frame: 54,
                    next: "ghost_diea6",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea6"),
                MonsterFrame {
                    frame: 55,
                    next: "ghost_diea7",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea7"),
                MonsterFrame {
                    frame: 56,
                    next: "ghost_diea8",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea8"),
                MonsterFrame {
                    frame: 57,
                    next: "ghost_diea9",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea9"),
                MonsterFrame {
                    frame: 58,
                    next: "ghost_diea10",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea10"),
                MonsterFrame {
                    frame: 59,
                    next: "ghost_diea11",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diea11"),
                MonsterFrame {
                    frame: 60,
                    next: "ghost_diea11",
                    operations: OPS_GHOST_DIEA11,
                },
            ),
            (
                String::from("ghost_dieb1"),
                MonsterFrame {
                    frame: 61,
                    next: "ghost_dieb2",
                    operations: OPS_GHOST_DIEB1,
                },
            ),
            (
                String::from("ghost_dieb2"),
                MonsterFrame {
                    frame: 62,
                    next: "ghost_dieb3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb3"),
                MonsterFrame {
                    frame: 63,
                    next: "ghost_dieb4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb4"),
                MonsterFrame {
                    frame: 64,
                    next: "ghost_dieb5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb5"),
                MonsterFrame {
                    frame: 65,
                    next: "ghost_dieb6",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb6"),
                MonsterFrame {
                    frame: 66,
                    next: "ghost_dieb7",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb7"),
                MonsterFrame {
                    frame: 67,
                    next: "ghost_dieb8",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb8"),
                MonsterFrame {
                    frame: 68,
                    next: "ghost_dieb9",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_dieb9"),
                MonsterFrame {
                    frame: 69,
                    next: "ghost_dieb9",
                    operations: OPS_GHOST_DIEB9,
                },
            ),
            (
                String::from("ghost_diec1"),
                MonsterFrame {
                    frame: 70,
                    next: "ghost_diec2",
                    operations: OPS_GHOST_DIEC1,
                },
            ),
            (
                String::from("ghost_diec2"),
                MonsterFrame {
                    frame: 71,
                    next: "ghost_diec3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec3"),
                MonsterFrame {
                    frame: 72,
                    next: "ghost_diec4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec4"),
                MonsterFrame {
                    frame: 73,
                    next: "ghost_diec5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec5"),
                MonsterFrame {
                    frame: 74,
                    next: "ghost_diec6",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec6"),
                MonsterFrame {
                    frame: 75,
                    next: "ghost_diec7",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec7"),
                MonsterFrame {
                    frame: 76,
                    next: "ghost_diec8",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec8"),
                MonsterFrame {
                    frame: 77,
                    next: "ghost_diec9",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec9"),
                MonsterFrame {
                    frame: 78,
                    next: "ghost_diec10",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec10"),
                MonsterFrame {
                    frame: 79,
                    next: "ghost_diec11",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec11"),
                MonsterFrame {
                    frame: 80,
                    next: "ghost_diec12",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec12"),
                MonsterFrame {
                    frame: 81,
                    next: "ghost_diec13",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec13"),
                MonsterFrame {
                    frame: 82,
                    next: "ghost_diec14",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec14"),
                MonsterFrame {
                    frame: 83,
                    next: "ghost_diec15",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diec15"),
                MonsterFrame {
                    frame: 84,
                    next: "ghost_diec15",
                    operations: OPS_GHOST_DIEC15,
                },
            ),
            (
                String::from("ghost_died1"),
                MonsterFrame {
                    frame: 85,
                    next: "ghost_died2",
                    operations: OPS_GHOST_DIED1,
                },
            ),
            (
                String::from("ghost_died2"),
                MonsterFrame {
                    frame: 86,
                    next: "ghost_died3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died3"),
                MonsterFrame {
                    frame: 87,
                    next: "ghost_died4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died4"),
                MonsterFrame {
                    frame: 88,
                    next: "ghost_died5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died5"),
                MonsterFrame {
                    frame: 89,
                    next: "ghost_died6",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died6"),
                MonsterFrame {
                    frame: 90,
                    next: "ghost_died7",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died7"),
                MonsterFrame {
                    frame: 91,
                    next: "ghost_died8",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died8"),
                MonsterFrame {
                    frame: 92,
                    next: "ghost_died9",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_died9"),
                MonsterFrame {
                    frame: 93,
                    next: "ghost_died9",
                    operations: OPS_GHOST_DIED9,
                },
            ),
            (
                String::from("ghost_diee1"),
                MonsterFrame {
                    frame: 94,
                    next: "ghost_diee2",
                    operations: OPS_GHOST_DIEE1,
                },
            ),
            (
                String::from("ghost_diee2"),
                MonsterFrame {
                    frame: 95,
                    next: "ghost_diee3",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee3"),
                MonsterFrame {
                    frame: 96,
                    next: "ghost_diee4",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee4"),
                MonsterFrame {
                    frame: 97,
                    next: "ghost_diee5",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee5"),
                MonsterFrame {
                    frame: 98,
                    next: "ghost_diee6",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee6"),
                MonsterFrame {
                    frame: 99,
                    next: "ghost_diee7",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee7"),
                MonsterFrame {
                    frame: 100,
                    next: "ghost_diee8",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee8"),
                MonsterFrame {
                    frame: 101,
                    next: "ghost_diee9",
                    operations: &[],
                },
            ),
            (
                String::from("ghost_diee9"),
                MonsterFrame {
                    frame: 93,
                    next: "ghost_diee9",
                    operations: OPS_GHOST_DIEE9,
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
        assert_eq!(frames().len(), 64);
        for (name, frame) in frames() {
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
