//! insane move tables (`src/content/q2/base/monsters/tables/insane.ts`).
//!
//! Original Quake II m_insane.c frame order and distances. id Software, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `insaneFrame`.
pub mod insane_frame {
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
    /// Frame `stand8`.
    pub const STAND8: i32 = 7;
    /// Frame `stand9`.
    pub const STAND9: i32 = 8;
    /// Frame `stand10`.
    pub const STAND10: i32 = 9;
    /// Frame `stand11`.
    pub const STAND11: i32 = 10;
    /// Frame `stand12`.
    pub const STAND12: i32 = 11;
    /// Frame `stand13`.
    pub const STAND13: i32 = 12;
    /// Frame `stand14`.
    pub const STAND14: i32 = 13;
    /// Frame `stand15`.
    pub const STAND15: i32 = 14;
    /// Frame `stand16`.
    pub const STAND16: i32 = 15;
    /// Frame `stand17`.
    pub const STAND17: i32 = 16;
    /// Frame `stand18`.
    pub const STAND18: i32 = 17;
    /// Frame `stand19`.
    pub const STAND19: i32 = 18;
    /// Frame `stand20`.
    pub const STAND20: i32 = 19;
    /// Frame `stand21`.
    pub const STAND21: i32 = 20;
    /// Frame `stand22`.
    pub const STAND22: i32 = 21;
    /// Frame `stand23`.
    pub const STAND23: i32 = 22;
    /// Frame `stand24`.
    pub const STAND24: i32 = 23;
    /// Frame `stand25`.
    pub const STAND25: i32 = 24;
    /// Frame `stand26`.
    pub const STAND26: i32 = 25;
    /// Frame `stand27`.
    pub const STAND27: i32 = 26;
    /// Frame `stand28`.
    pub const STAND28: i32 = 27;
    /// Frame `stand29`.
    pub const STAND29: i32 = 28;
    /// Frame `stand30`.
    pub const STAND30: i32 = 29;
    /// Frame `stand31`.
    pub const STAND31: i32 = 30;
    /// Frame `stand32`.
    pub const STAND32: i32 = 31;
    /// Frame `stand33`.
    pub const STAND33: i32 = 32;
    /// Frame `stand34`.
    pub const STAND34: i32 = 33;
    /// Frame `stand35`.
    pub const STAND35: i32 = 34;
    /// Frame `stand36`.
    pub const STAND36: i32 = 35;
    /// Frame `stand37`.
    pub const STAND37: i32 = 36;
    /// Frame `stand38`.
    pub const STAND38: i32 = 37;
    /// Frame `stand39`.
    pub const STAND39: i32 = 38;
    /// Frame `stand40`.
    pub const STAND40: i32 = 39;
    /// Frame `stand41`.
    pub const STAND41: i32 = 40;
    /// Frame `stand42`.
    pub const STAND42: i32 = 41;
    /// Frame `stand43`.
    pub const STAND43: i32 = 42;
    /// Frame `stand44`.
    pub const STAND44: i32 = 43;
    /// Frame `stand45`.
    pub const STAND45: i32 = 44;
    /// Frame `stand46`.
    pub const STAND46: i32 = 45;
    /// Frame `stand47`.
    pub const STAND47: i32 = 46;
    /// Frame `stand48`.
    pub const STAND48: i32 = 47;
    /// Frame `stand49`.
    pub const STAND49: i32 = 48;
    /// Frame `stand50`.
    pub const STAND50: i32 = 49;
    /// Frame `stand51`.
    pub const STAND51: i32 = 50;
    /// Frame `stand52`.
    pub const STAND52: i32 = 51;
    /// Frame `stand53`.
    pub const STAND53: i32 = 52;
    /// Frame `stand54`.
    pub const STAND54: i32 = 53;
    /// Frame `stand55`.
    pub const STAND55: i32 = 54;
    /// Frame `stand56`.
    pub const STAND56: i32 = 55;
    /// Frame `stand57`.
    pub const STAND57: i32 = 56;
    /// Frame `stand58`.
    pub const STAND58: i32 = 57;
    /// Frame `stand59`.
    pub const STAND59: i32 = 58;
    /// Frame `stand60`.
    pub const STAND60: i32 = 59;
    /// Frame `stand61`.
    pub const STAND61: i32 = 60;
    /// Frame `stand62`.
    pub const STAND62: i32 = 61;
    /// Frame `stand63`.
    pub const STAND63: i32 = 62;
    /// Frame `stand64`.
    pub const STAND64: i32 = 63;
    /// Frame `stand65`.
    pub const STAND65: i32 = 64;
    /// Frame `stand66`.
    pub const STAND66: i32 = 65;
    /// Frame `stand67`.
    pub const STAND67: i32 = 66;
    /// Frame `stand68`.
    pub const STAND68: i32 = 67;
    /// Frame `stand69`.
    pub const STAND69: i32 = 68;
    /// Frame `stand70`.
    pub const STAND70: i32 = 69;
    /// Frame `stand71`.
    pub const STAND71: i32 = 70;
    /// Frame `stand72`.
    pub const STAND72: i32 = 71;
    /// Frame `stand73`.
    pub const STAND73: i32 = 72;
    /// Frame `stand74`.
    pub const STAND74: i32 = 73;
    /// Frame `stand75`.
    pub const STAND75: i32 = 74;
    /// Frame `stand76`.
    pub const STAND76: i32 = 75;
    /// Frame `stand77`.
    pub const STAND77: i32 = 76;
    /// Frame `stand78`.
    pub const STAND78: i32 = 77;
    /// Frame `stand79`.
    pub const STAND79: i32 = 78;
    /// Frame `stand80`.
    pub const STAND80: i32 = 79;
    /// Frame `stand81`.
    pub const STAND81: i32 = 80;
    /// Frame `stand82`.
    pub const STAND82: i32 = 81;
    /// Frame `stand83`.
    pub const STAND83: i32 = 82;
    /// Frame `stand84`.
    pub const STAND84: i32 = 83;
    /// Frame `stand85`.
    pub const STAND85: i32 = 84;
    /// Frame `stand86`.
    pub const STAND86: i32 = 85;
    /// Frame `stand87`.
    pub const STAND87: i32 = 86;
    /// Frame `stand88`.
    pub const STAND88: i32 = 87;
    /// Frame `stand89`.
    pub const STAND89: i32 = 88;
    /// Frame `stand90`.
    pub const STAND90: i32 = 89;
    /// Frame `stand91`.
    pub const STAND91: i32 = 90;
    /// Frame `stand92`.
    pub const STAND92: i32 = 91;
    /// Frame `stand93`.
    pub const STAND93: i32 = 92;
    /// Frame `stand94`.
    pub const STAND94: i32 = 93;
    /// Frame `stand95`.
    pub const STAND95: i32 = 94;
    /// Frame `stand96`.
    pub const STAND96: i32 = 95;
    /// Frame `stand97`.
    pub const STAND97: i32 = 96;
    /// Frame `stand98`.
    pub const STAND98: i32 = 97;
    /// Frame `stand99`.
    pub const STAND99: i32 = 98;
    /// Frame `stand100`.
    pub const STAND100: i32 = 99;
    /// Frame `stand101`.
    pub const STAND101: i32 = 100;
    /// Frame `stand102`.
    pub const STAND102: i32 = 101;
    /// Frame `stand103`.
    pub const STAND103: i32 = 102;
    /// Frame `stand104`.
    pub const STAND104: i32 = 103;
    /// Frame `stand105`.
    pub const STAND105: i32 = 104;
    /// Frame `stand106`.
    pub const STAND106: i32 = 105;
    /// Frame `stand107`.
    pub const STAND107: i32 = 106;
    /// Frame `stand108`.
    pub const STAND108: i32 = 107;
    /// Frame `stand109`.
    pub const STAND109: i32 = 108;
    /// Frame `stand110`.
    pub const STAND110: i32 = 109;
    /// Frame `stand111`.
    pub const STAND111: i32 = 110;
    /// Frame `stand112`.
    pub const STAND112: i32 = 111;
    /// Frame `stand113`.
    pub const STAND113: i32 = 112;
    /// Frame `stand114`.
    pub const STAND114: i32 = 113;
    /// Frame `stand115`.
    pub const STAND115: i32 = 114;
    /// Frame `stand116`.
    pub const STAND116: i32 = 115;
    /// Frame `stand117`.
    pub const STAND117: i32 = 116;
    /// Frame `stand118`.
    pub const STAND118: i32 = 117;
    /// Frame `stand119`.
    pub const STAND119: i32 = 118;
    /// Frame `stand120`.
    pub const STAND120: i32 = 119;
    /// Frame `stand121`.
    pub const STAND121: i32 = 120;
    /// Frame `stand122`.
    pub const STAND122: i32 = 121;
    /// Frame `stand123`.
    pub const STAND123: i32 = 122;
    /// Frame `stand124`.
    pub const STAND124: i32 = 123;
    /// Frame `stand125`.
    pub const STAND125: i32 = 124;
    /// Frame `stand126`.
    pub const STAND126: i32 = 125;
    /// Frame `stand127`.
    pub const STAND127: i32 = 126;
    /// Frame `stand128`.
    pub const STAND128: i32 = 127;
    /// Frame `stand129`.
    pub const STAND129: i32 = 128;
    /// Frame `stand130`.
    pub const STAND130: i32 = 129;
    /// Frame `stand131`.
    pub const STAND131: i32 = 130;
    /// Frame `stand132`.
    pub const STAND132: i32 = 131;
    /// Frame `stand133`.
    pub const STAND133: i32 = 132;
    /// Frame `stand134`.
    pub const STAND134: i32 = 133;
    /// Frame `stand135`.
    pub const STAND135: i32 = 134;
    /// Frame `stand136`.
    pub const STAND136: i32 = 135;
    /// Frame `stand137`.
    pub const STAND137: i32 = 136;
    /// Frame `stand138`.
    pub const STAND138: i32 = 137;
    /// Frame `stand139`.
    pub const STAND139: i32 = 138;
    /// Frame `stand140`.
    pub const STAND140: i32 = 139;
    /// Frame `stand141`.
    pub const STAND141: i32 = 140;
    /// Frame `stand142`.
    pub const STAND142: i32 = 141;
    /// Frame `stand143`.
    pub const STAND143: i32 = 142;
    /// Frame `stand144`.
    pub const STAND144: i32 = 143;
    /// Frame `stand145`.
    pub const STAND145: i32 = 144;
    /// Frame `stand146`.
    pub const STAND146: i32 = 145;
    /// Frame `stand147`.
    pub const STAND147: i32 = 146;
    /// Frame `stand148`.
    pub const STAND148: i32 = 147;
    /// Frame `stand149`.
    pub const STAND149: i32 = 148;
    /// Frame `stand150`.
    pub const STAND150: i32 = 149;
    /// Frame `stand151`.
    pub const STAND151: i32 = 150;
    /// Frame `stand152`.
    pub const STAND152: i32 = 151;
    /// Frame `stand153`.
    pub const STAND153: i32 = 152;
    /// Frame `stand154`.
    pub const STAND154: i32 = 153;
    /// Frame `stand155`.
    pub const STAND155: i32 = 154;
    /// Frame `stand156`.
    pub const STAND156: i32 = 155;
    /// Frame `stand157`.
    pub const STAND157: i32 = 156;
    /// Frame `stand158`.
    pub const STAND158: i32 = 157;
    /// Frame `stand159`.
    pub const STAND159: i32 = 158;
    /// Frame `stand160`.
    pub const STAND160: i32 = 159;
    /// Frame `walk27`.
    pub const WALK27: i32 = 160;
    /// Frame `walk28`.
    pub const WALK28: i32 = 161;
    /// Frame `walk29`.
    pub const WALK29: i32 = 162;
    /// Frame `walk30`.
    pub const WALK30: i32 = 163;
    /// Frame `walk31`.
    pub const WALK31: i32 = 164;
    /// Frame `walk32`.
    pub const WALK32: i32 = 165;
    /// Frame `walk33`.
    pub const WALK33: i32 = 166;
    /// Frame `walk34`.
    pub const WALK34: i32 = 167;
    /// Frame `walk35`.
    pub const WALK35: i32 = 168;
    /// Frame `walk36`.
    pub const WALK36: i32 = 169;
    /// Frame `walk37`.
    pub const WALK37: i32 = 170;
    /// Frame `walk38`.
    pub const WALK38: i32 = 171;
    /// Frame `walk39`.
    pub const WALK39: i32 = 172;
    /// Frame `walk1`.
    pub const WALK1: i32 = 173;
    /// Frame `walk2`.
    pub const WALK2: i32 = 174;
    /// Frame `walk3`.
    pub const WALK3: i32 = 175;
    /// Frame `walk4`.
    pub const WALK4: i32 = 176;
    /// Frame `walk5`.
    pub const WALK5: i32 = 177;
    /// Frame `walk6`.
    pub const WALK6: i32 = 178;
    /// Frame `walk7`.
    pub const WALK7: i32 = 179;
    /// Frame `walk8`.
    pub const WALK8: i32 = 180;
    /// Frame `walk9`.
    pub const WALK9: i32 = 181;
    /// Frame `walk10`.
    pub const WALK10: i32 = 182;
    /// Frame `walk11`.
    pub const WALK11: i32 = 183;
    /// Frame `walk12`.
    pub const WALK12: i32 = 184;
    /// Frame `walk13`.
    pub const WALK13: i32 = 185;
    /// Frame `walk14`.
    pub const WALK14: i32 = 186;
    /// Frame `walk15`.
    pub const WALK15: i32 = 187;
    /// Frame `walk16`.
    pub const WALK16: i32 = 188;
    /// Frame `walk17`.
    pub const WALK17: i32 = 189;
    /// Frame `walk18`.
    pub const WALK18: i32 = 190;
    /// Frame `walk19`.
    pub const WALK19: i32 = 191;
    /// Frame `walk20`.
    pub const WALK20: i32 = 192;
    /// Frame `walk21`.
    pub const WALK21: i32 = 193;
    /// Frame `walk22`.
    pub const WALK22: i32 = 194;
    /// Frame `walk23`.
    pub const WALK23: i32 = 195;
    /// Frame `walk24`.
    pub const WALK24: i32 = 196;
    /// Frame `walk25`.
    pub const WALK25: i32 = 197;
    /// Frame `walk26`.
    pub const WALK26: i32 = 198;
    /// Frame `st_pain2`.
    pub const ST_PAIN2: i32 = 199;
    /// Frame `st_pain3`.
    pub const ST_PAIN3: i32 = 200;
    /// Frame `st_pain4`.
    pub const ST_PAIN4: i32 = 201;
    /// Frame `st_pain5`.
    pub const ST_PAIN5: i32 = 202;
    /// Frame `st_pain6`.
    pub const ST_PAIN6: i32 = 203;
    /// Frame `st_pain7`.
    pub const ST_PAIN7: i32 = 204;
    /// Frame `st_pain8`.
    pub const ST_PAIN8: i32 = 205;
    /// Frame `st_pain9`.
    pub const ST_PAIN9: i32 = 206;
    /// Frame `st_pain10`.
    pub const ST_PAIN10: i32 = 207;
    /// Frame `st_pain11`.
    pub const ST_PAIN11: i32 = 208;
    /// Frame `st_pain12`.
    pub const ST_PAIN12: i32 = 209;
    /// Frame `st_death2`.
    pub const ST_DEATH2: i32 = 210;
    /// Frame `st_death3`.
    pub const ST_DEATH3: i32 = 211;
    /// Frame `st_death4`.
    pub const ST_DEATH4: i32 = 212;
    /// Frame `st_death5`.
    pub const ST_DEATH5: i32 = 213;
    /// Frame `st_death6`.
    pub const ST_DEATH6: i32 = 214;
    /// Frame `st_death7`.
    pub const ST_DEATH7: i32 = 215;
    /// Frame `st_death8`.
    pub const ST_DEATH8: i32 = 216;
    /// Frame `st_death9`.
    pub const ST_DEATH9: i32 = 217;
    /// Frame `st_death10`.
    pub const ST_DEATH10: i32 = 218;
    /// Frame `st_death11`.
    pub const ST_DEATH11: i32 = 219;
    /// Frame `st_death12`.
    pub const ST_DEATH12: i32 = 220;
    /// Frame `st_death13`.
    pub const ST_DEATH13: i32 = 221;
    /// Frame `st_death14`.
    pub const ST_DEATH14: i32 = 222;
    /// Frame `st_death15`.
    pub const ST_DEATH15: i32 = 223;
    /// Frame `st_death16`.
    pub const ST_DEATH16: i32 = 224;
    /// Frame `st_death17`.
    pub const ST_DEATH17: i32 = 225;
    /// Frame `st_death18`.
    pub const ST_DEATH18: i32 = 226;
    /// Frame `crawl1`.
    pub const CRAWL1: i32 = 227;
    /// Frame `crawl2`.
    pub const CRAWL2: i32 = 228;
    /// Frame `crawl3`.
    pub const CRAWL3: i32 = 229;
    /// Frame `crawl4`.
    pub const CRAWL4: i32 = 230;
    /// Frame `crawl5`.
    pub const CRAWL5: i32 = 231;
    /// Frame `crawl6`.
    pub const CRAWL6: i32 = 232;
    /// Frame `crawl7`.
    pub const CRAWL7: i32 = 233;
    /// Frame `crawl8`.
    pub const CRAWL8: i32 = 234;
    /// Frame `crawl9`.
    pub const CRAWL9: i32 = 235;
    /// Frame `cr_pain2`.
    pub const CR_PAIN2: i32 = 236;
    /// Frame `cr_pain3`.
    pub const CR_PAIN3: i32 = 237;
    /// Frame `cr_pain4`.
    pub const CR_PAIN4: i32 = 238;
    /// Frame `cr_pain5`.
    pub const CR_PAIN5: i32 = 239;
    /// Frame `cr_pain6`.
    pub const CR_PAIN6: i32 = 240;
    /// Frame `cr_pain7`.
    pub const CR_PAIN7: i32 = 241;
    /// Frame `cr_pain8`.
    pub const CR_PAIN8: i32 = 242;
    /// Frame `cr_pain9`.
    pub const CR_PAIN9: i32 = 243;
    /// Frame `cr_pain10`.
    pub const CR_PAIN10: i32 = 244;
    /// Frame `cr_death10`.
    pub const CR_DEATH10: i32 = 245;
    /// Frame `cr_death11`.
    pub const CR_DEATH11: i32 = 246;
    /// Frame `cr_death12`.
    pub const CR_DEATH12: i32 = 247;
    /// Frame `cr_death13`.
    pub const CR_DEATH13: i32 = 248;
    /// Frame `cr_death14`.
    pub const CR_DEATH14: i32 = 249;
    /// Frame `cr_death15`.
    pub const CR_DEATH15: i32 = 250;
    /// Frame `cr_death16`.
    pub const CR_DEATH16: i32 = 251;
    /// Frame `cross1`.
    pub const CROSS1: i32 = 252;
    /// Frame `cross2`.
    pub const CROSS2: i32 = 253;
    /// Frame `cross3`.
    pub const CROSS3: i32 = 254;
    /// Frame `cross4`.
    pub const CROSS4: i32 = 255;
    /// Frame `cross5`.
    pub const CROSS5: i32 = 256;
    /// Frame `cross6`.
    pub const CROSS6: i32 = 257;
    /// Frame `cross7`.
    pub const CROSS7: i32 = 258;
    /// Frame `cross8`.
    pub const CROSS8: i32 = 259;
    /// Frame `cross9`.
    pub const CROSS9: i32 = 260;
    /// Frame `cross10`.
    pub const CROSS10: i32 = 261;
    /// Frame `cross11`.
    pub const CROSS11: i32 = 262;
    /// Frame `cross12`.
    pub const CROSS12: i32 = 263;
    /// Frame `cross13`.
    pub const CROSS13: i32 = 264;
    /// Frame `cross14`.
    pub const CROSS14: i32 = 265;
    /// Frame `cross15`.
    pub const CROSS15: i32 = 266;
    /// Frame `cross16`.
    pub const CROSS16: i32 = 267;
    /// Frame `cross17`.
    pub const CROSS17: i32 = 268;
    /// Frame `cross18`.
    pub const CROSS18: i32 = 269;
    /// Frame `cross19`.
    pub const CROSS19: i32 = 270;
    /// Frame `cross20`.
    pub const CROSS20: i32 = 271;
    /// Frame `cross21`.
    pub const CROSS21: i32 = 272;
    /// Frame `cross22`.
    pub const CROSS22: i32 = 273;
    /// Frame `cross23`.
    pub const CROSS23: i32 = 274;
    /// Frame `cross24`.
    pub const CROSS24: i32 = 275;
    /// Frame `cross25`.
    pub const CROSS25: i32 = 276;
    /// Frame `cross26`.
    pub const CROSS26: i32 = 277;
    /// Frame `cross27`.
    pub const CROSS27: i32 = 278;
    /// Frame `cross28`.
    pub const CROSS28: i32 = 279;
    /// Frame `cross29`.
    pub const CROSS29: i32 = 280;
    /// Frame `cross30`.
    pub const CROSS30: i32 = 281;
}

