//! stalker move tables (`src/content/q2/missionpacks/monsters/tables/rogue-stalker.ts`).
//!
//! Original Quake II rogue/m_stalker.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `stalkerFrame`.
pub mod stalker_frame {
    /// Frame `idle01`.
    pub const IDLE01: i32 = 0;
    /// Frame `idle02`.
    pub const IDLE02: i32 = 1;
    /// Frame `idle03`.
    pub const IDLE03: i32 = 2;
    /// Frame `idle04`.
    pub const IDLE04: i32 = 3;
    /// Frame `idle05`.
    pub const IDLE05: i32 = 4;
    /// Frame `idle06`.
    pub const IDLE06: i32 = 5;
    /// Frame `idle07`.
    pub const IDLE07: i32 = 6;
    /// Frame `idle08`.
    pub const IDLE08: i32 = 7;
    /// Frame `idle09`.
    pub const IDLE09: i32 = 8;
    /// Frame `idle10`.
    pub const IDLE10: i32 = 9;
    /// Frame `idle11`.
    pub const IDLE11: i32 = 10;
    /// Frame `idle12`.
    pub const IDLE12: i32 = 11;
    /// Frame `idle13`.
    pub const IDLE13: i32 = 12;
    /// Frame `idle14`.
    pub const IDLE14: i32 = 13;
    /// Frame `idle15`.
    pub const IDLE15: i32 = 14;
    /// Frame `idle16`.
    pub const IDLE16: i32 = 15;
    /// Frame `idle17`.
    pub const IDLE17: i32 = 16;
    /// Frame `idle18`.
    pub const IDLE18: i32 = 17;
    /// Frame `idle19`.
    pub const IDLE19: i32 = 18;
    /// Frame `idle20`.
    pub const IDLE20: i32 = 19;
    /// Frame `idle21`.
    pub const IDLE21: i32 = 20;
    /// Frame `idle201`.
    pub const IDLE201: i32 = 21;
    /// Frame `idle202`.
    pub const IDLE202: i32 = 22;
    /// Frame `idle203`.
    pub const IDLE203: i32 = 23;
    /// Frame `idle204`.
    pub const IDLE204: i32 = 24;
    /// Frame `idle205`.
    pub const IDLE205: i32 = 25;
    /// Frame `idle206`.
    pub const IDLE206: i32 = 26;
    /// Frame `idle207`.
    pub const IDLE207: i32 = 27;
    /// Frame `idle208`.
    pub const IDLE208: i32 = 28;
    /// Frame `idle209`.
    pub const IDLE209: i32 = 29;
    /// Frame `idle210`.
    pub const IDLE210: i32 = 30;
    /// Frame `idle211`.
    pub const IDLE211: i32 = 31;
    /// Frame `idle212`.
    pub const IDLE212: i32 = 32;
    /// Frame `idle213`.
    pub const IDLE213: i32 = 33;
    /// Frame `walk01`.
    pub const WALK01: i32 = 34;
    /// Frame `walk02`.
    pub const WALK02: i32 = 35;
    /// Frame `walk03`.
    pub const WALK03: i32 = 36;
    /// Frame `walk04`.
    pub const WALK04: i32 = 37;
    /// Frame `walk05`.
    pub const WALK05: i32 = 38;
    /// Frame `walk06`.
    pub const WALK06: i32 = 39;
    /// Frame `walk07`.
    pub const WALK07: i32 = 40;
    /// Frame `walk08`.
    pub const WALK08: i32 = 41;
    /// Frame `jump01`.
    pub const JUMP01: i32 = 42;
    /// Frame `jump02`.
    pub const JUMP02: i32 = 43;
    /// Frame `jump03`.
    pub const JUMP03: i32 = 44;
    /// Frame `jump04`.
    pub const JUMP04: i32 = 45;
    /// Frame `jump05`.
    pub const JUMP05: i32 = 46;
    /// Frame `jump06`.
    pub const JUMP06: i32 = 47;
    /// Frame `jump07`.
    pub const JUMP07: i32 = 48;
    /// Frame `run01`.
    pub const RUN01: i32 = 49;
    /// Frame `run02`.
    pub const RUN02: i32 = 50;
    /// Frame `run03`.
    pub const RUN03: i32 = 51;
    /// Frame `run04`.
    pub const RUN04: i32 = 52;
    /// Frame `attack01`.
    pub const ATTACK01: i32 = 53;
    /// Frame `attack02`.
    pub const ATTACK02: i32 = 54;
    /// Frame `attack03`.
    pub const ATTACK03: i32 = 55;
    /// Frame `attack04`.
    pub const ATTACK04: i32 = 56;
    /// Frame `attack05`.
    pub const ATTACK05: i32 = 57;
    /// Frame `attack06`.
    pub const ATTACK06: i32 = 58;
    /// Frame `attack07`.
    pub const ATTACK07: i32 = 59;
    /// Frame `attack08`.
    pub const ATTACK08: i32 = 60;
    /// Frame `attack11`.
    pub const ATTACK11: i32 = 61;
    /// Frame `attack12`.
    pub const ATTACK12: i32 = 62;
    /// Frame `attack13`.
    pub const ATTACK13: i32 = 63;
    /// Frame `attack14`.
    pub const ATTACK14: i32 = 64;
    /// Frame `attack15`.
    pub const ATTACK15: i32 = 65;
    /// Frame `pain01`.
    pub const PAIN01: i32 = 66;
    /// Frame `pain02`.
    pub const PAIN02: i32 = 67;
    /// Frame `pain03`.
    pub const PAIN03: i32 = 68;
    /// Frame `pain04`.
    pub const PAIN04: i32 = 69;
    /// Frame `death01`.
    pub const DEATH01: i32 = 70;
    /// Frame `death02`.
    pub const DEATH02: i32 = 71;
    /// Frame `death03`.
    pub const DEATH03: i32 = 72;
    /// Frame `death04`.
    pub const DEATH04: i32 = 73;
    /// Frame `death05`.
    pub const DEATH05: i32 = 74;
    /// Frame `death06`.
    pub const DEATH06: i32 = 75;
    /// Frame `death07`.
    pub const DEATH07: i32 = 76;
    /// Frame `death08`.
    pub const DEATH08: i32 = 77;
    /// Frame `death09`.
    pub const DEATH09: i32 = 78;
    /// Frame `twitch01`.
    pub const TWITCH01: i32 = 79;
    /// Frame `twitch02`.
    pub const TWITCH02: i32 = 80;
    /// Frame `twitch03`.
    pub const TWITCH03: i32 = 81;
    /// Frame `twitch04`.
    pub const TWITCH04: i32 = 82;
    /// Frame `twitch05`.
    pub const TWITCH05: i32 = 83;
    /// Frame `twitch06`.
    pub const TWITCH06: i32 = 84;
    /// Frame `twitch07`.
    pub const TWITCH07: i32 = 85;
    /// Frame `twitch08`.
    pub const TWITCH08: i32 = 86;
    /// Frame `twitch09`.
    pub const TWITCH09: i32 = 87;
    /// Frame `twitch10`.
    pub const TWITCH10: i32 = 88;
    /// Frame `reactive01`.
    pub const REACTIVE01: i32 = 89;
    /// Frame `reactive02`.
    pub const REACTIVE02: i32 = 90;
    /// Frame `reactive03`.
    pub const REACTIVE03: i32 = 91;
    /// Frame `reactive04`.
    pub const REACTIVE04: i32 = 92;
}

