//! widow2 move tables (`src/content/q2/missionpacks/monsters/tables/rogue-widow2.ts`).
//!
//! Original Quake II rogue/m_widow2.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `widow2Frame`.
pub mod widow2_frame {
    /// Frame `blackwidow3`.
    pub const BLACKWIDOW3: i32 = 0;
    /// Frame `walk01`.
    pub const WALK01: i32 = 1;
    /// Frame `walk02`.
    pub const WALK02: i32 = 2;
    /// Frame `walk03`.
    pub const WALK03: i32 = 3;
    /// Frame `walk04`.
    pub const WALK04: i32 = 4;
    /// Frame `walk05`.
    pub const WALK05: i32 = 5;
    /// Frame `walk06`.
    pub const WALK06: i32 = 6;
    /// Frame `walk07`.
    pub const WALK07: i32 = 7;
    /// Frame `walk08`.
    pub const WALK08: i32 = 8;
    /// Frame `walk09`.
    pub const WALK09: i32 = 9;
    /// Frame `spawn01`.
    pub const SPAWN01: i32 = 10;
    /// Frame `spawn02`.
    pub const SPAWN02: i32 = 11;
    /// Frame `spawn03`.
    pub const SPAWN03: i32 = 12;
    /// Frame `spawn04`.
    pub const SPAWN04: i32 = 13;
    /// Frame `spawn05`.
    pub const SPAWN05: i32 = 14;
    /// Frame `spawn06`.
    pub const SPAWN06: i32 = 15;
    /// Frame `spawn07`.
    pub const SPAWN07: i32 = 16;
    /// Frame `spawn08`.
    pub const SPAWN08: i32 = 17;
    /// Frame `spawn09`.
    pub const SPAWN09: i32 = 18;
    /// Frame `spawn10`.
    pub const SPAWN10: i32 = 19;
    /// Frame `spawn11`.
    pub const SPAWN11: i32 = 20;
    /// Frame `spawn12`.
    pub const SPAWN12: i32 = 21;
    /// Frame `spawn13`.
    pub const SPAWN13: i32 = 22;
    /// Frame `spawn14`.
    pub const SPAWN14: i32 = 23;
    /// Frame `spawn15`.
    pub const SPAWN15: i32 = 24;
    /// Frame `spawn16`.
    pub const SPAWN16: i32 = 25;
    /// Frame `spawn17`.
    pub const SPAWN17: i32 = 26;
    /// Frame `spawn18`.
    pub const SPAWN18: i32 = 27;
    /// Frame `firea01`.
    pub const FIREA01: i32 = 28;
    /// Frame `firea02`.
    pub const FIREA02: i32 = 29;
    /// Frame `firea03`.
    pub const FIREA03: i32 = 30;
    /// Frame `firea04`.
    pub const FIREA04: i32 = 31;
    /// Frame `firea05`.
    pub const FIREA05: i32 = 32;
    /// Frame `firea06`.
    pub const FIREA06: i32 = 33;
    /// Frame `firea07`.
    pub const FIREA07: i32 = 34;
    /// Frame `fireb01`.
    pub const FIREB01: i32 = 35;
    /// Frame `fireb02`.
    pub const FIREB02: i32 = 36;
    /// Frame `fireb03`.
    pub const FIREB03: i32 = 37;
    /// Frame `fireb04`.
    pub const FIREB04: i32 = 38;
    /// Frame `fireb05`.
    pub const FIREB05: i32 = 39;
    /// Frame `fireb06`.
    pub const FIREB06: i32 = 40;
    /// Frame `fireb07`.
    pub const FIREB07: i32 = 41;
    /// Frame `fireb08`.
    pub const FIREB08: i32 = 42;
    /// Frame `fireb09`.
    pub const FIREB09: i32 = 43;
    /// Frame `fireb10`.
    pub const FIREB10: i32 = 44;
    /// Frame `fireb11`.
    pub const FIREB11: i32 = 45;
    /// Frame `fireb12`.
    pub const FIREB12: i32 = 46;
    /// Frame `tongs01`.
    pub const TONGS01: i32 = 47;
    /// Frame `tongs02`.
    pub const TONGS02: i32 = 48;
    /// Frame `tongs03`.
    pub const TONGS03: i32 = 49;
    /// Frame `tongs04`.
    pub const TONGS04: i32 = 50;
    /// Frame `tongs05`.
    pub const TONGS05: i32 = 51;
    /// Frame `tongs06`.
    pub const TONGS06: i32 = 52;
    /// Frame `tongs07`.
    pub const TONGS07: i32 = 53;
    /// Frame `tongs08`.
    pub const TONGS08: i32 = 54;
    /// Frame `pain01`.
    pub const PAIN01: i32 = 55;
    /// Frame `pain02`.
    pub const PAIN02: i32 = 56;
    /// Frame `pain03`.
    pub const PAIN03: i32 = 57;
    /// Frame `pain04`.
    pub const PAIN04: i32 = 58;
    /// Frame `pain05`.
    pub const PAIN05: i32 = 59;
    /// Frame `death01`.
    pub const DEATH01: i32 = 60;
    /// Frame `death02`.
    pub const DEATH02: i32 = 61;
    /// Frame `death03`.
    pub const DEATH03: i32 = 62;
    /// Frame `death04`.
    pub const DEATH04: i32 = 63;
    /// Frame `death05`.
    pub const DEATH05: i32 = 64;
    /// Frame `death06`.
    pub const DEATH06: i32 = 65;
    /// Frame `death07`.
    pub const DEATH07: i32 = 66;
    /// Frame `death08`.
    pub const DEATH08: i32 = 67;
    /// Frame `death09`.
    pub const DEATH09: i32 = 68;
    /// Frame `death10`.
    pub const DEATH10: i32 = 69;
    /// Frame `death11`.
    pub const DEATH11: i32 = 70;
    /// Frame `death12`.
    pub const DEATH12: i32 = 71;
    /// Frame `death13`.
    pub const DEATH13: i32 = 72;
    /// Frame `death14`.
    pub const DEATH14: i32 = 73;
    /// Frame `death15`.
    pub const DEATH15: i32 = 74;
    /// Frame `death16`.
    pub const DEATH16: i32 = 75;
    /// Frame `death17`.
    pub const DEATH17: i32 = 76;
    /// Frame `death18`.
    pub const DEATH18: i32 = 77;
    /// Frame `death19`.
    pub const DEATH19: i32 = 78;
    /// Frame `death20`.
    pub const DEATH20: i32 = 79;
    /// Frame `death21`.
    pub const DEATH21: i32 = 80;
    /// Frame `death22`.
    pub const DEATH22: i32 = 81;
    /// Frame `death23`.
    pub const DEATH23: i32 = 82;
    /// Frame `death24`.
    pub const DEATH24: i32 = 83;
    /// Frame `death25`.
    pub const DEATH25: i32 = 84;
    /// Frame `death26`.
    pub const DEATH26: i32 = 85;
    /// Frame `death27`.
    pub const DEATH27: i32 = 86;
    /// Frame `death28`.
    pub const DEATH28: i32 = 87;
    /// Frame `death29`.
    pub const DEATH29: i32 = 88;
    /// Frame `death30`.
    pub const DEATH30: i32 = 89;
    /// Frame `death31`.
    pub const DEATH31: i32 = 90;
    /// Frame `death32`.
    pub const DEATH32: i32 = 91;
    /// Frame `death33`.
    pub const DEATH33: i32 = 92;
    /// Frame `death34`.
    pub const DEATH34: i32 = 93;
    /// Frame `death35`.
    pub const DEATH35: i32 = 94;
    /// Frame `death36`.
    pub const DEATH36: i32 = 95;
    /// Frame `death37`.
    pub const DEATH37: i32 = 96;
    /// Frame `death38`.
    pub const DEATH38: i32 = 97;
    /// Frame `death39`.
    pub const DEATH39: i32 = 98;
    /// Frame `death40`.
    pub const DEATH40: i32 = 99;
    /// Frame `death41`.
    pub const DEATH41: i32 = 100;
    /// Frame `death42`.
    pub const DEATH42: i32 = 101;
    /// Frame `death43`.
    pub const DEATH43: i32 = 102;
    /// Frame `death44`.
    pub const DEATH44: i32 = 103;
    /// Frame `dthsrh01`.
    pub const DTHSRH01: i32 = 104;
    /// Frame `dthsrh02`.
    pub const DTHSRH02: i32 = 105;
    /// Frame `dthsrh03`.
    pub const DTHSRH03: i32 = 106;
    /// Frame `dthsrh04`.
    pub const DTHSRH04: i32 = 107;
    /// Frame `dthsrh05`.
    pub const DTHSRH05: i32 = 108;
    /// Frame `dthsrh06`.
    pub const DTHSRH06: i32 = 109;
    /// Frame `dthsrh07`.
    pub const DTHSRH07: i32 = 110;
    /// Frame `dthsrh08`.
    pub const DTHSRH08: i32 = 111;
    /// Frame `dthsrh09`.
    pub const DTHSRH09: i32 = 112;
    /// Frame `dthsrh10`.
    pub const DTHSRH10: i32 = 113;
    /// Frame `dthsrh11`.
    pub const DTHSRH11: i32 = 114;
    /// Frame `dthsrh12`.
    pub const DTHSRH12: i32 = 115;
    /// Frame `dthsrh13`.
    pub const DTHSRH13: i32 = 116;
    /// Frame `dthsrh14`.
    pub const DTHSRH14: i32 = 117;
    /// Frame `dthsrh15`.
    pub const DTHSRH15: i32 = 118;
    /// Frame `dthsrh16`.
    pub const DTHSRH16: i32 = 119;
    /// Frame `dthsrh17`.
    pub const DTHSRH17: i32 = 120;
    /// Frame `dthsrh18`.
    pub const DTHSRH18: i32 = 121;
    /// Frame `dthsrh19`.
    pub const DTHSRH19: i32 = 122;
    /// Frame `dthsrh20`.
    pub const DTHSRH20: i32 = 123;
    /// Frame `dthsrh21`.
    pub const DTHSRH21: i32 = 124;
    /// Frame `dthsrh22`.
    pub const DTHSRH22: i32 = 125;
}

