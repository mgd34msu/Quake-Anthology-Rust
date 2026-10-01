//! Native rerelease bot objectives over the shared match.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-objectives.ts`.
//!
//! Missing siblings: `SharedSimulation` (`runtime.ts`, runtime partition).
//! The [`BotObjectiveSimulation`] seam exposes exactly the donor's simulation
//! surface, including the Q2 match-mode discriminants; the runtime partition
//! implements it post-merge over `qa_content::q2` and `qa_world`.

use std::rc::Rc;

use qa_bots::behavior::rerelease::data::knowledge::{BotGameModeT, BotGameType, BotKnowledge};
use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use thiserror::Error;

use super::bot_rerelease_world::RereleaseBotObjectives;

/// Simulation play mode (donor `simulation.options.mode` spellings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotSimulationMode {
    /// Cooperative.
    Coop,
    /// Single player.
    Singleplayer,
    /// Deathmatch.
    Deathmatch,
    /// Any other mode.
    Other,
}

/// Q2 match-mode discriminant (donor `match instanceof` chain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotMatchKind {
    /// Open match.
    Open,
    /// Capture the flag.
    Ctf,
    /// LMCTF.
    Lmctf,
    /// Deathball.
    DeathBall,
    /// Tag.
    Tag,
}

/// Source objective projection (donor `sourceObjectives` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct BotSourceObjective {
    /// Bot-goal objective.
    pub bot_goal: bool,
    /// Completed.
    pub complete: bool,
    /// Target actor.
    pub target: Option<ActorId>,
    /// Carrier actor.
    pub carrier: Option<ActorId>,
}

/// Simulation surface for rerelease bot objectives.
pub trait BotObjectiveSimulation {
    /// Donor `q1?.cvars ?? simulation.q2ServerCvars()` presence.
    fn has_objective_registry(&self) -> bool;
    /// Donor `registry.variableValue(name)`.
    fn objective_cvar(&self, name: &str) -> f64;
    /// Donor `simulation.players()`.
    fn objective_players(&self) -> Vec<ActorId>;
    /// Donor `simulation.playerCommand(actor, "team", [team])`.
    fn player_team_command(&self, actor: &ActorId, team: &str);
    /// Donor `simulation.options.mode`.
    fn simulation_mode(&self) -> BotSimulationMode;
    /// Donor `simulation.movementPlayer(actor) !== null`.
    fn has_movement_player(&self, actor: &ActorId) -> bool;
    /// Donor `simulation.combat.read(actor)?.team`.
    fn combat_team(&self, actor: &ActorId) -> Option<String>;
    /// Donor `q1?.composition.ctf` presence.
    fn has_q1_ctf(&self) -> bool;
    /// Donor `q1.composition.ctf.carried(actor) !== null`.
    fn q1_ctf_carried(&self, actor: &ActorId) -> bool;
    /// Donor Q2 match discriminant.
    fn match_kind(&self) -> BotMatchKind;
    /// Donor CTF/LMCTF `match.states.get(actor)?.team ?? 0`.
    fn match_team(&self, actor: &ActorId) -> i32;
    /// Donor deathball `match.hooks.skin(actor)`.
    fn deathball_skin(&self, actor: &ActorId) -> Option<String>;
    /// Donor deathball `(settings.team1Skin, settings.team2Skin)`.
    fn deathball_teams(&self) -> Option<(String, String)>;
    /// Donor deathball `match.ballActor()`.
    fn deathball_ball(&self) -> Option<ActorId>;
    /// Donor LMCTF `match.flags.carried(actor, game) !== null`.
    fn lmctf_flag_carried(&self, actor: &ActorId) -> bool;
    /// Donor tag `match.ownerActor()`.
    fn tag_owner(&self) -> Option<ActorId>;
    /// Donor `simulation.sourceObjectives()`.
    fn source_objectives(&self) -> Vec<BotSourceObjective>;
    /// Donor `simulation.bodies.read(actor)?.origin`.
    fn body_origin(&self, actor: &ActorId) -> Option<Vec3>;
    /// Donor `simulation.inventory.count(actor, item)`.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32;
    /// Donor `q2?.product.rerelease?.entities.poi?.origin`.
    fn rerelease_poi(&self) -> Option<Vec3>;
}