/// `insaneMoves` move tables.
pub fn insane_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("insane_move_stand_normal", 59, 64, Some("insane_stand"), vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("insane_checkdown")], -1),
        ]),
        monster_move("insane_move_stand_insane", 64, 93, Some("insane_stand"), vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("insane_shake")], -1),
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
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("insane_checkdown")], -1),
        ]),
        monster_move("insane_move_uptodown", 0, 39, Some("insane_onground"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_moan")], -1),
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
            monster_frame(MonsterAi::Move, 2.7, vec![], -1),
            monster_frame(MonsterAi::Move, 4.1, vec![], -1),
            monster_frame(MonsterAi::Move, 6.0, vec![], -1),
            monster_frame(MonsterAi::Move, 7.6, vec![], -1),
            monster_frame(MonsterAi::Move, 3.6, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_fist")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_fist")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_downtoup", 40, 58, Some("insane_stand"), vec![
            monster_frame(MonsterAi::Move, -0.7, vec![], -1),
            monster_frame(MonsterAi::Move, -1.2, vec![], -1),
            monster_frame(MonsterAi::Move, -1.5, vec![], -1),
            monster_frame(MonsterAi::Move, -4.5, vec![], -1),
            monster_frame(MonsterAi::Move, -3.5, vec![], -1),
            monster_frame(MonsterAi::Move, -0.2, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.3, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.3, vec![], -1),
            monster_frame(MonsterAi::Move, -1.6, vec![], -1),
            monster_frame(MonsterAi::Move, -0.3, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_jumpdown", 95, 99, Some("insane_onground"), vec![
            monster_frame(MonsterAi::Move, 0.2, vec![], -1),
            monster_frame(MonsterAi::Move, 11.5, vec![], -1),
            monster_frame(MonsterAi::Move, 5.1, vec![], -1),
            monster_frame(MonsterAi::Move, 7.1, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_down", 99, 159, Some("insane_onground"), vec![
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
            monster_frame(MonsterAi::Move, -1.7, vec![], -1),
            monster_frame(MonsterAi::Move, -1.6, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_fist")], -1),
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
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_moan")], -1),
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
            monster_frame(MonsterAi::Move, 0.5, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -0.2, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.2, vec![], -1),
            monster_frame(MonsterAi::Move, 0.4, vec![], -1),
            monster_frame(MonsterAi::Move, 0.6, vec![], -1),
            monster_frame(MonsterAi::Move, 0.8, vec![], -1),
            monster_frame(MonsterAi::Move, 0.7, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_checkup")], -1),
        ]),
        monster_move("insane_move_walk_normal", 160, 172, Some("insane_walk"), vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Walk, 2.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.7, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.3, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.3, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_run_normal", 160, 172, Some("insane_run"), vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Walk, 2.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.7, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.3, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.3, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_walk_insane", 173, 198, Some("insane_walk"), vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Walk, 3.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.7, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.8, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.3, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.1, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.7, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.8, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.8, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_run_insane", 173, 198, Some("insane_run"), vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Walk, 3.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.7, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.8, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.3, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.1, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.7, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.8, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.8, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_stand_pain", 199, 209, Some("insane_run"), vec![
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
        monster_move("insane_move_stand_death", 210, 226, Some("insane_dead"), vec![
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
        monster_move("insane_move_crawl", 227, 235, None, vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Walk, 1.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.1, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.4, vec![], -1),
        ]),
        monster_move("insane_move_runcrawl", 227, 235, None, vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![MonsterAction::name("insane_scream")], -1),
            monster_frame(MonsterAi::Walk, 1.5, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.1, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.4, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.4, vec![], -1),
        ]),
        monster_move("insane_move_crawl_pain", 236, 244, Some("insane_run"), vec![
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
        monster_move("insane_move_crawl_death", 245, 251, Some("insane_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("insane_move_cross", 252, 266, Some("insane_cross"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_moan")], -1),
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
        monster_move("insane_move_struggle_cross", 267, 281, Some("insane_cross"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("insane_scream")], -1),
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
