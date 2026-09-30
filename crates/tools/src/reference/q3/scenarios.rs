//! Q3 source-derived scenarios (donor `tools/reference/q3/scenarios.ts`).
//!
//! Eight scenarios evaluate the [`crate::reference::q3::semantics`] operations
//! over fixed fixture inputs and check the results against separately
//! written literal expectations. Fixture inputs are typed values rendered
//! into the record; expectations are JSON literals in donor key order.

use crate::json::{deep_strict_equal, parse_json, Json};
use crate::reference::q3::semantics::{
    client_connect, clip_velocity, drop_timers, nested_vm, single_clock, subdivide_move, weapon_sequence, Ammo,
    ClipInput, ConnectInput, MoveInput, PriorVm, Subdivision, TimerInput, Weapon, WeaponInput, WeaponState, WeaponStep,
};

/// One checked expectation.
#[derive(Debug, Clone)]
pub struct SourceAssertion {
    /// Stable assertion identifier.
    pub id: &'static str,
    /// Why the expectation holds.
    pub derivation: &'static str,
    /// Literal expectation.
    pub expected: Json,
    /// Evaluated value.
    pub actual: Json,
    /// Whether they agree.
    pub passed: bool,
}

impl SourceAssertion {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(self.id)),
            ("derivation".to_owned(), Json::string(self.derivation)),
            ("expected".to_owned(), self.expected.clone()),
            ("actual".to_owned(), self.actual.clone()),
            ("passed".to_owned(), Json::boolean(self.passed)),
        ])
    }
}

/// One source-derived scenario.
#[derive(Debug, Clone)]
pub struct SourceScenario {
    /// Stable scenario identifier.
    pub id: &'static str,
    /// Pinned original-source locations.
    pub source_locations: Vec<String>,
    /// Fixture assumptions.
    pub assumptions: Vec<String>,
    /// Fixture input record.
    pub input: Json,
    /// Evaluated output record.
    pub output: Json,
    /// Checked assertions.
    pub assertions: Vec<SourceAssertion>,
}

impl SourceScenario {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(self.id)),
            (
                "sourceLocations".to_owned(),
                Json::array(self.source_locations.iter().map(Json::string).collect()),
            ),
            (
                "assumptions".to_owned(),
                Json::array(self.assumptions.iter().map(Json::string).collect()),
            ),
            ("input".to_owned(), self.input.clone()),
            ("output".to_owned(), self.output.clone()),
            (
                "assertions".to_owned(),
                Json::array(self.assertions.iter().map(SourceAssertion::to_json).collect()),
            ),
        ])
    }
}

fn check(id: &'static str, derivation: &'static str, expected: Json, actual: Json) -> SourceAssertion {
    let passed = deep_strict_equal(&expected, &actual);
    SourceAssertion {
        id,
        derivation,
        expected,
        actual,
        passed,
    }
}

/// Parse a literal expectation (fixed scenario data).
fn literal(text: &str) -> Json {
    parse_json(text).expect("valid scenario expectation")
}

