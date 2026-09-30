//! Q1 mg3 demodog frames (`src/content/q1/addons/monsters/demodog-frames.ts`).
//!
//! `quakec_mg3/monsters/mg3_demodog.qc` frame declarations.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::Q1SoundChannel;

/// Append one `demodog_<name><n>` sequence (`sequence`).
#[allow(clippy::too_many_arguments)]
fn sequence(
    frames: &mut HashMap<String, MonsterFrame>,
    name: &str,
    first: i32,
    distances: &[f64],
    ai: Option<MonsterAi>,
    last: &str,
    actions: &HashMap<usize, Vec<MonsterOperation>>,
) {
    for (index, distance) in distances.iter().enumerate() {
        let mut operations = Vec::new();
        if let Some(ai) = ai {
            operations.push(MonsterOperation::Ai {
                mode: ai,
                distance: *distance,
            });
        }
        if let Some(extra) = actions.get(&(index + 1)) {
            operations.extend_from_slice(extra);
        }
        let leaked: &'static [MonsterOperation] = Box::leak(operations.into_boxed_slice());
        let next = if index + 1 < distances.len() {
            format!("demodog_{name}{}", index + 2)
        } else {
            last.to_string()
        };
        frames.insert(
            format!("demodog_{name}{}", index + 1),
            MonsterFrame {
                frame: first + index as i32,
                next: Box::leak(next.into_boxed_str()),
                operations: leaked,
            },
        );
    }
}

fn ai(mode: MonsterAi, distance: f64) -> MonsterOperation {
    MonsterOperation::Ai { mode, distance }
}

fn action(name: &'static str) -> MonsterOperation {
    MonsterOperation::Action { name }
}

fn sound(path: &'static str, attenuation: f64, chance: Option<f64>) -> MonsterOperation {
    MonsterOperation::Sound {
        path,
        channel: Q1SoundChannel::Voice,
        attenuation,
        comparison: SoundComparison::Less,
        chance,
    }
}

fn build() -> HashMap<String, MonsterFrame> {
    let mut frames = HashMap::new();
    let idle = sound("dog/idle.wav", 2.0, Some(0.2));
    sequence(
        &mut frames,
        "stand",
        69,
        &[0.0; 9],
        Some(MonsterAi::Stand),
        "demodog_stand1",
        &HashMap::new(),
    );
    let mut walk = HashMap::new();
    for index in 1..=8 {
        walk.insert(
            index,
            if index == 1 {
                vec![idle, ai(MonsterAi::Walk, 8.0)]
            } else {
                vec![ai(MonsterAi::Walk, 8.0)]
            },
        );
    }
    sequence(&mut frames, "walk", 78, &[8.0; 8], None, "demodog_walk1", &walk);
    let run = [16.0, 32.0, 32.0, 20.0, 64.0, 32.0, 16.0, 32.0, 32.0, 20.0, 64.0, 32.0];
    let mut run_actions = HashMap::new();
    for (index, distance) in run.iter().enumerate() {
        run_actions.insert(
            index + 1,
            if index == 0 {
                vec![idle, ai(MonsterAi::Run, *distance)]
            } else {
                vec![ai(MonsterAi::Run, *distance)]
            },
        );
    }
    sequence(&mut frames, "run", 48, &run, None, "demodog_run1", &run_actions);
    let mut attack = HashMap::new();
    for index in 1..=8 {
        attack.insert(
            index,
            if index == 4 {
                vec![sound("dog/dattack1.wav", 1.0, None), action("demodog_bite")]
            } else {
                vec![ai(MonsterAi::Charge, 10.0)]
            },
        );
    }
    sequence(&mut frames, "atta", 0, &[10.0; 8], None, "demodog_run1", &attack);
    let leap = HashMap::from([
        (1, vec![ai(MonsterAi::Face, 0.0)]),
        (2, vec![ai(MonsterAi::Face, 0.0), action("demodog_jump")]),
    ]);
    sequence(&mut frames, "leap", 60, &[0.0; 9], None, "demodog_leap9", &leap);
    sequence(
        &mut frames,
        "pain",
        26,
        &[0.0; 6],
        None,
        "demodog_run1",
        &HashMap::new(),
    );
    let painb = HashMap::from([
        (3, vec![ai(MonsterAi::Pain, 4.0)]),
        (4, vec![ai(MonsterAi::Pain, 12.0)]),
        (5, vec![ai(MonsterAi::Pain, 12.0)]),
        (6, vec![ai(MonsterAi::Pain, 2.0)]),
        (8, vec![ai(MonsterAi::Pain, 4.0)]),
        (10, vec![ai(MonsterAi::Pain, 10.0)]),
    ]);
    sequence(&mut frames, "painb", 32, &[0.0; 16], None, "demodog_run1", &painb);
    sequence(&mut frames, "die", 8, &[0.0; 9], None, "demodog_die9", &HashMap::new());
    sequence(
        &mut frames,
        "dieb",
        17,
        &[0.0; 9],
        None,
        "demodog_dieb9",
        &HashMap::new(),
    );
    frames
}

/// Mg3 demodog frames (`demodogFrames`).
pub fn demodog_frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuations_resolve() {
        assert_eq!(demodog_frames().len(), 9 + 8 + 12 + 8 + 9 + 6 + 16 + 9 + 9);
        for (name, frame) in demodog_frames() {
            assert!(
                demodog_frames().contains_key(frame.next),
                "dangling {name} -> {}",
                frame.next
            );
        }
        let stand = &demodog_frames()["demodog_stand1"];
        assert_eq!((stand.frame, stand.next), (69, "demodog_stand2"));
        let bite = &demodog_frames()["demodog_atta4"];
        assert!(bite.operations.contains(&action("demodog_bite")));
    }
}
