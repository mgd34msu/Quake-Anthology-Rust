//! arachnid move tables (`src/content/q2/rerelease/monsters/tables/arachnid.ts`).

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `arachnidFrame`.
pub mod arachnid_frame {
    /// Frame `rails1`.
    pub const RAILS1: i32 = 0;
    /// Frame `rails2`.
    pub const RAILS2: i32 = 1;
    /// Frame `rails3`.
    pub const RAILS3: i32 = 2;
    /// Frame `rails4`.
    pub const RAILS4: i32 = 3;
    /// Frame `rails5`.
    pub const RAILS5: i32 = 4;
    /// Frame `rails6`.
    pub const RAILS6: i32 = 5;
    /// Frame `rails7`.
    pub const RAILS7: i32 = 6;
    /// Frame `rails8`.
    pub const RAILS8: i32 = 7;
    /// Frame `rails9`.
    pub const RAILS9: i32 = 8;
    /// Frame `rails10`.
    pub const RAILS10: i32 = 9;
    /// Frame `rails11`.
    pub const RAILS11: i32 = 10;
    /// Frame `death1`.
    pub const DEATH1: i32 = 11;
    /// Frame `death2`.
    pub const DEATH2: i32 = 12;
    /// Frame `death3`.
    pub const DEATH3: i32 = 13;
    /// Frame `death4`.
    pub const DEATH4: i32 = 14;
    /// Frame `death5`.
    pub const DEATH5: i32 = 15;
    /// Frame `death6`.
    pub const DEATH6: i32 = 16;
    /// Frame `death7`.
    pub const DEATH7: i32 = 17;
    /// Frame `death8`.
    pub const DEATH8: i32 = 18;
    /// Frame `death9`.
    pub const DEATH9: i32 = 19;
    /// Frame `death10`.
    pub const DEATH10: i32 = 20;
    /// Frame `death11`.
    pub const DEATH11: i32 = 21;
    /// Frame `death12`.
    pub const DEATH12: i32 = 22;
    /// Frame `death13`.
    pub const DEATH13: i32 = 23;
    /// Frame `death14`.
    pub const DEATH14: i32 = 24;
    /// Frame `death15`.
    pub const DEATH15: i32 = 25;
    /// Frame `death16`.
    pub const DEATH16: i32 = 26;
    /// Frame `death17`.
    pub const DEATH17: i32 = 27;
    /// Frame `death18`.
    pub const DEATH18: i32 = 28;
    /// Frame `death19`.
    pub const DEATH19: i32 = 29;
    /// Frame `death20`.
    pub const DEATH20: i32 = 30;
    /// Frame `melee_atk1`.
    pub const MELEE_ATK1: i32 = 31;
    /// Frame `melee_atk2`.
    pub const MELEE_ATK2: i32 = 32;
    /// Frame `melee_atk3`.
    pub const MELEE_ATK3: i32 = 33;
    /// Frame `melee_atk4`.
    pub const MELEE_ATK4: i32 = 34;
    /// Frame `melee_atk5`.
    pub const MELEE_ATK5: i32 = 35;
    /// Frame `melee_atk6`.
    pub const MELEE_ATK6: i32 = 36;
    /// Frame `melee_atk7`.
    pub const MELEE_ATK7: i32 = 37;
    /// Frame `melee_atk8`.
    pub const MELEE_ATK8: i32 = 38;
    /// Frame `melee_atk9`.
    pub const MELEE_ATK9: i32 = 39;
    /// Frame `melee_atk10`.
    pub const MELEE_ATK10: i32 = 40;
    /// Frame `melee_atk11`.
    pub const MELEE_ATK11: i32 = 41;
    /// Frame `melee_atk12`.
    pub const MELEE_ATK12: i32 = 42;
    /// Frame `pain11`.
    pub const PAIN11: i32 = 43;
    /// Frame `pain12`.
    pub const PAIN12: i32 = 44;
    /// Frame `pain13`.
    pub const PAIN13: i32 = 45;
    /// Frame `pain14`.
    pub const PAIN14: i32 = 46;
    /// Frame `pain15`.
    pub const PAIN15: i32 = 47;
    /// Frame `idle1`.
    pub const IDLE1: i32 = 48;
    /// Frame `idle2`.
    pub const IDLE2: i32 = 49;
    /// Frame `idle3`.
    pub const IDLE3: i32 = 50;
    /// Frame `idle4`.
    pub const IDLE4: i32 = 51;
    /// Frame `idle5`.
    pub const IDLE5: i32 = 52;
    /// Frame `idle6`.
    pub const IDLE6: i32 = 53;
    /// Frame `idle7`.
    pub const IDLE7: i32 = 54;
    /// Frame `idle8`.
    pub const IDLE8: i32 = 55;
    /// Frame `idle9`.
    pub const IDLE9: i32 = 56;
    /// Frame `idle10`.
    pub const IDLE10: i32 = 57;
    /// Frame `idle11`.
    pub const IDLE11: i32 = 58;
    /// Frame `idle12`.
    pub const IDLE12: i32 = 59;
    /// Frame `idle13`.
    pub const IDLE13: i32 = 60;
    /// Frame `walk1`.
    pub const WALK1: i32 = 61;
    /// Frame `walk2`.
    pub const WALK2: i32 = 62;
    /// Frame `walk3`.
    pub const WALK3: i32 = 63;
    /// Frame `walk4`.
    pub const WALK4: i32 = 64;
    /// Frame `walk5`.
    pub const WALK5: i32 = 65;
    /// Frame `walk6`.
    pub const WALK6: i32 = 66;
    /// Frame `walk7`.
    pub const WALK7: i32 = 67;
    /// Frame `walk8`.
    pub const WALK8: i32 = 68;
    /// Frame `walk9`.
    pub const WALK9: i32 = 69;
    /// Frame `walk10`.
    pub const WALK10: i32 = 70;
    /// Frame `turn1`.
    pub const TURN1: i32 = 71;
    /// Frame `turn2`.
    pub const TURN2: i32 = 72;
    /// Frame `turn3`.
    pub const TURN3: i32 = 73;
    /// Frame `melee_out1`.
    pub const MELEE_OUT1: i32 = 74;
    /// Frame `melee_out2`.
    pub const MELEE_OUT2: i32 = 75;
    /// Frame `melee_out3`.
    pub const MELEE_OUT3: i32 = 76;
    /// Frame `pain21`.
    pub const PAIN21: i32 = 77;
    /// Frame `pain22`.
    pub const PAIN22: i32 = 78;
    /// Frame `pain23`.
    pub const PAIN23: i32 = 79;
    /// Frame `pain24`.
    pub const PAIN24: i32 = 80;
    /// Frame `pain25`.
    pub const PAIN25: i32 = 81;
    /// Frame `pain26`.
    pub const PAIN26: i32 = 82;
    /// Frame `melee_pain1`.
    pub const MELEE_PAIN1: i32 = 83;
    /// Frame `melee_pain2`.
    pub const MELEE_PAIN2: i32 = 84;
    /// Frame `melee_pain3`.
    pub const MELEE_PAIN3: i32 = 85;
    /// Frame `melee_pain4`.
    pub const MELEE_PAIN4: i32 = 86;
    /// Frame `melee_pain5`.
    pub const MELEE_PAIN5: i32 = 87;
    /// Frame `melee_pain6`.
    pub const MELEE_PAIN6: i32 = 88;
    /// Frame `melee_pain7`.
    pub const MELEE_PAIN7: i32 = 89;
    /// Frame `melee_pain8`.
    pub const MELEE_PAIN8: i32 = 90;
    /// Frame `melee_pain9`.
    pub const MELEE_PAIN9: i32 = 91;
    /// Frame `melee_pain10`.
    pub const MELEE_PAIN10: i32 = 92;
    /// Frame `melee_pain11`.
    pub const MELEE_PAIN11: i32 = 93;
    /// Frame `melee_pain12`.
    pub const MELEE_PAIN12: i32 = 94;
    /// Frame `melee_pain13`.
    pub const MELEE_PAIN13: i32 = 95;
    /// Frame `melee_pain14`.
    pub const MELEE_PAIN14: i32 = 96;
    /// Frame `melee_pain15`.
    pub const MELEE_PAIN15: i32 = 97;
    /// Frame `melee_pain16`.
    pub const MELEE_PAIN16: i32 = 98;
    /// Frame `melee_in1`.
    pub const MELEE_IN1: i32 = 99;
    /// Frame `melee_in2`.
    pub const MELEE_IN2: i32 = 100;
    /// Frame `melee_in3`.
    pub const MELEE_IN3: i32 = 101;
    /// Frame `melee_in4`.
    pub const MELEE_IN4: i32 = 102;
    /// Frame `melee_in5`.
    pub const MELEE_IN5: i32 = 103;
    /// Frame `melee_in6`.
    pub const MELEE_IN6: i32 = 104;
    /// Frame `melee_in7`.
    pub const MELEE_IN7: i32 = 105;
    /// Frame `melee_in8`.
    pub const MELEE_IN8: i32 = 106;
    /// Frame `melee_in9`.
    pub const MELEE_IN9: i32 = 107;
    /// Frame `melee_in10`.
    pub const MELEE_IN10: i32 = 108;
    /// Frame `melee_in11`.
    pub const MELEE_IN11: i32 = 109;
    /// Frame `melee_in12`.
    pub const MELEE_IN12: i32 = 110;
    /// Frame `melee_in13`.
    pub const MELEE_IN13: i32 = 111;
    /// Frame `melee_in14`.
    pub const MELEE_IN14: i32 = 112;
    /// Frame `melee_in15`.
    pub const MELEE_IN15: i32 = 113;
    /// Frame `melee_in16`.
    pub const MELEE_IN16: i32 = 114;
    /// Frame `rails_up1`.
    pub const RAILS_UP1: i32 = 115;
    /// Frame `rails_up2`.
    pub const RAILS_UP2: i32 = 116;
    /// Frame `rails_up3`.
    pub const RAILS_UP3: i32 = 117;
    /// Frame `rails_up4`.
    pub const RAILS_UP4: i32 = 118;
    /// Frame `rails_up5`.
    pub const RAILS_UP5: i32 = 119;
    /// Frame `rails_up6`.
    pub const RAILS_UP6: i32 = 120;
    /// Frame `rails_up7`.
    pub const RAILS_UP7: i32 = 121;
    /// Frame `rails_up8`.
    pub const RAILS_UP8: i32 = 122;
    /// Frame `rails_up9`.
    pub const RAILS_UP9: i32 = 123;
    /// Frame `rails_up10`.
    pub const RAILS_UP10: i32 = 124;
    /// Frame `rails_up11`.
    pub const RAILS_UP11: i32 = 125;
    /// Frame `rails_up12`.
    pub const RAILS_UP12: i32 = 126;
    /// Frame `rails_up13`.
    pub const RAILS_UP13: i32 = 127;
    /// Frame `rails_up14`.
    pub const RAILS_UP14: i32 = 128;
    /// Frame `rails_up15`.
    pub const RAILS_UP15: i32 = 129;
    /// Frame `rails_up16`.
    pub const RAILS_UP16: i32 = 130;
}