/// `stalkerMoves` move tables.
pub fn stalker_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("stalker_move_idle", 0, 20, Some("stalker_stand"), vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("stalker_idle_noise")], -1),
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
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("stalker_move_idle2", 21, 33, Some("stalker_stand"), vec![
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
        monster_move("stalker_move_stand", 0, 20, Some("stalker_stand"), vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("stalker_idle_noise")], -1),
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
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("stalker_move_run", 49, 52, None, vec![
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 17.0, vec![], -1),
            monster_frame(MonsterAi::Run, 21.0, vec![], -1),
            monster_frame(MonsterAi::Run, 18.0, vec![], -1),
        ]),
        monster_move("stalker_move_walk", 34, 41, Some("stalker_walk"), vec![
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 8.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 8.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
        ]),
        monster_move("stalker_move_false_death_end", 89, 92, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("stalker_move_false_death", 79, 88, Some("stalker_false_death"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_heal")], -1),
        ]),
        monster_move("stalker_move_false_death_start", 70, 78, Some("stalker_false_death"), vec![
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
        monster_move("stalker_move_pain", 66, 69, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("stalker_move_shoot", 49, 52, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Charge, 13.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 17.0, vec![MonsterAction::name("stalker_shoot_attack")], -1),
            monster_frame(MonsterAi::Charge, 21.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 18.0, vec![MonsterAction::name("stalker_shoot_attack2")], -1),
        ]),
        monster_move("stalker_move_swing_l", 53, 60, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![MonsterAction::name("stalker_swing_attack")], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
        ]),
        monster_move("stalker_move_swing_r", 61, 65, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![MonsterAction::name("stalker_swing_attack")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
        ]),
        monster_move("stalker_move_jump_straightup", 45, 48, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Move, 1.0, vec![MonsterAction::name("stalker_jump_straightup")], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![MonsterAction::name("stalker_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
        ]),
        monster_move("stalker_move_dodge_run", 49, 52, None, vec![
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 17.0, vec![], -1),
            monster_frame(MonsterAi::Run, 21.0, vec![], -1),
            monster_frame(MonsterAi::Run, 18.0, vec![MonsterAction::name("monster_done_dodge")], -1),
        ]),
        monster_move("stalker_move_jump_up", 42, 48, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Move, -8.0, vec![], -1),
            monster_frame(MonsterAi::Move, -8.0, vec![], -1),
            monster_frame(MonsterAi::Move, -8.0, vec![], -1),
            monster_frame(MonsterAi::Move, -8.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_jump_up")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("stalker_move_jump_down", 42, 48, Some("stalker_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_jump_down")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("stalker_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("stalker_move_death", 70, 78, Some("stalker_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![], -1),
            monster_frame(MonsterAi::Move, -10.0, vec![], -1),
            monster_frame(MonsterAi::Move, -20.0, vec![], -1),
            monster_frame(MonsterAi::Move, -10.0, vec![], -1),
            monster_frame(MonsterAi::Move, -10.0, vec![], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
    ]
}