/// `widow2Moves` move tables.
pub fn widow2_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("widow2_move_stand", 0, 0, None, vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("widow2_move_walk", 1, 9, None, vec![
            monster_frame(MonsterAi::Walk, 9.01, vec![], -1),
            monster_frame(MonsterAi::Walk, 7.55, vec![], -1),
            monster_frame(MonsterAi::Walk, 7.01, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.66, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.78, vec![], -1),
            monster_frame(MonsterAi::Walk, 7.25, vec![], -1),
            monster_frame(MonsterAi::Walk, 8.37, vec![], -1),
            monster_frame(MonsterAi::Walk, 10.41, vec![], -1),
        ]),
        monster_move("widow2_move_run", 1, 9, None, vec![
            monster_frame(MonsterAi::Run, 9.01, vec![], -1),
            monster_frame(MonsterAi::Run, 7.55, vec![], -1),
            monster_frame(MonsterAi::Run, 7.01, vec![], -1),
            monster_frame(MonsterAi::Run, 6.66, vec![], -1),
            monster_frame(MonsterAi::Run, 6.2, vec![], -1),
            monster_frame(MonsterAi::Run, 5.78, vec![], -1),
            monster_frame(MonsterAi::Run, 7.25, vec![], -1),
            monster_frame(MonsterAi::Run, 8.37, vec![], -1),
            monster_frame(MonsterAi::Run, 10.41, vec![], -1),
        ]),
        monster_move("widow2_move_attack_pre_beam", 35, 38, None, vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![MonsterAction::name("widow2_attack_beam")], -1),
        ]),
        monster_move("widow2_move_attack_beam", 39, 43, None, vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow2_reattack_beam")], -1),
        ]),
        monster_move("widow2_move_attack_post_beam", 40, 41, Some("widow2_run"), vec![
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
        ]),
        monster_move("widow2_move_attack_disrupt", 28, 34, Some("widow2_run"), vec![
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("Widow2SaveDisruptLoc")], -1),
            monster_frame(MonsterAi::Charge, -20.0, vec![MonsterAction::name("WidowDisrupt")], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("widow2_disrupt_reattack")], -1),
        ]),
        monster_move("widow2_move_spawn", 10, 27, None, vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow_start_spawn")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow2_ready_spawn")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Beam")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow2_spawn_check")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow2_reattack_beam")], -1),
        ]),
        monster_move("widow2_move_tongs", 47, 54, Some("widow2_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Tongue")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Tongue")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Tongue")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2TonguePull")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2TonguePull")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2TonguePull")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Crunch")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("Widow2Toss")], -1),
        ]),
        monster_move("widow2_move_pain", 55, 59, Some("widow2_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("widow2_move_death", 60, 103, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion1")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion2")], -1),
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
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion3")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion4")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion5")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosionLeg")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion6")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplosion7")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("WidowExplode")], -1),
        ]),
        monster_move("widow2_move_dead", 104, 118, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("widow2_start_searching")], -1),
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
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("widow2_keep_searching")], -1),
        ]),
        monster_move("widow2_move_really_dead", 119, 125, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("widow2_finaldeath")], -1),
        ]),
    ]
}
