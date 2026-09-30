//! Q1 oracle cases (donor `tools/reference/q1/cases.ts`).
//!
//! Pinned input/expected pairs with derivations. Inputs run through the
//! oracle parser before evaluation, so decimal literals normalize exactly
//! like the donor (`storedFloat`).

use crate::json::Json;
use crate::reference::q1::oracle::{CallbackEntry, CallbackGlobals, Q1Input, Q1Output, ScalarOperation, ScalarOperator, TargetCallback, ThinkEffect};

/// A pinned oracle case.
#[derive(Debug, Clone)]
pub struct Q1Case {
    /// Stable case identifier.
    pub id: &'static str,
    /// Backing source pin identifiers.
    pub sources: &'static [&'static str],
    /// Why this input pins this output.
    pub derivation: &'static str,
    /// Oracle input.
    pub input: Q1Input,
    /// Expected oracle output.
    pub expected: Q1Output,
}

impl Q1Case {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(self.id)),
            ("sources".to_owned(), Json::array(self.sources.iter().map(|source| Json::string(*source)).collect())),
            ("derivation".to_owned(), Json::string(self.derivation)),
            ("input".to_owned(), self.input.to_json()),
            ("expected".to_owned(), self.expected.to_json()),
        ])
    }
}

const THINK_SOURCES: [&str; 4] = ["quake-sv-phys", "quake-prog-fields", "quake-server-clock", "quake-host-clock"];
const SCALAR_SOURCES: [&str; 2] = ["quake-pr-exec", "quake-eval-type"];
const MG1_SOURCES: [&str; 2] = ["mg1-hub", "mg1-sigils"];
const MG3_SOURCES: [&str; 3] = ["mg3-counter", "mg3-defs", "mg3-subs"];
const REMOVE_SOURCES: [&str; 6] =
    ["quake-sv-phys", "quake-prog-fields", "quake-server-clock", "quake-host-clock", "quake-remove-builtin", "quake-free-edict"];

const INITIAL_GLOBALS: CallbackGlobals = CallbackGlobals { time: 7.0, self_entity: 99, other_entity: 98 };

fn op(operator: ScalarOperator, operand: f64) -> ScalarOperation {
    ScalarOperation { operator, operand }
}

fn think_output(
    ran: bool,
    continue_physics: bool,
    next_think: f64,
    free: bool,
    globals: CallbackGlobals,
    entry: Option<CallbackEntry>,
) -> Q1Output {
    Q1Output::RunThink { ran, continue_physics, next_think, free, globals, callback_entry: entry }
}

fn entry(time: f64, self_entity: i64) -> CallbackEntry {
    CallbackEntry { time, self_entity, other_entity: 0 }
}

fn entry_globals(time: f64, self_entity: i64) -> CallbackGlobals {
    CallbackGlobals { time, self_entity, other_entity: 0 }
}

