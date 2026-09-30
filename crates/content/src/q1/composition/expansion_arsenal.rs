//! Selected Q1 mission-weapon dispatch (`src/content/composition/q1-expansion-arsenal.ts`).
//!
//! Donor `registerSelectedQ1MissionWeapons` picks the impulse/frame/pickup
//! bundle by source program: mission-pack registrars for hipnotic/rogue, an
//! mg3 addon context with unofficial-campaign base rules, and the base
//! weapon impulse otherwise. The sole donor call site builds a fresh game,
//! so the mg3 branch registers base content instead of replacing it.

use crate::q1::addons::context::{
    addon_emit, frame_addons, register_addon_context, Q1AddonEvent, Q1AddonProgram, Q1AddonServices,
};
use crate::q1::addons::items::{handle_mg3_item_impulse, register_mg3_items};
use crate::q1::base::provider::{register_q1_base, Q1BaseOptions};
use crate::q1::composition::commands::q1_weapon_impulse;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::Q1PlayerState;
use crate::q1::missionpacks::arsenal::{register_hipnotic_weapons, register_mission_pack_arsenal, MissionPackArsenal};
use crate::q1::missionpacks::selection::mission_weapon_impulse;
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::{q1_error, Q1Error};

/// Selected Q1 source program (`SelectedQ1Program`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedQ1Program {
    /// Original campaign.
    Id1,
    /// Hipnotic expansion.
    Hipnotic,
    /// Rogue expansion.
    Rogue,
    /// Dimension of the Past.
    Dopa,
    /// Dimension of the Machine.
    Mg1,
    /// Honey expansion.
    Mg3,
}

/// Program-selected mission weapons (donor `registerSelectedQ1MissionWeapons`
/// return bundle).
pub struct SelectedQ1Arsenal {
    /// Selected program.
    program: SelectedQ1Program,
    /// Rogue arsenal, present only for the rogue program.
    rogue: Option<MissionPackArsenal>,
}

/// Register program-selected mission weapons (`registerSelectedQ1MissionWeapons`).
pub fn register_selected_q1_mission_weapons(
    game: &mut Q1EntityServices,
    program: SelectedQ1Program,
    services: Box<dyn Q1AddonServices>,
) -> Result<SelectedQ1Arsenal, Q1Error> {
    match program {
        SelectedQ1Program::Hipnotic => {
            register_hipnotic_weapons(game)?;
            Ok(SelectedQ1Arsenal { program, rogue: None })
        }
        SelectedQ1Program::Rogue => {
            let rogue = register_mission_pack_arsenal(game, Q1MissionPack::Rogue)?;
            Ok(SelectedQ1Arsenal {
                program,
                rogue: Some(rogue),
            })
        }
        SelectedQ1Program::Mg3 => {
            register_q1_base(
                game,
                Q1BaseOptions {
                    official_campaign: Some(false),
                    ..Default::default()
                },
            )?;
            register_addon_context(game, Q1AddonProgram::Mg3, services)?;
            register_mg3_items(game)?;
            Ok(SelectedQ1Arsenal { program, rogue: None })
        }
        _ => Ok(SelectedQ1Arsenal { program, rogue: None }),
    }
}

impl SelectedQ1Arsenal {
    /// Selected program.
    #[must_use]
    pub fn program(&self) -> SelectedQ1Program {
        self.program
    }

    /// Handle a weapon impulse (`impulse`).
    pub fn impulse(&self, game: &mut Q1EntityServices, player: &Q1PlayerState, impulse: i32) -> Result<bool, Q1Error> {
        match self.program {
            SelectedQ1Program::Hipnotic => {
                mission_weapon_impulse(game, player.actor.id(), Q1MissionPack::Hipnotic, impulse)
            }
            SelectedQ1Program::Rogue => self
                .rogue
                .as_ref()
                .ok_or_else(|| q1_error("Q1 rogue arsenal was not registered"))?
                .impulse(game, player.actor.id(), impulse),
            SelectedQ1Program::Mg3 => {
                let mut pending = Vec::new();
                let handled = handle_mg3_item_impulse(game, player.actor.id(), impulse, &mut |text| {
                    pending.push(text.to_string())
                })?;
                for text in pending {
                    addon_emit(game, Q1AddonEvent::DeveloperMessage { text })?;
                }
                if handled {
                    Ok(true)
                } else {
                    q1_weapon_impulse(game, player, impulse)
                }
            }
            _ => q1_weapon_impulse(game, player, impulse),
        }
    }

