//! Mission-pack runtime (src/content/q1/missionpacks/runtime.ts).

use std::rc::Rc;
use std::sync::Arc;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::q1::base::player::{Q1CharacterPresentation, Q1CharacterSourcePose};
use crate::q1::base::rules::{Q1Obituary, Q1SourceFinale};
use crate::q1::base::travel::Q1TravelState;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::Q1PlayerExtension;
use crate::q1::Q1Error;

use super::arsenal::{register_mission_pack_arsenal, MissionPackArsenal};
use super::backpacks::drop_mission_pack_backpack;
use super::commands::{
    dump_mission_pack_coordinates, mission_pack_command, CheatArsenalCategory, MissionPackCommandOptions,
};
use super::monsters::{become_decoy, register_mission_pack_monsters, MissionMonsterHooks, Q1MissionPackMonsters};
use super::obituaries::{mission_pack_obituary, MissionPackObituaryContext, Q1MissionPackObituaryInput};
use super::presentation::{mission_pack_character_pose, MissionPackCharacterEffects};
use super::travel::{
    admit_mission_pack_travel, capture_mission_pack_travel, decode_mission_pack_travel, new_mission_pack_travel,
};
use super::types::Q1MissionPack;
use super::world::{register_missionpack_world, MissionpackWorldHooks, Q1MissionpackWorld};

/// Mission-pack runtime options (`Q1MissionPackOptions`).
#[derive(Clone, Default)]
#[allow(clippy::type_complexity)]
pub struct Q1MissionPackOptions {
    /// Finale presentation sink.
    pub present_finale: Option<Arc<dyn Fn(&Q1SourceFinale) + Send + Sync>>,
    /// Session game config flags probe.
    pub gamecfg: Option<Arc<dyn Fn() -> i32 + Send + Sync>>,
    /// Team color probe.
    pub team_color: Option<Arc<dyn Fn(&ActorId) -> i32 + Send + Sync>>,
    /// Team color sink.
    pub set_team_color: Option<Arc<dyn Fn(&ActorId, i32) + Send + Sync>>,
    /// Frag adjustment sink.
    pub add_frags: Option<Arc<dyn Fn(&ActorId, i32) + Send + Sync>>,
    /// Frag probe.
    pub frags: Option<Arc<dyn Fn(&ActorId) -> i32 + Send + Sync>>,
    /// Disconnect sink.
    pub disconnect: Option<Arc<dyn Fn(&ActorId) + Send + Sync>>,
    /// Player frame probe.
    pub player_frame: Option<Arc<dyn Fn(&ActorId) -> i32 + Send + Sync>>,
    /// Player name probe.
    pub player_name: Option<Arc<dyn Fn(&ActorId) -> String + Send + Sync>>,
    /// Session cheat-arsenal override.
    pub cheat_arsenal: Option<Rc<dyn Fn(&ActorId, CheatArsenalCategory) -> bool>>,
    /// Session cheat permission probe.
    pub cheats_allowed: Option<Rc<dyn Fn() -> bool>>,
    /// Developer log sink.
    pub developer_message: Option<Rc<dyn Fn(&str)>>,
    /// Footstep probe.
    pub footsteps: Option<Rc<dyn Fn() -> bool>>,
}

impl std::fmt::Debug for Q1MissionPackOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Q1MissionPackOptions").finish_non_exhaustive()
    }
}

impl Q1MissionPackOptions {
    /// Command options view.
    fn command_options(&self) -> MissionPackCommandOptions {
        MissionPackCommandOptions {
            cheat_arsenal: self.cheat_arsenal.clone(),
            cheats_allowed: self.cheats_allowed.clone(),
            developer_message: self.developer_message.clone(),
        }
    }