fn numeric_scenario() -> SourceScenario {
    let entering = ClipInput {
        velocity: [-100.0, 2.0, 3.0],
        normal: [1.0, 0.0, 0.0],
        overbounce: 1.125,
    };
    let leaving = ClipInput {
        velocity: [100.0, 2.0, 3.0],
        normal: [1.0, 0.0, 0.0],
        overbounce: 1.25,
    };
    let cancellation = ClipInput {
        velocity: [16777216.0, 1.0, -16777216.0],
        normal: [1.0, 1.0, 1.0],
        overbounce: 1.0,
    };
    let clock_deltas = [-1.0, 0.0, 1.0, 125.0, 200.0, 201.0];
    let entered = clip_velocity(&entering);
    let left = clip_velocity(&leaving);
    let cancelled = clip_velocity(&cancellation);
    let clocks: Vec<Json> = clock_deltas
        .iter()
        .map(|delta| single_clock(100.0, 100.0 + delta).to_json())
        .collect();
    SourceScenario {
        id: "q3-numeric-source",
        source_locations: ["code/game/bg_pmove.c:145-162", "code/game/bg_pmove.c:1894-1910", "code/game/q_shared.h:611"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        assumptions: [
            "Float arithmetic uses IEEE-754 binary32 round-to-nearest ties-to-even at each float operation, with no excess precision or reassociation.",
            "The non-unit cancellation normal is an arithmetic probe of PM_ClipVelocity, not a collision-world normal.",
            "The 0.001 literal promotes multiplication to binary64 before assignment to pml.frametime rounds to binary32.",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        input: Json::object(vec![
            ("entering".to_owned(), entering.to_json()),
            ("leaving".to_owned(), leaving.to_json()),
            ("cancellation".to_owned(), cancellation.to_json()),
            ("clockDeltas".to_owned(), Json::array(clock_deltas.iter().map(|delta| Json::float(*delta)).collect())),
        ]),
        output: Json::object(vec![
            ("entering".to_owned(), entered.to_json()),
            ("leaving".to_owned(), left.to_json()),
            ("cancellation".to_owned(), cancelled.to_json()),
            ("clocks".to_owned(), Json::array(clocks.clone())),
        ]),
        assertions: vec![
            check(
                "entering",
                "Negative dot -100 is multiplied by 9/8 to -112.5; x minus backoff is 12.5. All values are exactly binary-representable.",
                literal(r#"{"backoff": -112.5, "velocity": [12.5, 2, 3]}"#),
                entered.to_json(),
            ),
            check(
                "leaving",
                "Positive dot 100 is divided by 5/4 to 80; x minus backoff is 20. All values are exactly representable.",
                literal(r#"{"backoff": 80, "velocity": [20, 2, 3]}"#),
                left.to_json(),
            ),
            check(
                "binary32-dot-staging",
                "At 2^24 the binary32 spacing is two; ties-to-even rounds 2^24+1 to 2^24 before subtracting 2^24, yielding zero.",
                literal(r#"{"backoff": 0, "velocity": [16777216, 1, -16777216]}"#),
                cancelled.to_json(),
            ),
            check(
                "single-clock-clamps",
                "PmoveSingle clamps local duration to [1,200] while assigning commandTime the original serverTime. 1/1000 and 1/5 round to the listed binary32 values; 1/8 is exact.",
                literal(
                    r#"[{"commandTime": 99, "msec": 1, "frametime": 0.0010000000474974513}, {"commandTime": 100, "msec": 1, "frametime": 0.0010000000474974513}, {"commandTime": 101, "msec": 1, "frametime": 0.0010000000474974513}, {"commandTime": 225, "msec": 125, "frametime": 0.125}, {"commandTime": 300, "msec": 200, "frametime": 0.20000000298023224}, {"commandTime": 301, "msec": 200, "frametime": 0.20000000298023224}]"#,
                ),
                Json::array(clocks),
            ),
        ],
    }
}

fn move_input(
    command_time: f64,
    server_time: f64,
    framecount: f64,
    subdivision: Subdivision,
    jump_held: bool,
    upmove: f64,
) -> MoveInput {
    MoveInput {
        command_time,
        server_time,
        framecount,
        subdivision,
        jump_held,
        upmove,
    }
}

fn movement_scenario() -> SourceScenario {
    let variable = move_input(0.0, 133.0, 63.0, Subdivision::Variable, true, 127.0);
    let fixed = move_input(100.0, 125.0, 7.0, Subdivision::Fixed { msec: 8.0 }, false, 0.0);
    let catchup = move_input(0.0, 1500.0, 63.0, Subdivision::Variable, false, 127.0);
    let equal = move_input(0.0, 0.0, 63.0, Subdivision::Variable, true, 127.0);
    let backwards = move_input(0.0, -1.0, 63.0, Subdivision::Variable, true, 127.0);
    let variable_out = subdivide_move(&variable).expect("variable subdivision");
    let fixed_out = subdivide_move(&fixed).expect("fixed subdivision");
    let catchup_out = subdivide_move(&catchup).expect("catchup subdivision");
    let equal_out = subdivide_move(&equal).expect("equal subdivision");
    let backwards_out = subdivide_move(&backwards).expect("backwards subdivision");
    let boundary_lengths: Vec<Json> = [65.0, 66.0, 67.0]
        .iter()
        .map(|server_time| {
            let output = subdivide_move(&move_input(0.0, *server_time, 63.0, Subdivision::Variable, true, 127.0))
                .expect("boundary subdivision");
            Json::array(output.steps.iter().map(|step| Json::float(step.msec)).collect())
        })
        .collect();
    let catchup_times = Json::array(
        catchup_out
            .steps
            .iter()
            .map(|step| Json::float(step.command_time))
            .collect(),
    );
    SourceScenario {
        id: "q3-pmove-subdivision-source",
        source_locations: ["code/game/bg_pmove.c:2026-2081", "code/game/q_shared.h:1136"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        assumptions: [
            "Signed time arithmetic stays within int32 range.",
            "PmoveSingle is reduced to its clock writes; PMF_JUMP_HELD is held at the stated value throughout. Movement and collision are outside this specimen.",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        input: Json::object(vec![
            ("variable".to_owned(), variable.to_json()),
            ("fixed".to_owned(), fixed.to_json()),
            ("catchup".to_owned(), catchup.to_json()),
            ("equal".to_owned(), equal.to_json()),
            ("backwards".to_owned(), backwards.to_json()),
            (
                "boundaryServerTimes".to_owned(),
                Json::array([65.0, 66.0, 67.0].iter().map(|time| Json::float(*time)).collect()),
            ),
        ]),
        output: Json::object(vec![
            ("variable".to_owned(), variable_out.to_json()),
            ("fixed".to_owned(), fixed_out.to_json()),
            ("catchup".to_owned(), catchup_out.to_json()),
            ("equal".to_owned(), equal_out.to_json()),
            ("backwards".to_owned(), backwards_out.to_json()),
            ("boundaryLengths".to_owned(), Json::array(boundary_lengths.clone())),
        ]),
        assertions: vec![
            check(
                "variable",
                "133=66+66+1; the six-bit frame count wraps 63 to 0; upmove becomes 20 only after the first PmoveSingle returns.",
                literal(
                    r#"{"commandTime": 133, "framecount": 0, "upmove": 20, "steps": [{"commandTime": 66, "msec": 66, "upmove": 127}, {"commandTime": 132, "msec": 66, "upmove": 20}, {"commandTime": 133, "msec": 1, "upmove": 20}]}"#,
                ),
                variable_out.to_json(),
            ),
            check(
                "fixed",
                "25=8+8+8+1; frame count increments once per Pmove, not once per subdivision.",
                literal(
                    r#"{"commandTime": 125, "framecount": 8, "upmove": 0, "steps": [{"commandTime": 108, "msec": 8, "upmove": 0}, {"commandTime": 116, "msec": 8, "upmove": 0}, {"commandTime": 124, "msec": 8, "upmove": 0}, {"commandTime": 125, "msec": 1, "upmove": 0}]}"#,
                ),
                fixed_out.to_json(),
            ),
            check(
                "catchup-times",
                "The 1500 ms gap resets commandTime to 500. The remaining 1000 ms consists of fifteen 66 ms steps and one 10 ms step.",
                literal(r#"[566, 632, 698, 764, 830, 896, 962, 1028, 1094, 1160, 1226, 1292, 1358, 1424, 1490, 1500]"#),
                catchup_times,
            ),
            check(
                "equal",
                "Equal time skips the loop but still increments and masks pmove_framecount.",
                literal(r#"{"commandTime": 0, "framecount": 0, "upmove": 127, "steps": []}"#),
                equal_out.to_json(),
            ),
            check(
                "backwards",
                "The backward-time return precedes both catch-up and frame-count mutation.",
                literal(r#"{"commandTime": 0, "framecount": 63, "upmove": 127, "steps": []}"#),
                backwards_out.to_json(),
            ),
            check(
                "subdivision-boundary",
                "The variable branch caps only durations greater than 66.",
                literal(r#"[[65], [66], [66, 1]]"#),
                Json::array(boundary_lengths),
            ),
        ],
    }
}

fn timer_scenario() -> SourceScenario {
    let input = TimerInput {
        msec: 0.0,
        pm_time: 66.0,
        flags: 355.0,
        legs_timer: 66.0,
        torso_timer: 67.0,
    };
    let output: Vec<Json> = [65.0, 66.0, 67.0]
        .iter()
        .map(|msec| drop_timers(&TimerInput { msec: *msec, ..input }).to_json())
        .collect();
    SourceScenario {
        id: "q3-timer-expiry-source",
        source_locations: ["code/game/bg_pmove.c:1762-1787", "code/game/bg_public.h:142-157"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        assumptions: ["The input flags are DUCKED|JUMP_HELD|TIME_LAND|TIME_KNOCKBACK|TIME_WATERJUMP, numerically 355."]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        input: Json::object(vec![
            ("pmTime".to_owned(), Json::float(66.0)),
            ("flags".to_owned(), Json::float(355.0)),
            ("legsTimer".to_owned(), Json::float(66.0)),
            ("torsoTimer".to_owned(), Json::float(67.0)),
            ("durations".to_owned(), Json::array([65.0, 66.0, 67.0].iter().map(|msec| Json::float(*msec)).collect())),
        ]),
        output: Json::array(output.clone()),
        assertions: vec![check(
            "expiry-minus-exact-plus",
            "PM_DropTimers clears all three time flags at msec>=pm_time and preserves DUCKED|JUMP_HELD=3. Animation timers independently clamp at zero.",
            literal(
                r#"[{"pmTime": 1, "flags": 355, "legsTimer": 1, "torsoTimer": 2}, {"pmTime": 0, "flags": 3, "legsTimer": 0, "torsoTimer": 1}, {"pmTime": 0, "flags": 3, "legsTimer": 0, "torsoTimer": 0}]"#,
            ),
            Json::array(output),
        )],
    }
}

fn weapon_step(msec: f64, weapon: Weapon, attack: bool, haste: bool) -> WeaponStep {
    WeaponStep {
        msec,
        weapon,
        attack,
        haste,
    }
}

fn firing_scenario() -> SourceScenario {
    let input = WeaponInput {
        weapon: Weapon::Machinegun,
        weapon_state: WeaponState::Firing,
        weapon_time: 30.0,
        torso_anim: 0.0,
        ammo: Ammo {
            machinegun: 2.0,
            rocket: 4.0,
            lightning: -1.0,
        },
        event_sequence: 0.0,
        steps: vec![
            weapon_step(66.0, Weapon::Machinegun, true, false),
            weapon_step(66.0, Weapon::Machinegun, true, false),
            weapon_step(100.0, Weapon::Machinegun, true, false),
        ],
    };
    let output = weapon_sequence(&input);
    let state_rows = Json::array(
        output
            .states
            .iter()
            .map(|state| {
                Json::array(vec![
                    Json::float(state.weapon_time),
                    Json::float(state.ammo.machinegun),
                    Json::string(state.weapon_state.as_str()),
                    Json::float(state.torso_anim),
                ])
            })
            .collect(),
    );
    let event_rows = Json::array(
        output
            .events
            .iter()
            .map(|event| {
                Json::array(vec![
                    Json::float(event.event),
                    Json::float(event.index),
                    Json::float(event.state.event_sequence),
                    Json::float(event.state.ammo.machinegun),
                    Json::float(event.state.weapon_time),
                ])
            })
            .collect(),
    );
    let ring_record = Json::object(vec![
        (
            "ring".to_owned(),
            Json::array(output.ring.iter().map(|slot| Json::float(*slot)).collect()),
        ),
        (
            "sequence".to_owned(),
            output
                .states
                .last()
                .map_or(Json::Null, |state| Json::float(state.event_sequence)),
        ),
    ]);
    SourceScenario {
        id: "q3-weapon-ammo-events-source",
        source_locations: [
            "code/game/bg_pmove.c:58-60",
            "code/game/bg_pmove.c:1538-1706",
            "code/game/bg_misc.c:1390-1408",
            "code/game/bg_public.h:347-378",
            "code/game/q_shared.h:1134",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        assumptions: [
            "Live non-spectator player, no respawn flag or holdable use, owned machinegun, no mission pack; fixture steps are direct PM_Weapon durations.",
            "No unrelated events occur. The initial two-slot event ring and event parameters are zero.",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        input: input.to_json(),
        output: output.to_json(),
        assertions: vec![
            check(
                "overshoot-ammo-state",
                "30-66+100=64, then 64-66+100=98, then 98-100+500=498. Ammo falls 2 to 1 to 0, and the third attempt emits NOAMMO while remaining FIRING.",
                literal(r#"[[64, 1, "firing", 135], [98, 0, "firing", 7], [498, 0, "firing", 135]]"#),
                state_rows,
            ),
            check(
                "event-observation",
                "Animation and FIRING state precede ammo consumption. FIRE_WEAPON is appended after consumption and before cooldown addition; NOAMMO precedes +500. Sequences use index sequence&1.",
                literal(r#"[[23, 0, 0, 1, -36], [23, 1, 1, 0, -2], [21, 0, 2, 0, -2]]"#),
                event_rows,
            ),
            check(
                "event-ring-wrap",
                "The third event replaces ring slot zero, while sequence grows to three.",
                literal(r#"{"ring": [21, 23], "sequence": 3}"#),
                ring_record,
            ),
            check(
                "event-parameters",
                "PM_AddEvent passes zero as the event parameter to BG_AddPredictableEventToPlayerstate for both FIRE_WEAPON and NOAMMO.",
                literal(r#"[0, 0, 0]"#),
                Json::array(output.events.iter().map(|event| Json::float(event.parm)).collect()),
            ),
            check(
                "causal-order",
                "Each shot restarts TORSO_ATTACK=7 by flipping ANIM_TOGGLEBIT=128. Ammo consumption is visible before the corresponding predictable event.",
                literal(
                    r#"["step:0", "animation:135", "ammo:machinegun:1", "event:23:sequence:0", "step:1", "animation:7", "ammo:machinegun:0", "event:23:sequence:1", "step:2", "animation:135", "event:21:sequence:2"]"#,
                ),
                Json::array(output.trace.iter().map(Json::string).collect()),
            ),
        ],
    }
}

fn switch_scenario() -> SourceScenario {
    let input = WeaponInput {
        weapon: Weapon::Machinegun,
        weapon_state: WeaponState::Ready,
        weapon_time: 0.0,
        torso_anim: 0.0,
        ammo: Ammo {
            machinegun: 1.0,
            rocket: 4.0,
            lightning: -1.0,
        },
        event_sequence: 0.0,
        steps: vec![
            weapon_step(1.0, Weapon::Rocket, true, false),
            weapon_step(199.0, Weapon::Rocket, true, false),
            weapon_step(1.0, Weapon::Rocket, true, false),
            weapon_step(249.0, Weapon::Rocket, true, false),
            weapon_step(1.0, Weapon::Rocket, true, false),
            weapon_step(1.0, Weapon::Rocket, true, false),
        ],
    };
    let output = weapon_sequence(&input);
    let state_rows = Json::array(
        output
            .states
            .iter()
            .map(|state| {
                Json::array(vec![
                    Json::string(state.weapon.as_str()),
                    Json::string(state.weapon_state.as_str()),
                    Json::float(state.weapon_time),
                    Json::float(state.ammo.rocket),
                    Json::float(state.torso_anim),
                ])
            })
            .collect(),
    );
    let event_rows = Json::array(
        output
            .events
            .iter()
            .map(|event| {
                Json::array(vec![
                    Json::float(event.event),
                    Json::string(event.state.weapon.as_str()),
                    Json::string(event.state.weapon_state.as_str()),
                    Json::float(event.state.weapon_time),
                    Json::float(event.state.ammo.rocket),
                ])
            })
            .collect(),
    );
    SourceScenario {
        id: "q3-weapon-switch-boundaries-source",
        source_locations: ["code/game/bg_pmove.c:1469-1510", "code/game/bg_pmove.c:1586-1688", "code/game/bg_pmove.c:95-101"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        assumptions: ["Live normal player; both weapons are owned. Attack remains held. Each input is a direct PM_Weapon call, so the 199/249 ms intervals are intentionally unsplit."]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        input: input.to_json(),
        output: output.to_json(),
        assertions: vec![
            check(
                "switch-state-boundaries",
                "Begin adds 200; the exact dropping deadline changes weapon and adds 250; the exact raising deadline returns READY without firing. Attack fires only on the following call.",
                literal(
                    r#"[["machinegun", "dropping", 200, 4, 137], ["machinegun", "dropping", 1, 4, 137], ["rocket", "raising", 250, 4, 10], ["rocket", "raising", 1, 4, 10], ["rocket", "ready", 0, 4, 139], ["rocket", "firing", 800, 3, 7]]"#,
                ),
                state_rows,
            ),
            check(
                "switch-event-order",
                "CHANGE_WEAPON is emitted while weaponstate is still READY and before the drop timer/animation writes; the next event is FIRE_WEAPON after rocket ammo is consumed.",
                literal(r#"[[22, "machinegun", "ready", 0, 4], [23, "rocket", "firing", 0, 3]]"#),
                event_rows,
            ),
        ],
    }
}

fn haste_scenario() -> SourceScenario {
    let input = WeaponInput {
        weapon: Weapon::Lightning,
        weapon_state: WeaponState::Ready,
        weapon_time: 0.0,
        torso_anim: 0.0,
        ammo: Ammo {
            machinegun: 1.0,
            rocket: 1.0,
            lightning: -1.0,
        },
        event_sequence: 1.0,
        steps: vec![weapon_step(1.0, Weapon::Lightning, true, true)],
    };
    let output = weapon_sequence(&input);
    let actual = Json::object(vec![
        (
            "time".to_owned(),
            output
                .states
                .last()
                .map_or(Json::Null, |state| Json::float(state.weapon_time)),
        ),
        (
            "ammo".to_owned(),
            output
                .states
                .last()
                .map_or(Json::Null, |state| Json::float(state.ammo.lightning)),
        ),
        (
            "ring".to_owned(),
            Json::array(output.ring.iter().map(|slot| Json::float(*slot)).collect()),
        ),
        (
            "trace".to_owned(),
            Json::array(output.trace.iter().map(Json::string).collect()),
        ),
    ]);
    SourceScenario {
        id: "q3-haste-infinite-ammo-source",
        source_locations: ["code/game/bg_pmove.c:1628-1706"].into_iter().map(str::to_owned).collect(),
        assumptions: ["Base-game haste is active; weapon is lightning; -1 denotes infinite ammunition; no holdable or respawn guard."]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        input: input.to_json(),
        output: output.to_json(),
        assertions: vec![check(
            "integer-haste-infinite",
            "C compound assignment addTime/=1.3 converts 50/1.3 back to int by truncation, yielding 38. Ammo -1 skips consumption, and sequence one uses slot one.",
            literal(r#"{"time": 38, "ammo": -1, "ring": [0, 23], "trace": ["step:0", "animation:135", "event:23:sequence:1"]}"#),
            actual,
        )],
    }
}

fn connect_input(
    banned: bool,
    ip: &str,
    configured_password: &str,
    provided_password: &str,
    existing_bot_flag: bool,
    prior_vm: Option<PriorVm>,
) -> ConnectInput {
    ConnectInput {
        banned,
        ip: ip.to_owned(),
        configured_password: configured_password.to_owned(),
        provided_password: provided_password.to_owned(),
        existing_bot_flag,
        prior_vm,
    }
}

fn connect_scenario() -> SourceScenario {
    let password = connect_input(false, "203.0.113.4", "secret", "bad", false, None);
    let banned = connect_input(true, "203.0.113.4", "secret", "bad", false, None);
    let local = connect_input(false, "localhost", "secret", "bad", false, Some(PriorVm::Ui));
    let none_password = connect_input(false, "203.0.113.4", "NoNe", "bad", false, None);
    let existing_bot = connect_input(false, "203.0.113.4", "secret", "bad", true, None);
    let password_out = client_connect(&password);
    let banned_out = client_connect(&banned);
    let local_out = client_connect(&local);
    let none_password_out = client_connect(&none_password);
    let existing_bot_out = client_connect(&existing_bot);
    SourceScenario {
        id: "q3-client-connect-immediate-source",
        source_locations: [
            "code/game/g_client.c:903-988",
            "code/game/g_main.c:203-213",
            "code/server/sv_client.c:423-440",
            "code/qcommon/vm.c:625-641",
            "code/qcommon/vm.c:668-710",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        assumptions: [
            "Direct-connect call arguments are firstTime=true and isBot=false. Non-team game; successful imported session/userinfo/rank operations are represented by their call sites.",
            "Denial string address is represented by synthetic nonzero QVM offset 32 in a 256-byte memory region; this is not an observed original address or a QVM interpreter test.",
            "G_FilterPacket outcome and userinfo lookup values are fixed inputs. Existing SVF_BOT is distinct from the isBot argument.",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        input: Json::object(vec![
            ("password".to_owned(), password.to_json()),
            ("banned".to_owned(), banned.to_json()),
            ("local".to_owned(), local.to_json()),
            ("nonePassword".to_owned(), none_password.to_json()),
            ("existingBot".to_owned(), existing_bot.to_json()),
        ]),
        output: Json::object(vec![
            ("password".to_owned(), password_out.to_json()),
            ("banned".to_owned(), banned_out.to_json()),
            ("local".to_owned(), local_out.to_json()),
            ("nonePassword".to_owned(), none_password_out.to_json()),
            ("existingBot".to_owned(), existing_bot_out.to_json()),
        ]),
        assertions: vec![
            check(
                "password-immediate-return",
                "ClientConnect returns Invalid password before initializing a client. vmMain forwards the nonzero return, VM_Call returns it synchronously, then SV_DirectConnect resolves and prints the denial and returns before connectResponse.",
                literal(
                    r#"{"returnValue": 32, "denial": "Invalid password", "serverState": "free", "currentVm": "game", "trace": ["VM_Call:game:enter", "vmMain:GAME_CLIENT_CONNECT", "trap_GetUserinfo", "G_FilterPacket", "password:check", "ClientConnect:return:32", "vmMain:return:32", "VM_Call:return:32", "VM_ExplicitArgPtr:game:32", "NET_OutOfBandPrint:print\nInvalid password\n", "server:return"]}"#,
                ),
                password_out.to_json(),
            ),
            check(
                "banned-precedes-password",
                "The IP filter denial occurs before the password branch and every accepted-client side effect.",
                literal(
                    r#"["VM_Call:game:enter", "vmMain:GAME_CLIENT_CONNECT", "trap_GetUserinfo", "G_FilterPacket", "ClientConnect:return:32", "vmMain:return:32", "VM_Call:return:32", "VM_ExplicitArgPtr:game:32", "NET_OutOfBandPrint:print\nYou are banned from this server.\n", "server:return"]"#,
                ),
                Json::array(banned_out.trace.iter().map(Json::string).collect()),
            ),
            check(
                "local-accepted-order",
                "localhost skips password comparison; game initialization and userinfo callbacks finish before null returns and the server sends connectResponse. VM_Call restores the previous ui context.",
                literal(
                    r#"{"returnValue": 0, "denial": null, "serverState": "connected", "currentVm": "ui", "trace": ["VM_Call:game:enter", "vmMain:GAME_CLIENT_CONNECT", "trap_GetUserinfo", "G_FilterPacket", "client:zero", "connected:CON_CONNECTING", "G_InitSessionData", "G_ReadSessionData", "G_LogPrintf", "ClientUserinfoChanged", "trap_SendServerCommand:connected", "CalculateRanks", "ClientConnect:return:0", "vmMain:return:0", "VM_Call:return:0", "SV_UserinfoChanged", "NET_OutOfBandPrint:connectResponse", "server:CS_CONNECTED"]}"#,
                ),
                local_out.to_json(),
            ),
            check(
                "password-exemptions",
                "The configured value none is case-insensitive; an existing SVF_BOT flag bypasses password checks even though the direct-connect isBot argument is false.",
                literal(r#"[0, 0]"#),
                Json::array(vec![
                    Json::float(none_password_out.return_value),
                    Json::float(existing_bot_out.return_value),
                ]),
            ),
        ],
    }
}

fn nested_scenario() -> SourceScenario {
    let outermost = nested_vm(None);
    let nested = nested_vm(Some("ui"));
    SourceScenario {
        id: "q3-nested-vm-callback-source",
        source_locations: ["code/qcommon/vm.c:668-710"].into_iter().map(str::to_owned).collect(),
        assumptions: ["Synthetic entry callbacks exercise only VM_Call save/restore and synchronous integer return flow. They do not execute QVM opcodes, host syscalls, or real gameplay."]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        input: Json::object(vec![
            (
                "priorContexts".to_owned(),
                Json::array(vec![Json::Null, Json::string("ui")]),
            ),
            ("nestedReturn".to_owned(), Json::int(7)),
        ]),
        output: Json::object(vec![
            ("outermost".to_owned(), outermost.to_json()),
            ("nested".to_owned(), nested.to_json()),
        ]),
        assertions: vec![
            check(
                "outermost-context-retained",
                "A nested call restores game before its caller resumes. At the outermost return oldVM is null, so the guarded restoration leaves currentVM=game.",
                literal(
                    r#"{"result": 8, "currentVm": "game", "trace": ["enter:game:current:game", "host:before:current:game", "enter:cgame:current:cgame", "return:cgame:7:current:game", "host:after:7:current:game", "return:game:8:current:game"]}"#,
                ),
                outermost.to_json(),
            ),
            check(
                "nested-context-restored",
                "A non-null prior VM is restored only after the game entry returns, while the host callback observes game immediately after cgame returns.",
                literal(
                    r#"{"result": 8, "currentVm": "ui", "trace": ["enter:game:current:game", "host:before:current:game", "enter:cgame:current:cgame", "return:cgame:7:current:game", "host:after:7:current:game", "return:game:8:current:ui"]}"#,
                ),
                nested.to_json(),
            ),
        ],
    }
}

/// Evaluate every Q3 source-derived scenario.
#[must_use]
pub fn evaluate_scenarios() -> Vec<SourceScenario> {
    vec![
        numeric_scenario(),
        movement_scenario(),
        timer_scenario(),
        firing_scenario(),
        switch_scenario(),
        haste_scenario(),
        connect_scenario(),
        nested_scenario(),
    ]
}

#[cfg(test)]
mod tests {
    // Covers tools/reference/q3/scenarios.test.ts: both donor cases (scenario
    // assertions pass, invalid fixed subdivision is rejected).
    use crate::reference::q3::semantics::MoveInput;

    use super::*;

    #[test]
    fn scenario_assertions_pass() {
        let mut count = 0;
        for scenario in evaluate_scenarios() {
            for assertion in &scenario.assertions {
                count += 1;
                assert!(
                    deep_strict_equal(&assertion.actual, &assertion.expected),
                    "{}/{}",
                    scenario.id,
                    assertion.id
                );
                assert!(assertion.passed, "{}/{}", scenario.id, assertion.id);
            }
        }
        assert_eq!(count, 25);
    }

    #[test]
    fn invalid_fixed_subdivision_cannot_hang() {
        for msec in [0.0, -1.0, 0.5, f64::NAN] {
            let error = subdivide_move(&MoveInput {
                command_time: 0.0,
                server_time: 1.0,
                framecount: 0.0,
                subdivision: Subdivision::Fixed { msec },
                jump_held: false,
                upmove: 0.0,
            })
            .expect_err("invalid subdivision");
            assert_eq!(error.to_string(), "Fixed subdivision must be a positive integer");
        }
    }
}
