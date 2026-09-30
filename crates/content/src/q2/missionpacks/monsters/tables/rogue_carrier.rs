//! carrier move tables (`src/content/q2/missionpacks/monsters/tables/rogue-carrier.ts`).
//!
//! Original Quake II rogue/m_carrier.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `carrierFrame`.
pub mod carrier_frame {
    /// Frame `search01`.
    pub const SEARCH01: i32 = 0;
    /// Frame `search02`.
    pub const SEARCH02: i32 = 1;
    /// Frame `search03`.
    pub const SEARCH03: i32 = 2;
    /// Frame `search04`.
    pub const SEARCH04: i32 = 3;
    /// Frame `search05`.
    pub const SEARCH05: i32 = 4;
    /// Frame `search06`.
    pub const SEARCH06: i32 = 5;
    /// Frame `search07`.
    pub const SEARCH07: i32 = 6;
    /// Frame `search08`.
    pub const SEARCH08: i32 = 7;
    /// Frame `search09`.
    pub const SEARCH09: i32 = 8;
    /// Frame `search10`.
    pub const SEARCH10: i32 = 9;
    /// Frame `search11`.
    pub const SEARCH11: i32 = 10;
    /// Frame `search12`.
    pub const SEARCH12: i32 = 11;
    /// Frame `search13`.
    pub const SEARCH13: i32 = 12;
    /// Frame `firea01`.
    pub const FIREA01: i32 = 13;
    /// Frame `firea02`.
    pub const FIREA02: i32 = 14;
    /// Frame `firea03`.
    pub const FIREA03: i32 = 15;
    /// Frame `firea04`.
    pub const FIREA04: i32 = 16;
    /// Frame `firea05`.
    pub const FIREA05: i32 = 17;
    /// Frame `firea06`.
    pub const FIREA06: i32 = 18;
    /// Frame `firea07`.
    pub const FIREA07: i32 = 19;
    /// Frame `firea08`.
    pub const FIREA08: i32 = 20;
    /// Frame `firea09`.
    pub const FIREA09: i32 = 21;
    /// Frame `firea10`.
    pub const FIREA10: i32 = 22;
    /// Frame `firea11`.
    pub const FIREA11: i32 = 23;
    /// Frame `firea12`.
    pub const FIREA12: i32 = 24;
    /// Frame `firea13`.
    pub const FIREA13: i32 = 25;
    /// Frame `firea14`.
    pub const FIREA14: i32 = 26;
    /// Frame `firea15`.
    pub const FIREA15: i32 = 27;
    /// Frame `fireb01`.
    pub const FIREB01: i32 = 28;
    /// Frame `fireb02`.
    pub const FIREB02: i32 = 29;
    /// Frame `fireb03`.
    pub const FIREB03: i32 = 30;
    /// Frame `fireb04`.
    pub const FIREB04: i32 = 31;
    /// Frame `fireb05`.
    pub const FIREB05: i32 = 32;
    /// Frame `fireb06`.
    pub const FIREB06: i32 = 33;
    /// Frame `fireb07`.
    pub const FIREB07: i32 = 34;
    /// Frame `fireb08`.
    pub const FIREB08: i32 = 35;
    /// Frame `fireb09`.
    pub const FIREB09: i32 = 36;
    /// Frame `fireb10`.
    pub const FIREB10: i32 = 37;
    /// Frame `fireb11`.
    pub const FIREB11: i32 = 38;
    /// Frame `fireb12`.
    pub const FIREB12: i32 = 39;
    /// Frame `fireb13`.
    pub const FIREB13: i32 = 40;
    /// Frame `fireb14`.
    pub const FIREB14: i32 = 41;
    /// Frame `fireb15`.
    pub const FIREB15: i32 = 42;
    /// Frame `fireb16`.
    pub const FIREB16: i32 = 43;
    /// Frame `spawn01`.
    pub const SPAWN01: i32 = 44;
    /// Frame `spawn02`.
    pub const SPAWN02: i32 = 45;
    /// Frame `spawn03`.
    pub const SPAWN03: i32 = 46;
    /// Frame `spawn04`.
    pub const SPAWN04: i32 = 47;
    /// Frame `spawn05`.
    pub const SPAWN05: i32 = 48;
    /// Frame `spawn06`.
    pub const SPAWN06: i32 = 49;
    /// Frame `spawn07`.
    pub const SPAWN07: i32 = 50;
    /// Frame `spawn08`.
    pub const SPAWN08: i32 = 51;
    /// Frame `spawn09`.
    pub const SPAWN09: i32 = 52;
    /// Frame `spawn10`.
    pub const SPAWN10: i32 = 53;
    /// Frame `spawn11`.
    pub const SPAWN11: i32 = 54;
    /// Frame `spawn12`.
    pub const SPAWN12: i32 = 55;
    /// Frame `spawn13`.
    pub const SPAWN13: i32 = 56;
    /// Frame `spawn14`.
    pub const SPAWN14: i32 = 57;
    /// Frame `spawn15`.
    pub const SPAWN15: i32 = 58;
    /// Frame `spawn16`.
    pub const SPAWN16: i32 = 59;
    /// Frame `spawn17`.
    pub const SPAWN17: i32 = 60;
    /// Frame `spawn18`.
    pub const SPAWN18: i32 = 61;
    /// Frame `death01`.
    pub const DEATH01: i32 = 62;
    /// Frame `death02`.
    pub const DEATH02: i32 = 63;
    /// Frame `death03`.
    pub const DEATH03: i32 = 64;
    /// Frame `death04`.
    pub const DEATH04: i32 = 65;
    /// Frame `death05`.
    pub const DEATH05: i32 = 66;
    /// Frame `death06`.
    pub const DEATH06: i32 = 67;
    /// Frame `death07`.
    pub const DEATH07: i32 = 68;
    /// Frame `death08`.
    pub const DEATH08: i32 = 69;
    /// Frame `death09`.
    pub const DEATH09: i32 = 70;
    /// Frame `death10`.
    pub const DEATH10: i32 = 71;
    /// Frame `death11`.
    pub const DEATH11: i32 = 72;
    /// Frame `death12`.
    pub const DEATH12: i32 = 73;
    /// Frame `death13`.
    pub const DEATH13: i32 = 74;
    /// Frame `death14`.
    pub const DEATH14: i32 = 75;
    /// Frame `death15`.
    pub const DEATH15: i32 = 76;
    /// Frame `death16`.
    pub const DEATH16: i32 = 77;
}