    /// World hooks with the arsenal and monster services attached.
    /// Assumes the sibling world hooks mirror the donor with game-passing
    /// function pointers for game-reaching hooks and shared closures for
    /// session hooks.
    fn world_hooks(&self) -> MissionpackWorldHooks {
        MissionpackWorldHooks {
            charmer: Some(Box::new(MissionPackArsenal::horn_charmer)),
            charm: Some(Box::new(Q1MissionPackMonsters::charm)),
            become_decoy: Some(Box::new(become_decoy)),
            present_finale: self.present_finale.clone().map(|hook| {
                Box::new(move |result: &Q1SourceFinale| hook(result)) as Box<dyn Fn(&Q1SourceFinale) + Send>
            }),
            gamecfg: self
                .gamecfg
                .clone()
                .map(|hook| Box::new(move || hook()) as Box<dyn Fn() -> i32 + Send>),
            team_color: self
                .team_color
                .clone()
                .map(|hook| Box::new(move |actor: &ActorId| hook(actor)) as Box<dyn Fn(&ActorId) -> i32 + Send>),
            set_team_color: self.set_team_color.clone().map(|hook| {
                Box::new(move |actor: &ActorId, team: i32| hook(actor, team)) as Box<dyn Fn(&ActorId, i32) + Send>
            }),
            add_frags: self.add_frags.clone().map(|hook| {
                Box::new(move |actor: &ActorId, delta: i32| hook(actor, delta)) as Box<dyn Fn(&ActorId, i32) + Send>
            }),
            frags: self
                .frags
                .clone()
                .map(|hook| Box::new(move |actor: &ActorId| hook(actor)) as Box<dyn Fn(&ActorId) -> i32 + Send>),
            disconnect: self
                .disconnect
                .clone()
                .map(|hook| Box::new(move |actor: &ActorId| hook(actor)) as Box<dyn Fn(&ActorId) + Send>),
            player_frame: self
                .player_frame
                .clone()
                .map(|hook| Box::new(move |actor: &ActorId| hook(actor)) as Box<dyn Fn(&ActorId) -> i32 + Send>),
            player_name: self
                .player_name
                .clone()
                .map(|hook| Box::new(move |actor: &ActorId| hook(actor)) as Box<dyn Fn(&ActorId) -> String + Send>),
        }
    }
}

/// World postthink hook (`q1:{pack}:world-postthink`).
fn world_postthink(game: &mut Q1EntityServices, player: &ActorId, seconds: f64) -> Result<(), Q1Error> {
    Q1MissionpackWorld::after_physics(game, player, seconds)
}

/// Coordinate dump hook (`q1:hipnotic:coordinate-dump`).
fn coordinate_dump(game: &mut Q1EntityServices, player: &ActorId, _seconds: f64) -> Result<(), Q1Error> {
    dump_mission_pack_coordinates(game, player);
    Ok(())
}

/// Mission-pack runtime (`Q1MissionPackRuntime`).
pub struct Q1MissionPackRuntime {
    /// Mission pack.
    pub pack: Q1MissionPack,
    /// Runtime options.
    pub options: Q1MissionPackOptions,
    /// Mission-pack arsenal.
    pub arsenal: MissionPackArsenal,
    /// Mission-pack monsters.
    pub monsters: Q1MissionPackMonsters,
    /// Mission-pack world.
    pub world: Q1MissionpackWorld,
    /// Character effects.
    character_effects: MissionPackCharacterEffects,
}

