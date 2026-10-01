//! gladiator move tables (`src/content/q2/rerelease/monsters/tables/gladiator.ts`).

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `gladiatorFrame`.
pub mod gladiator_frame {
    /// Frame `stand1`.
    pub const STAND1: i32 = 0;
    /// Frame `stand2`.
    pub const STAND2: i32 = 1;
    /// Frame `stand3`.
    pub const STAND3: i32 = 2;
    /// Frame `stand4`.
    pub const STAND4: i32 = 3;
    /// Frame `stand5`.
    pub const STAND5: i32 = 4;
    /// Frame `stand6`.
    pub const STAND6: i32 = 5;
    /// Frame `stand7`.
    pub const STAND7: i32 = 6;
    /// Frame `walk1`.
    pub const WALK1: i32 = 7;
    /// Frame `walk2`.
    pub const WALK2: i32 = 8;
    /// Frame `walk3`.
    pub const WALK3: i32 = 9;
    /// Frame `walk4`.
    pub const WALK4: i32 = 10;
    /// Frame `walk5`.
    pub const WALK5: i32 = 11;
    /// Frame `walk6`.
    pub const WALK6: i32 = 12;
    /// Frame `walk7`.
    pub const WALK7: i32 = 13;
    /// Frame `walk8`.
    pub const WALK8: i32 = 14;
    /// Frame `walk9`.
    pub const WALK9: i32 = 15;
    /// Frame `walk10`.
    pub const WALK10: i32 = 16;
    /// Frame `walk11`.
    pub const WALK11: i32 = 17;
    /// Frame `walk12`.
    pub const WALK12: i32 = 18;
    /// Frame `walk13`.
    pub const WALK13: i32 = 19;
    /// Frame `walk14`.
    pub const WALK14: i32 = 20;
    /// Frame `walk15`.
    pub const WALK15: i32 = 21;
    /// Frame `walk16`.
    pub const WALK16: i32 = 22;
    /// Frame `run1`.
    pub const RUN1: i32 = 23;
    /// Frame `run2`.
    pub const RUN2: i32 = 24;
    /// Frame `run3`.
    pub const RUN3: i32 = 25;
    /// Frame `run4`.
    pub const RUN4: i32 = 26;
    /// Frame `run5`.
    pub const RUN5: i32 = 27;
    /// Frame `run6`.
    pub const RUN6: i32 = 28;
    /// Frame `melee1`.
    pub const MELEE1: i32 = 29;
    /// Frame `melee2`.
    pub const MELEE2: i32 = 30;
    /// Frame `melee3`.
    pub const MELEE3: i32 = 31;
    /// Frame `melee4`.
    pub const MELEE4: i32 = 32;
    /// Frame `melee5`.
    pub const MELEE5: i32 = 33;
    /// Frame `melee6`.
    pub const MELEE6: i32 = 34;
    /// Frame `melee7`.
    pub const MELEE7: i32 = 35;
    /// Frame `melee8`.
    pub const MELEE8: i32 = 36;
    /// Frame `melee9`.
    pub const MELEE9: i32 = 37;
    /// Frame `melee10`.
    pub const MELEE10: i32 = 38;
    /// Frame `melee11`.
    pub const MELEE11: i32 = 39;
    /// Frame `melee12`.
    pub const MELEE12: i32 = 40;
    /// Frame `melee13`.
    pub const MELEE13: i32 = 41;
    /// Frame `melee14`.
    pub const MELEE14: i32 = 42;
    /// Frame `melee15`.
    pub const MELEE15: i32 = 43;
    /// Frame `melee16`.
    pub const MELEE16: i32 = 44;
    /// Frame `melee17`.
    pub const MELEE17: i32 = 45;
    /// Frame `attack1`.
    pub const ATTACK1: i32 = 46;
    /// Frame `attack2`.
    pub const ATTACK2: i32 = 47;
    /// Frame `attack3`.
    pub const ATTACK3: i32 = 48;
    /// Frame `attack4`.
    pub const ATTACK4: i32 = 49;
    /// Frame `attack5`.
    pub const ATTACK5: i32 = 50;
    /// Frame `attack6`.
    pub const ATTACK6: i32 = 51;
    /// Frame `attack7`.
    pub const ATTACK7: i32 = 52;
    /// Frame `attack8`.
    pub const ATTACK8: i32 = 53;
    /// Frame `attack9`.
    pub const ATTACK9: i32 = 54;
    /// Frame `pain1`.
    pub const PAIN1: i32 = 55;
    /// Frame `pain2`.
    pub const PAIN2: i32 = 56;
    /// Frame `pain3`.
    pub const PAIN3: i32 = 57;
    /// Frame `pain4`.
    pub const PAIN4: i32 = 58;
    /// Frame `pain5`.
    pub const PAIN5: i32 = 59;
    /// Frame `pain6`.
    pub const PAIN6: i32 = 60;
    /// Frame `death1`.
    pub const DEATH1: i32 = 61;
    /// Frame `death2`.
    pub const DEATH2: i32 = 62;
    /// Frame `death3`.
    pub const DEATH3: i32 = 63;
    /// Frame `death4`.
    pub const DEATH4: i32 = 64;
    /// Frame `death5`.
    pub const DEATH5: i32 = 65;
    /// Frame `death6`.
    pub const DEATH6: i32 = 66;
    /// Frame `death7`.
    pub const DEATH7: i32 = 67;
    /// Frame `death8`.
    pub const DEATH8: i32 = 68;
    /// Frame `death9`.
    pub const DEATH9: i32 = 69;
    /// Frame `death10`.
    pub const DEATH10: i32 = 70;
    /// Frame `death11`.
    pub const DEATH11: i32 = 71;
    /// Frame `death12`.
    pub const DEATH12: i32 = 72;
    /// Frame `death13`.
    pub const DEATH13: i32 = 73;
    /// Frame `death14`.
    pub const DEATH14: i32 = 74;
    /// Frame `death15`.
    pub const DEATH15: i32 = 75;
    /// Frame `death16`.
    pub const DEATH16: i32 = 76;
    /// Frame `death17`.
    pub const DEATH17: i32 = 77;
    /// Frame `death18`.
    pub const DEATH18: i32 = 78;
    /// Frame `death19`.
    pub const DEATH19: i32 = 79;
    /// Frame `death20`.
    pub const DEATH20: i32 = 80;
    /// Frame `death21`.
    pub const DEATH21: i32 = 81;
    /// Frame `death22`.
    pub const DEATH22: i32 = 82;
    /// Frame `painup1`.
    pub const PAINUP1: i32 = 83;
    /// Frame `painup2`.
    pub const PAINUP2: i32 = 84;
    /// Frame `painup3`.
    pub const PAINUP3: i32 = 85;
    /// Frame `painup4`.
    pub const PAINUP4: i32 = 86;
    /// Frame `painup5`.
    pub const PAINUP5: i32 = 87;
    /// Frame `painup6`.
    pub const PAINUP6: i32 = 88;
    /// Frame `painup7`.
    pub const PAINUP7: i32 = 89;
}

