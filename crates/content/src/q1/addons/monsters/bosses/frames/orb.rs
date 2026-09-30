//! Q1 Mg3 orb frames (`src/content/q1/addons/monsters/bosses/frames/orb.ts`).
//!
//! mg3_orb.qc source frame declarations. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_ORB_STAND1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_stand1" }];
static OPS_ORB_WALK1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_walk1" }];
static OPS_ORB_SIDE1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_side1" }];
static OPS_ORB_RUN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_run1" }];
static OPS_ORB_FAST1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_fast1" }];
static OPS_ORB_FAST3: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_fast3" }];
static OPS_ORB_FAST4: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_fast4" }];
static OPS_ORB_FAST5: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_fast5" }];
static OPS_ORB_PAIN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_pain1" }];
static OPS_ORB_PAIN2: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_pain2" }];
static OPS_ORB_DEATH1: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_death1" }];
static OPS_ORB_DEATH3: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_death3" }];
static OPS_ORB_DEATH4: &[MonsterOperation] = &[MonsterOperation::Action { name: "orb:orb_death4" }];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("orb_stand1"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_stand1",
                    operations: OPS_ORB_STAND1,
                },
            ),
            (
                String::from("orb_walk1"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_walk1",
                    operations: OPS_ORB_WALK1,
                },
            ),
            (
                String::from("orb_side1"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_side1",
                    operations: OPS_ORB_SIDE1,
                },
            ),
            (
                String::from("orb_run1"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_run1",
                    operations: OPS_ORB_RUN1,
                },
            ),
            (
                String::from("orb_fast1"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_fast2",
                    operations: OPS_ORB_FAST1,
                },
            ),
            (
                String::from("orb_fast2"),
                MonsterFrame {
                    frame: 1,
                    next: "orb_fast3",
                    operations: &[],
                },
            ),
            (
                String::from("orb_fast3"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_fast4",
                    operations: OPS_ORB_FAST3,
                },
            ),
            (
                String::from("orb_fast4"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_fast5",
                    operations: OPS_ORB_FAST4,
                },
            ),
            (
                String::from("orb_fast5"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_run1",
                    operations: OPS_ORB_FAST5,
                },
            ),
            (
                String::from("orb_pain1"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_pain2",
                    operations: OPS_ORB_PAIN1,
                },
            ),
            (
                String::from("orb_pain2"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_pain3",
                    operations: OPS_ORB_PAIN2,
                },
            ),
            (
                String::from("orb_pain3"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_pain4",
                    operations: &[],
                },
            ),
            (
                String::from("orb_pain4"),
                MonsterFrame {
                    frame: 0,
                    next: "orb_run1",
                    operations: &[],
                },
            ),
            (
                String::from("orb_death1"),
                MonsterFrame {
                    frame: 1,
                    next: "orb_death2",
                    operations: OPS_ORB_DEATH1,
                },
            ),
            (
                String::from("orb_death2"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_death3",
                    operations: &[],
                },
            ),
            (
                String::from("orb_death3"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_death4",
                    operations: OPS_ORB_DEATH3,
                },
            ),
            (
                String::from("orb_death4"),
                MonsterFrame {
                    frame: 2,
                    next: "orb_death4",
                    operations: OPS_ORB_DEATH4,
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
        assert_eq!(frames().len(), 17);
        for (name, frame) in frames() {
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
