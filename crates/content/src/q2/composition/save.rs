//! Q2 product save (`/home/buzzkill/Projects/quake-typescript/src/content/composition/q2/save.ts`).
//!
//! The donor frames every module checkpoint as provider bytes
//! (`ProviderCheckpoint`). Byte framing lives with the `qa-app`
//! persistence codecs; the port captures and restores the struct
//! checkpoints directly, keeping the donor's record order, validation,
//! and restore order.

use super::product::Q2ProductRuntime;
use super::types::Q2MatchSelection;
use crate::q2::base::entities::Q2BaseEntitiesCheckpoint;
use crate::q2::base::player::checkpoint::Q2PlayersCheckpoint;
use crate::q2::foundation::checkpoint::Q2FoundationCheckpoint;
use crate::q2::foundation::host::{Q2Edition, Q2GameServices};
use crate::q2::foundation::items::Q2ItemsCheckpoint;
use crate::q2::foundation::monsters::checkpoint::Q2MonstersCheckpoint;
use crate::q2::foundation::monsters::{capture_monsters, restore_monsters};
use crate::q2::foundation::movers::Q2MoversCheckpoint;
use crate::q2::foundation::weapons::checkpoint::{capture_q2_weapons, restore_q2_weapons, Q2WeaponsCheckpoint};
use crate::q2::missionpacks::entities::rogue::Q2RogueEntitiesCheckpoint;
use crate::q2::missionpacks::items::Q2MissionPackItemsCheckpoint;
use crate::q2::missionpacks::modes::deathball::{deathball_hooks, Q2DeathBall, Q2DeathBallCheckpoint};
use crate::q2::missionpacks::modes::tag::{tag_hooks, Q2Tag, Q2TagCheckpoint};
use crate::q2::missionpacks::monsters::state::MissionPackMonstersCheckpoint;
use crate::q2::missionpacks::monsters::{capture_mission_pack_monsters, restore_mission_pack_monsters};
use crate::q2::missionpacks::types::Q2MissionPack;
use crate::q2::multiplayer::ctf::checkpoint::Q2CtfCheckpoint;
use crate::q2::multiplayer::ctf::ctf_hooks;
use crate::q2::multiplayer::ctf::index::Q2Ctf;
use crate::q2::multiplayer::lmctf::lmctf_hooks;
use crate::q2::multiplayer::lmctf::runtime::{LmctfCheckpoint, Q2Lmctf};
use crate::q2::rerelease::checkpoint::{Q2RereleaseModuleCheckpoint, Q2RereleasePlayersCheckpoint};

/// Expansion checkpoint (`q2:<pack>-monsters` + `q2:<pack>-entities`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ProductExpansionCheckpoint {
    /// Mission pack.
    pub pack: Q2MissionPack,
    /// Pack monsters.
    pub monsters: MissionPackMonstersCheckpoint,
    /// Pack entities.
    pub entities: Q2RogueEntitiesCheckpoint,
}

/// Match-mode checkpoint (`q2:match`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2ProductMatchCheckpoint {
    /// Tag state.
    Tag(Q2TagCheckpoint),
    /// Deathball state.
    Deathball(Q2DeathBallCheckpoint),
    /// CTF state.
    Ctf(Q2CtfCheckpoint),
    /// LMCTF state.
    Lmctf(LmctfCheckpoint),
}

/// Rerelease checkpoint (`q2:rerelease-players` + `q2:rerelease-entities`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ProductRereleaseCheckpoint {
    /// Rerelease players.
    pub players: Q2RereleasePlayersCheckpoint,
    /// Rerelease entities.
    pub entities: Q2RereleaseModuleCheckpoint,
}