    /// Advance addon frame time (`frame`).
    pub fn frame(&self, game: &mut Q1EntityServices, elapsed_seconds: f64) -> Result<(), Q1Error> {
        if self.program == SelectedQ1Program::Mg3 {
            frame_addons(game, elapsed_seconds)?;
        }
        Ok(())
    }

    /// Enable rogue combo pickups before pickup (`preparePickup`).
    pub fn prepare_pickup(&self, game: &mut Q1EntityServices, player: &Q1PlayerState) -> Result<(), Q1Error> {
        if let Some(rogue) = &self.rogue {
            rogue.players.enable_combos(game, player.actor.id())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{addon_frame_time, addons_registered, attach_test_player, TestAddonServices};
    use crate::q1::base::provider::{base_registered, official_campaign_flag, Q1BaseGuard};
    use crate::q1::missionpacks::types::test_game;

    /// Fresh game with base content and an attached player.
    fn armed() -> (Q1EntityServices, Q1BaseGuard, Q1PlayerState) {
        let mut game = test_game();
        let guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let actor = attach_test_player(&mut game);
        let player = game.player_ref(&actor).cloned().expect("player");
        (game, guard, player)
    }

    #[test]
    fn default_program_uses_base_impulse() {
        let (mut game, _guard, player) = armed();
        for program in [SelectedQ1Program::Id1, SelectedQ1Program::Dopa, SelectedQ1Program::Mg1] {
            let arsenal =
                register_selected_q1_mission_weapons(&mut game, program, Box::new(TestAddonServices::default()))
                    .expect("register");
            assert_eq!(arsenal.program(), program);
            assert!(arsenal.impulse(&mut game, &player, 1).expect("impulse"));
            arsenal.frame(&mut game, 0.016).expect("frame");
            arsenal.prepare_pickup(&mut game, &player).expect("pickup");
        }
    }

    #[test]
    fn hipnotic_routes_to_mission_impulse() {
        let (mut game, _guard, player) = armed();
        let arsenal = register_selected_q1_mission_weapons(
            &mut game,
            SelectedQ1Program::Hipnotic,
            Box::new(TestAddonServices::default()),
        )
        .expect("register");
        assert!(arsenal.impulse(&mut game, &player, 10).expect("impulse"));
    }

    #[test]
    fn rogue_routes_to_arsenal_and_combos() {
        let (mut game, _guard, player) = armed();
        let arsenal = register_selected_q1_mission_weapons(
            &mut game,
            SelectedQ1Program::Rogue,
            Box::new(TestAddonServices::default()),
        )
        .expect("register");
        arsenal.prepare_pickup(&mut game, &player).expect("pickup");
        assert!(arsenal.impulse(&mut game, &player, 20).expect("impulse"));
    }

    #[test]
    fn mg3_registers_unofficial_base_and_addons() {
        let mut game = test_game();
        let arsenal = register_selected_q1_mission_weapons(
            &mut game,
            SelectedQ1Program::Mg3,
            Box::new(TestAddonServices::default()),
        )
        .expect("register");
        assert!(base_registered(&game));
        assert!(!official_campaign_flag(&game).expect("flag"));
        assert!(addons_registered(&game));
        arsenal.frame(&mut game, 0.5).expect("frame");
        assert_eq!(addon_frame_time(&game).expect("time"), 0.5);
        let actor = attach_test_player(&mut game);
        let player = game.player_ref(&actor).cloned().expect("player");
        assert!(arsenal.impulse(&mut game, &player, 1).expect("impulse"));
    }
}
