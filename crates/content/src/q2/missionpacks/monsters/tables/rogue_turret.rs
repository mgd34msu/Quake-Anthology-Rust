//! turret move tables (`src/content/q2/missionpacks/monsters/tables/rogue-turret.ts`).
//!
//! Original Quake II rogue/m_turret.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `turretFrame`.
pub mod turret_frame {
    /// Frame `stand01`.
    pub const STAND01: i32 = 0;
    /// Frame `stand02`.
    pub const STAND02: i32 = 1;
    /// Frame `active01`.
    pub const ACTIVE01: i32 = 2;
    /// Frame `active02`.
    pub const ACTIVE02: i32 = 3;
    /// Frame `active03`.
    pub const ACTIVE03: i32 = 4;
    /// Frame `active04`.
    pub const ACTIVE04: i32 = 5;
    /// Frame `active05`.
    pub const ACTIVE05: i32 = 6;
    /// Frame `active06`.
    pub const ACTIVE06: i32 = 7;
    /// Frame `run01`.
    pub const RUN01: i32 = 8;
    /// Frame `run02`.
    pub const RUN02: i32 = 9;
    /// Frame `pow01`.
    pub const POW01: i32 = 10;
    /// Frame `pow02`.
    pub const POW02: i32 = 11;
    /// Frame `pow03`.
    pub const POW03: i32 = 12;
    /// Frame `pow04`.
    pub const POW04: i32 = 13;
    /// Frame `death01`.
    pub const DEATH01: i32 = 14;
    /// Frame `death02`.
    pub const DEATH02: i32 = 15;
}

/// `turretMoves` move tables.
pub fn turret_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("turret_move_stand", 0, 1, None, vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("turret_move_ready_gun", 2, 8, Some("turret_run"), vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("turret_move_seek", 8, 9, None, vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("TurretAim")], -1),
        ]),
        monster_move("turret_move_run", 8, 9, Some("turret_run"), vec![
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
        ]),
        monster_move("turret_move_fire", 10, 13, Some("turret_run"), vec![
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretFire")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
        ]),
        monster_move("turret_move_fire_blind", 10, 13, Some("turret_run"), vec![
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretAim")], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![MonsterAction::name("TurretFireBlind")], -1),
        ]),
    ]
}
