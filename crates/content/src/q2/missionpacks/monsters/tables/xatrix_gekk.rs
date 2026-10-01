//! gekk move tables (`src/content/q2/missionpacks/monsters/tables/xatrix-gekk.ts`).
//!
//! Original Quake II xatrix/m_gekk.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `gekkFrame`.
pub mod gekk_frame {
    /// Frame `stand_01`.
    pub const STAND_01: i32 = 0;
    /// Frame `stand_02`.
    pub const STAND_02: i32 = 1;
    /// Frame `stand_03`.
    pub const STAND_03: i32 = 2;
    /// Frame `stand_04`.
    pub const STAND_04: i32 = 3;
    /// Frame `stand_05`.
    pub const STAND_05: i32 = 4;
    /// Frame `stand_06`.
    pub const STAND_06: i32 = 5;
    /// Frame `stand_07`.
    pub const STAND_07: i32 = 6;
    /// Frame `stand_08`.
    pub const STAND_08: i32 = 7;
    /// Frame `stand_09`.
    pub const STAND_09: i32 = 8;
    /// Frame `stand_10`.
    pub const STAND_10: i32 = 9;
    /// Frame `stand_11`.
    pub const STAND_11: i32 = 10;
    /// Frame `stand_12`.
    pub const STAND_12: i32 = 11;
    /// Frame `stand_13`.
    pub const STAND_13: i32 = 12;
    /// Frame `stand_14`.
    pub const STAND_14: i32 = 13;
    /// Frame `stand_15`.
    pub const STAND_15: i32 = 14;
    /// Frame `stand_16`.
    pub const STAND_16: i32 = 15;
    /// Frame `stand_17`.
    pub const STAND_17: i32 = 16;
    /// Frame `stand_18`.
    pub const STAND_18: i32 = 17;
    /// Frame `stand_19`.
    pub const STAND_19: i32 = 18;
    /// Frame `stand_20`.
    pub const STAND_20: i32 = 19;
    /// Frame `stand_21`.
    pub const STAND_21: i32 = 20;
    /// Frame `stand_22`.
    pub const STAND_22: i32 = 21;
    /// Frame `stand_23`.
    pub const STAND_23: i32 = 22;
    /// Frame `stand_24`.
    pub const STAND_24: i32 = 23;
    /// Frame `stand_25`.
    pub const STAND_25: i32 = 24;
    /// Frame `stand_26`.
    pub const STAND_26: i32 = 25;
    /// Frame `stand_27`.
    pub const STAND_27: i32 = 26;
    /// Frame `stand_28`.
    pub const STAND_28: i32 = 27;
    /// Frame `stand_29`.
    pub const STAND_29: i32 = 28;
    /// Frame `stand_30`.
    pub const STAND_30: i32 = 29;
    /// Frame `stand_31`.
    pub const STAND_31: i32 = 30;
    /// Frame `stand_32`.
    pub const STAND_32: i32 = 31;
    /// Frame `stand_33`.
    pub const STAND_33: i32 = 32;
    /// Frame `stand_34`.
    pub const STAND_34: i32 = 33;
    /// Frame `stand_35`.
    pub const STAND_35: i32 = 34;
    /// Frame `stand_36`.
    pub const STAND_36: i32 = 35;
    /// Frame `stand_37`.
    pub const STAND_37: i32 = 36;
    /// Frame `stand_38`.
    pub const STAND_38: i32 = 37;
    /// Frame `stand_39`.
    pub const STAND_39: i32 = 38;
    /// Frame `run_01`.
    pub const RUN_01: i32 = 39;
    /// Frame `run_02`.
    pub const RUN_02: i32 = 40;
    /// Frame `run_03`.
    pub const RUN_03: i32 = 41;
    /// Frame `run_04`.
    pub const RUN_04: i32 = 42;
    /// Frame `run_05`.
    pub const RUN_05: i32 = 43;
    /// Frame `run_06`.
    pub const RUN_06: i32 = 44;
    /// Frame `clawatk3_01`.
    pub const CLAWATK3_01: i32 = 45;
    /// Frame `clawatk3_02`.
    pub const CLAWATK3_02: i32 = 46;
    /// Frame `clawatk3_03`.
    pub const CLAWATK3_03: i32 = 47;
    /// Frame `clawatk3_04`.
    pub const CLAWATK3_04: i32 = 48;
    /// Frame `clawatk3_05`.
    pub const CLAWATK3_05: i32 = 49;
    /// Frame `clawatk3_06`.
    pub const CLAWATK3_06: i32 = 50;
    /// Frame `clawatk3_07`.
    pub const CLAWATK3_07: i32 = 51;
    /// Frame `clawatk3_08`.
    pub const CLAWATK3_08: i32 = 52;
    /// Frame `clawatk3_09`.
    pub const CLAWATK3_09: i32 = 53;
    /// Frame `clawatk4_01`.
    pub const CLAWATK4_01: i32 = 54;
    /// Frame `clawatk4_02`.
    pub const CLAWATK4_02: i32 = 55;
    /// Frame `clawatk4_03`.
    pub const CLAWATK4_03: i32 = 56;
    /// Frame `clawatk4_04`.
    pub const CLAWATK4_04: i32 = 57;
    /// Frame `clawatk4_05`.
    pub const CLAWATK4_05: i32 = 58;
    /// Frame `clawatk4_06`.
    pub const CLAWATK4_06: i32 = 59;
    /// Frame `clawatk4_07`.
    pub const CLAWATK4_07: i32 = 60;
    /// Frame `clawatk4_08`.
    pub const CLAWATK4_08: i32 = 61;
    /// Frame `clawatk5_01`.
    pub const CLAWATK5_01: i32 = 62;
    /// Frame `clawatk5_02`.
    pub const CLAWATK5_02: i32 = 63;
    /// Frame `clawatk5_03`.
    pub const CLAWATK5_03: i32 = 64;
    /// Frame `clawatk5_04`.
    pub const CLAWATK5_04: i32 = 65;
    /// Frame `clawatk5_05`.
    pub const CLAWATK5_05: i32 = 66;
    /// Frame `clawatk5_06`.
    pub const CLAWATK5_06: i32 = 67;
    /// Frame `clawatk5_07`.
    pub const CLAWATK5_07: i32 = 68;
    /// Frame `clawatk5_08`.
    pub const CLAWATK5_08: i32 = 69;
    /// Frame `clawatk5_09`.
    pub const CLAWATK5_09: i32 = 70;
    /// Frame `leapatk_01`.
    pub const LEAPATK_01: i32 = 71;
    /// Frame `leapatk_02`.
    pub const LEAPATK_02: i32 = 72;
    /// Frame `leapatk_03`.
    pub const LEAPATK_03: i32 = 73;
    /// Frame `leapatk_04`.
    pub const LEAPATK_04: i32 = 74;
    /// Frame `leapatk_05`.
    pub const LEAPATK_05: i32 = 75;
    /// Frame `leapatk_06`.
    pub const LEAPATK_06: i32 = 76;
    /// Frame `leapatk_07`.
    pub const LEAPATK_07: i32 = 77;
    /// Frame `leapatk_08`.
    pub const LEAPATK_08: i32 = 78;
    /// Frame `leapatk_09`.
    pub const LEAPATK_09: i32 = 79;
    /// Frame `leapatk_10`.
    pub const LEAPATK_10: i32 = 80;
    /// Frame `leapatk_11`.
    pub const LEAPATK_11: i32 = 81;
    /// Frame `leapatk_12`.
    pub const LEAPATK_12: i32 = 82;
    /// Frame `leapatk_13`.
    pub const LEAPATK_13: i32 = 83;
    /// Frame `leapatk_14`.
    pub const LEAPATK_14: i32 = 84;
    /// Frame `leapatk_15`.
    pub const LEAPATK_15: i32 = 85;
    /// Frame `leapatk_16`.
    pub const LEAPATK_16: i32 = 86;
    /// Frame `leapatk_17`.
    pub const LEAPATK_17: i32 = 87;
    /// Frame `leapatk_18`.
    pub const LEAPATK_18: i32 = 88;
    /// Frame `leapatk_19`.
    pub const LEAPATK_19: i32 = 89;
    /// Frame `pain3_01`.
    pub const PAIN3_01: i32 = 90;
    /// Frame `pain3_02`.
    pub const PAIN3_02: i32 = 91;
    /// Frame `pain3_03`.
    pub const PAIN3_03: i32 = 92;
    /// Frame `pain3_04`.
    pub const PAIN3_04: i32 = 93;
    /// Frame `pain3_05`.
    pub const PAIN3_05: i32 = 94;
    /// Frame `pain3_06`.
    pub const PAIN3_06: i32 = 95;
    /// Frame `pain3_07`.
    pub const PAIN3_07: i32 = 96;
    /// Frame `pain3_08`.
    pub const PAIN3_08: i32 = 97;
    /// Frame `pain3_09`.
    pub const PAIN3_09: i32 = 98;
    /// Frame `pain3_10`.
    pub const PAIN3_10: i32 = 99;
    /// Frame `pain3_11`.
    pub const PAIN3_11: i32 = 100;
    /// Frame `pain4_01`.
    pub const PAIN4_01: i32 = 101;
    /// Frame `pain4_02`.
    pub const PAIN4_02: i32 = 102;
    /// Frame `pain4_03`.
    pub const PAIN4_03: i32 = 103;
    /// Frame `pain4_04`.
    pub const PAIN4_04: i32 = 104;
    /// Frame `pain4_05`.
    pub const PAIN4_05: i32 = 105;
    /// Frame `pain4_06`.
    pub const PAIN4_06: i32 = 106;
    /// Frame `pain4_07`.
    pub const PAIN4_07: i32 = 107;
    /// Frame `pain4_08`.
    pub const PAIN4_08: i32 = 108;
    /// Frame `pain4_09`.
    pub const PAIN4_09: i32 = 109;
    /// Frame `pain4_10`.
    pub const PAIN4_10: i32 = 110;
    /// Frame `pain4_11`.
    pub const PAIN4_11: i32 = 111;
    /// Frame `pain4_12`.
    pub const PAIN4_12: i32 = 112;
    /// Frame `pain4_13`.
    pub const PAIN4_13: i32 = 113;
    /// Frame `death1_01`.
    pub const DEATH1_01: i32 = 114;
    /// Frame `death1_02`.
    pub const DEATH1_02: i32 = 115;
    /// Frame `death1_03`.
    pub const DEATH1_03: i32 = 116;
    /// Frame `death1_04`.
    pub const DEATH1_04: i32 = 117;
    /// Frame `death1_05`.
    pub const DEATH1_05: i32 = 118;
    /// Frame `death1_06`.
    pub const DEATH1_06: i32 = 119;
    /// Frame `death1_07`.
    pub const DEATH1_07: i32 = 120;
    /// Frame `death1_08`.
    pub const DEATH1_08: i32 = 121;
    /// Frame `death1_09`.
    pub const DEATH1_09: i32 = 122;
    /// Frame `death1_10`.
    pub const DEATH1_10: i32 = 123;
    /// Frame `death2_01`.
    pub const DEATH2_01: i32 = 124;
    /// Frame `death2_02`.
    pub const DEATH2_02: i32 = 125;
    /// Frame `death2_03`.
    pub const DEATH2_03: i32 = 126;
    /// Frame `death2_04`.
    pub const DEATH2_04: i32 = 127;
    /// Frame `death2_05`.
    pub const DEATH2_05: i32 = 128;
    /// Frame `death2_06`.
    pub const DEATH2_06: i32 = 129;
    /// Frame `death2_07`.
    pub const DEATH2_07: i32 = 130;
    /// Frame `death2_08`.
    pub const DEATH2_08: i32 = 131;
    /// Frame `death2_09`.
    pub const DEATH2_09: i32 = 132;
    /// Frame `death2_10`.
    pub const DEATH2_10: i32 = 133;
    /// Frame `death2_11`.
    pub const DEATH2_11: i32 = 134;
    /// Frame `death3_01`.
    pub const DEATH3_01: i32 = 135;
    /// Frame `death3_02`.
    pub const DEATH3_02: i32 = 136;
    /// Frame `death3_03`.
    pub const DEATH3_03: i32 = 137;
    /// Frame `death3_04`.
    pub const DEATH3_04: i32 = 138;
    /// Frame `death3_05`.
    pub const DEATH3_05: i32 = 139;
    /// Frame `death3_06`.
    pub const DEATH3_06: i32 = 140;
    /// Frame `death3_07`.
    pub const DEATH3_07: i32 = 141;
    /// Frame `death4_01`.
    pub const DEATH4_01: i32 = 142;
    /// Frame `death4_02`.
    pub const DEATH4_02: i32 = 143;
    /// Frame `death4_03`.
    pub const DEATH4_03: i32 = 144;
    /// Frame `death4_04`.
    pub const DEATH4_04: i32 = 145;
    /// Frame `death4_05`.
    pub const DEATH4_05: i32 = 146;
    /// Frame `death4_06`.
    pub const DEATH4_06: i32 = 147;
    /// Frame `death4_07`.
    pub const DEATH4_07: i32 = 148;
    /// Frame `death4_08`.
    pub const DEATH4_08: i32 = 149;
    /// Frame `death4_09`.
    pub const DEATH4_09: i32 = 150;
    /// Frame `death4_10`.
    pub const DEATH4_10: i32 = 151;
    /// Frame `death4_11`.
    pub const DEATH4_11: i32 = 152;
    /// Frame `death4_12`.
    pub const DEATH4_12: i32 = 153;
    /// Frame `death4_13`.
    pub const DEATH4_13: i32 = 154;
    /// Frame `death4_14`.
    pub const DEATH4_14: i32 = 155;
    /// Frame `death4_15`.
    pub const DEATH4_15: i32 = 156;
    /// Frame `death4_16`.
    pub const DEATH4_16: i32 = 157;
    /// Frame `death4_17`.
    pub const DEATH4_17: i32 = 158;
    /// Frame `death4_18`.
    pub const DEATH4_18: i32 = 159;
    /// Frame `death4_19`.
    pub const DEATH4_19: i32 = 160;
    /// Frame `death4_20`.
    pub const DEATH4_20: i32 = 161;
    /// Frame `death4_21`.
    pub const DEATH4_21: i32 = 162;
    /// Frame `death4_22`.
    pub const DEATH4_22: i32 = 163;
    /// Frame `death4_23`.
    pub const DEATH4_23: i32 = 164;
    /// Frame `death4_24`.
    pub const DEATH4_24: i32 = 165;
    /// Frame `death4_25`.
    pub const DEATH4_25: i32 = 166;
    /// Frame `death4_26`.
    pub const DEATH4_26: i32 = 167;
    /// Frame `death4_27`.
    pub const DEATH4_27: i32 = 168;
    /// Frame `death4_28`.
    pub const DEATH4_28: i32 = 169;
    /// Frame `death4_29`.
    pub const DEATH4_29: i32 = 170;
    /// Frame `death4_30`.
    pub const DEATH4_30: i32 = 171;
    /// Frame `death4_31`.
    pub const DEATH4_31: i32 = 172;
    /// Frame `death4_32`.
    pub const DEATH4_32: i32 = 173;
    /// Frame `death4_33`.
    pub const DEATH4_33: i32 = 174;
    /// Frame `death4_34`.
    pub const DEATH4_34: i32 = 175;
    /// Frame `death4_35`.
    pub const DEATH4_35: i32 = 176;
    /// Frame `rduck_01`.
    pub const RDUCK_01: i32 = 177;
    /// Frame `rduck_02`.
    pub const RDUCK_02: i32 = 178;
    /// Frame `rduck_03`.
    pub const RDUCK_03: i32 = 179;
    /// Frame `rduck_04`.
    pub const RDUCK_04: i32 = 180;
    /// Frame `rduck_05`.
    pub const RDUCK_05: i32 = 181;
    /// Frame `rduck_06`.
    pub const RDUCK_06: i32 = 182;
    /// Frame `rduck_07`.
    pub const RDUCK_07: i32 = 183;
    /// Frame `rduck_08`.
    pub const RDUCK_08: i32 = 184;
    /// Frame `rduck_09`.
    pub const RDUCK_09: i32 = 185;
    /// Frame `rduck_10`.
    pub const RDUCK_10: i32 = 186;
    /// Frame `rduck_11`.
    pub const RDUCK_11: i32 = 187;
    /// Frame `rduck_12`.
    pub const RDUCK_12: i32 = 188;
    /// Frame `rduck_13`.
    pub const RDUCK_13: i32 = 189;
    /// Frame `lduck_01`.
    pub const LDUCK_01: i32 = 190;
    /// Frame `lduck_02`.
    pub const LDUCK_02: i32 = 191;
    /// Frame `lduck_03`.
    pub const LDUCK_03: i32 = 192;
    /// Frame `lduck_04`.
    pub const LDUCK_04: i32 = 193;
    /// Frame `lduck_05`.
    pub const LDUCK_05: i32 = 194;
    /// Frame `lduck_06`.
    pub const LDUCK_06: i32 = 195;
    /// Frame `lduck_07`.
    pub const LDUCK_07: i32 = 196;
    /// Frame `lduck_08`.
    pub const LDUCK_08: i32 = 197;
    /// Frame `lduck_09`.
    pub const LDUCK_09: i32 = 198;
    /// Frame `lduck_10`.
    pub const LDUCK_10: i32 = 199;
    /// Frame `lduck_11`.
    pub const LDUCK_11: i32 = 200;
    /// Frame `lduck_12`.
    pub const LDUCK_12: i32 = 201;
    /// Frame `lduck_13`.
    pub const LDUCK_13: i32 = 202;
    /// Frame `idle_01`.
    pub const IDLE_01: i32 = 203;
    /// Frame `idle_02`.
    pub const IDLE_02: i32 = 204;
    /// Frame `idle_03`.
    pub const IDLE_03: i32 = 205;
    /// Frame `idle_04`.
    pub const IDLE_04: i32 = 206;
    /// Frame `idle_05`.
    pub const IDLE_05: i32 = 207;
    /// Frame `idle_06`.
    pub const IDLE_06: i32 = 208;
    /// Frame `idle_07`.
    pub const IDLE_07: i32 = 209;
    /// Frame `idle_08`.
    pub const IDLE_08: i32 = 210;
    /// Frame `idle_09`.
    pub const IDLE_09: i32 = 211;
    /// Frame `idle_10`.
    pub const IDLE_10: i32 = 212;
    /// Frame `idle_11`.
    pub const IDLE_11: i32 = 213;
    /// Frame `idle_12`.
    pub const IDLE_12: i32 = 214;
    /// Frame `idle_13`.
    pub const IDLE_13: i32 = 215;
    /// Frame `idle_14`.
    pub const IDLE_14: i32 = 216;
    /// Frame `idle_15`.
    pub const IDLE_15: i32 = 217;
    /// Frame `idle_16`.
    pub const IDLE_16: i32 = 218;
    /// Frame `idle_17`.
    pub const IDLE_17: i32 = 219;
    /// Frame `idle_18`.
    pub const IDLE_18: i32 = 220;
    /// Frame `idle_19`.
    pub const IDLE_19: i32 = 221;
    /// Frame `idle_20`.
    pub const IDLE_20: i32 = 222;
    /// Frame `idle_21`.
    pub const IDLE_21: i32 = 223;
    /// Frame `idle_22`.
    pub const IDLE_22: i32 = 224;
    /// Frame `idle_23`.
    pub const IDLE_23: i32 = 225;
    /// Frame `idle_24`.
    pub const IDLE_24: i32 = 226;
    /// Frame `idle_25`.
    pub const IDLE_25: i32 = 227;
    /// Frame `idle_26`.
    pub const IDLE_26: i32 = 228;
    /// Frame `idle_27`.
    pub const IDLE_27: i32 = 229;
    /// Frame `idle_28`.
    pub const IDLE_28: i32 = 230;
    /// Frame `idle_29`.
    pub const IDLE_29: i32 = 231;
    /// Frame `idle_30`.
    pub const IDLE_30: i32 = 232;
    /// Frame `idle_31`.
    pub const IDLE_31: i32 = 233;
    /// Frame `idle_32`.
    pub const IDLE_32: i32 = 234;
    /// Frame `spit_01`.
    pub const SPIT_01: i32 = 235;
    /// Frame `spit_02`.
    pub const SPIT_02: i32 = 236;
    /// Frame `spit_03`.
    pub const SPIT_03: i32 = 237;
    /// Frame `spit_04`.
    pub const SPIT_04: i32 = 238;
    /// Frame `spit_05`.
    pub const SPIT_05: i32 = 239;
    /// Frame `spit_06`.
    pub const SPIT_06: i32 = 240;
    /// Frame `spit_07`.
    pub const SPIT_07: i32 = 241;
    /// Frame `amb_01`.
    pub const AMB_01: i32 = 242;
    /// Frame `amb_02`.
    pub const AMB_02: i32 = 243;
    /// Frame `amb_03`.
    pub const AMB_03: i32 = 244;
    /// Frame `amb_04`.
    pub const AMB_04: i32 = 245;
    /// Frame `wdeath_01`.
    pub const WDEATH_01: i32 = 246;
    /// Frame `wdeath_02`.
    pub const WDEATH_02: i32 = 247;
    /// Frame `wdeath_03`.
    pub const WDEATH_03: i32 = 248;
    /// Frame `wdeath_04`.
    pub const WDEATH_04: i32 = 249;
    /// Frame `wdeath_05`.
    pub const WDEATH_05: i32 = 250;
    /// Frame `wdeath_06`.
    pub const WDEATH_06: i32 = 251;
    /// Frame `wdeath_07`.
    pub const WDEATH_07: i32 = 252;
    /// Frame `wdeath_08`.
    pub const WDEATH_08: i32 = 253;
    /// Frame `wdeath_09`.
    pub const WDEATH_09: i32 = 254;
    /// Frame `wdeath_10`.
    pub const WDEATH_10: i32 = 255;
    /// Frame `wdeath_11`.
    pub const WDEATH_11: i32 = 256;
    /// Frame `wdeath_12`.
    pub const WDEATH_12: i32 = 257;
    /// Frame `wdeath_13`.
    pub const WDEATH_13: i32 = 258;
    /// Frame `wdeath_14`.
    pub const WDEATH_14: i32 = 259;
    /// Frame `wdeath_15`.
    pub const WDEATH_15: i32 = 260;
    /// Frame `wdeath_16`.
    pub const WDEATH_16: i32 = 261;
    /// Frame `wdeath_17`.
    pub const WDEATH_17: i32 = 262;
    /// Frame `wdeath_18`.
    pub const WDEATH_18: i32 = 263;
    /// Frame `wdeath_19`.
    pub const WDEATH_19: i32 = 264;
    /// Frame `wdeath_20`.
    pub const WDEATH_20: i32 = 265;
    /// Frame `wdeath_21`.
    pub const WDEATH_21: i32 = 266;
    /// Frame `wdeath_22`.
    pub const WDEATH_22: i32 = 267;
    /// Frame `wdeath_23`.
    pub const WDEATH_23: i32 = 268;
    /// Frame `wdeath_24`.
    pub const WDEATH_24: i32 = 269;
    /// Frame `wdeath_25`.
    pub const WDEATH_25: i32 = 270;
    /// Frame `wdeath_26`.
    pub const WDEATH_26: i32 = 271;
    /// Frame `wdeath_27`.
    pub const WDEATH_27: i32 = 272;
    /// Frame `wdeath_28`.
    pub const WDEATH_28: i32 = 273;
    /// Frame `wdeath_29`.
    pub const WDEATH_29: i32 = 274;
    /// Frame `wdeath_30`.
    pub const WDEATH_30: i32 = 275;
    /// Frame `wdeath_31`.
    pub const WDEATH_31: i32 = 276;
    /// Frame `wdeath_32`.
    pub const WDEATH_32: i32 = 277;
    /// Frame `wdeath_33`.
    pub const WDEATH_33: i32 = 278;
    /// Frame `wdeath_34`.
    pub const WDEATH_34: i32 = 279;
    /// Frame `wdeath_35`.
    pub const WDEATH_35: i32 = 280;
    /// Frame `wdeath_36`.
    pub const WDEATH_36: i32 = 281;
    /// Frame `wdeath_37`.
    pub const WDEATH_37: i32 = 282;
    /// Frame `wdeath_38`.
    pub const WDEATH_38: i32 = 283;
    /// Frame `wdeath_39`.
    pub const WDEATH_39: i32 = 284;
    /// Frame `wdeath_40`.
    pub const WDEATH_40: i32 = 285;
    /// Frame `wdeath_41`.
    pub const WDEATH_41: i32 = 286;
    /// Frame `wdeath_42`.
    pub const WDEATH_42: i32 = 287;
    /// Frame `wdeath_43`.
    pub const WDEATH_43: i32 = 288;
    /// Frame `wdeath_44`.
    pub const WDEATH_44: i32 = 289;
    /// Frame `wdeath_45`.
    pub const WDEATH_45: i32 = 290;
    /// Frame `swim_01`.
    pub const SWIM_01: i32 = 291;
    /// Frame `swim_02`.
    pub const SWIM_02: i32 = 292;
    /// Frame `swim_03`.
    pub const SWIM_03: i32 = 293;
    /// Frame `swim_04`.
    pub const SWIM_04: i32 = 294;
    /// Frame `swim_05`.
    pub const SWIM_05: i32 = 295;
    /// Frame `swim_06`.
    pub const SWIM_06: i32 = 296;
    /// Frame `swim_07`.
    pub const SWIM_07: i32 = 297;
    /// Frame `swim_08`.
    pub const SWIM_08: i32 = 298;
    /// Frame `swim_09`.
    pub const SWIM_09: i32 = 299;
    /// Frame `swim_10`.
    pub const SWIM_10: i32 = 300;
    /// Frame `swim_11`.
    pub const SWIM_11: i32 = 301;
    /// Frame `swim_12`.
    pub const SWIM_12: i32 = 302;
    /// Frame `swim_13`.
    pub const SWIM_13: i32 = 303;
    /// Frame `swim_14`.
    pub const SWIM_14: i32 = 304;
    /// Frame `swim_15`.
    pub const SWIM_15: i32 = 305;
    /// Frame `swim_16`.
    pub const SWIM_16: i32 = 306;
    /// Frame `swim_17`.
    pub const SWIM_17: i32 = 307;
    /// Frame `swim_18`.
    pub const SWIM_18: i32 = 308;
    /// Frame `swim_19`.
    pub const SWIM_19: i32 = 309;
    /// Frame `swim_20`.
    pub const SWIM_20: i32 = 310;
    /// Frame `swim_21`.
    pub const SWIM_21: i32 = 311;
    /// Frame `swim_22`.
    pub const SWIM_22: i32 = 312;
    /// Frame `swim_23`.
    pub const SWIM_23: i32 = 313;
    /// Frame `swim_24`.
    pub const SWIM_24: i32 = 314;
    /// Frame `swim_25`.
    pub const SWIM_25: i32 = 315;
    /// Frame `swim_26`.
    pub const SWIM_26: i32 = 316;
    /// Frame `swim_27`.
    pub const SWIM_27: i32 = 317;
    /// Frame `swim_28`.
    pub const SWIM_28: i32 = 318;
    /// Frame `swim_29`.
    pub const SWIM_29: i32 = 319;
    /// Frame `swim_30`.
    pub const SWIM_30: i32 = 320;
    /// Frame `swim_31`.
    pub const SWIM_31: i32 = 321;
    /// Frame `swim_32`.
    pub const SWIM_32: i32 = 322;
    /// Frame `attack_01`.
    pub const ATTACK_01: i32 = 323;
    /// Frame `attack_02`.
    pub const ATTACK_02: i32 = 324;
    /// Frame `attack_03`.
    pub const ATTACK_03: i32 = 325;
    /// Frame `attack_04`.
    pub const ATTACK_04: i32 = 326;
    /// Frame `attack_05`.
    pub const ATTACK_05: i32 = 327;
    /// Frame `attack_06`.
    pub const ATTACK_06: i32 = 328;
    /// Frame `attack_07`.
    pub const ATTACK_07: i32 = 329;
    /// Frame `attack_08`.
    pub const ATTACK_08: i32 = 330;
    /// Frame `attack_09`.
    pub const ATTACK_09: i32 = 331;
    /// Frame `attack_10`.
    pub const ATTACK_10: i32 = 332;
    /// Frame `attack_11`.
    pub const ATTACK_11: i32 = 333;
    /// Frame `attack_12`.
    pub const ATTACK_12: i32 = 334;
    /// Frame `attack_13`.
    pub const ATTACK_13: i32 = 335;
    /// Frame `attack_14`.
    pub const ATTACK_14: i32 = 336;
    /// Frame `attack_15`.
    pub const ATTACK_15: i32 = 337;
    /// Frame `attack_16`.
    pub const ATTACK_16: i32 = 338;
    /// Frame `attack_17`.
    pub const ATTACK_17: i32 = 339;
    /// Frame `attack_18`.
    pub const ATTACK_18: i32 = 340;
    /// Frame `attack_19`.
    pub const ATTACK_19: i32 = 341;
    /// Frame `attack_20`.
    pub const ATTACK_20: i32 = 342;
    /// Frame `attack_21`.
    pub const ATTACK_21: i32 = 343;
    /// Frame `pain_01`.
    pub const PAIN_01: i32 = 344;
    /// Frame `pain_02`.
    pub const PAIN_02: i32 = 345;
    /// Frame `pain_03`.
    pub const PAIN_03: i32 = 346;
    /// Frame `pain_04`.
    pub const PAIN_04: i32 = 347;
    /// Frame `pain_05`.
    pub const PAIN_05: i32 = 348;
    /// Frame `pain_06`.
    pub const PAIN_06: i32 = 349;
}

