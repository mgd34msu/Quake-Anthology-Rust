//! Local-seat input routing data ported from
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/input.ts`.
//!
//! Ports the donor exports that are expressible with the ported sibling
//! surface: [`Q3CommandSelection`], [`movement_dialect`], and
//! [`default_binding_capabilities`].
//!
//! The donor `ApplicationInput` orchestrator (seat inputs, SDL controllers,
//! haptics, console routing, QVM client bindings, prepared-startup candidate
//! publication) is NOT ported: it requires unported siblings
//! (`startup-config.ts`, `client-bootstrap.ts`, `prepared-startup.ts`,
//! `configuration.ts`, `q3-client/*`, `q1/q2-client-commands.ts`,
//! `console.ts` routing, `audio/commands.ts`, `config-scripts.ts`) plus
//! unported input layers (`input/seat.ts`, `input/router.ts`,
//! `input/user-command.ts`, `input/mouse.ts`, `input/client-commands.ts`,
//! `platform/controller.ts`, `platform/sdl.ts`). `LocalPlayer`/`LocalInput`
//! reference those same unported seat types and are omitted with it.

use qa_client::ui::settings::action_catalog::{BindingCapabilities, ScoreCommand};
use qa_content::contract::{ExecutableRecipe, ExecutionModule, ModuleRole, NativeModuleApi};
use qa_core::cmd::Dialect;
use qa_core::identity::ProviderId;
use qa_core::time::ClockProfile;

use crate::options::{ApplicationOptions, GameFamily, Network};

/// Quake III per-seat command selection (donor `Q3CommandSelection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3CommandSelection {
    /// Selected weapon number.
    pub weapon: u32,
    /// Command-time mouse sensitivity.
    pub sensitivity: f64,
}

/// [`movement_dialect`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovementDialectError {
    /// The recipe has no timing profile for its movement provider.
    MissingTiming(ProviderId),
}

impl std::fmt::Display for MovementDialectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MovementDialectError::MissingTiming(provider) => {
                write!(f, "Recipe has no timing for {provider:?}")
            }
        }
    }
}

impl std::error::Error for MovementDialectError {}

/// Clock profile to movement dialect (donor `timing.clock.kind`).
#[must_use]
pub fn clock_dialect(clock: &ClockProfile) -> Dialect {
    match clock {
        ClockProfile::Q1Netquake { .. } => Dialect::Q1Netquake,
        ClockProfile::Q1Quakeworld { .. } => Dialect::Q1Quakeworld,
        ClockProfile::Q2Classic => Dialect::Q2Classic,
        ClockProfile::Q2Rerelease { .. } => Dialect::Q2Rerelease,
        ClockProfile::Q3 { .. } => Dialect::Q3,
    }
}

/// Resolve the movement command dialect (donor `movementDialect`).
///
/// A recipe with a native server-game module speaking a Quake II game API
/// pins the dialect; otherwise the movement provider's timing clock wins.
/// Without a recipe, QuakeWorld clients speak `q1-quakeworld` and the
/// movement family picks the fallback.
pub fn movement_dialect(
    options: &ApplicationOptions,
    recipe: Option<&ExecutableRecipe>,
) -> Result<Dialect, MovementDialectError> {
    if let Some(recipe) = recipe {
        for module in &recipe.execution {
            let ExecutionModule::Native { role, api, .. } = module else {
                continue;
            };
            if *role != ModuleRole::ServerGame {
                continue;
            }
            match api {
                NativeModuleApi::Q2RereleaseGame => return Ok(Dialect::Q2Rerelease),
                NativeModuleApi::Q2ClassicGame => return Ok(Dialect::Q2Classic),
                _ => {}
            }
        }
        let timing = recipe
            .timing
            .iter()
            .find(|profile| profile.provider == recipe.movement.provider)
            .ok_or_else(|| MovementDialectError::MissingTiming(recipe.movement.provider.clone()))?;
        return Ok(clock_dialect(&timing.clock));
    }
    if matches!(options.network, Network::QwClient { .. }) {
        return Ok(Dialect::Q1Quakeworld);
    }
    Ok(match options.movement {
        GameFamily::Q1 => Dialect::Q1Netquake,
        GameFamily::Q2 => Dialect::Q2Classic,
        GameFamily::Q3 => Dialect::Q3,
    })
}

/// Default binding capabilities before caller overrides (donor
/// `ApplicationInput.bindingCapabilities` fallback).
#[must_use]
pub fn default_binding_capabilities(network: &Network) -> BindingCapabilities {
    let chat = matches!(
        network,
        Network::Q1Client { .. }
            | Network::QwClient { .. }
            | Network::Q2Client { .. }
            | Network::Q3Client { .. }
            | Network::UnifiedClient { .. }
    );
    let score_command = if matches!(network, Network::Q2Client { .. }) {
        ScoreCommand::Score
    } else {
        ScoreCommand::Scores
    };
    BindingCapabilities {
        chat,
        score_command: Some(score_command),
        offhand_grapple: false,
        offhand_grenades: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qw_client_without_recipe_speaks_quakeworld() {
        let options = ApplicationOptions {
            network: Network::QwClient {
                remote: "qw.example".to_owned(),
            },
            movement: GameFamily::Q3,
            ..ApplicationOptions::default()
        };
        assert_eq!(movement_dialect(&options, None), Ok(Dialect::Q1Quakeworld));
    }

    #[test]
    fn movement_family_picks_fallback_dialect() {
        let mut options = ApplicationOptions {
            network: Network::Offline,
            ..ApplicationOptions::default()
        };
        for (family, dialect) in [
            (GameFamily::Q1, Dialect::Q1Netquake),
            (GameFamily::Q2, Dialect::Q2Classic),
            (GameFamily::Q3, Dialect::Q3),
        ] {
            options.movement = family;
            assert_eq!(movement_dialect(&options, None), Ok(dialect));
        }
    }

    #[test]
    fn clock_profiles_map_to_dialects() {
        assert_eq!(clock_dialect(&ClockProfile::Q2Classic), Dialect::Q2Classic);
        assert_eq!(
            clock_dialect(&ClockProfile::Q3 {
                server_frame_milliseconds: 100.0,
                fixed_movement_milliseconds: None,
            }),
            Dialect::Q3
        );
    }

    #[test]
    fn binding_defaults_follow_network_role() {
        let q2 = default_binding_capabilities(&Network::Q2Client {
            remote: "q2.example".to_owned(),
        });
        assert!(q2.chat);
        assert_eq!(q2.score_command, Some(ScoreCommand::Score));
        assert!(!q2.offhand_grapple && !q2.offhand_grenades);
        let offline = default_binding_capabilities(&Network::Offline);
        assert!(!offline.chat);
        assert_eq!(offline.score_command, Some(ScoreCommand::Scores));
    }

    #[test]
    fn q3_selection_holds_weapon_and_sensitivity() {
        let selection = Q3CommandSelection {
            weapon: 5,
            sensitivity: 2.5,
        };
        assert_eq!(selection.weapon, 5);
        assert_eq!(selection.sensitivity, 2.5);
    }
}
