//! boss2 move tables (`src/content/q2/rerelease/monsters/tables/boss2.ts`).

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `boss2Frame`.
pub mod boss2_frame {
    /// Frame `stand30`.
    pub const STAND30: i32 = 0;
    /// Frame `stand31`.
    pub const STAND31: i32 = 1;
    /// Frame `stand32`.
    pub const STAND32: i32 = 2;
    /// Frame `stand33`.
    pub const STAND33: i32 = 3;
    /// Frame `stand34`.
    pub const STAND34: i32 = 4;
    /// Frame `stand35`.
    pub const STAND35: i32 = 5;
    /// Frame `stand36`.
    pub const STAND36: i32 = 6;
    /// Frame `stand37`.
    pub const STAND37: i32 = 7;
    /// Frame `stand38`.
    pub const STAND38: i32 = 8;
    /// Frame `stand39`.
    pub const STAND39: i32 = 9;
    /// Frame `stand40`.
    pub const STAND40: i32 = 10;
    /// Frame `stand41`.
    pub const STAND41: i32 = 11;
    /// Frame `stand42`.
    pub const STAND42: i32 = 12;
    /// Frame `stand43`.
    pub const STAND43: i32 = 13;
    /// Frame `stand44`.
    pub const STAND44: i32 = 14;
    /// Frame `stand45`.
    pub const STAND45: i32 = 15;
    /// Frame `stand46`.
    pub const STAND46: i32 = 16;
    /// Frame `stand47`.
    pub const STAND47: i32 = 17;
    /// Frame `stand48`.
    pub const STAND48: i32 = 18;
    /// Frame `stand49`.
    pub const STAND49: i32 = 19;
    /// Frame `stand50`.
    pub const STAND50: i32 = 20;
    /// Frame `stand1`.
    pub const STAND1: i32 = 21;
    /// Frame `stand2`.
    pub const STAND2: i32 = 22;
    /// Frame `stand3`.
    pub const STAND3: i32 = 23;
    /// Frame `stand4`.
    pub const STAND4: i32 = 24;
    /// Frame `stand5`.
    pub const STAND5: i32 = 25;
    /// Frame `stand6`.
    pub const STAND6: i32 = 26;
    /// Frame `stand7`.
    pub const STAND7: i32 = 27;
    /// Frame `stand8`.
    pub const STAND8: i32 = 28;
    /// Frame `stand9`.
    pub const STAND9: i32 = 29;
    /// Frame `stand10`.
    pub const STAND10: i32 = 30;
    /// Frame `stand11`.
    pub const STAND11: i32 = 31;
    /// Frame `stand12`.
    pub const STAND12: i32 = 32;
    /// Frame `stand13`.
    pub const STAND13: i32 = 33;
    /// Frame `stand14`.
    pub const STAND14: i32 = 34;
    /// Frame `stand15`.
    pub const STAND15: i32 = 35;
    /// Frame `stand16`.
    pub const STAND16: i32 = 36;
    /// Frame `stand17`.
    pub const STAND17: i32 = 37;
    /// Frame `stand18`.
    pub const STAND18: i32 = 38;
    /// Frame `stand19`.
    pub const STAND19: i32 = 39;
    /// Frame `stand20`.
    pub const STAND20: i32 = 40;
    /// Frame `stand21`.
    pub const STAND21: i32 = 41;
    /// Frame `stand22`.
    pub const STAND22: i32 = 42;
    /// Frame `stand23`.
    pub const STAND23: i32 = 43;
    /// Frame `stand24`.
    pub const STAND24: i32 = 44;
    /// Frame `stand25`.
    pub const STAND25: i32 = 45;
    /// Frame `stand26`.
    pub const STAND26: i32 = 46;
    /// Frame `stand27`.
    pub const STAND27: i32 = 47;
    /// Frame `stand28`.
    pub const STAND28: i32 = 48;
    /// Frame `stand29`.
    pub const STAND29: i32 = 49;
    /// Frame `walk1`.
    pub const WALK1: i32 = 50;
    /// Frame `walk2`.
    pub const WALK2: i32 = 51;
    /// Frame `walk3`.
    pub const WALK3: i32 = 52;
    /// Frame `walk4`.
    pub const WALK4: i32 = 53;
    /// Frame `walk5`.
    pub const WALK5: i32 = 54;
    /// Frame `walk6`.
    pub const WALK6: i32 = 55;
    /// Frame `walk7`.
    pub const WALK7: i32 = 56;
    /// Frame `walk8`.
    pub const WALK8: i32 = 57;
    /// Frame `walk9`.
    pub const WALK9: i32 = 58;
    /// Frame `walk10`.
    pub const WALK10: i32 = 59;
    /// Frame `walk11`.
    pub const WALK11: i32 = 60;
    /// Frame `walk12`.
    pub const WALK12: i32 = 61;
    /// Frame `walk13`.
    pub const WALK13: i32 = 62;
    /// Frame `walk14`.
    pub const WALK14: i32 = 63;
    /// Frame `walk15`.
    pub const WALK15: i32 = 64;
    /// Frame `walk16`.
    pub const WALK16: i32 = 65;
    /// Frame `walk17`.
    pub const WALK17: i32 = 66;
    /// Frame `walk18`.
    pub const WALK18: i32 = 67;
    /// Frame `walk19`.
    pub const WALK19: i32 = 68;
    /// Frame `walk20`.
    pub const WALK20: i32 = 69;
    /// Frame `attack1`.
    pub const ATTACK1: i32 = 70;
    /// Frame `attack2`.
    pub const ATTACK2: i32 = 71;
    /// Frame `attack3`.
    pub const ATTACK3: i32 = 72;
    /// Frame `attack4`.
    pub const ATTACK4: i32 = 73;
    /// Frame `attack5`.
    pub const ATTACK5: i32 = 74;
    /// Frame `attack6`.
    pub const ATTACK6: i32 = 75;
    /// Frame `attack7`.
    pub const ATTACK7: i32 = 76;
    /// Frame `attack8`.
    pub const ATTACK8: i32 = 77;
    /// Frame `attack9`.
    pub const ATTACK9: i32 = 78;
    /// Frame `attack10`.
    pub const ATTACK10: i32 = 79;
    /// Frame `attack11`.
    pub const ATTACK11: i32 = 80;
    /// Frame `attack12`.
    pub const ATTACK12: i32 = 81;
    /// Frame `attack13`.
    pub const ATTACK13: i32 = 82;
    /// Frame `attack14`.
    pub const ATTACK14: i32 = 83;
    /// Frame `attack15`.
    pub const ATTACK15: i32 = 84;
    /// Frame `attack16`.
    pub const ATTACK16: i32 = 85;
    /// Frame `attack17`.
    pub const ATTACK17: i32 = 86;
    /// Frame `attack18`.
    pub const ATTACK18: i32 = 87;
    /// Frame `attack19`.
    pub const ATTACK19: i32 = 88;
    /// Frame `attack20`.
    pub const ATTACK20: i32 = 89;
    /// Frame `attack21`.
    pub const ATTACK21: i32 = 90;
    /// Frame `attack22`.
    pub const ATTACK22: i32 = 91;
    /// Frame `attack23`.
    pub const ATTACK23: i32 = 92;
    /// Frame `attack24`.
    pub const ATTACK24: i32 = 93;
    /// Frame `attack25`.
    pub const ATTACK25: i32 = 94;
    /// Frame `attack26`.
    pub const ATTACK26: i32 = 95;
    /// Frame `attack27`.
    pub const ATTACK27: i32 = 96;
    /// Frame `attack28`.
    pub const ATTACK28: i32 = 97;
    /// Frame `attack29`.
    pub const ATTACK29: i32 = 98;
    /// Frame `attack30`.
    pub const ATTACK30: i32 = 99;
    /// Frame `attack31`.
    pub const ATTACK31: i32 = 100;
    /// Frame `attack32`.
    pub const ATTACK32: i32 = 101;
    /// Frame `attack33`.
    pub const ATTACK33: i32 = 102;
    /// Frame `attack34`.
    pub const ATTACK34: i32 = 103;
    /// Frame `attack35`.
    pub const ATTACK35: i32 = 104;
    /// Frame `attack36`.
    pub const ATTACK36: i32 = 105;
    /// Frame `attack37`.
    pub const ATTACK37: i32 = 106;
    /// Frame `attack38`.
    pub const ATTACK38: i32 = 107;
    /// Frame `attack39`.
    pub const ATTACK39: i32 = 108;
    /// Frame `attack40`.
    pub const ATTACK40: i32 = 109;
    /// Frame `pain2`.
    pub const PAIN2: i32 = 110;
    /// Frame `pain3`.
    pub const PAIN3: i32 = 111;
    /// Frame `pain4`.
    pub const PAIN4: i32 = 112;
    /// Frame `pain5`.
    pub const PAIN5: i32 = 113;
    /// Frame `pain6`.
    pub const PAIN6: i32 = 114;
    /// Frame `pain7`.
    pub const PAIN7: i32 = 115;
    /// Frame `pain8`.
    pub const PAIN8: i32 = 116;
    /// Frame `pain9`.
    pub const PAIN9: i32 = 117;
    /// Frame `pain10`.
    pub const PAIN10: i32 = 118;
    /// Frame `pain11`.
    pub const PAIN11: i32 = 119;
    /// Frame `pain12`.
    pub const PAIN12: i32 = 120;
    /// Frame `pain13`.
    pub const PAIN13: i32 = 121;
    /// Frame `pain14`.
    pub const PAIN14: i32 = 122;
    /// Frame `pain15`.
    pub const PAIN15: i32 = 123;
    /// Frame `pain16`.
    pub const PAIN16: i32 = 124;
    /// Frame `pain17`.
    pub const PAIN17: i32 = 125;
    /// Frame `pain18`.
    pub const PAIN18: i32 = 126;
    /// Frame `pain19`.
    pub const PAIN19: i32 = 127;
    /// Frame `pain20`.
    pub const PAIN20: i32 = 128;
    /// Frame `pain21`.
    pub const PAIN21: i32 = 129;
    /// Frame `pain22`.
    pub const PAIN22: i32 = 130;
    /// Frame `pain23`.
    pub const PAIN23: i32 = 131;
    /// Frame `death2`.
    pub const DEATH2: i32 = 132;
    /// Frame `death3`.
    pub const DEATH3: i32 = 133;
    /// Frame `death4`.
    pub const DEATH4: i32 = 134;
    /// Frame `death5`.
    pub const DEATH5: i32 = 135;
    /// Frame `death6`.
    pub const DEATH6: i32 = 136;
    /// Frame `death7`.
    pub const DEATH7: i32 = 137;
    /// Frame `death8`.
    pub const DEATH8: i32 = 138;
    /// Frame `death9`.
    pub const DEATH9: i32 = 139;
    /// Frame `death10`.
    pub const DEATH10: i32 = 140;
    /// Frame `death11`.
    pub const DEATH11: i32 = 141;
    /// Frame `death12`.
    pub const DEATH12: i32 = 142;
    /// Frame `death13`.
    pub const DEATH13: i32 = 143;
    /// Frame `death14`.
    pub const DEATH14: i32 = 144;
    /// Frame `death15`.
    pub const DEATH15: i32 = 145;
    /// Frame `death16`.
    pub const DEATH16: i32 = 146;
    /// Frame `death17`.
    pub const DEATH17: i32 = 147;
    /// Frame `death18`.
    pub const DEATH18: i32 = 148;
    /// Frame `death19`.
    pub const DEATH19: i32 = 149;
    /// Frame `death20`.
    pub const DEATH20: i32 = 150;
    /// Frame `death21`.
    pub const DEATH21: i32 = 151;
    /// Frame `death22`.
    pub const DEATH22: i32 = 152;
    /// Frame `death23`.
    pub const DEATH23: i32 = 153;
    /// Frame `death24`.
    pub const DEATH24: i32 = 154;
    /// Frame `death25`.
    pub const DEATH25: i32 = 155;
    /// Frame `death26`.
    pub const DEATH26: i32 = 156;
    /// Frame `death27`.
    pub const DEATH27: i32 = 157;
    /// Frame `death28`.
    pub const DEATH28: i32 = 158;
    /// Frame `death29`.
    pub const DEATH29: i32 = 159;
    /// Frame `death30`.
    pub const DEATH30: i32 = 160;
    /// Frame `death31`.
    pub const DEATH31: i32 = 161;
    /// Frame `death32`.
    pub const DEATH32: i32 = 162;
    /// Frame `death33`.
    pub const DEATH33: i32 = 163;
    /// Frame `death34`.
    pub const DEATH34: i32 = 164;
    /// Frame `death35`.
    pub const DEATH35: i32 = 165;
    /// Frame `death36`.
    pub const DEATH36: i32 = 166;
    /// Frame `death37`.
    pub const DEATH37: i32 = 167;
    /// Frame `death38`.
    pub const DEATH38: i32 = 168;
    /// Frame `death39`.
    pub const DEATH39: i32 = 169;
    /// Frame `death40`.
    pub const DEATH40: i32 = 170;
    /// Frame `death41`.
    pub const DEATH41: i32 = 171;
    /// Frame `death42`.
    pub const DEATH42: i32 = 172;
    /// Frame `death43`.
    pub const DEATH43: i32 = 173;
    /// Frame `death44`.
    pub const DEATH44: i32 = 174;
    /// Frame `death45`.
    pub const DEATH45: i32 = 175;
    /// Frame `death46`.
    pub const DEATH46: i32 = 176;
    /// Frame `death47`.
    pub const DEATH47: i32 = 177;
    /// Frame `death48`.
    pub const DEATH48: i32 = 178;
    /// Frame `death49`.
    pub const DEATH49: i32 = 179;
    /// Frame `death50`.
    pub const DEATH50: i32 = 180;
}

