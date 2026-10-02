//! Quake III `map`/`devmap`/`spmap` launch resolution.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-map-command.ts`
//! (`Q3MapLaunch`, `q3MapLaunch`, `applyQ3MapLaunch`). The product policy is the canonical
//! [`Q3ProductPolicy`](qa_content::q3::product_restriction::Q3ProductPolicy) port; this
//! donor only reads which map commands the policy allows, so [`Q3MapCommandPolicy`] is a
//! thin alias over it and [`q3_map_command_policy_commands`] delegates the
//! prerelease-demo check to the canonical
//! [`q3_prerelease_demo`](qa_content::q3::product_restriction::q3_prerelease_demo).

use qa_content::q3::product_restriction::{q3_prerelease_demo, Q3ProductPolicy, TeamArenaUi};
use qa_core::cvar::{CvarError, CvarRegistry};
use thiserror::Error;

/// Map-command view of the canonical donor `Q3ProductPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3MapCommandPolicy {
    /// Retail build: all four map commands.
    Retail,
    /// Prerelease demo: `map` only.
    PrereleaseDemo,
    /// Prerelease Team Arena demo: all four map commands.
    PrereleaseTeamArenaDemo,
}

impl From<Q3MapCommandPolicy> for Q3ProductPolicy {
    /// Lower the map-command view onto the canonical policy.
    fn from(policy: Q3MapCommandPolicy) -> Self {
        match policy {
            Q3MapCommandPolicy::Retail => Q3ProductPolicy::Retail,
            Q3MapCommandPolicy::PrereleaseDemo => Q3ProductPolicy::PrereleaseDemo {
                team_arena_ui: TeamArenaUi::Retail,
            },
            Q3MapCommandPolicy::PrereleaseTeamArenaDemo => Q3ProductPolicy::PrereleaseTaDemo,
        }
    }
}

/// Map commands a policy allows (donor `q3ProductMapCommands`).
#[must_use]
pub fn q3_map_command_policy_commands(policy: Q3MapCommandPolicy) -> &'static [&'static str] {
    if q3_prerelease_demo(policy.into()) {
        &["map"]
    } else {
        &["map", "devmap", "spmap", "spdevmap"]
    }
}

/// Failure of a map-command operation.
#[derive(Debug, Error)]
pub enum Q3MapCommandError {
    /// The command is not allowed by the product policy.
    #[error("Unknown Q3 map command: {0}")]
    UnknownCommand(String),
    /// Registry failure while applying launch cvars.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// One launch cvar assignment (donor `Q3MapLaunch["cvars"]` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3MapCvar {
    /// Variable name.
    pub name: String,
    /// Value text.
    pub value: String,
}

/// Resolved map launch (donor `Q3MapLaunch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3MapLaunch {
    /// Resolved game type.
    pub game_type: i32,
    /// Whether bots are removed for the launch.
    pub kill_bots: bool,
    /// Whether this is a single-player launch.
    pub single_player: bool,
    /// Single-player client cap (`Some(8)` for `sp*`, else [`None`]).
    pub max_clients: Option<u8>,
    /// Cvar assignments, in donor order.
    pub cvars: Vec<Q3MapCvar>,
}

/// Which launch assignments to apply (donor `applyQ3MapLaunch` phase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q3MapLaunchPhase {
    /// Apply every assignment.
    #[default]
    All,
    /// Apply everything except `sv_cheats`.
    Spawn,
    /// Apply only `sv_cheats`.
    Finish,
}