/// `arachnidMoves` move tables.
pub fn arachnid_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "arachnid_move_stand",
            48,
            60,
            None,
            vec![
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
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
            "arachnid_move_walk",
            61,
            70,
            None,
            vec![
                monster_frame(
                    MonsterAi::Walk,
                    (8f32) as f64,
                    vec![MonsterAction::name("arachnid_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Walk,
                    (8f32) as f64,
                    vec![MonsterAction::name("arachnid_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_move_run",
            61,
            70,
            None,
            vec![
                monster_frame(
                    MonsterAi::Run,
                    (8f32) as f64,
                    vec![MonsterAction::name("arachnid_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Run,
                    (8f32) as f64,
                    vec![MonsterAction::name("arachnid_footstep")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_move_pain1",
            43,
            47,
            Some("arachnid_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_move_pain2",
            77,
            82,
            Some("arachnid_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_attack1",
            0,
            10,
            Some("arachnid_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_charge_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_rail")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_charge_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_attack_up1",
            115,
            130,
            Some("arachnid_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_charge_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_rail")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_charge_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_melee",
            31,
            42,
            Some("arachnid_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_melee_charge")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_melee_hit")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_melee_charge")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("arachnid_melee_hit")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "arachnid_move_death",
            11,
            30,
            Some("arachnid_dead"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.23f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.23f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.23f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.23f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.64f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.64f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-2.45f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-8.63f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-4.5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-6.8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-5.4f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-3.4f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1.9f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
    ]
}