impl<S: BotObjectiveSimulation + ?Sized> BotObjectiveSimulation for Rc<S> {
    fn has_objective_registry(&self) -> bool {
        (**self).has_objective_registry()
    }
    fn objective_cvar(&self, name: &str) -> f64 {
        (**self).objective_cvar(name)
    }
    fn objective_players(&self) -> Vec<ActorId> {
        (**self).objective_players()
    }
    fn player_team_command(&self, actor: &ActorId, team: &str) {
        (**self).player_team_command(actor, team);
    }
    fn simulation_mode(&self) -> BotSimulationMode {
        (**self).simulation_mode()
    }
    fn has_movement_player(&self, actor: &ActorId) -> bool {
        (**self).has_movement_player(actor)
    }
    fn combat_team(&self, actor: &ActorId) -> Option<String> {
        (**self).combat_team(actor)
    }
    fn has_q1_ctf(&self) -> bool {
        (**self).has_q1_ctf()
    }
    fn q1_ctf_carried(&self, actor: &ActorId) -> bool {
        (**self).q1_ctf_carried(actor)
    }
    fn match_kind(&self) -> BotMatchKind {
        (**self).match_kind()
    }
    fn match_team(&self, actor: &ActorId) -> i32 {
        (**self).match_team(actor)
    }
    fn deathball_skin(&self, actor: &ActorId) -> Option<String> {
        (**self).deathball_skin(actor)
    }
    fn deathball_teams(&self) -> Option<(String, String)> {
        (**self).deathball_teams()
    }
    fn deathball_ball(&self) -> Option<ActorId> {
        (**self).deathball_ball()
    }
    fn lmctf_flag_carried(&self, actor: &ActorId) -> bool {
        (**self).lmctf_flag_carried(actor)
    }
    fn tag_owner(&self) -> Option<ActorId> {
        (**self).tag_owner()
    }
    fn source_objectives(&self) -> Vec<BotSourceObjective> {
        (**self).source_objectives()
    }
    fn body_origin(&self, actor: &ActorId) -> Option<Vec3> {
        (**self).body_origin(actor)
    }
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
        (**self).inventory_count(actor, item)
    }
    fn rerelease_poi(&self) -> Option<Vec3> {
        (**self).rerelease_poi()
    }
}

/// Objective construction failures.
#[derive(Debug, Error)]
pub enum BotObjectiveError {
    /// Bot objective source registry unavailable.
    #[error("Bot objective source registry unavailable")]
    MissingRegistry,
}

/// Rerelease bot objectives (donor `rereleaseBotObjectives` result).
pub struct RereleaseBotObjectivesImpl<S> {
    simulation: S,
    knowledge: Rc<BotKnowledge>,
}

/// Create the objectives (donor `rereleaseBotObjectives`).
pub fn rerelease_bot_objectives<S: BotObjectiveSimulation>(
    simulation: S,
    knowledge: Rc<BotKnowledge>,
) -> Result<RereleaseBotObjectivesImpl<S>, BotObjectiveError> {
    if !simulation.has_objective_registry() {
        return Err(BotObjectiveError::MissingRegistry);
    }
    Ok(RereleaseBotObjectivesImpl { simulation, knowledge })
}

impl<S: BotObjectiveSimulation> RereleaseBotObjectives for RereleaseBotObjectivesImpl<S> {
    fn admit(&self, actor: &ActorId) {
        if !matches!(self.simulation.match_kind(), BotMatchKind::Ctf | BotMatchKind::Lmctf)
            || self.simulation.match_team(actor) != 0
        {
            return;
        }
        let mut red = 0;
        let mut blue = 0;
        for player in self.simulation.objective_players() {
            match self.simulation.match_team(&player) {
                1 => red += 1,
                2 => blue += 1,
                _ => {}
            }
        }
        self.simulation
            .player_team_command(actor, if red <= blue { "red" } else { "blue" });
    }