/// Pinned Q1 oracle cases.
#[must_use]
pub fn q1_cases() -> Vec<Q1Case> {
    vec![
        Q1Case {
            id: "q1.scalar.store-after-each-op",
            sources: &SCALAR_SOURCES,
            derivation: "At 2^24 binary32 spacing is 2. 2^24+1 is halfway and rounds to even 2^24 at the OP_ADD_F store; subtracting 2^24 then stores zero. Keeping a binary64 intermediate would incorrectly produce one.",
            input: Q1Input::ScalarProgram {
                initial: 16_777_216.0,
                operations: vec![op(ScalarOperator::Add, 1.0), op(ScalarOperator::Subtract, 16_777_216.0)],
            },
            expected: Q1Output::ScalarProgram {
                values: vec![16_777_216.0, 16_777_216.0, 0.0],
                bits: ["4b800000", "4b800000", "00000000"].into_iter().map(str::to_owned).collect(),
            },
        },
        Q1Case {
            id: "q1.scalar.ties-round-up-to-even",
            sources: &SCALAR_SOURCES,
            derivation: "16777218 has an odd low significand bit. Adding one reaches the midpoint and rounds to the even 16777220.",
            input: Q1Input::ScalarProgram { initial: 16_777_218.0, operations: vec![op(ScalarOperator::Add, 1.0)] },
            expected: Q1Output::ScalarProgram {
                values: vec![16_777_218.0, 16_777_220.0],
                bits: ["4b800001", "4b800002"].into_iter().map(str::to_owned).collect(),
            },
        },
        Q1Case {
            id: "q1.scalar.divide-multiply-stores",
            sources: &SCALAR_SOURCES,
            derivation: "1/10 stores binary32 bits 0x3dcccccd, exactly 0.10000000149011612. Multiplying the stored value by 10 gives 1.0000000149011612, which rounds to 1 at the next store.",
            input: Q1Input::ScalarProgram {
                initial: 1.0,
                operations: vec![op(ScalarOperator::Divide, 10.0), op(ScalarOperator::Multiply, 10.0)],
            },
            expected: Q1Output::ScalarProgram {
                values: vec![1.0, 0.10000000149011612, 1.0],
                bits: ["3f800000", "3dcccccd", "3f800000"].into_iter().map(str::to_owned).collect(),
            },
        },
        Q1Case {
            id: "q1.scalar.bitwise-truncates-fractions",
            sources: &SCALAR_SOURCES,
            derivation: "C conversion truncates 15.75 to signed integer 15. 15 & 31 is 15; 15 | 16 is 31. Each result is stored as float.",
            input: Q1Input::ScalarProgram {
                initial: 15.75,
                operations: vec![op(ScalarOperator::BitAnd, 31.0), op(ScalarOperator::BitOr, 16.0)],
            },
            expected: Q1Output::ScalarProgram {
                values: vec![15.75, 15.0, 31.0],
                bits: ["417c0000", "41700000", "41f80000"].into_iter().map(str::to_owned).collect(),
            },
        },
        Q1Case {
            id: "q1.think.zero-is-disabled",
            sources: &THINK_SOURCES,
            derivation: "The thinktime <= 0 guard returns true without clearing state or executing a callback.",
            input: Q1Input::RunThink {
                server_time: 10.0,
                frame_time: 0.125,
                next_think: 0.0,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Remove,
            },
            expected: think_output(false, true, 0.0, false, INITIAL_GLOBALS, None),
        },
        Q1Case {
            id: "q1.think.negative-is-disabled",
            sources: &THINK_SOURCES,
            derivation: "A negative nextthink also satisfies the disabled guard and leaves callback globals unchanged.",
            input: Q1Input::RunThink {
                server_time: 10.0,
                frame_time: 0.125,
                next_think: -1.0,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Remove,
            },
            expected: think_output(false, true, -1.0, false, INITIAL_GLOBALS, None),
        },
        Q1Case {
            id: "q1.think.inclusive-frame-end",
            sources: &THINK_SOURCES,
            derivation: "10.125 equals the double frame endpoint 10+0.125. Only greater times are skipped. The callback sees nextthink zero, its entity as self, world as other and the due time.",
            input: Q1Input::RunThink {
                server_time: 10.0,
                frame_time: 0.125,
                next_think: 10.125,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Retain,
            },
            expected: think_output(true, true, 0.0, false, entry_globals(10.125, 3), Some(entry(10.125, 3))),
        },
        Q1Case {
            id: "q1.think.next-float-after-end",
            sources: &THINK_SOURCES,
            derivation: "The next binary32 value after 10.125 is 10.125000953674316. It is greater than the frame endpoint and must remain pending.",
            input: Q1Input::RunThink {
                server_time: 10.0,
                frame_time: 0.125,
                next_think: 10.125000953674316,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Remove,
            },
            expected: think_output(false, true, 10.125000953674316, false, INITIAL_GLOBALS, None),
        },
        Q1Case {
            id: "q1.think.double-endpoint",
            sources: &THINK_SOURCES,
            derivation: "The stored binary32 nextthink for decimal 0.1 is 0.10000000149011612. It exceeds the binary64 endpoint 0+0.1, so it is not due. Rounding the endpoint to binary32 would change this branch.",
            input: Q1Input::RunThink {
                server_time: 0.0,
                frame_time: 0.1,
                next_think: 0.1,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Retain,
            },
            expected: think_output(false, true, 0.10000000149011612, false, INITIAL_GLOBALS, None),
        },
        Q1Case {
            id: "q1.think.overdue-clamps-through-float",
            sources: &THINK_SOURCES,
            derivation: "The overdue value 9 is clamped by assigning double sv.time=10.1 to float thinktime, yielding exactly 10.100000381469727. Callback globals remain installed after return.",
            input: Q1Input::RunThink {
                server_time: 10.1,
                frame_time: 0.01,
                next_think: 9.0,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Retain,
            },
            expected: think_output(true, true, 0.0, false, entry_globals(10.100000381469727, 3), Some(entry(10.100000381469727, 3))),
        },
        Q1Case {
            id: "q1.think.removal-stops-physics",
            sources: &REMOVE_SOURCES,
            derivation: "The callback sees nextthink zero. Its remove(self) calls PF_Remove then ED_Free, setting free and nextthink=-1. SV_RunThink returns !free, therefore false.",
            input: Q1Input::RunThink {
                server_time: 10.0,
                frame_time: 0.125,
                next_think: 10.0,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Remove,
            },
            expected: think_output(true, false, -1.0, true, entry_globals(10.0, 3), Some(entry(10.0, 3))),
        },
        Q1Case {
            id: "q1.think.reschedule-runs-once",
            sources: &THINK_SOURCES,
            derivation: "The callback sees nextthink cleared, then reschedules to 10.0625 inside the same frame. SV_RunThink has no loop, so it returns after one invocation and retains the new schedule.",
            input: Q1Input::RunThink {
                server_time: 10.0,
                frame_time: 0.125,
                next_think: 10.0,
                entity: 3,
                globals: INITIAL_GLOBALS,
                effect: ThinkEffect::Reschedule { next_think: 10.0625 },
            },
            expected: think_output(true, true, 10.0625, false, entry_globals(10.0, 3), Some(entry(10.0, 3))),
        },
        Q1Case {
            id: "q1.mg1.four-sigils-blocked",
            sources: &MG1_SOURCES,
            derivation: "SIGIL_ALL=1|2|4|8|16=31. Flags 15 omit E5, so the entity removes itself and never calls trigger_changelevel.",
            input: Q1Input::Mg1Hub { server_flags: 15 },
            expected: Q1Output::Mg1Hub { present_mask: 15, calls: vec!["remove(self)".to_owned()] },
        },
        Q1Case {
            id: "q1.mg1.five-sigils-open",
            sources: &MG1_SOURCES,
            derivation: "31 & 31 equals 31, so the wrapper invokes trigger_changelevel once.",
            input: Q1Input::Mg1Hub { server_flags: 31 },
            expected: Q1Output::Mg1Hub { present_mask: 31, calls: vec!["trigger_changelevel()".to_owned()] },
        },
        Q1Case {
            id: "q1.mg1.sixth-bit-does-not-fill-missing-fifth",
            sources: &MG1_SOURCES,
            derivation: "Flags 47 contain E1..E4 and E6. E6 is absent from SIGIL_ALL, so 47 & 31=15 and the gate stays closed.",
            input: Q1Input::Mg1Hub { server_flags: 47 },
            expected: Q1Output::Mg1Hub { present_mask: 15, calls: vec!["remove(self)".to_owned()] },
        },
        Q1Case {
            id: "q1.mg1.extra-flags-preserve-open-gate",
            sources: &MG1_SOURCES,
            derivation: "63 includes all five required bits and the unrelated sixth bit. Masking gives 31 and opens the gate.",
            input: Q1Input::Mg1Hub { server_flags: 63 },
            expected: Q1Output::Mg1Hub { present_mask: 31, calls: vec!["trigger_changelevel()".to_owned()] },
        },
        Q1Case {
            id: "q1.mg3.default-count-below-threshold",
            sources: &MG3_SOURCES,
            derivation: "Spawn count zero is replaced by 2. Flags 1 contain one counted rune, so use leaves SUB_UseTargets uncalled.",
            input: Q1Input::Mg3Counter { server_flags: 1, count: 0.0, coop: false, spawn_flags: 0, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter { removed: false, count: 2.0, use_callback: Some("rune_counter_use".to_owned()), runes: Some(1), callback: None },
        },
        Q1Case {
            id: "q1.mg3.exact-threshold-calls-targets",
            sources: &MG3_SOURCES,
            derivation: "Flags 5 contain E1 and E3, exactly two runes. 2>=2 invokes SUB_UseTargets with the same self and activator.",
            input: Q1Input::Mg3Counter { server_flags: 5, count: 2.0, coop: false, spawn_flags: 0, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter {
                removed: false,
                count: 2.0,
                use_callback: Some("rune_counter_use".to_owned()),
                runes: Some(2),
                callback: Some(TargetCallback { self_entity: 40, activator: 2 }),
            },
        },
        Q1Case {
            id: "q1.mg3.fifth-sigil-is-not-counted",
            sources: &MG3_SOURCES,
            derivation: "Flags 17 contain E1 and E5. The four source conditions count only E1, so the two-rune threshold is not reached.",
            input: Q1Input::Mg3Counter { server_flags: 17, count: 2.0, coop: false, spawn_flags: 0, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter { removed: false, count: 2.0, use_callback: Some("rune_counter_use".to_owned()), runes: Some(1), callback: None },
        },
        Q1Case {
            id: "q1.mg3.fractional-threshold-is-preserved",
            sources: &MG3_SOURCES,
            derivation: "Count 2.5 is nonzero and stays fractional. Two runes do not satisfy 2>=2.5.",
            input: Q1Input::Mg3Counter { server_flags: 3, count: 2.5, coop: false, spawn_flags: 0, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter { removed: false, count: 2.5, use_callback: Some("rune_counter_use".to_owned()), runes: Some(2), callback: None },
        },
        Q1Case {
            id: "q1.mg3.negative-count-is-not-defaulted",
            sources: &MG3_SOURCES,
            derivation: "Only a zero count selects the default. Count -1 is retained and zero runes satisfy 0>=-1.",
            input: Q1Input::Mg3Counter { server_flags: 0, count: -1.0, coop: false, spawn_flags: 0, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter {
                removed: false,
                count: -1.0,
                use_callback: Some("rune_counter_use".to_owned()),
                runes: Some(0),
                callback: Some(TargetCallback { self_entity: 40, activator: 2 }),
            },
        },
        Q1Case {
            id: "q1.mg3.inhibited-in-coop",
            sources: &MG3_SOURCES,
            derivation: "NOT_IN_COOP is 131072. In coop its macro removes self and returns before count default or callback installation.",
            input: Q1Input::Mg3Counter { server_flags: 15, count: 0.0, coop: true, spawn_flags: 131_072, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter { removed: true, count: 0.0, use_callback: None, runes: None, callback: None },
        },
        Q1Case {
            id: "q1.mg3.coop-only-inhibited-outside-coop",
            sources: &MG3_SOURCES,
            derivation: "COOP_ONLY is 32768. RemovedOutsideCoop removes self and returns before count default or callback installation.",
            input: Q1Input::Mg3Counter { server_flags: 15, count: 0.0, coop: false, spawn_flags: 32_768, entity: 40, activator: 2 },
            expected: Q1Output::Mg3Counter { removed: true, count: 0.0, use_callback: None, runes: None, callback: None },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::q1::oracle::run_q1_oracle;

    #[test]
    fn all_cases_evaluate_to_expected() {
        for case in q1_cases() {
            let actual = run_q1_oracle(&case.input.to_json()).unwrap_or_else(|error| panic!("{}: {error}", case.id));
            assert_eq!(actual.to_json(), case.expected.to_json(), "{}", case.id);
        }
    }
}