/// Captured Q2 product (`captureQ2Product`).
#[derive(Debug, Clone)]
pub struct Q2ProductCheckpoint {
    /// Composition edition.
    pub edition: Q2Edition,
    /// Composition program.
    pub program: String,
    /// Match selection.
    pub match_selection: Q2MatchSelection,
    /// Deathmatch flags.
    pub deathmatch_flags: i32,
    /// Foundation state.
    pub foundation: Q2FoundationCheckpoint,
    /// Items state.
    pub items: Q2ItemsCheckpoint,
    /// Movers state.
    pub movers: Q2MoversCheckpoint,
    /// Monsters state.
    pub monsters: Q2MonstersCheckpoint,
    /// Weapons state.
    pub weapons: Q2WeaponsCheckpoint,
    /// Players state.
    pub players: Q2PlayersCheckpoint,
    /// Base entities state.
    pub base_entities: Q2BaseEntitiesCheckpoint,
    /// Mission-pack items state.
    pub missionpack_items: Option<Q2MissionPackItemsCheckpoint>,
    /// Expansion states.
    pub expansions: Vec<Q2ProductExpansionCheckpoint>,
    /// Rerelease state.
    pub rerelease: Option<Q2ProductRereleaseCheckpoint>,
    /// Match-mode state.
    pub match_state: Option<Q2ProductMatchCheckpoint>,
}

/// Match kind name for save validation (`match.kind`).
fn q2_match_kind_name(selection: &Q2MatchSelection) -> &'static str {
    match selection {
        Q2MatchSelection::Standard => "standard",
        Q2MatchSelection::Ctf => "ctf",
        Q2MatchSelection::Tag => "tag",
        Q2MatchSelection::Deathball { .. } => "deathball",
        Q2MatchSelection::Lmctf { .. } => "lmctf",
    }
}

/// Product identity for save validation.
#[derive(Debug, Clone, PartialEq)]
struct Q2ProductIdentity {
    /// Composition edition.
    edition: Q2Edition,
    /// Composition program.
    program: String,
    /// Match selection.
    selection: Q2MatchSelection,
}

/// Validate a checkpoint identity against the runtime (`restoreQ2Product` guards).
fn validate_q2_product_identity(runtime: &Q2ProductIdentity, saved: &Q2ProductIdentity) {
    if saved.edition != runtime.edition {
        panic!(
            "Q2 save composition edition ({:?}) differs from this source ({:?})",
            saved.edition, runtime.edition
        );
    }
    if saved.program != runtime.program {
        panic!(
            "Q2 save composition program ({}) differs from this source ({})",
            saved.program, runtime.program
        );
    }
    let saved_kind = q2_match_kind_name(&saved.selection);
    let kind = q2_match_kind_name(&runtime.selection);
    if saved_kind != kind {
        panic!("Q2 save composition match ({saved_kind}) differs from this source ({kind})");
    }
    if let (
        Q2MatchSelection::Deathball {
            team1_skin,
            team2_skin,
            goal_limit,
        },
        Q2MatchSelection::Deathball {
            team1_skin: saved_team1,
            team2_skin: saved_team2,
            goal_limit: saved_goal,
        },
    ) = (&runtime.selection, &saved.selection)
    {
        if saved_team1 != team1_skin || saved_team2 != team2_skin {
            panic!("Q2 save composition deathball team skins differ from this source");
        }
        if saved_goal != goal_limit {
            panic!("Q2 save composition deathball goal limit differs from this source");
        }
    }
}