    fn mode(&self) -> BotGameModeT {
        let mode = self.knowledge.game_mode(&|name| self.simulation.objective_cvar(name));
        if self.simulation.has_q1_ctf()
            || matches!(self.simulation.match_kind(), BotMatchKind::Ctf | BotMatchKind::Lmctf)
        {
            return BotGameModeT {
                game_type: BotGameType::CTF.to_string(),
                has_teams: Some(true),
                ..mode
            };
        }
        if matches!(
            self.simulation.simulation_mode(),
            BotSimulationMode::Coop | BotSimulationMode::Singleplayer
        ) {
            return BotGameModeT {
                game_type: if self.simulation.objective_cvar("horde") != 0.0 {
                    BotGameType::HORDE.to_string()
                } else {
                    BotGameType::COOP.to_string()
                },
                has_teams: Some(true),
                ..mode
            };
        }
        if self.simulation.match_kind() == BotMatchKind::DeathBall {
            return BotGameModeT {
                has_teams: Some(true),
                ..mode
            };
        }
        mode
    }

    fn team(&self, actor: &ActorId) -> i32 {
        if self.simulation.simulation_mode() != BotSimulationMode::Deathmatch
            && self.simulation.has_movement_player(actor)
        {
            return 1;
        }
        if matches!(self.simulation.match_kind(), BotMatchKind::Ctf | BotMatchKind::Lmctf) {
            return self.simulation.match_team(actor);
        }
        if self.simulation.match_kind() == BotMatchKind::DeathBall {
            let skin = self.simulation.deathball_skin(actor);
            let settings = self.simulation.deathball_teams();
            return match (skin, settings) {
                (Some(skin), Some((team1, _))) if skin == team1 => 1,
                (Some(skin), Some((_, team2))) if skin == team2 => 2,
                _ => 0,
            };
        }
        match self.simulation.combat_team(actor).as_deref() {
            Some("team:red" | "red" | "1") => 1,
            Some("team:blue" | "blue" | "2") => 2,
            _ => 0,
        }
    }

    fn carrying(&self, actor: &ActorId) -> bool {
        if self
            .simulation
            .source_objectives()
            .iter()
            .any(|objective| objective.carrier.as_ref() == Some(actor))
        {
            return true;
        }
        if self.simulation.has_q1_ctf() {
            return self.simulation.q1_ctf_carried(actor);
        }
        if self.simulation.match_kind() == BotMatchKind::Lmctf {
            return self.simulation.lmctf_flag_carried(actor);
        }
        if self.simulation.match_kind() == BotMatchKind::Ctf {
            return self.simulation.inventory_count(actor, "q2:item_flag_team1") > 0
                || self.simulation.inventory_count(actor, "q2:item_flag_team2") > 0;
        }
        self.simulation.match_kind() == BotMatchKind::Tag && self.simulation.tag_owner().as_ref() == Some(actor)
    }

