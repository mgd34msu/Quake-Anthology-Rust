//! guardian move tables (`src/content/q2/rerelease/monsters/tables/guardian.ts`).

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `guardianFrame`.
pub mod guardian_frame {
    /// Frame `sleep1`.
    pub const SLEEP1: i32 = 0;
    /// Frame `sleep2`.
    pub const SLEEP2: i32 = 1;
    /// Frame `sleep3`.
    pub const SLEEP3: i32 = 2;
    /// Frame `sleep4`.
    pub const SLEEP4: i32 = 3;
    /// Frame `sleep5`.
    pub const SLEEP5: i32 = 4;
    /// Frame `sleep6`.
    pub const SLEEP6: i32 = 5;
    /// Frame `sleep7`.
    pub const SLEEP7: i32 = 6;
    /// Frame `sleep8`.
    pub const SLEEP8: i32 = 7;
    /// Frame `sleep9`.
    pub const SLEEP9: i32 = 8;
    /// Frame `sleep10`.
    pub const SLEEP10: i32 = 9;
    /// Frame `sleep11`.
    pub const SLEEP11: i32 = 10;
    /// Frame `sleep12`.
    pub const SLEEP12: i32 = 11;
    /// Frame `sleep13`.
    pub const SLEEP13: i32 = 12;
    /// Frame `sleep14`.
    pub const SLEEP14: i32 = 13;
    /// Frame `death1`.
    pub const DEATH1: i32 = 14;
    /// Frame `death2`.
    pub const DEATH2: i32 = 15;
    /// Frame `death3`.
    pub const DEATH3: i32 = 16;
    /// Frame `death4`.
    pub const DEATH4: i32 = 17;
    /// Frame `death5`.
    pub const DEATH5: i32 = 18;
    /// Frame `death6`.
    pub const DEATH6: i32 = 19;
    /// Frame `death7`.
    pub const DEATH7: i32 = 20;
    /// Frame `death8`.
    pub const DEATH8: i32 = 21;
    /// Frame `death9`.
    pub const DEATH9: i32 = 22;
    /// Frame `death10`.
    pub const DEATH10: i32 = 23;
    /// Frame `death11`.
    pub const DEATH11: i32 = 24;
    /// Frame `death12`.
    pub const DEATH12: i32 = 25;
    /// Frame `death13`.
    pub const DEATH13: i32 = 26;
    /// Frame `death14`.
    pub const DEATH14: i32 = 27;
    /// Frame `death15`.
    pub const DEATH15: i32 = 28;
    /// Frame `death16`.
    pub const DEATH16: i32 = 29;
    /// Frame `death17`.
    pub const DEATH17: i32 = 30;
    /// Frame `death18`.
    pub const DEATH18: i32 = 31;
    /// Frame `death19`.
    pub const DEATH19: i32 = 32;
    /// Frame `death20`.
    pub const DEATH20: i32 = 33;
    /// Frame `death21`.
    pub const DEATH21: i32 = 34;
    /// Frame `death22`.
    pub const DEATH22: i32 = 35;
    /// Frame `death23`.
    pub const DEATH23: i32 = 36;
    /// Frame `death24`.
    pub const DEATH24: i32 = 37;
    /// Frame `death25`.
    pub const DEATH25: i32 = 38;
    /// Frame `death26`.
    pub const DEATH26: i32 = 39;
    /// Frame `atk1_out1`.
    pub const ATK1_OUT1: i32 = 40;
    /// Frame `atk1_out2`.
    pub const ATK1_OUT2: i32 = 41;
    /// Frame `atk1_out3`.
    pub const ATK1_OUT3: i32 = 42;
    /// Frame `atk2_out1`.
    pub const ATK2_OUT1: i32 = 43;
    /// Frame `atk2_out2`.
    pub const ATK2_OUT2: i32 = 44;
    /// Frame `atk2_out3`.
    pub const ATK2_OUT3: i32 = 45;
    /// Frame `atk2_out4`.
    pub const ATK2_OUT4: i32 = 46;
    /// Frame `atk2_out5`.
    pub const ATK2_OUT5: i32 = 47;
    /// Frame `atk2_out6`.
    pub const ATK2_OUT6: i32 = 48;
    /// Frame `atk2_out7`.
    pub const ATK2_OUT7: i32 = 49;
    /// Frame `kick_out1`.
    pub const KICK_OUT1: i32 = 50;
    /// Frame `kick_out2`.
    pub const KICK_OUT2: i32 = 51;
    /// Frame `kick_out3`.
    pub const KICK_OUT3: i32 = 52;
    /// Frame `kick_out4`.
    pub const KICK_OUT4: i32 = 53;
    /// Frame `kick_out5`.
    pub const KICK_OUT5: i32 = 54;
    /// Frame `kick_out6`.
    pub const KICK_OUT6: i32 = 55;
    /// Frame `kick_out7`.
    pub const KICK_OUT7: i32 = 56;
    /// Frame `kick_out8`.
    pub const KICK_OUT8: i32 = 57;
    /// Frame `kick_out9`.
    pub const KICK_OUT9: i32 = 58;
    /// Frame `kick_out10`.
    pub const KICK_OUT10: i32 = 59;
    /// Frame `kick_out11`.
    pub const KICK_OUT11: i32 = 60;
    /// Frame `kick_out12`.
    pub const KICK_OUT12: i32 = 61;
    /// Frame `pain1_1`.
    pub const PAIN1_1: i32 = 62;
    /// Frame `pain1_2`.
    pub const PAIN1_2: i32 = 63;
    /// Frame `pain1_3`.
    pub const PAIN1_3: i32 = 64;
    /// Frame `pain1_4`.
    pub const PAIN1_4: i32 = 65;
    /// Frame `pain1_5`.
    pub const PAIN1_5: i32 = 66;
    /// Frame `pain1_6`.
    pub const PAIN1_6: i32 = 67;
    /// Frame `pain1_7`.
    pub const PAIN1_7: i32 = 68;
    /// Frame `pain1_8`.
    pub const PAIN1_8: i32 = 69;
    /// Frame `idle1`.
    pub const IDLE1: i32 = 70;
    /// Frame `idle2`.
    pub const IDLE2: i32 = 71;
    /// Frame `idle3`.
    pub const IDLE3: i32 = 72;
    /// Frame `idle4`.
    pub const IDLE4: i32 = 73;
    /// Frame `idle5`.
    pub const IDLE5: i32 = 74;
    /// Frame `idle6`.
    pub const IDLE6: i32 = 75;
    /// Frame `idle7`.
    pub const IDLE7: i32 = 76;
    /// Frame `idle8`.
    pub const IDLE8: i32 = 77;
    /// Frame `idle9`.
    pub const IDLE9: i32 = 78;
    /// Frame `idle10`.
    pub const IDLE10: i32 = 79;
    /// Frame `idle11`.
    pub const IDLE11: i32 = 80;
    /// Frame `idle12`.
    pub const IDLE12: i32 = 81;
    /// Frame `idle13`.
    pub const IDLE13: i32 = 82;
    /// Frame `idle14`.
    pub const IDLE14: i32 = 83;
    /// Frame `idle15`.
    pub const IDLE15: i32 = 84;
    /// Frame `idle16`.
    pub const IDLE16: i32 = 85;
    /// Frame `idle17`.
    pub const IDLE17: i32 = 86;
    /// Frame `idle18`.
    pub const IDLE18: i32 = 87;
    /// Frame `idle19`.
    pub const IDLE19: i32 = 88;
    /// Frame `idle20`.
    pub const IDLE20: i32 = 89;
    /// Frame `idle21`.
    pub const IDLE21: i32 = 90;
    /// Frame `idle22`.
    pub const IDLE22: i32 = 91;
    /// Frame `idle23`.
    pub const IDLE23: i32 = 92;
    /// Frame `idle24`.
    pub const IDLE24: i32 = 93;
    /// Frame `idle25`.
    pub const IDLE25: i32 = 94;
    /// Frame `idle26`.
    pub const IDLE26: i32 = 95;
    /// Frame `idle27`.
    pub const IDLE27: i32 = 96;
    /// Frame `idle28`.
    pub const IDLE28: i32 = 97;
    /// Frame `idle29`.
    pub const IDLE29: i32 = 98;
    /// Frame `idle30`.
    pub const IDLE30: i32 = 99;
    /// Frame `idle31`.
    pub const IDLE31: i32 = 100;
    /// Frame `idle32`.
    pub const IDLE32: i32 = 101;
    /// Frame `idle33`.
    pub const IDLE33: i32 = 102;
    /// Frame `idle34`.
    pub const IDLE34: i32 = 103;
    /// Frame `idle35`.
    pub const IDLE35: i32 = 104;
    /// Frame `idle36`.
    pub const IDLE36: i32 = 105;
    /// Frame `idle37`.
    pub const IDLE37: i32 = 106;
    /// Frame `idle38`.
    pub const IDLE38: i32 = 107;
    /// Frame `idle39`.
    pub const IDLE39: i32 = 108;
    /// Frame `idle40`.
    pub const IDLE40: i32 = 109;
    /// Frame `idle41`.
    pub const IDLE41: i32 = 110;
    /// Frame `idle42`.
    pub const IDLE42: i32 = 111;
    /// Frame `idle43`.
    pub const IDLE43: i32 = 112;
    /// Frame `idle44`.
    pub const IDLE44: i32 = 113;
    /// Frame `idle45`.
    pub const IDLE45: i32 = 114;
    /// Frame `idle46`.
    pub const IDLE46: i32 = 115;
    /// Frame `idle47`.
    pub const IDLE47: i32 = 116;
    /// Frame `idle48`.
    pub const IDLE48: i32 = 117;
    /// Frame `idle49`.
    pub const IDLE49: i32 = 118;
    /// Frame `idle50`.
    pub const IDLE50: i32 = 119;
    /// Frame `idle51`.
    pub const IDLE51: i32 = 120;
    /// Frame `idle52`.
    pub const IDLE52: i32 = 121;
    /// Frame `atk1_in1`.
    pub const ATK1_IN1: i32 = 122;
    /// Frame `atk1_in2`.
    pub const ATK1_IN2: i32 = 123;
    /// Frame `atk1_in3`.
    pub const ATK1_IN3: i32 = 124;
    /// Frame `kick_in1`.
    pub const KICK_IN1: i32 = 125;
    /// Frame `kick_in2`.
    pub const KICK_IN2: i32 = 126;
    /// Frame `kick_in3`.
    pub const KICK_IN3: i32 = 127;
    /// Frame `kick_in4`.
    pub const KICK_IN4: i32 = 128;
    /// Frame `kick_in5`.
    pub const KICK_IN5: i32 = 129;
    /// Frame `kick_in6`.
    pub const KICK_IN6: i32 = 130;
    /// Frame `kick_in7`.
    pub const KICK_IN7: i32 = 131;
    /// Frame `kick_in8`.
    pub const KICK_IN8: i32 = 132;
    /// Frame `kick_in9`.
    pub const KICK_IN9: i32 = 133;
    /// Frame `kick_in10`.
    pub const KICK_IN10: i32 = 134;
    /// Frame `kick_in11`.
    pub const KICK_IN11: i32 = 135;
    /// Frame `kick_in12`.
    pub const KICK_IN12: i32 = 136;
    /// Frame `kick_in13`.
    pub const KICK_IN13: i32 = 137;
    /// Frame `walk1`.
    pub const WALK1: i32 = 138;
    /// Frame `walk2`.
    pub const WALK2: i32 = 139;
    /// Frame `walk3`.
    pub const WALK3: i32 = 140;
    /// Frame `walk4`.
    pub const WALK4: i32 = 141;
    /// Frame `walk5`.
    pub const WALK5: i32 = 142;
    /// Frame `walk6`.
    pub const WALK6: i32 = 143;
    /// Frame `walk7`.
    pub const WALK7: i32 = 144;
    /// Frame `walk8`.
    pub const WALK8: i32 = 145;
    /// Frame `walk9`.
    pub const WALK9: i32 = 146;
    /// Frame `walk10`.
    pub const WALK10: i32 = 147;
    /// Frame `walk11`.
    pub const WALK11: i32 = 148;
    /// Frame `walk12`.
    pub const WALK12: i32 = 149;
    /// Frame `walk13`.
    pub const WALK13: i32 = 150;
    /// Frame `walk14`.
    pub const WALK14: i32 = 151;
    /// Frame `walk15`.
    pub const WALK15: i32 = 152;
    /// Frame `walk16`.
    pub const WALK16: i32 = 153;
    /// Frame `walk17`.
    pub const WALK17: i32 = 154;
    /// Frame `walk18`.
    pub const WALK18: i32 = 155;
    /// Frame `walk19`.
    pub const WALK19: i32 = 156;
    /// Frame `wake1`.
    pub const WAKE1: i32 = 157;
    /// Frame `wake2`.
    pub const WAKE2: i32 = 158;
    /// Frame `wake3`.
    pub const WAKE3: i32 = 159;
    /// Frame `wake4`.
    pub const WAKE4: i32 = 160;
    /// Frame `wake5`.
    pub const WAKE5: i32 = 161;
    /// Frame `atk1_spin1`.
    pub const ATK1_SPIN1: i32 = 162;
    /// Frame `atk1_spin2`.
    pub const ATK1_SPIN2: i32 = 163;
    /// Frame `atk1_spin3`.
    pub const ATK1_SPIN3: i32 = 164;
    /// Frame `atk1_spin4`.
    pub const ATK1_SPIN4: i32 = 165;
    /// Frame `atk1_spin5`.
    pub const ATK1_SPIN5: i32 = 166;
    /// Frame `atk1_spin6`.
    pub const ATK1_SPIN6: i32 = 167;
    /// Frame `atk1_spin7`.
    pub const ATK1_SPIN7: i32 = 168;
    /// Frame `atk1_spin8`.
    pub const ATK1_SPIN8: i32 = 169;
    /// Frame `atk1_spin9`.
    pub const ATK1_SPIN9: i32 = 170;
    /// Frame `atk1_spin10`.
    pub const ATK1_SPIN10: i32 = 171;
    /// Frame `atk1_spin11`.
    pub const ATK1_SPIN11: i32 = 172;
    /// Frame `atk1_spin12`.
    pub const ATK1_SPIN12: i32 = 173;
    /// Frame `atk1_spin13`.
    pub const ATK1_SPIN13: i32 = 174;
    /// Frame `atk1_spin14`.
    pub const ATK1_SPIN14: i32 = 175;
    /// Frame `atk1_spin15`.
    pub const ATK1_SPIN15: i32 = 176;
    /// Frame `atk2_fire1`.
    pub const ATK2_FIRE1: i32 = 177;
    /// Frame `atk2_fire2`.
    pub const ATK2_FIRE2: i32 = 178;
    /// Frame `atk2_fire3`.
    pub const ATK2_FIRE3: i32 = 179;
    /// Frame `atk2_fire4`.
    pub const ATK2_FIRE4: i32 = 180;
    /// Frame `turnl_1`.
    pub const TURNL_1: i32 = 181;
    /// Frame `turnl_2`.
    pub const TURNL_2: i32 = 182;
    /// Frame `turnl_3`.
    pub const TURNL_3: i32 = 183;
    /// Frame `turnl_4`.
    pub const TURNL_4: i32 = 184;
    /// Frame `turnl_5`.
    pub const TURNL_5: i32 = 185;
    /// Frame `turnl_6`.
    pub const TURNL_6: i32 = 186;
    /// Frame `turnl_7`.
    pub const TURNL_7: i32 = 187;
    /// Frame `turnl_8`.
    pub const TURNL_8: i32 = 188;
    /// Frame `turnl_9`.
    pub const TURNL_9: i32 = 189;
    /// Frame `turnl_10`.
    pub const TURNL_10: i32 = 190;
    /// Frame `turnl_11`.
    pub const TURNL_11: i32 = 191;
    /// Frame `turnr_1`.
    pub const TURNR_1: i32 = 192;
    /// Frame `turnr_2`.
    pub const TURNR_2: i32 = 193;
    /// Frame `turnr_3`.
    pub const TURNR_3: i32 = 194;
    /// Frame `turnr_4`.
    pub const TURNR_4: i32 = 195;
    /// Frame `turnr_5`.
    pub const TURNR_5: i32 = 196;
    /// Frame `turnr_6`.
    pub const TURNR_6: i32 = 197;
    /// Frame `turnr_7`.
    pub const TURNR_7: i32 = 198;
    /// Frame `turnr_8`.
    pub const TURNR_8: i32 = 199;
    /// Frame `turnr_9`.
    pub const TURNR_9: i32 = 200;
    /// Frame `turnr_10`.
    pub const TURNR_10: i32 = 201;
    /// Frame `turnr_11`.
    pub const TURNR_11: i32 = 202;
    /// Frame `atk2_in1`.
    pub const ATK2_IN1: i32 = 203;
    /// Frame `atk2_in2`.
    pub const ATK2_IN2: i32 = 204;
    /// Frame `atk2_in3`.
    pub const ATK2_IN3: i32 = 205;
    /// Frame `atk2_in4`.
    pub const ATK2_IN4: i32 = 206;
    /// Frame `atk2_in5`.
    pub const ATK2_IN5: i32 = 207;
    /// Frame `atk2_in6`.
    pub const ATK2_IN6: i32 = 208;
    /// Frame `atk2_in7`.
    pub const ATK2_IN7: i32 = 209;
    /// Frame `atk2_in8`.
    pub const ATK2_IN8: i32 = 210;
    /// Frame `atk2_in9`.
    pub const ATK2_IN9: i32 = 211;
    /// Frame `atk2_in10`.
    pub const ATK2_IN10: i32 = 212;
    /// Frame `atk2_in11`.
    pub const ATK2_IN11: i32 = 213;
    /// Frame `atk2_in12`.
    pub const ATK2_IN12: i32 = 214;
}

/// `guardianMoves` move tables.
pub fn guardian_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("guardian_move_stand", 70, 121, None, vec![
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
        ]),
        monster_move("guardian_move_walk", 138, 156, None, vec![
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_run", 138, 156, None, vec![
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_pain1", 62, 69, Some("guardian_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_atk1_out", 40, 42, Some("guardian_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_atk1_spin", 162, 176, Some("guardian_atk1_finish"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_atk1_charge")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_atk1_in", 122, 124, Some("guardian_atk1"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_atk2_out", 43, 49, Some("guardian_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_atk2_fire", 177, 180, Some("guardian_atk2_out"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_laser_fire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_laser_fire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_laser_fire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_laser_fire")], -1),
        ]),
        monster_move("guardian_move_atk2_in", 203, 214, Some("guardian_atk2"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_kick", 125, 137, Some("guardian_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_kick")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("guardian_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("guardian_move_death", 14, 39, Some("guardian_dead"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("BossExplode")], -1),
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
        ]),
    ]
}
