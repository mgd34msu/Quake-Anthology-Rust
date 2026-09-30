//! Q1 mg3 infected frames (`src/content/q1/addons/monsters/infected/frames.ts`).
//!
//! `quakec_mg3/monsters/{soldier,mg3_hknight_infected}.qc`.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

/// Append one `<name><n>` sequence (`sequence`). Additions precede the
/// steering step, unlike the demodog builder.
fn sequence(
    frames: &mut HashMap<String, MonsterFrame>,
    name: &str,
    first: i32,
    distances: &[f64],
    ai: Option<MonsterAi>,
    last: &str,
    additions: &HashMap<usize, Vec<MonsterOperation>>,
) {
    for (index, distance) in distances.iter().enumerate() {
        let mut operations = Vec::new();
        if let Some(extra) = additions.get(&(index + 1)) {
            operations.extend_from_slice(extra);
        }
        if let Some(ai) = ai {
            operations.push(MonsterOperation::Ai {
                mode: ai,
                distance: *distance,
            });
        }
        let leaked: &'static [MonsterOperation] = Box::leak(operations.into_boxed_slice());
        let next = if index + 1 < distances.len() {
            format!("{name}{}", index + 2)
        } else {
            last.to_string()
        };
        frames.insert(
            format!("{name}{}", index + 1),
            MonsterFrame {
                frame: first + index as i32,
                next: Box::leak(next.into_boxed_str()),
                operations: leaked,
            },
        );
    }
}

fn action(name: &'static str) -> MonsterOperation {
    MonsterOperation::Action { name }
}

fn step(mode: MonsterAi, distance: f64) -> MonsterOperation {
    MonsterOperation::Ai { mode, distance }
}

fn insert(
    frames: &mut HashMap<String, MonsterFrame>,
    name: String,
    frame: i32,
    next: String,
    operations: Vec<MonsterOperation>,
) {
    frames.insert(
        name,
        MonsterFrame {
            frame,
            next: Box::leak(next.into_boxed_str()),
            operations: Box::leak(operations.into_boxed_slice()),
        },
    );
}

/// Append one corpse resurrection chain (`rise`).
fn rise(frames: &mut HashMap<String, MonsterFrame>, corpse: u32, poses: &[i32], distances: &HashMap<usize, f64>) {
    for (index, pose) in poses.iter().enumerate() {
        let frame_step = index + 1;
        let mut operations = Vec::new();
        if frame_step == 2 {
            operations.push(action("infected_rise_pain"));
        }
        if let Some(distance) = distances.get(&frame_step) {
            operations.push(step(MonsterAi::Forward, *distance));
        }
        let next = if frame_step == poses.len() {
            operations.push(action("infected_resurrect"));
            String::from("hknight_run1")
        } else {
            format!("hknight_corpse{corpse}_rise{}", frame_step + 1)
        };
        insert(
            frames,
            format!("hknight_corpse{corpse}_rise{frame_step}"),
            *pose,
            next,
            operations,
        );
    }
}

fn build() -> HashMap<String, MonsterFrame> {
    let mut frames = HashMap::new();
    let idle = MonsterOperation::Sound {
        path: "soldier/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    };
    sequence(
        &mut frames,
        "army_stand",
        0,
        &[0.0; 8],
        Some(MonsterAi::Stand),
        "army_stand1",
        &HashMap::new(),
    );
    let walk_distances = [
        1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 4.0, 4.0, 2.0, 2.0, 2.0, 1.0, 0.0, 1.0, 1.0, 1.0, 3.0, 3.0, 3.0, 3.0, 2.0, 1.0,
        1.0, 1.0,
    ];
    sequence(
        &mut frames,
        "army_walk",
        90,
        &walk_distances,
        Some(MonsterAi::Walk),
        "army_walk1",
        &HashMap::from([(1, vec![idle])]),
    );
    sequence(
        &mut frames,
        "army_run",
        73,
        &[11.0, 15.0, 10.0, 10.0, 8.0, 15.0, 10.0, 8.0],
        Some(MonsterAi::Run),
        "army_run1",
        &HashMap::from([(1, vec![idle])]),
    );
    let mut attack = HashMap::new();
    for index in 1..=9 {
        let mut operations = vec![step(MonsterAi::Face, 0.0)];
        if index == 5 {
            operations.push(action("army_fire"));
        } else if index == 7 {
            operations.push(action("army_refire"));
        }
        attack.insert(index, operations);
    }
    sequence(&mut frames, "army_atk", 81, &[0.0; 9], None, "army_run1", &attack);
    sequence(
        &mut frames,
        "army_pain",
        40,
        &[0.0; 6],
        None,
        "army_run1",
        &HashMap::from([(6, vec![step(MonsterAi::Pain, 1.0)])]),
    );
    sequence(
        &mut frames,
        "army_painb",
        46,
        &[0.0; 14],
        None,
        "army_run1",
        &HashMap::from([
            (2, vec![step(MonsterAi::Painforward, 13.0)]),
            (3, vec![step(MonsterAi::Painforward, 9.0)]),
            (12, vec![step(MonsterAi::Pain, 2.0)]),
        ]),
    );
    sequence(
        &mut frames,
        "army_painc",
        60,
        &[0.0; 13],
        None,
        "army_run1",
        &HashMap::from([
            (2, vec![step(MonsterAi::Pain, 1.0)]),
            (5, vec![step(MonsterAi::Painforward, 1.0)]),
            (6, vec![step(MonsterAi::Painforward, 1.0)]),
            (8, vec![step(MonsterAi::Pain, 1.0)]),
            (9, vec![step(MonsterAi::Painforward, 4.0)]),
            (10, vec![step(MonsterAi::Painforward, 3.0)]),
            (11, vec![step(MonsterAi::Painforward, 6.0)]),
            (12, vec![step(MonsterAi::Painforward, 8.0)]),
        ]),
    );
    for (corpse, pose) in [(1, 53), (2, 62)] {
        insert(
            &mut frames,
            format!("hknight_corpse{corpse}"),
            pose,
            format!("hknight_corpse{corpse}_2"),
            vec![MonsterOperation::Solid { solid: Q1Solid::None }],
        );
        insert(
            &mut frames,
            format!("hknight_corpse{corpse}_2"),
            pose,
            format!("hknight_corpse{corpse}_2"),
            vec![action("infected_corpse_hold")],
        );
        insert(
            &mut frames,
            format!("hknight_corpse{corpse}_rise0"),
            pose,
            format!("hknight_corpse{corpse}_rise{}", if corpse == 1 { 1 } else { 2 }),
            vec![action("infected_test_rise")],
        );
    }
    rise(
        &mut frames,
        1,
        &[53, 52, 51, 50, 49, 48, 47, 46, 45, 44, 43, 42, 0, 1],
        &HashMap::from([(4, -11.0), (5, -10.0), (10, -7.0), (11, -8.0), (14, -10.0)]),
    );
    rise(
        &mut frames,
        2,
        &[62, 61, 60, 59, 58, 57, 56, 55, 55, 0, 1],
        &HashMap::new(),
    );
    frames
}

/// Mg3 infected frames (`infectedFrames`).
pub fn infected_frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuations_resolve() {
        let frames = infected_frames();
        for (name, frame) in frames {
            if frame.next == "hknight_run1" {
                continue;
            }
            assert!(frames.contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
        assert_eq!(&frames["army_atk5"].operations[1], &action("army_fire"));
        assert_eq!(&frames["army_atk7"].operations[1], &action("army_refire"));
        assert_eq!(frames["hknight_corpse1_rise14"].next, "hknight_run1");
    }
}
