//! Selected Q1 expansion registration.
//!
//! Absolute donor:
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/monster-sources.ts`
//!
//! The donor builds a `Q1Base` over the game and routes mission packs
//! through it while addons get a fresh `Q1AddonContext`. The Rust content
//! registers base state behind a [`Q1BaseGuard`] (dropped state unregisters,
//! so the guard is returned), takes mission-pack hooks by value, and binds
//! addon engine services through [`register_addon_context`]. The port keeps
//! the donor routing and returns everything the caller must hold.

use qa_content::monsters::MonsterProgram;
use qa_content::q1::addons::context::{register_addon_context, Q1AddonContext, Q1AddonProgram, Q1AddonServices};
use qa_content::q1::addons::monsters::register_addon_monsters;
use qa_content::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
use qa_content::q1::foundation::entity_services::Q1EntityServices;
use qa_content::q1::missionpacks::monsters::register_mission_pack_monsters;
use qa_content::q1::missionpacks::monsters::{MissionMonsterHooks, Q1MissionPackMonsters};
use qa_content::q1::missionpacks::types::Q1MissionPack;
use qa_content::q1::Q1Error;

/// Registered Q1 expansion content (donor `Q1AddonContext | null`).
pub struct SelectedQ1Expansion {
    /// Base content registration; dropping it unregisters base state.
    pub base: Q1BaseGuard,
    /// Addon context, for addon programs.
    pub context: Option<Q1AddonContext>,
    /// Mission-pack monsters, for pack programs.
    pub pack: Option<Q1MissionPackMonsters>,
}

/// Register the selected Q1 expansion's monsters into a game.
///
/// Mission-pack programs consume `hooks`; addon programs consume `services`.
/// The base registers with `official_campaign: false`, matching the donor.
pub fn register_selected_q1_expansion(
    game: &mut Q1EntityServices,
    program: MonsterProgram,
    services: Box<dyn Q1AddonServices>,
    hooks: MissionMonsterHooks,
) -> Result<SelectedQ1Expansion, Q1Error> {
    let base = Q1BaseGuard::register(
        game,
        Q1BaseOptions {
            campaign: None,
            registered: None,
            finale_finished: None,
            finish_campaign: None,
            same_level: None,
            player_exited: None,
            official_campaign: Some(false),
        },
    )?;
    match program {
        MonsterProgram::Hipnotic | MonsterProgram::Rogue => {
            let pack = if program == MonsterProgram::Hipnotic {
                Q1MissionPack::Hipnotic
            } else {
                Q1MissionPack::Rogue
            };
            let monsters = register_mission_pack_monsters(game, pack, hooks)?;
            Ok(SelectedQ1Expansion {
                base,
                context: None,
                pack: Some(monsters),
            })
        }
        MonsterProgram::Dopa | MonsterProgram::Mg1 | MonsterProgram::Mg3 => {
            let addon = match program {
                MonsterProgram::Dopa => Q1AddonProgram::Dopa,
                MonsterProgram::Mg1 => Q1AddonProgram::Mg1,
                _ => Q1AddonProgram::Mg3,
            };
            let context = register_addon_context(game, addon, services)?;
            register_addon_monsters(&context, game)?;
            Ok(SelectedQ1Expansion {
                base,
                context: Some(context),
                pack: None,
            })
        }
        MonsterProgram::Id1 | MonsterProgram::Baseq2 | MonsterProgram::Xatrix | MonsterProgram::Mg2 => Err(
            Q1Error::Message(format!("selected Q1 expansion has no {} program", program.as_str())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q1::addons::context::Q1AddonEvent;
    use qa_core::identity::ActorId;

    use super::super::test_hosts::test_q1_game;

    struct DummyAddonServices;

    impl Q1AddonServices for DummyAddonServices {
        fn emit(&mut self, _event: Q1AddonEvent) {}

        fn is_monster(&mut self, _game: &Q1EntityServices, _actor: &ActorId) -> bool {
            false
        }

        fn cvar(&mut self, _name: &str) -> f64 {
            0.0
        }

        fn set_cvar(&mut self, _name: &str, _value: &str) {}
    }

    fn services() -> Box<dyn Q1AddonServices> {
        Box::new(DummyAddonServices)
    }

    fn hooks() -> MissionMonsterHooks {
        MissionMonsterHooks { charmer: None }
    }

    #[test]
    fn registers_hipnotic_pack_without_context() {
        let (mut game, _handles) = test_q1_game();
        let expansion =
            register_selected_q1_expansion(&mut game, MonsterProgram::Hipnotic, services(), hooks()).expect("hipnotic");
        assert!(expansion.context.is_none());
        assert!(expansion.pack.is_some());
    }

    #[test]
    fn registers_rogue_pack_without_context() {
        let (mut game, _handles) = test_q1_game();
        let expansion =
            register_selected_q1_expansion(&mut game, MonsterProgram::Rogue, services(), hooks()).expect("rogue");
        assert!(expansion.context.is_none());
        assert!(expansion.pack.is_some());
    }

    #[test]
    fn registers_dopa_addon_with_context() {
        let (mut game, _handles) = test_q1_game();
        let expansion =
            register_selected_q1_expansion(&mut game, MonsterProgram::Dopa, services(), hooks()).expect("dopa");
        assert_eq!(expansion.context.expect("context").program(), Q1AddonProgram::Dopa);
        assert!(expansion.pack.is_none());
    }

    #[test]
    fn rejects_base_programs() {
        let (mut game, _handles) = test_q1_game();
        assert!(register_selected_q1_expansion(&mut game, MonsterProgram::Id1, services(), hooks()).is_err());
        let (mut game, _handles) = test_q1_game();
        assert!(register_selected_q1_expansion(&mut game, MonsterProgram::Baseq2, services(), hooks()).is_err());
    }
}