/// Capture the Q2 product (`captureQ2Product`).
pub fn capture_q2_product(game: &mut Q2GameServices, runtime: &Q2ProductRuntime) -> Q2ProductCheckpoint {
    let mut expansions = Vec::new();
    for expansion in &runtime.expansions {
        expansions.push(Q2ProductExpansionCheckpoint {
            pack: expansion.pack,
            monsters: capture_mission_pack_monsters(game, expansion.pack == Q2MissionPack::Rogue),
            entities: expansion.entities.capture(game),
        });
    }
    let match_state = match &runtime.selection {
        Q2MatchSelection::Tag => Some(Q2ProductMatchCheckpoint::Tag(
            Q2Tag { hooks: tag_hooks(game) }.capture(game),
        )),
        Q2MatchSelection::Deathball { .. } => Some(Q2ProductMatchCheckpoint::Deathball(
            Q2DeathBall {
                hooks: deathball_hooks(game),
            }
            .capture(game),
        )),
        Q2MatchSelection::Ctf => Some(Q2ProductMatchCheckpoint::Ctf(
            Q2Ctf { hooks: ctf_hooks(game) }.capture(game),
        )),
        Q2MatchSelection::Lmctf { .. } => Some(Q2ProductMatchCheckpoint::Lmctf(
            Q2Lmctf {
                hooks: lmctf_hooks(game),
            }
            .capture(game),
        )),
        Q2MatchSelection::Standard => None,
    };
    Q2ProductCheckpoint {
        edition: game.options.edition,
        program: runtime.program.clone(),
        match_selection: runtime.selection.clone(),
        deathmatch_flags: game.deathmatch_flags(),
        foundation: game.capture_foundation(),
        items: runtime.items.capture(game),
        movers: runtime.movers.capture(game),
        monsters: capture_monsters(game),
        weapons: capture_q2_weapons(game),
        players: runtime.players.capture(game),
        base_entities: runtime.base_entities.capture(game),
        missionpack_items: runtime.armory.map(|armory| armory.items.capture(game)),
        expansions,
        rerelease: runtime.rerelease.map(|rerelease| Q2ProductRereleaseCheckpoint {
            players: rerelease.players.capture_rerelease(game),
            entities: rerelease.entities.capture(game),
        }),
        match_state,
    }
}