/// `carrierMoves` move tables.
pub fn carrier_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("carrier_move_stand", 0, 12, None, vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("carrier_move_walk", 0, 12, None, vec![
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
        ]),
        monster_move("carrier_move_run", 0, 12, None, vec![
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
        ]),
        monster_move("carrier_move_attack_pre_mg", 13, 20, None, vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("carrier_attack_mg")], -1),
        ]),
        monster_move("carrier_move_attack_mg", 21, 23, None, vec![
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("carrier_reattack_mg")], -1),
        ]),
        monster_move("carrier_move_attack_post_mg", 24, 27, Some("carrier_run"), vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
        ]),
        monster_move("carrier_move_attack_pre_gren", 28, 33, None, vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("carrier_attack_gren")], -1),
        ]),
        monster_move("carrier_move_attack_gren", 34, 37, None, vec![
            monster_frame(MonsterAi::Charge, -15.0, vec![MonsterAction::name("CarrierGrenade")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("carrier_reattack_gren")], -1),
        ]),
        monster_move("carrier_move_attack_post_gren", 38, 43, Some("carrier_run"), vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
        ]),
        monster_move("carrier_move_attack_rocket", 28, 28, Some("carrier_run"), vec![
            monster_frame(MonsterAi::Charge, 15.0, vec![MonsterAction::name("CarrierRocket")], -1),
        ]),
        monster_move("carrier_move_attack_rail", 0, 8, Some("carrier_run"), vec![
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierSaveLoc")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, -20.0, vec![MonsterAction::name("CarrierRail")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("CarrierCoopCheck")], -1),
        ]),
        monster_move("carrier_move_spawn", 44, 61, None, vec![
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("carrier_prep_spawn")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("carrier_start_spawn")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("carrier_ready_spawn")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -10.0, vec![MonsterAction::name("carrier_spawn_check")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("CarrierMachineGun")], -1),
            monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("carrier_reattack_mg")], -1),
        ]),
        monster_move("carrier_move_pain_heavy", 62, 71, Some("carrier_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("carrier_move_pain_light", 44, 47, Some("carrier_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("carrier_move_death", 62, 77, Some("carrier_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("BossExplode")], -1),
        ]),
    ]
}
