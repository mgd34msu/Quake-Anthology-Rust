//! supertank move tables (`src/content/q2/base/monsters/tables/supertank.ts`).
//!
//! Original Quake II m_supertank.c frame order and distances. id Software, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `supertankFrame`.
pub mod supertank_frame {
    /// Frame `attak1_1`.
    pub const ATTAK1_1: i32 = 0;
    /// Frame `attak1_2`.
    pub const ATTAK1_2: i32 = 1;
    /// Frame `attak1_3`.
    pub const ATTAK1_3: i32 = 2;
    /// Frame `attak1_4`.
    pub const ATTAK1_4: i32 = 3;
    /// Frame `attak1_5`.
    pub const ATTAK1_5: i32 = 4;
    /// Frame `attak1_6`.
    pub const ATTAK1_6: i32 = 5;
    /// Frame `attak1_7`.
    pub const ATTAK1_7: i32 = 6;
    /// Frame `attak1_8`.
    pub const ATTAK1_8: i32 = 7;
    /// Frame `attak1_9`.
    pub const ATTAK1_9: i32 = 8;
    /// Frame `attak1_10`.
    pub const ATTAK1_10: i32 = 9;
    /// Frame `attak1_11`.
    pub const ATTAK1_11: i32 = 10;
    /// Frame `attak1_12`.
    pub const ATTAK1_12: i32 = 11;
    /// Frame `attak1_13`.
    pub const ATTAK1_13: i32 = 12;
    /// Frame `attak1_14`.
    pub const ATTAK1_14: i32 = 13;
    /// Frame `attak1_15`.
    pub const ATTAK1_15: i32 = 14;
    /// Frame `attak1_16`.
    pub const ATTAK1_16: i32 = 15;
    /// Frame `attak1_17`.
    pub const ATTAK1_17: i32 = 16;
    /// Frame `attak1_18`.
    pub const ATTAK1_18: i32 = 17;
    /// Frame `attak1_19`.
    pub const ATTAK1_19: i32 = 18;
    /// Frame `attak1_20`.
    pub const ATTAK1_20: i32 = 19;
    /// Frame `attak2_1`.
    pub const ATTAK2_1: i32 = 20;
    /// Frame `attak2_2`.
    pub const ATTAK2_2: i32 = 21;
    /// Frame `attak2_3`.
    pub const ATTAK2_3: i32 = 22;
    /// Frame `attak2_4`.
    pub const ATTAK2_4: i32 = 23;
    /// Frame `attak2_5`.
    pub const ATTAK2_5: i32 = 24;
    /// Frame `attak2_6`.
    pub const ATTAK2_6: i32 = 25;
    /// Frame `attak2_7`.
    pub const ATTAK2_7: i32 = 26;
    /// Frame `attak2_8`.
    pub const ATTAK2_8: i32 = 27;
    /// Frame `attak2_9`.
    pub const ATTAK2_9: i32 = 28;
    /// Frame `attak2_10`.
    pub const ATTAK2_10: i32 = 29;
    /// Frame `attak2_11`.
    pub const ATTAK2_11: i32 = 30;
    /// Frame `attak2_12`.
    pub const ATTAK2_12: i32 = 31;
    /// Frame `attak2_13`.
    pub const ATTAK2_13: i32 = 32;
    /// Frame `attak2_14`.
    pub const ATTAK2_14: i32 = 33;
    /// Frame `attak2_15`.
    pub const ATTAK2_15: i32 = 34;
    /// Frame `attak2_16`.
    pub const ATTAK2_16: i32 = 35;
    /// Frame `attak2_17`.
    pub const ATTAK2_17: i32 = 36;
    /// Frame `attak2_18`.
    pub const ATTAK2_18: i32 = 37;
    /// Frame `attak2_19`.
    pub const ATTAK2_19: i32 = 38;
    /// Frame `attak2_20`.
    pub const ATTAK2_20: i32 = 39;
    /// Frame `attak2_21`.
    pub const ATTAK2_21: i32 = 40;
    /// Frame `attak2_22`.
    pub const ATTAK2_22: i32 = 41;
    /// Frame `attak2_23`.
    pub const ATTAK2_23: i32 = 42;
    /// Frame `attak2_24`.
    pub const ATTAK2_24: i32 = 43;
    /// Frame `attak2_25`.
    pub const ATTAK2_25: i32 = 44;
    /// Frame `attak2_26`.
    pub const ATTAK2_26: i32 = 45;
    /// Frame `attak2_27`.
    pub const ATTAK2_27: i32 = 46;
    /// Frame `attak3_1`.
    pub const ATTAK3_1: i32 = 47;
    /// Frame `attak3_2`.
    pub const ATTAK3_2: i32 = 48;
    /// Frame `attak3_3`.
    pub const ATTAK3_3: i32 = 49;
    /// Frame `attak3_4`.
    pub const ATTAK3_4: i32 = 50;
    /// Frame `attak3_5`.
    pub const ATTAK3_5: i32 = 51;
    /// Frame `attak3_6`.
    pub const ATTAK3_6: i32 = 52;
    /// Frame `attak3_7`.
    pub const ATTAK3_7: i32 = 53;
    /// Frame `attak3_8`.
    pub const ATTAK3_8: i32 = 54;
    /// Frame `attak3_9`.
    pub const ATTAK3_9: i32 = 55;
    /// Frame `attak3_10`.
    pub const ATTAK3_10: i32 = 56;
    /// Frame `attak3_11`.
    pub const ATTAK3_11: i32 = 57;
    /// Frame `attak3_12`.
    pub const ATTAK3_12: i32 = 58;
    /// Frame `attak3_13`.
    pub const ATTAK3_13: i32 = 59;
    /// Frame `attak3_14`.
    pub const ATTAK3_14: i32 = 60;
    /// Frame `attak3_15`.
    pub const ATTAK3_15: i32 = 61;
    /// Frame `attak3_16`.
    pub const ATTAK3_16: i32 = 62;
    /// Frame `attak3_17`.
    pub const ATTAK3_17: i32 = 63;
    /// Frame `attak3_18`.
    pub const ATTAK3_18: i32 = 64;
    /// Frame `attak3_19`.
    pub const ATTAK3_19: i32 = 65;
    /// Frame `attak3_20`.
    pub const ATTAK3_20: i32 = 66;
    /// Frame `attak3_21`.
    pub const ATTAK3_21: i32 = 67;
    /// Frame `attak3_22`.
    pub const ATTAK3_22: i32 = 68;
    /// Frame `attak3_23`.
    pub const ATTAK3_23: i32 = 69;
    /// Frame `attak3_24`.
    pub const ATTAK3_24: i32 = 70;
    /// Frame `attak3_25`.
    pub const ATTAK3_25: i32 = 71;
    /// Frame `attak3_26`.
    pub const ATTAK3_26: i32 = 72;
    /// Frame `attak3_27`.
    pub const ATTAK3_27: i32 = 73;
    /// Frame `attak4_1`.
    pub const ATTAK4_1: i32 = 74;
    /// Frame `attak4_2`.
    pub const ATTAK4_2: i32 = 75;
    /// Frame `attak4_3`.
    pub const ATTAK4_3: i32 = 76;
    /// Frame `attak4_4`.
    pub const ATTAK4_4: i32 = 77;
    /// Frame `attak4_5`.
    pub const ATTAK4_5: i32 = 78;
    /// Frame `attak4_6`.
    pub const ATTAK4_6: i32 = 79;
    /// Frame `backwd_1`.
    pub const BACKWD_1: i32 = 80;
    /// Frame `backwd_2`.
    pub const BACKWD_2: i32 = 81;
    /// Frame `backwd_3`.
    pub const BACKWD_3: i32 = 82;
    /// Frame `backwd_4`.
    pub const BACKWD_4: i32 = 83;
    /// Frame `backwd_5`.
    pub const BACKWD_5: i32 = 84;
    /// Frame `backwd_6`.
    pub const BACKWD_6: i32 = 85;
    /// Frame `backwd_7`.
    pub const BACKWD_7: i32 = 86;
    /// Frame `backwd_8`.
    pub const BACKWD_8: i32 = 87;
    /// Frame `backwd_9`.
    pub const BACKWD_9: i32 = 88;
    /// Frame `backwd_10`.
    pub const BACKWD_10: i32 = 89;
    /// Frame `backwd_11`.
    pub const BACKWD_11: i32 = 90;
    /// Frame `backwd_12`.
    pub const BACKWD_12: i32 = 91;
    /// Frame `backwd_13`.
    pub const BACKWD_13: i32 = 92;
    /// Frame `backwd_14`.
    pub const BACKWD_14: i32 = 93;
    /// Frame `backwd_15`.
    pub const BACKWD_15: i32 = 94;
    /// Frame `backwd_16`.
    pub const BACKWD_16: i32 = 95;
    /// Frame `backwd_17`.
    pub const BACKWD_17: i32 = 96;
    /// Frame `backwd_18`.
    pub const BACKWD_18: i32 = 97;
    /// Frame `death_1`.
    pub const DEATH_1: i32 = 98;
    /// Frame `death_2`.
    pub const DEATH_2: i32 = 99;
    /// Frame `death_3`.
    pub const DEATH_3: i32 = 100;
    /// Frame `death_4`.
    pub const DEATH_4: i32 = 101;
    /// Frame `death_5`.
    pub const DEATH_5: i32 = 102;
    /// Frame `death_6`.
    pub const DEATH_6: i32 = 103;
    /// Frame `death_7`.
    pub const DEATH_7: i32 = 104;
    /// Frame `death_8`.
    pub const DEATH_8: i32 = 105;
    /// Frame `death_9`.
    pub const DEATH_9: i32 = 106;
    /// Frame `death_10`.
    pub const DEATH_10: i32 = 107;
    /// Frame `death_11`.
    pub const DEATH_11: i32 = 108;
    /// Frame `death_12`.
    pub const DEATH_12: i32 = 109;
    /// Frame `death_13`.
    pub const DEATH_13: i32 = 110;
    /// Frame `death_14`.
    pub const DEATH_14: i32 = 111;
    /// Frame `death_15`.
    pub const DEATH_15: i32 = 112;
    /// Frame `death_16`.
    pub const DEATH_16: i32 = 113;
    /// Frame `death_17`.
    pub const DEATH_17: i32 = 114;
    /// Frame `death_18`.
    pub const DEATH_18: i32 = 115;
    /// Frame `death_19`.
    pub const DEATH_19: i32 = 116;
    /// Frame `death_20`.
    pub const DEATH_20: i32 = 117;
    /// Frame `death_21`.
    pub const DEATH_21: i32 = 118;
    /// Frame `death_22`.
    pub const DEATH_22: i32 = 119;
    /// Frame `death_23`.
    pub const DEATH_23: i32 = 120;
    /// Frame `death_24`.
    pub const DEATH_24: i32 = 121;
    /// Frame `death_31`.
    pub const DEATH_31: i32 = 122;
    /// Frame `death_32`.
    pub const DEATH_32: i32 = 123;
    /// Frame `death_33`.
    pub const DEATH_33: i32 = 124;
    /// Frame `death_45`.
    pub const DEATH_45: i32 = 125;
    /// Frame `death_46`.
    pub const DEATH_46: i32 = 126;
    /// Frame `death_47`.
    pub const DEATH_47: i32 = 127;
    /// Frame `forwrd_1`.
    pub const FORWRD_1: i32 = 128;
    /// Frame `forwrd_2`.
    pub const FORWRD_2: i32 = 129;
    /// Frame `forwrd_3`.
    pub const FORWRD_3: i32 = 130;
    /// Frame `forwrd_4`.
    pub const FORWRD_4: i32 = 131;
    /// Frame `forwrd_5`.
    pub const FORWRD_5: i32 = 132;
    /// Frame `forwrd_6`.
    pub const FORWRD_6: i32 = 133;
    /// Frame `forwrd_7`.
    pub const FORWRD_7: i32 = 134;
    /// Frame `forwrd_8`.
    pub const FORWRD_8: i32 = 135;
    /// Frame `forwrd_9`.
    pub const FORWRD_9: i32 = 136;
    /// Frame `forwrd_10`.
    pub const FORWRD_10: i32 = 137;
    /// Frame `forwrd_11`.
    pub const FORWRD_11: i32 = 138;
    /// Frame `forwrd_12`.
    pub const FORWRD_12: i32 = 139;
    /// Frame `forwrd_13`.
    pub const FORWRD_13: i32 = 140;
    /// Frame `forwrd_14`.
    pub const FORWRD_14: i32 = 141;
    /// Frame `forwrd_15`.
    pub const FORWRD_15: i32 = 142;
    /// Frame `forwrd_16`.
    pub const FORWRD_16: i32 = 143;
    /// Frame `forwrd_17`.
    pub const FORWRD_17: i32 = 144;
    /// Frame `forwrd_18`.
    pub const FORWRD_18: i32 = 145;
    /// Frame `left_1`.
    pub const LEFT_1: i32 = 146;
    /// Frame `left_2`.
    pub const LEFT_2: i32 = 147;
    /// Frame `left_3`.
    pub const LEFT_3: i32 = 148;
    /// Frame `left_4`.
    pub const LEFT_4: i32 = 149;
    /// Frame `left_5`.
    pub const LEFT_5: i32 = 150;
    /// Frame `left_6`.
    pub const LEFT_6: i32 = 151;
    /// Frame `left_7`.
    pub const LEFT_7: i32 = 152;
    /// Frame `left_8`.
    pub const LEFT_8: i32 = 153;
    /// Frame `left_9`.
    pub const LEFT_9: i32 = 154;
    /// Frame `left_10`.
    pub const LEFT_10: i32 = 155;
    /// Frame `left_11`.
    pub const LEFT_11: i32 = 156;
    /// Frame `left_12`.
    pub const LEFT_12: i32 = 157;
    /// Frame `left_13`.
    pub const LEFT_13: i32 = 158;
    /// Frame `left_14`.
    pub const LEFT_14: i32 = 159;
    /// Frame `left_15`.
    pub const LEFT_15: i32 = 160;
    /// Frame `left_16`.
    pub const LEFT_16: i32 = 161;
    /// Frame `left_17`.
    pub const LEFT_17: i32 = 162;
    /// Frame `left_18`.
    pub const LEFT_18: i32 = 163;
    /// Frame `pain1_1`.
    pub const PAIN1_1: i32 = 164;
    /// Frame `pain1_2`.
    pub const PAIN1_2: i32 = 165;
    /// Frame `pain1_3`.
    pub const PAIN1_3: i32 = 166;
    /// Frame `pain1_4`.
    pub const PAIN1_4: i32 = 167;
    /// Frame `pain2_5`.
    pub const PAIN2_5: i32 = 168;
    /// Frame `pain2_6`.
    pub const PAIN2_6: i32 = 169;
    /// Frame `pain2_7`.
    pub const PAIN2_7: i32 = 170;
    /// Frame `pain2_8`.
    pub const PAIN2_8: i32 = 171;
    /// Frame `pain3_9`.
    pub const PAIN3_9: i32 = 172;
    /// Frame `pain3_10`.
    pub const PAIN3_10: i32 = 173;
    /// Frame `pain3_11`.
    pub const PAIN3_11: i32 = 174;
    /// Frame `pain3_12`.
    pub const PAIN3_12: i32 = 175;
    /// Frame `right_1`.
    pub const RIGHT_1: i32 = 176;
    /// Frame `right_2`.
    pub const RIGHT_2: i32 = 177;
    /// Frame `right_3`.
    pub const RIGHT_3: i32 = 178;
    /// Frame `right_4`.
    pub const RIGHT_4: i32 = 179;
    /// Frame `right_5`.
    pub const RIGHT_5: i32 = 180;
    /// Frame `right_6`.
    pub const RIGHT_6: i32 = 181;
    /// Frame `right_7`.
    pub const RIGHT_7: i32 = 182;
    /// Frame `right_8`.
    pub const RIGHT_8: i32 = 183;
    /// Frame `right_9`.
    pub const RIGHT_9: i32 = 184;
    /// Frame `right_10`.
    pub const RIGHT_10: i32 = 185;
    /// Frame `right_11`.
    pub const RIGHT_11: i32 = 186;
    /// Frame `right_12`.
    pub const RIGHT_12: i32 = 187;
    /// Frame `right_13`.
    pub const RIGHT_13: i32 = 188;
    /// Frame `right_14`.
    pub const RIGHT_14: i32 = 189;
    /// Frame `right_15`.
    pub const RIGHT_15: i32 = 190;
    /// Frame `right_16`.
    pub const RIGHT_16: i32 = 191;
    /// Frame `right_17`.
    pub const RIGHT_17: i32 = 192;
    /// Frame `right_18`.
    pub const RIGHT_18: i32 = 193;
    /// Frame `stand_1`.
    pub const STAND_1: i32 = 194;
    /// Frame `stand_2`.
    pub const STAND_2: i32 = 195;
    /// Frame `stand_3`.
    pub const STAND_3: i32 = 196;
    /// Frame `stand_4`.
    pub const STAND_4: i32 = 197;
    /// Frame `stand_5`.
    pub const STAND_5: i32 = 198;
    /// Frame `stand_6`.
    pub const STAND_6: i32 = 199;
    /// Frame `stand_7`.
    pub const STAND_7: i32 = 200;
    /// Frame `stand_8`.
    pub const STAND_8: i32 = 201;
    /// Frame `stand_9`.
    pub const STAND_9: i32 = 202;
    /// Frame `stand_10`.
    pub const STAND_10: i32 = 203;
    /// Frame `stand_11`.
    pub const STAND_11: i32 = 204;
    /// Frame `stand_12`.
    pub const STAND_12: i32 = 205;
    /// Frame `stand_13`.
    pub const STAND_13: i32 = 206;
    /// Frame `stand_14`.
    pub const STAND_14: i32 = 207;
    /// Frame `stand_15`.
    pub const STAND_15: i32 = 208;
    /// Frame `stand_16`.
    pub const STAND_16: i32 = 209;
    /// Frame `stand_17`.
    pub const STAND_17: i32 = 210;
    /// Frame `stand_18`.
    pub const STAND_18: i32 = 211;
    /// Frame `stand_19`.
    pub const STAND_19: i32 = 212;
    /// Frame `stand_20`.
    pub const STAND_20: i32 = 213;
    /// Frame `stand_21`.
    pub const STAND_21: i32 = 214;
    /// Frame `stand_22`.
    pub const STAND_22: i32 = 215;
    /// Frame `stand_23`.
    pub const STAND_23: i32 = 216;
    /// Frame `stand_24`.
    pub const STAND_24: i32 = 217;
    /// Frame `stand_25`.
    pub const STAND_25: i32 = 218;
    /// Frame `stand_26`.
    pub const STAND_26: i32 = 219;
    /// Frame `stand_27`.
    pub const STAND_27: i32 = 220;
    /// Frame `stand_28`.
    pub const STAND_28: i32 = 221;
    /// Frame `stand_29`.
    pub const STAND_29: i32 = 222;
    /// Frame `stand_30`.
    pub const STAND_30: i32 = 223;
    /// Frame `stand_31`.
    pub const STAND_31: i32 = 224;
    /// Frame `stand_32`.
    pub const STAND_32: i32 = 225;
    /// Frame `stand_33`.
    pub const STAND_33: i32 = 226;
    /// Frame `stand_34`.
    pub const STAND_34: i32 = 227;
    /// Frame `stand_35`.
    pub const STAND_35: i32 = 228;
    /// Frame `stand_36`.
    pub const STAND_36: i32 = 229;
    /// Frame `stand_37`.
    pub const STAND_37: i32 = 230;
    /// Frame `stand_38`.
    pub const STAND_38: i32 = 231;
    /// Frame `stand_39`.
    pub const STAND_39: i32 = 232;
    /// Frame `stand_40`.
    pub const STAND_40: i32 = 233;
    /// Frame `stand_41`.
    pub const STAND_41: i32 = 234;
    /// Frame `stand_42`.
    pub const STAND_42: i32 = 235;
    /// Frame `stand_43`.
    pub const STAND_43: i32 = 236;
    /// Frame `stand_44`.
    pub const STAND_44: i32 = 237;
    /// Frame `stand_45`.
    pub const STAND_45: i32 = 238;
    /// Frame `stand_46`.
    pub const STAND_46: i32 = 239;
    /// Frame `stand_47`.
    pub const STAND_47: i32 = 240;
    /// Frame `stand_48`.
    pub const STAND_48: i32 = 241;
    /// Frame `stand_49`.
    pub const STAND_49: i32 = 242;
    /// Frame `stand_50`.
    pub const STAND_50: i32 = 243;
    /// Frame `stand_51`.
    pub const STAND_51: i32 = 244;
    /// Frame `stand_52`.
    pub const STAND_52: i32 = 245;
    /// Frame `stand_53`.
    pub const STAND_53: i32 = 246;
    /// Frame `stand_54`.
    pub const STAND_54: i32 = 247;
    /// Frame `stand_55`.
    pub const STAND_55: i32 = 248;
    /// Frame `stand_56`.
    pub const STAND_56: i32 = 249;
    /// Frame `stand_57`.
    pub const STAND_57: i32 = 250;
    /// Frame `stand_58`.
    pub const STAND_58: i32 = 251;
    /// Frame `stand_59`.
    pub const STAND_59: i32 = 252;
    /// Frame `stand_60`.
    pub const STAND_60: i32 = 253;
}