/// `gekkMoves` move tables.
pub fn gekk_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "gekk_move_stand",
            0,
            38,
            None,
            vec![
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Source("ai_stand2".to_string()),
                    0.0,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_standunderwater",
            242,
            245,
            None,
            vec![
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Source("ai_stand2".to_string()),
                    0.0,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_swim_loop",
            242,
            245,
            Some("gekk_swim_loop"),
            vec![
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![MonsterAction::name("gekk_swim")], -1),
            ],
        ),
        monster_move(
            "gekk_move_swim_start",
            291,
            322,
            Some("gekk_swim_loop"),
            vec![
                monster_frame(MonsterAi::Run, 14.0, vec![], -1),
                monster_frame(MonsterAi::Run, 14.0, vec![], -1),
                monster_frame(MonsterAi::Run, 14.0, vec![], -1),
                monster_frame(MonsterAi::Run, 14.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![MonsterAction::name("gekk_hit_left")], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
                monster_frame(MonsterAi::Run, 20.0, vec![], -1),
                monster_frame(MonsterAi::Run, 20.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![MonsterAction::name("gekk_hit_right")], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 26.0, vec![], -1),
                monster_frame(MonsterAi::Run, 26.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![MonsterAction::name("gekk_bite")], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_idle",
            203,
            234,
            Some("gekk_stand"),
            vec![
                monster_frame(
                    MonsterAi::Source("ai_stand2".to_string()),
                    0.0,
                    vec![MonsterAction::name("gekk_search")],
                    -1,
                ),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Source("ai_stand2".to_string()),
                    0.0,
                    vec![MonsterAction::name("gekk_idle_loop")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_idle2",
            203,
            234,
            Some("gekk_face"),
            vec![
                monster_frame(
                    MonsterAi::Source("ai_stand2".to_string()),
                    0.0,
                    vec![MonsterAction::name("gekk_search")],
                    -1,
                ),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_stand2".to_string()), 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Source("ai_stand2".to_string()),
                    0.0,
                    vec![MonsterAction::name("gekk_idle_loop")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_chant",
            203,
            234,
            Some("gekk_chant"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("gekk_search")], -1),
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
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("gekk_idle_loop")], -1),
            ],
        ),
        monster_move(
            "gekk_move_walk",
            39,
            44,
            None,
            vec![
                monster_frame(
                    MonsterAi::Walk,
                    3.849,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
                monster_frame(MonsterAi::Walk, 19.606, vec![], -1),
                monster_frame(MonsterAi::Walk, 25.583, vec![], -1),
                monster_frame(MonsterAi::Walk, 34.625, vec![MonsterAction::name("gekk_step")], -1),
                monster_frame(MonsterAi::Walk, 27.365, vec![], -1),
                monster_frame(MonsterAi::Walk, 28.48, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_run",
            39,
            44,
            None,
            vec![
                monster_frame(
                    MonsterAi::Run,
                    3.849,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, 19.606, vec![], -1),
                monster_frame(MonsterAi::Run, 25.583, vec![], -1),
                monster_frame(MonsterAi::Run, 34.625, vec![MonsterAction::name("gekk_step")], -1),
                monster_frame(MonsterAi::Run, 27.365, vec![], -1),
                monster_frame(MonsterAi::Run, 28.48, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_run_start",
            0,
            1,
            Some("gekk_run"),
            vec![
                monster_frame(MonsterAi::Run, 0.212, vec![], -1),
                monster_frame(MonsterAi::Run, 19.753, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_spit",
            235,
            241,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("loogie")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("reloogie")], -1),
            ],
        ),
        monster_move(
            "gekk_move_attack1",
            45,
            53,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("gekk_hit_left")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("gekk_check_refire")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_attack2",
            62,
            70,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("gekk_hit_left")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("gekk_hit_right")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("gekk_check_refire")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_leapatk",
            71,
            89,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -0.387, vec![], -1),
                monster_frame(MonsterAi::Charge, -1.113, vec![], -1),
                monster_frame(MonsterAi::Charge, -0.237, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    6.72,
                    vec![MonsterAction::name("gekk_jump_takeoff")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 6.414, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.163, vec![], -1),
                monster_frame(MonsterAi::Charge, 28.316, vec![], -1),
                monster_frame(MonsterAi::Charge, 24.198, vec![], -1),
                monster_frame(MonsterAi::Charge, 31.742, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    35.977,
                    vec![MonsterAction::name("gekk_check_landing")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    12.303,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    20.122,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -1.042,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    2.556,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.544,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    1.862,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    1.224,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -0.457,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_leapatk2",
            71,
            89,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -0.387, vec![], -1),
                monster_frame(MonsterAi::Charge, -1.113, vec![], -1),
                monster_frame(MonsterAi::Charge, -0.237, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    6.72,
                    vec![MonsterAction::name("gekk_jump_takeoff2")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 6.414, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.163, vec![], -1),
                monster_frame(MonsterAi::Charge, 28.316, vec![], -1),
                monster_frame(MonsterAi::Charge, 24.198, vec![], -1),
                monster_frame(MonsterAi::Charge, 31.742, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    35.977,
                    vec![MonsterAction::name("gekk_check_landing")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    12.303,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    20.122,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -1.042,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    2.556,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.544,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    1.862,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    1.224,
                    vec![MonsterAction::name("gekk_stop_skid")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -0.457,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_attack",
            323,
            343,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Charge, 16.0, vec![MonsterAction::name("gekk_preattack")], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![MonsterAction::name("gekk_bite")], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![MonsterAction::name("gekk_bite")], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![MonsterAction::name("gekk_hit_left")], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![MonsterAction::name("gekk_hit_right")], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 16.0, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_pain",
            344,
            349,
            Some("gekk_run_start"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_pain1",
            90,
            100,
            Some("gekk_run_start"),
            vec![
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
                monster_frame(
                    MonsterAi::Move,
                    0.0,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_pain2",
            101,
            113,
            Some("gekk_run_start"),
            vec![
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
                monster_frame(
                    MonsterAi::Move,
                    0.0,
                    vec![MonsterAction::name("gekk_check_underwater")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "gekk_move_death1",
            114,
            123,
            Some("gekk_dead"),
            vec![
                monster_frame(MonsterAi::Move, -5.151, vec![], -1),
                monster_frame(MonsterAi::Move, -12.223, vec![], -1),
                monster_frame(MonsterAi::Move, -11.484, vec![], -1),
                monster_frame(MonsterAi::Move, -17.952, vec![], -1),
                monster_frame(MonsterAi::Move, -6.953, vec![], -1),
                monster_frame(MonsterAi::Move, -7.393, vec![], -1),
                monster_frame(MonsterAi::Move, -10.713, vec![], -1),
                monster_frame(MonsterAi::Move, -17.464, vec![], -1),
                monster_frame(MonsterAi::Move, -11.678, vec![], -1),
                monster_frame(MonsterAi::Move, -11.678, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_death3",
            135,
            141,
            Some("gekk_dead"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.022, vec![], -1),
                monster_frame(MonsterAi::Move, 0.169, vec![], -1),
                monster_frame(MonsterAi::Move, -0.71, vec![], -1),
                monster_frame(MonsterAi::Move, -13.446, vec![], -1),
                monster_frame(MonsterAi::Move, -7.654, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -31.951, vec![], -1),
            ],
        ),
        monster_move(
            "gekk_move_death4",
            142,
            176,
            Some("gekk_dead"),
            vec![
                monster_frame(MonsterAi::Move, 5.103, vec![], -1),
                monster_frame(MonsterAi::Move, -4.808, vec![], -1),
                monster_frame(MonsterAi::Move, -10.509, vec![], -1),
                monster_frame(MonsterAi::Move, -9.899, vec![], -1),
                monster_frame(MonsterAi::Move, 4.033, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -5.197, vec![], -1),
                monster_frame(MonsterAi::Move, -0.919, vec![], -1),
                monster_frame(MonsterAi::Move, -8.821, vec![], -1),
                monster_frame(MonsterAi::Move, -5.626, vec![], -1),
                monster_frame(MonsterAi::Move, -8.865, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -0.845, vec![], -1),
                monster_frame(MonsterAi::Move, 1.986, vec![], -1),
                monster_frame(MonsterAi::Move, 0.17, vec![], -1),
                monster_frame(MonsterAi::Move, 1.339, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -0.922, vec![], -1),
                monster_frame(MonsterAi::Move, 0.818, vec![], -1),
                monster_frame(MonsterAi::Move, -1.288, vec![], -1),
                monster_frame(MonsterAi::Move, -1.408, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -7.787, vec![], -1),
                monster_frame(MonsterAi::Move, -3.995, vec![], -1),
                monster_frame(MonsterAi::Move, -4.604, vec![], -1),
                monster_frame(MonsterAi::Move, -1.715, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -0.564, vec![], -1),
                monster_frame(MonsterAi::Move, -0.597, vec![], -1),
                monster_frame(MonsterAi::Move, 0.074, vec![], -1),
                monster_frame(MonsterAi::Move, -0.309, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -0.395, vec![], -1),
                monster_frame(MonsterAi::Move, -0.501, vec![], -1),
                monster_frame(MonsterAi::Move, -0.325, vec![], -1),
                monster_frame(MonsterAi::Move, -0.931, vec![MonsterAction::name("isgibfest")], -1),
                monster_frame(MonsterAi::Move, -1.433, vec![], -1),
                monster_frame(MonsterAi::Move, -1.626, vec![], -1),
                monster_frame(MonsterAi::Move, 4.68, vec![], -1),
                monster_frame(MonsterAi::Move, 0.56, vec![], -1),
                monster_frame(MonsterAi::Move, -0.549, vec![MonsterAction::name("gekk_gibfest")], -1),
            ],
        ),
        monster_move(
            "gekk_move_wdeath",
            246,
            290,
            Some("gekk_dead"),
            vec![
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
            ],
        ),
        monster_move(
            "gekk_move_lduck",
            190,
            202,
            Some("gekk_run_start"),
            vec![
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
            ],
        ),
        monster_move(
            "gekk_move_rduck",
            177,
            189,
            Some("gekk_run_start"),
            vec![
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
            ],
        ),
    ]
}