    fn goal(&self, actor: &ActorId) -> Option<Vec3> {
        let objective = self.simulation.source_objectives().into_iter().find(|objective| {
            objective.bot_goal && !objective.complete && objective.target.as_ref().is_some_and(|target| target != actor)
        });
        if let Some(target) = objective.and_then(|objective| objective.target) {
            return self.simulation.body_origin(&target);
        }
        if self.simulation.simulation_mode() != BotSimulationMode::Deathmatch {
            return self.simulation.rerelease_poi();
        }
        let target = match self.simulation.match_kind() {
            BotMatchKind::Tag => self.simulation.tag_owner(),
            BotMatchKind::DeathBall => self.simulation.deathball_ball(),
            _ => None,
        };
        match target {
            None => None,
            Some(target) if target == *actor => None,
            Some(target) => self.simulation.body_origin(&target),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct FakeSimulation {
        mode: BotSimulationMode,
        kind: BotMatchKind,
        teams: HashMap<ActorId, i32>,
        players: Vec<ActorId>,
        commands: RefCell<Vec<(ActorId, String)>>,
        objectives: Vec<BotSourceObjective>,
        origins: HashMap<ActorId, Vec3>,
    }

    impl BotObjectiveSimulation for FakeSimulation {
        fn has_objective_registry(&self) -> bool {
            true
        }
        fn objective_cvar(&self, _name: &str) -> f64 {
            0.0
        }
        fn objective_players(&self) -> Vec<ActorId> {
            self.players.clone()
        }
        fn player_team_command(&self, actor: &ActorId, team: &str) {
            self.commands.borrow_mut().push((actor.clone(), team.to_string()));
        }
        fn simulation_mode(&self) -> BotSimulationMode {
            self.mode
        }
        fn has_movement_player(&self, _actor: &ActorId) -> bool {
            true
        }
        fn combat_team(&self, _actor: &ActorId) -> Option<String> {
            None
        }
        fn has_q1_ctf(&self) -> bool {
            false
        }
        fn q1_ctf_carried(&self, _actor: &ActorId) -> bool {
            false
        }
        fn match_kind(&self) -> BotMatchKind {
            self.kind
        }
        fn match_team(&self, actor: &ActorId) -> i32 {
            self.teams.get(actor).copied().unwrap_or(0)
        }
        fn deathball_skin(&self, _actor: &ActorId) -> Option<String> {
            None
        }
        fn deathball_teams(&self) -> Option<(String, String)> {
            None
        }
        fn deathball_ball(&self) -> Option<ActorId> {
            None
        }
        fn lmctf_flag_carried(&self, _actor: &ActorId) -> bool {
            false
        }
        fn tag_owner(&self) -> Option<ActorId> {
            None
        }
        fn source_objectives(&self) -> Vec<BotSourceObjective> {
            self.objectives.clone()
        }
        fn body_origin(&self, actor: &ActorId) -> Option<Vec3> {
            self.origins.get(actor).copied()
        }
        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }
        fn rerelease_poi(&self) -> Option<Vec3> {
            None
        }
    }

    fn fixture(kind: BotMatchKind) -> (IdentityOwner, ActorId, FakeSimulation) {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let red = owner.actor(2, 1);
        let mut teams = HashMap::new();
        teams.insert(red.clone(), 1);
        (
            owner,
            actor.clone(),
            FakeSimulation {
                mode: BotSimulationMode::Deathmatch,
                kind,
                teams,
                players: vec![actor, red],
                commands: RefCell::new(Vec::new()),
                objectives: Vec::new(),
                origins: HashMap::new(),
            },
        )
    }

    #[test]
    fn admit_balances_ctf_teams() {
        let (_owner, actor, sim) = fixture(BotMatchKind::Ctf);
        let objectives = rerelease_bot_objectives(sim, Rc::new(BotKnowledge::default())).unwrap();
        objectives.admit(&actor);
        let commands = objectives.simulation.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].1, "blue");
    }

    #[test]
    fn mode_reports_ctf_with_teams() {
        let (_owner, actor, sim) = fixture(BotMatchKind::Ctf);
        let objectives = rerelease_bot_objectives(sim, Rc::new(BotKnowledge::default())).unwrap();
        let mode = objectives.mode();
        assert_eq!(mode.game_type, "ctf");
        assert_eq!(mode.has_teams, Some(true));
        assert_eq!(objectives.team(&actor), 0);
    }

    #[test]
    fn coop_players_share_team_one() {
        let (_owner, actor, mut sim) = fixture(BotMatchKind::Open);
        sim.mode = BotSimulationMode::Coop;
        let objectives = rerelease_bot_objectives(sim, Rc::new(BotKnowledge::default())).unwrap();
        assert_eq!(objectives.team(&actor), 1);
        assert_eq!(objectives.mode().game_type, "coop");
    }

    #[test]
    fn goal_prefers_live_bot_objectives() {
        let (_owner, actor, mut sim) = fixture(BotMatchKind::Open);
        let target = sim.players[1].clone();
        sim.objectives.push(BotSourceObjective {
            bot_goal: true,
            complete: false,
            target: Some(target.clone()),
            carrier: None,
        });
        sim.origins.insert(target, Vec3 { x: 5.0, y: 0.0, z: 0.0 });
        let objectives = rerelease_bot_objectives(sim, Rc::new(BotKnowledge::default())).unwrap();
        assert_eq!(objectives.goal(&actor), Some(Vec3 { x: 5.0, y: 0.0, z: 0.0 }));
    }
}
