//! Hipnotic hipdecoy frames (src/content/q1/missionpacks/monsters/tables/hipdecoy.ts).
//!
//! quakec_hipnotic/hipdecoy.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_DECOY_STAND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipdecoy:decoy_stand1",
}];
static OPS_DECOY_WALK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipdecoy:decoy_walk1",
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "decoy_stand1",
        MonsterFrame {
            frame: 17,
            next: "decoy_stand1",
            operations: OPS_DECOY_STAND1,
        },
    ),
    (
        "decoy_walk1",
        MonsterFrame {
            frame: 6,
            next: "decoy_walk1",
            operations: OPS_DECOY_WALK1,
        },
    ),
];

/// Look up a hipdecoy frame by name.
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
        assert_eq!(FRAMES.len(), 2);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("decoy_stand1").expect("first frame");
        assert_eq!((head.frame, head.next), (17, "decoy_stand1"));
        let tail = frame("decoy_walk1").expect("last frame");
        assert_eq!((tail.frame, tail.next), (6, "decoy_walk1"));
        assert!(frame("no_such_frame").is_none());
    }
}