/// `supertankMoves` move tables.
pub fn supertank_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("supertank_move_stand", 194, 253, None, vec![
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
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_run", 128, 145, None, vec![
            monster_frame(MonsterAi::Run, 12.0, vec![MonsterAction::name("TreadSound")], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
            monster_frame(MonsterAi::Run, 12.0, vec![], -1),
        ]),
        monster_move("supertank_move_forward", 128, 145, None, vec![
            monster_frame(MonsterAi::Walk, 4.0, vec![MonsterAction::name("TreadSound")], -1),
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
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
        ]),
        monster_move("supertank_move_turn_right", 176, 193, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("TreadSound")], -1),
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
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_turn_left", 146, 163, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("TreadSound")], -1),
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
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_pain3", 172, 175, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_pain2", 168, 171, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_pain1", 164, 167, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_death", 98, 121, Some("supertank_dead"), vec![
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
        monster_move("supertank_move_backward", 80, 97, None, vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("TreadSound")], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_attack4", 74, 79, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("supertank_move_attack3", 47, 73, Some("supertank_run"), vec![
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
        ]),
        monster_move("supertank_move_attack2", 20, 46, Some("supertank_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankRocket")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("supertankRocket")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("supertankRocket")], -1),
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
        ]),
        monster_move("supertank_move_attack1", 0, 5, Some("supertank_reattack1"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankMachineGun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankMachineGun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankMachineGun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankMachineGun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankMachineGun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("supertankMachineGun")], -1),
        ]),
        monster_move("supertank_move_end_attack1", 6, 19, Some("supertank_run"), vec![
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
        ]),
    ]
}