impl Q1MissionPackRuntime {
    /// Register a mission-pack runtime on a game.
    pub fn new(
        game: &mut Q1EntityServices,
        pack: Q1MissionPack,
        options: Q1MissionPackOptions,
    ) -> Result<Self, Q1Error> {
        let arsenal = register_mission_pack_arsenal(game, pack)?;
        let footsteps = options.footsteps.clone().unwrap_or_else(|| Rc::new(|| false));
        let character_effects = MissionPackCharacterEffects::new(game, pack, footsteps)?;
        let monsters = register_mission_pack_monsters(
            game,
            pack,
            MissionMonsterHooks {
                charmer: Some(MissionPackArsenal::horn_charmer),
            },
        )?;
        let world = register_missionpack_world(game, pack, options.world_hooks())?;
        game.register_player_extension(Q1PlayerExtension {
            id: format!("q1:{}:world-postthink", pack.as_str()),
            after_physics: Some(world_postthink),
            ..Default::default()
        })?;
        if pack == Q1MissionPack::Hipnotic {
            game.register_player_extension(Q1PlayerExtension {
                id: String::from("q1:hipnotic:coordinate-dump"),
                frame: Some(coordinate_dump),
                ..Default::default()
            })?;
        }
        Ok(Self {
            pack,
            options,
            arsenal,
            monsters,
            world,
            character_effects,
        })
    }

    /// Handle an impulse (`impulse`).
    pub fn impulse(&self, game: &mut Q1EntityServices, actor: &ActorId, impulse: i32) -> Result<bool, Q1Error> {
        let player = match game.player_ref(actor) {
            Some(player) => player.actor.id().clone(),
            None => return Ok(false),
        };
        if mission_pack_command(
            game,
            &self.arsenal.players,
            &player,
            self.pack,
            impulse,
            &self.options.command_options(),
        )? {
            return Ok(true);
        }
        if self.arsenal.impulse(game, actor, impulse)? {
            return Ok(true);
        }
        Q1MissionpackWorld::impulse(game, actor, impulse)
    }

    /// Note a player spawn (`playerSpawned`).
    pub fn player_spawned(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        Q1MissionpackWorld::player_spawned(game, actor)
    }

    /// Note confirmed damage (`confirmedDamage`).
    pub fn confirmed_damage(
        &self,
        game: &mut Q1EntityServices,
        target: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        Q1MissionpackWorld::confirmed_damage(game, target, attacker)
    }

    /// Note a player death (`playerDied`).
    pub fn player_died(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        let source = attacker.and_then(|attacker| game.entity_ref(attacker));
        let shield_owner = match source {
            Some(source) if self.pack == Q1MissionPack::Rogue && source.classname == "power_shield" => {
                source.owner.clone()
            }
            _ => attacker.cloned(),
        };
        Q1MissionpackWorld::player_died(game, actor, shield_owner.as_ref())
    }

    /// Fresh travel state (`newTravel`).
    pub fn new_travel(&self, game: &mut Q1EntityServices) -> Q1TravelState {
        new_mission_pack_travel(game, self.pack)
    }

    /// Capture travel state (`captureTravel`).
    pub fn capture_travel(&self, game: &mut Q1EntityServices, actor: &OwnedActor) -> Result<Q1TravelState, Q1Error> {
        capture_mission_pack_travel(game, actor, self.pack)
    }

    /// Decode travel state (`decodeTravel`).
    pub fn decode_travel(
        &self,
        game: &mut Q1EntityServices,
        state: &Q1TravelState,
        server_flags: i32,
    ) -> Q1TravelState {
        decode_mission_pack_travel(game, state, server_flags, self.pack)
    }

    /// Admit travel state (`admitTravel`).
    pub fn admit_travel(
        &self,
        game: &mut Q1EntityServices,
        actor: &OwnedActor,
        state: &Q1TravelState,
    ) -> Result<(), Q1Error> {
        admit_mission_pack_travel(game, actor, state, self.pack)
    }

    /// Drop a death backpack (`dropBackpack`).
    pub fn drop_backpack(&self, game: &mut Q1EntityServices, actor: &OwnedActor) -> Result<Option<ActorId>, Q1Error> {
        drop_mission_pack_backpack(game, actor, self.pack)
    }

    /// Character pose (`characterPose`).
    pub fn character_pose(&self, game: &Q1EntityServices, actor: &ActorId) -> Q1CharacterSourcePose {
        mission_pack_character_pose(game, actor, self.pack)
    }