/// Resolve a map command (donor `q3MapLaunch`, `SV_Map_f`).
///
/// `spdevmap` follows the single-player branch, which disables cheats.
pub fn q3_map_launch(
    policy: Q3MapCommandPolicy,
    command: &str,
    current_game_type: i32,
) -> Result<Q3MapLaunch, Q3MapCommandError> {
    if !q3_map_command_policy_commands(policy).contains(&command) {
        return Err(Q3MapCommandError::UnknownCommand(command.to_string()));
    }
    let single_player = command.starts_with("sp");
    let game_type = if single_player {
        2
    } else if current_game_type == 2 {
        0
    } else {
        current_game_type
    };
    let mut cvars = vec![
        Q3MapCvar {
            name: "g_gametype".to_string(),
            value: game_type.to_string(),
        },
        Q3MapCvar {
            name: "sv_cheats".to_string(),
            value: if command == "devmap" { "1" } else { "0" }.to_string(),
        },
    ];
    if single_player {
        cvars.push(Q3MapCvar {
            name: "g_doWarmup".to_string(),
            value: "0".to_string(),
        });
        cvars.push(Q3MapCvar {
            name: "sv_maxclients".to_string(),
            value: "8".to_string(),
        });
    }
    Ok(Q3MapLaunch {
        game_type,
        kill_bots: single_player || command == "devmap",
        single_player,
        max_clients: single_player.then_some(8),
        cvars,
    })
}

/// Apply a resolved launch (donor `applyQ3MapLaunch`).
pub fn apply_q3_map_launch(
    cvars: &mut CvarRegistry,
    launch: &Q3MapLaunch,
    phase: Q3MapLaunchPhase,
) -> Result<(), CvarError> {
    if phase != Q3MapLaunchPhase::Finish {
        cvars.apply_latched(None)?;
    }
    for setting in &launch.cvars {
        if phase == Q3MapLaunchPhase::Spawn && setting.name == "sv_cheats"
            || phase == Q3MapLaunchPhase::Finish && setting.name != "sv_cheats"
        {
            continue;
        }
        cvars.set(&setting.name, &setting.value, true)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    #[test]
    fn devmap_enables_cheats_and_kills_bots() {
        let launch = q3_map_launch(Q3MapCommandPolicy::Retail, "devmap", 0).unwrap();
        assert_eq!(launch.game_type, 0);
        assert!(launch.kill_bots);
        assert!(!launch.single_player);
        assert_eq!(launch.max_clients, None);
        let cheats = launch.cvars.iter().find(|cvar| cvar.name == "sv_cheats").unwrap();
        assert_eq!(cheats.value, "1");
    }

    #[test]
    fn spdevmap_is_single_player_without_cheats() {
        let launch = q3_map_launch(Q3MapCommandPolicy::Retail, "spdevmap", 0).unwrap();
        assert!(launch.single_player);
        assert!(launch.kill_bots);
        assert_eq!(launch.game_type, 2);
        assert_eq!(launch.max_clients, Some(8));
        let cheats = launch.cvars.iter().find(|cvar| cvar.name == "sv_cheats").unwrap();
        assert_eq!(cheats.value, "0");
        assert!(launch
            .cvars
            .iter()
            .any(|cvar| cvar.name == "g_doWarmup" && cvar.value == "0"));
        assert!(launch
            .cvars
            .iter()
            .any(|cvar| cvar.name == "sv_maxclients" && cvar.value == "8"));
    }

    #[test]
    fn map_resets_lingering_single_player_type() {
        let launch = q3_map_launch(Q3MapCommandPolicy::Retail, "map", 2).unwrap();
        assert_eq!(launch.game_type, 0);
        assert!(!launch.kill_bots);
    }

    #[test]
    fn prerelease_demo_rejects_devmap() {
        let error = q3_map_launch(Q3MapCommandPolicy::PrereleaseDemo, "devmap", 0).unwrap_err();
        assert!(matches!(error, Q3MapCommandError::UnknownCommand(_)));
        assert!(q3_map_launch(Q3MapCommandPolicy::PrereleaseDemo, "map", 0).is_ok());
        assert!(q3_map_launch(Q3MapCommandPolicy::PrereleaseTeamArenaDemo, "devmap", 0).is_ok());
    }

    #[test]
    fn phases_split_cheats_last() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let launch = q3_map_launch(Q3MapCommandPolicy::Retail, "devmap", 4).unwrap();
        apply_q3_map_launch(&mut cvars, &launch, Q3MapLaunchPhase::Spawn).unwrap();
        assert_eq!(cvars.variable_string("g_gametype"), "4");
        assert_eq!(cvars.variable_string("sv_cheats"), "");
        apply_q3_map_launch(&mut cvars, &launch, Q3MapLaunchPhase::Finish).unwrap();
        assert_eq!(cvars.variable_string("sv_cheats"), "1");
    }
}