/// Restore the Q2 product (`restoreQ2Product`).
pub fn restore_q2_product(game: &mut Q2GameServices, runtime: &Q2ProductRuntime, checkpoint: &Q2ProductCheckpoint) {
    runtime.register_callbacks(game);
    validate_q2_product_identity(
        &Q2ProductIdentity {
            edition: game.options.edition,
            program: runtime.program.clone(),
            selection: runtime.selection.clone(),
        },
        &Q2ProductIdentity {
            edition: checkpoint.edition,
            program: checkpoint.program.clone(),
            selection: checkpoint.match_selection.clone(),
        },
    );
    runtime.set_deathmatch_flags(game, checkpoint.deathmatch_flags);
    game.restore_foundation(&checkpoint.foundation);
    runtime.items.restore(game, &checkpoint.items);
    runtime.movers.restore(game, &checkpoint.movers);
    if let Some(armory) = runtime.armory {
        if let Some(items) = &checkpoint.missionpack_items {
            armory.items.restore(game, items);
        }
    }
    for expansion in &runtime.expansions {
        let saved = checkpoint.expansions.iter().find(|saved| saved.pack == expansion.pack);
        let Some(saved) = saved else { continue };
        expansion.entities.restore(game, saved.entities);
        restore_mission_pack_monsters(game, &saved.monsters, expansion.pack == Q2MissionPack::Rogue);
    }
    restore_q2_weapons(game, &checkpoint.weapons);
    restore_monsters(game, &checkpoint.monsters);
    runtime.base_entities.restore(game, &checkpoint.base_entities);
    runtime.players.restore(game, &checkpoint.players);
    if runtime.rerelease.is_some() && runtime.selection == Q2MatchSelection::Standard {
        if let Some(rerelease) = runtime.rerelease {
            if let Some(saved) = &checkpoint.rerelease {
                rerelease.players.restore_rerelease(game, &saved.players);
                rerelease.entities.restore(game, &saved.entities);
            }
        }
    }
    match (&runtime.selection, &checkpoint.match_state) {
        (Q2MatchSelection::Tag, Some(Q2ProductMatchCheckpoint::Tag(saved))) => {
            Q2Tag { hooks: tag_hooks(game) }.restore(game, *saved);
        }
        (Q2MatchSelection::Deathball { .. }, Some(Q2ProductMatchCheckpoint::Deathball(saved))) => {
            Q2DeathBall {
                hooks: deathball_hooks(game),
            }
            .restore(game, *saved);
        }
        (Q2MatchSelection::Ctf, Some(Q2ProductMatchCheckpoint::Ctf(saved))) => {
            Q2Ctf { hooks: ctf_hooks(game) }.restore(saved, game);
        }
        (Q2MatchSelection::Lmctf { .. }, Some(Q2ProductMatchCheckpoint::Lmctf(saved))) => {
            Q2Lmctf {
                hooks: lmctf_hooks(game),
            }
            .restore(saved, game);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> Q2ProductIdentity {
        Q2ProductIdentity {
            edition: Q2Edition::Classic,
            program: "baseq2".to_string(),
            selection: Q2MatchSelection::Standard,
        }
    }

    fn deathball_selection() -> Q2MatchSelection {
        Q2MatchSelection::Deathball {
            team1_skin: "male/ctf_r".to_string(),
            team2_skin: "male/ctf_b".to_string(),
            goal_limit: 5.0,
        }
    }

    #[test]
    fn match_kinds_match_donor_names() {
        assert_eq!(q2_match_kind_name(&Q2MatchSelection::Standard), "standard");
        assert_eq!(q2_match_kind_name(&Q2MatchSelection::Ctf), "ctf");
        assert_eq!(q2_match_kind_name(&Q2MatchSelection::Tag), "tag");
        assert_eq!(q2_match_kind_name(&deathball_selection()), "deathball");
        assert_eq!(q2_match_kind_name(&Q2MatchSelection::Lmctf { travel: None }), "lmctf");
    }

    #[test]
    fn identical_identities_validate() {
        validate_q2_product_identity(&test_identity(), &test_identity());
    }

    #[test]
    #[should_panic(expected = "edition")]
    fn edition_mismatch_panics() {
        let saved = Q2ProductIdentity {
            edition: Q2Edition::Rerelease,
            ..test_identity()
        };
        validate_q2_product_identity(&test_identity(), &saved);
    }

    #[test]
    #[should_panic(expected = "program")]
    fn program_mismatch_panics() {
        let saved = Q2ProductIdentity {
            program: "xatrix".to_string(),
            ..test_identity()
        };
        validate_q2_product_identity(&test_identity(), &saved);
    }

    #[test]
    #[should_panic(expected = "match (ctf)")]
    fn match_mismatch_panics() {
        let saved = Q2ProductIdentity {
            selection: Q2MatchSelection::Ctf,
            ..test_identity()
        };
        validate_q2_product_identity(&test_identity(), &saved);
    }

    #[test]
    #[should_panic(expected = "team skins")]
    fn deathball_skin_mismatch_panics() {
        let runtime = Q2ProductIdentity {
            selection: deathball_selection(),
            ..test_identity()
        };
        let saved = Q2ProductIdentity {
            selection: Q2MatchSelection::Deathball {
                team1_skin: "female/ctf_r".to_string(),
                team2_skin: "male/ctf_b".to_string(),
                goal_limit: 5.0,
            },
            ..test_identity()
        };
        validate_q2_product_identity(&runtime, &saved);
    }

    #[test]
    #[should_panic(expected = "goal limit")]
    fn deathball_goal_mismatch_panics() {
        let runtime = Q2ProductIdentity {
            selection: deathball_selection(),
            ..test_identity()
        };
        let saved = Q2ProductIdentity {
            selection: Q2MatchSelection::Deathball {
                team1_skin: "male/ctf_r".to_string(),
                team2_skin: "male/ctf_b".to_string(),
                goal_limit: 10.0,
            },
            ..test_identity()
        };
        validate_q2_product_identity(&runtime, &saved);
    }

    #[test]
    fn matching_deathball_validates() {
        let runtime = Q2ProductIdentity {
            selection: deathball_selection(),
            ..test_identity()
        };
        let saved = Q2ProductIdentity {
            selection: deathball_selection(),
            ..test_identity()
        };
        validate_q2_product_identity(&runtime, &saved);
    }
}