    /// Character frame effects (`characterFrame`).
    pub fn character_frame(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        presentation: &Q1CharacterPresentation,
    ) -> Result<(), Q1Error> {
        self.character_effects.frame(game, actor, presentation)
    }

    /// Select a spawn point (`selectSpawn`).
    pub fn select_spawn(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Option<ActorId> {
        Q1MissionpackWorld::select_spawn(game, actor)
    }

    /// Mission-pack obituary (`obituary`).
    pub fn obituary(
        &self,
        game: &mut Q1EntityServices,
        input: &Q1MissionPackObituaryInput,
        inflictor: Option<&ActorId>,
    ) -> Q1Obituary {
        let inflictor_classname = inflictor
            .map(|inflictor| game.host.classname(inflictor))
            .unwrap_or_default();
        let attacker_death_type = input
            .attacker
            .as_ref()
            .and_then(|attacker| game.entity_ref(&attacker.actor))
            .map(|entity| entity.text("deathtype"))
            .unwrap_or_default();
        let victim_saved_team = Q1MissionpackWorld::saved_team(game, &input.victim.actor);
        let gamecfg = self.options.gamecfg.as_ref().map(|gamecfg| gamecfg()).unwrap_or(0);
        // The donor calls the tag probe lazily in the Rogue tag branch; this
        // port evaluates it eagerly in exactly that branch.
        let mut tag_score: Option<Box<dyn Fn() -> i32>> = None;
        if self.pack == Q1MissionPack::Rogue && input.teamplay == 3 {
            if let Some(attacker) = input.attacker.as_ref() {
                let score = Q1MissionpackWorld::tag_score(game, &input.victim.actor, &attacker.actor);
                tag_score = Some(Box::new(move || score));
            }
        }
        let context = MissionPackObituaryContext {
            pack: self.pack,
            inflictor_classname,
            attacker_death_type,
            victim_saved_team,
            gamecfg,
            tag_score,
        };
        mission_pack_obituary(&input.clone(), &context, &mut || game.host.random())
    }

    /// Charm a monster through the monster services.
    pub fn charm(&self, game: &mut Q1EntityServices, entity: &ActorId, charmer: &ActorId) -> Result<(), Q1Error> {
        Q1MissionPackMonsters::charm(game, entity, charmer)
    }

    /// Spawn a decoy through the monster services.
    pub fn decoy(&self, game: &mut Q1EntityServices, target: &str, origin: Vec3) -> Result<ActorId, Q1Error> {
        become_decoy(game, target, origin)
    }
}

/// Register a mission-pack runtime on a game (`registerQ1MissionPack`).
pub fn register_q1_mission_pack(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
    options: Q1MissionPackOptions,
) -> Result<Q1MissionPackRuntime, Q1Error> {
    Q1MissionPackRuntime::new(game, pack, options)
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};

    #[test]
    fn options_default_is_empty() {
        let options = Q1MissionPackOptions::default();
        assert!(options.gamecfg.is_none());
        assert!(options.footsteps.is_none());
        assert!(options.cheat_arsenal.is_none());
    }

    #[test]
    fn runtime_registers_pack() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let runtime = register_q1_mission_pack(&mut game, Q1MissionPack::Hipnotic, Q1MissionPackOptions::default())
            .expect("runtime");
        assert_eq!(runtime.pack, Q1MissionPack::Hipnotic);
        assert!(game
            .registered_weapons
            .contains_key(&crate::q1::foundation::types::Q1Weapon::HipnoticLaser));
    }

    #[test]
    fn runtime_registers_rogue_pack() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let runtime = register_q1_mission_pack(&mut game, Q1MissionPack::Rogue, Q1MissionPackOptions::default())
            .expect("runtime");
        assert_eq!(runtime.pack, Q1MissionPack::Rogue);
        assert!(game
            .registered_weapons
            .contains_key(&crate::q1::foundation::types::Q1Weapon::RoguePlasma));
    }
}
