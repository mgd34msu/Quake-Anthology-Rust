//! Q2 rerelease obituaries (`src/content/q2/rerelease/obituary.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use crate::q2::base::player::obituary::Q2ObituaryRecipient;
use crate::q2::base::player::types::Q2PlayerState;
use crate::q2::foundation::host::Q2Mode;

/// Generic death causes.
const GENERIC: &[(i32, &str)] = &[
    (23, "suicide"),
    (22, "falling"),
    (20, "crush"),
    (17, "water"),
    (18, "slime"),
    (19, "lava"),
    (25, "explosive"),
    (26, "explosive"),
    (28, "exit"),
    (30, "laser"),
    (33, "blaster"),
    (27, "hurt"),
    (29, "hurt"),
    (31, "hurt"),
    (38, "gekk"),
    (36, "gekk"),
];

/// Self-inflicted death causes.
const SELF: &[(i32, &str)] = &[
    (24, "held_grenade"),
    (16, "grenade_splash"),
    (7, "grenade_splash"),
    (9, "rocket_splash"),
    (13, "bfg_blast"),
    (39, "trap"),
    (53, "dopple_explode"),
];

/// Kill causes.
const KILL: &[(i32, &str)] = &[
    (1, "blaster"),
    (2, "shotgun"),
    (3, "sshotgun"),
    (4, "machinegun"),
    (5, "chaingun"),
    (6, "grenade"),
    (7, "grenade_splash"),
    (8, "rocket"),
    (9, "rocket_splash"),
    (10, "hyperblaster"),
    (11, "railgun"),
    (12, "bfg_laser"),
    (13, "bfg_blast"),
    (14, "bfg_effect"),
    (15, "handgrenade"),
    (16, "handgrenade_splash"),
    (24, "held_grenade"),
    (21, "telefrag"),
    (57, "telefrag"),
    (34, "ripper"),
    (35, "phalanx"),
    (39, "trap"),
    (40, "chainfist"),
    (41, "disintegrator"),
    (42, "etf_rifle"),
    (44, "heatbeam"),
    (45, "tesla"),
    (46, "prox"),
    (47, "nuke"),
    (48, "vengeance_sphere"),
    (49, "hunter_sphere"),
    (50, "defender_sphere"),
    (51, "tracker"),
    (53, "dopple_explode"),
    (54, "dopple_vengeance"),
    (55, "dopple_hunter"),
    (56, "grapple"),
];

/// Rerelease obituary text (`q2RereleaseObituary` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2RereleaseObituary {
    /// Text.
    pub text: String,
    /// Arguments.
    pub args: Vec<String>,
}

/// Look up a cause name.
fn cause_name(table: &[(i32, &'static str)], means: i32) -> Option<&'static str> {
    table.iter().find(|(kind, _)| *kind == means).map(|(_, name)| *name)
}

/// Plan a rerelease obituary and its score operations.
fn plan_rerelease_obituary(
    victim: &Q2PlayerState,
    attacker: Option<&Q2PlayerState>,
    suicide: bool,
    cause: i32,
    mode: Q2Mode,
    no_point_loss: bool,
) -> (Q2RereleaseObituary, Vec<(Q2ObituaryRecipient, i32)>) {
    let means = cause & !0x8000000;
    let friendly = cause & 0x8000000 != 0 || mode == Q2Mode::Coop && attacker.is_some();
    let mut ops = Vec::new();
    let source = if suicide {
        Some(format!("$g_mod_self_{}", cause_name(SELF, means).unwrap_or("default")))
    } else if let Some(generic) = cause_name(GENERIC, means) {
        Some(format!("$g_mod_generic_{generic}"))
    } else {
        None
    };
    if let Some(source) = source {
        if mode == Q2Mode::Deathmatch && !no_point_loss {
            ops.push((Q2ObituaryRecipient::Victim, -1));
        }
        let obituary = Q2RereleaseObituary {
            text: source,
            args: vec![victim.name.clone()],
        };
        return (obituary, ops);
    }
    if let Some(attacker) = attacker {
        if mode == Q2Mode::Deathmatch {
            if !friendly {
                ops.push((Q2ObituaryRecipient::Attacker, 1));
            } else if !no_point_loss {
                ops.push((Q2ObituaryRecipient::Attacker, -1));
            }
        } else if mode != Q2Mode::Coop {
            ops.push((Q2ObituaryRecipient::Victim, -1));
        }
        let obituary = Q2RereleaseObituary {
            text: format!("$g_mod_kill_{}", cause_name(KILL, means).unwrap_or("generic")),
            args: vec![victim.name.clone(), attacker.name.clone()],
        };
        return (obituary, ops);
    }
    if mode == Q2Mode::Deathmatch && !no_point_loss {
        ops.push((Q2ObituaryRecipient::Victim, -1));
    }
    let obituary = Q2RereleaseObituary {
        text: "$g_mod_generic_died".to_string(),
        args: vec![victim.name.clone()],
    };
    (obituary, ops)
}

/// Format a rerelease obituary (`q2RereleaseObituary`).
///
/// `suicide` mirrors the donor's attacker-is-victim identity check, which
/// the caller derives from actor ids.
pub fn q2_rerelease_obituary(
    victim: &Q2PlayerState,
    attacker: Option<&Q2PlayerState>,
    suicide: bool,
    cause: i32,
    mode: Q2Mode,
    no_point_loss: bool,
    score: &mut dyn FnMut(Q2ObituaryRecipient, i32),
) -> Q2RereleaseObituary {
    let (obituary, ops) = plan_rerelease_obituary(victim, attacker, suicide, cause, mode, no_point_loss);
    for (recipient, change) in ops {
        score(recipient, change);
    }
    obituary
}

/// Format a rerelease obituary with the default scorer.
pub fn q2_rerelease_obituary_scored(
    victim: &mut Q2PlayerState,
    mut attacker: Option<&mut Q2PlayerState>,
    suicide: bool,
    cause: i32,
    mode: Q2Mode,
    no_point_loss: bool,
) -> Q2RereleaseObituary {
    let (obituary, ops) = plan_rerelease_obituary(victim, attacker.as_deref(), suicide, cause, mode, no_point_loss);
    for (recipient, change) in ops {
        match recipient {
            Q2ObituaryRecipient::Victim => victim.score += change,
            Q2ObituaryRecipient::Attacker => match attacker.as_deref_mut() {
                Some(attacker) => attacker.score += change,
                None => victim.score += change,
            },
        }
    }
    obituary
}