/// `boss2Moves` move tables.
pub fn boss2_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "boss2_move_stand",
            0,
            20,
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
            "boss2_move_walk",
            50,
            69,
            None,
            vec![
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_run",
            50,
            69,
            None,
            vec![
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_attack_pre_mg",
            70,
            78,
            None,
            vec![
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("boss2_attack_mg")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "boss2_move_attack_mg",
            79,
            84,
            None,
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2MachineGun")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2MachineGun")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2MachineGun")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2MachineGun")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2MachineGun")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("boss2_reattack_mg")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "boss2_move_attack_hb",
            79,
            84,
            None,
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2HyperBlaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2HyperBlaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2HyperBlaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2HyperBlaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2HyperBlaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![
                        MonsterAction::name("Boss2HyperBlaster"),
                        MonsterAction::name("boss2_reattack_mg"),
                    ],
                    -1,
                ),
            ],
        ),
        monster_move(
            "boss2_move_attack_post_mg",
            85,
            88,
            Some("boss2_run"),
            vec![
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_attack_rocket",
            89,
            109,
            Some("boss2_run"),
            vec![
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (-5f32) as f64,
                    vec![MonsterAction::name("Boss2Rocket")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_attack_rocket2",
            89,
            108,
            Some("boss2_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2Rocket64")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2Rocket64")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2Rocket64")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2Rocket64")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("Boss2Rocket64")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_pain_heavy",
            110,
            127,
            Some("boss2_run"),
            vec![
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
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_pain_light",
            128,
            131,
            Some("boss2_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "boss2_move_death",
            132,
            180,
            Some("boss2_dead"),
            vec![
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("BossExplode")],
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
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("boss2_shrink")],
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