/// `gladiatorMoves` move tables.
pub fn gladiator_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "gladiator_move_stand",
            0,
            6,
            None,
            vec![
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "gladiator_move_walk",
            7,
            22,
            None,
            vec![
                monster_frame(MonsterAi::Walk, (15f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Walk,
                    (2f32) as f64,
                    vec![MonsterAction::name("monster_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (12f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Walk,
                    (2f32) as f64,
                    vec![MonsterAction::name("monster_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (1f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "gladiator_move_run",
            23,
            28,
            None,
            vec![
                monster_frame(MonsterAi::Run, (23f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (14f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Run,
                    (14f32) as f64,
                    vec![MonsterAction::name("monster_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (21f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Run,
                    (13f32) as f64,
                    vec![MonsterAction::name("monster_footstep")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gladiator_move_attack_melee",
            31,
            44,
            Some("gladiator_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("gladiator_cleaver_swing")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("GladiatorMelee")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("gladiator_cleaver_swing")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("GladiatorMelee")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "gladiator_move_attack_gun",
            46,
            54,
            Some("gladiator_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("GladiatorGun")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("monster_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "gladb_move_attack_gun",
            46,
            54,
            Some("gladiator_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("gladbGun")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("gladbGun")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("gladbGun_check")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gladiator_move_pain",
            56,
            59,
            Some("gladiator_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "gladiator_move_pain_air",
            84,
            88,
            Some("gladiator_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "gladiator_move_death",
            62,
            82,
            Some("gladiator_dead"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("gladiator_shrink")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("monster_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
    ]
}
