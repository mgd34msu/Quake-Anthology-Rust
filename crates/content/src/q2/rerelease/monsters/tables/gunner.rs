//! gunner move tables (`src/content/q2/rerelease/monsters/tables/gunner.ts`).

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `gunnerFrame`.
pub mod gunner_frame {
    /// Frame `stand01`.
    pub const STAND01: i32 = 0;
    /// Frame `stand02`.
    pub const STAND02: i32 = 1;
    /// Frame `stand03`.
    pub const STAND03: i32 = 2;
    /// Frame `stand04`.
    pub const STAND04: i32 = 3;
    /// Frame `stand05`.
    pub const STAND05: i32 = 4;
    /// Frame `stand06`.
    pub const STAND06: i32 = 5;
    /// Frame `stand07`.
    pub const STAND07: i32 = 6;
    /// Frame `stand08`.
    pub const STAND08: i32 = 7;
    /// Frame `stand09`.
    pub const STAND09: i32 = 8;
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
    /// Frame `walk01`.
    pub const WALK01: i32 = 70;
    /// Frame `walk02`.
    pub const WALK02: i32 = 71;
    /// Frame `walk03`.
    pub const WALK03: i32 = 72;
    /// Frame `walk04`.
    pub const WALK04: i32 = 73;
    /// Frame `walk05`.
    pub const WALK05: i32 = 74;
    /// Frame `walk06`.
    pub const WALK06: i32 = 75;
    /// Frame `walk07`.
    pub const WALK07: i32 = 76;
    /// Frame `walk08`.
    pub const WALK08: i32 = 77;
    /// Frame `walk09`.
    pub const WALK09: i32 = 78;
    /// Frame `walk10`.
    pub const WALK10: i32 = 79;
    /// Frame `walk11`.
    pub const WALK11: i32 = 80;
    /// Frame `walk12`.
    pub const WALK12: i32 = 81;
    /// Frame `walk13`.
    pub const WALK13: i32 = 82;
    /// Frame `walk14`.
    pub const WALK14: i32 = 83;
    /// Frame `walk15`.
    pub const WALK15: i32 = 84;
    /// Frame `walk16`.
    pub const WALK16: i32 = 85;
    /// Frame `walk17`.
    pub const WALK17: i32 = 86;
    /// Frame `walk18`.
    pub const WALK18: i32 = 87;
    /// Frame `walk19`.
    pub const WALK19: i32 = 88;
    /// Frame `walk20`.
    pub const WALK20: i32 = 89;
    /// Frame `walk21`.
    pub const WALK21: i32 = 90;
    /// Frame `walk22`.
    pub const WALK22: i32 = 91;
    /// Frame `walk23`.
    pub const WALK23: i32 = 92;
    /// Frame `walk24`.
    pub const WALK24: i32 = 93;
    /// Frame `run01`.
    pub const RUN01: i32 = 94;
    /// Frame `run02`.
    pub const RUN02: i32 = 95;
    /// Frame `run03`.
    pub const RUN03: i32 = 96;
    /// Frame `run04`.
    pub const RUN04: i32 = 97;
    /// Frame `run05`.
    pub const RUN05: i32 = 98;
    /// Frame `run06`.
    pub const RUN06: i32 = 99;
    /// Frame `run07`.
    pub const RUN07: i32 = 100;
    /// Frame `run08`.
    pub const RUN08: i32 = 101;
    /// Frame `runs01`.
    pub const RUNS01: i32 = 102;
    /// Frame `runs02`.
    pub const RUNS02: i32 = 103;
    /// Frame `runs03`.
    pub const RUNS03: i32 = 104;
    /// Frame `runs04`.
    pub const RUNS04: i32 = 105;
    /// Frame `runs05`.
    pub const RUNS05: i32 = 106;
    /// Frame `runs06`.
    pub const RUNS06: i32 = 107;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 108;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 109;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 110;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 111;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 112;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 113;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 114;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 115;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 116;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 117;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 118;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 119;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 120;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 121;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 122;
    /// Frame `attak116`.
    pub const ATTAK116: i32 = 123;
    /// Frame `attak117`.
    pub const ATTAK117: i32 = 124;
    /// Frame `attak118`.
    pub const ATTAK118: i32 = 125;
    /// Frame `attak119`.
    pub const ATTAK119: i32 = 126;
    /// Frame `attak120`.
    pub const ATTAK120: i32 = 127;
    /// Frame `attak121`.
    pub const ATTAK121: i32 = 128;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 129;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 130;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 131;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 132;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 133;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 134;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 135;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 136;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 137;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 138;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 139;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 140;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 141;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 142;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 143;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 144;
    /// Frame `attak217`.
    pub const ATTAK217: i32 = 145;
    /// Frame `attak218`.
    pub const ATTAK218: i32 = 146;
    /// Frame `attak219`.
    pub const ATTAK219: i32 = 147;
    /// Frame `attak220`.
    pub const ATTAK220: i32 = 148;
    /// Frame `attak221`.
    pub const ATTAK221: i32 = 149;
    /// Frame `attak222`.
    pub const ATTAK222: i32 = 150;
    /// Frame `attak223`.
    pub const ATTAK223: i32 = 151;
    /// Frame `attak224`.
    pub const ATTAK224: i32 = 152;
    /// Frame `attak225`.
    pub const ATTAK225: i32 = 153;
    /// Frame `attak226`.
    pub const ATTAK226: i32 = 154;
    /// Frame `attak227`.
    pub const ATTAK227: i32 = 155;
    /// Frame `attak228`.
    pub const ATTAK228: i32 = 156;
    /// Frame `attak229`.
    pub const ATTAK229: i32 = 157;
    /// Frame `attak230`.
    pub const ATTAK230: i32 = 158;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 159;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 160;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 161;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 162;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 163;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 164;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 165;
    /// Frame `pain108`.
    pub const PAIN108: i32 = 166;
    /// Frame `pain109`.
    pub const PAIN109: i32 = 167;
    /// Frame `pain110`.
    pub const PAIN110: i32 = 168;
    /// Frame `pain111`.
    pub const PAIN111: i32 = 169;
    /// Frame `pain112`.
    pub const PAIN112: i32 = 170;
    /// Frame `pain113`.
    pub const PAIN113: i32 = 171;
    /// Frame `pain114`.
    pub const PAIN114: i32 = 172;
    /// Frame `pain115`.
    pub const PAIN115: i32 = 173;
    /// Frame `pain116`.
    pub const PAIN116: i32 = 174;
    /// Frame `pain117`.
    pub const PAIN117: i32 = 175;
    /// Frame `pain118`.
    pub const PAIN118: i32 = 176;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 177;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 178;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 179;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 180;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 181;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 182;
    /// Frame `pain207`.
    pub const PAIN207: i32 = 183;
    /// Frame `pain208`.
    pub const PAIN208: i32 = 184;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 185;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 186;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 187;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 188;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 189;
    /// Frame `death01`.
    pub const DEATH01: i32 = 190;
    /// Frame `death02`.
    pub const DEATH02: i32 = 191;
    /// Frame `death03`.
    pub const DEATH03: i32 = 192;
    /// Frame `death04`.
    pub const DEATH04: i32 = 193;
    /// Frame `death05`.
    pub const DEATH05: i32 = 194;
    /// Frame `death06`.
    pub const DEATH06: i32 = 195;
    /// Frame `death07`.
    pub const DEATH07: i32 = 196;
    /// Frame `death08`.
    pub const DEATH08: i32 = 197;
    /// Frame `death09`.
    pub const DEATH09: i32 = 198;
    /// Frame `death10`.
    pub const DEATH10: i32 = 199;
    /// Frame `death11`.
    pub const DEATH11: i32 = 200;
    /// Frame `duck01`.
    pub const DUCK01: i32 = 201;
    /// Frame `duck02`.
    pub const DUCK02: i32 = 202;
    /// Frame `duck03`.
    pub const DUCK03: i32 = 203;
    /// Frame `duck04`.
    pub const DUCK04: i32 = 204;
    /// Frame `duck05`.
    pub const DUCK05: i32 = 205;
    /// Frame `duck06`.
    pub const DUCK06: i32 = 206;
    /// Frame `duck07`.
    pub const DUCK07: i32 = 207;
    /// Frame `duck08`.
    pub const DUCK08: i32 = 208;
    /// Frame `jump01`.
    pub const JUMP01: i32 = 209;
    /// Frame `jump02`.
    pub const JUMP02: i32 = 210;
    /// Frame `jump03`.
    pub const JUMP03: i32 = 211;
    /// Frame `jump04`.
    pub const JUMP04: i32 = 212;
    /// Frame `jump05`.
    pub const JUMP05: i32 = 213;
    /// Frame `jump06`.
    pub const JUMP06: i32 = 214;
    /// Frame `jump07`.
    pub const JUMP07: i32 = 215;
    /// Frame `jump08`.
    pub const JUMP08: i32 = 216;
    /// Frame `jump09`.
    pub const JUMP09: i32 = 217;
    /// Frame `jump10`.
    pub const JUMP10: i32 = 218;
    /// Frame `shield01`.
    pub const SHIELD01: i32 = 219;
    /// Frame `shield02`.
    pub const SHIELD02: i32 = 220;
    /// Frame `shield03`.
    pub const SHIELD03: i32 = 221;
    /// Frame `shield04`.
    pub const SHIELD04: i32 = 222;
    /// Frame `shield05`.
    pub const SHIELD05: i32 = 223;
    /// Frame `shield06`.
    pub const SHIELD06: i32 = 224;
    /// Frame `attak301`.
    pub const ATTAK301: i32 = 225;
    /// Frame `attak302`.
    pub const ATTAK302: i32 = 226;
    /// Frame `attak303`.
    pub const ATTAK303: i32 = 227;
    /// Frame `attak304`.
    pub const ATTAK304: i32 = 228;
    /// Frame `attak305`.
    pub const ATTAK305: i32 = 229;
    /// Frame `attak306`.
    pub const ATTAK306: i32 = 230;
    /// Frame `attak307`.
    pub const ATTAK307: i32 = 231;
    /// Frame `attak308`.
    pub const ATTAK308: i32 = 232;
    /// Frame `attak309`.
    pub const ATTAK309: i32 = 233;
    /// Frame `attak310`.
    pub const ATTAK310: i32 = 234;
    /// Frame `attak311`.
    pub const ATTAK311: i32 = 235;
    /// Frame `attak312`.
    pub const ATTAK312: i32 = 236;
    /// Frame `attak313`.
    pub const ATTAK313: i32 = 237;
    /// Frame `attak314`.
    pub const ATTAK314: i32 = 238;
    /// Frame `attak315`.
    pub const ATTAK315: i32 = 239;
    /// Frame `attak316`.
    pub const ATTAK316: i32 = 240;
    /// Frame `attak317`.
    pub const ATTAK317: i32 = 241;
    /// Frame `attak318`.
    pub const ATTAK318: i32 = 242;
    /// Frame `attak319`.
    pub const ATTAK319: i32 = 243;
    /// Frame `attak320`.
    pub const ATTAK320: i32 = 244;
    /// Frame `attak321`.
    pub const ATTAK321: i32 = 245;
    /// Frame `attak322`.
    pub const ATTAK322: i32 = 246;
    /// Frame `attak323`.
    pub const ATTAK323: i32 = 247;
    /// Frame `attak324`.
    pub const ATTAK324: i32 = 248;
    /// Frame `c_stand101`.
    pub const C_STAND101: i32 = 249;
    /// Frame `c_stand102`.
    pub const C_STAND102: i32 = 250;
    /// Frame `c_stand103`.
    pub const C_STAND103: i32 = 251;
    /// Frame `c_stand104`.
    pub const C_STAND104: i32 = 252;
    /// Frame `c_stand105`.
    pub const C_STAND105: i32 = 253;
    /// Frame `c_stand106`.
    pub const C_STAND106: i32 = 254;
    /// Frame `c_stand107`.
    pub const C_STAND107: i32 = 255;
    /// Frame `c_stand108`.
    pub const C_STAND108: i32 = 256;
    /// Frame `c_stand109`.
    pub const C_STAND109: i32 = 257;
    /// Frame `c_stand110`.
    pub const C_STAND110: i32 = 258;
    /// Frame `c_stand111`.
    pub const C_STAND111: i32 = 259;
    /// Frame `c_stand112`.
    pub const C_STAND112: i32 = 260;
    /// Frame `c_stand113`.
    pub const C_STAND113: i32 = 261;
    /// Frame `c_stand114`.
    pub const C_STAND114: i32 = 262;
    /// Frame `c_stand115`.
    pub const C_STAND115: i32 = 263;
    /// Frame `c_stand116`.
    pub const C_STAND116: i32 = 264;
    /// Frame `c_stand117`.
    pub const C_STAND117: i32 = 265;
    /// Frame `c_stand118`.
    pub const C_STAND118: i32 = 266;
    /// Frame `c_stand119`.
    pub const C_STAND119: i32 = 267;
    /// Frame `c_stand120`.
    pub const C_STAND120: i32 = 268;
    /// Frame `c_stand121`.
    pub const C_STAND121: i32 = 269;
    /// Frame `c_stand122`.
    pub const C_STAND122: i32 = 270;
    /// Frame `c_stand123`.
    pub const C_STAND123: i32 = 271;
    /// Frame `c_stand124`.
    pub const C_STAND124: i32 = 272;
    /// Frame `c_stand125`.
    pub const C_STAND125: i32 = 273;
    /// Frame `c_stand126`.
    pub const C_STAND126: i32 = 274;
    /// Frame `c_stand127`.
    pub const C_STAND127: i32 = 275;
    /// Frame `c_stand128`.
    pub const C_STAND128: i32 = 276;
    /// Frame `c_stand129`.
    pub const C_STAND129: i32 = 277;
    /// Frame `c_stand130`.
    pub const C_STAND130: i32 = 278;
    /// Frame `c_stand131`.
    pub const C_STAND131: i32 = 279;
    /// Frame `c_stand132`.
    pub const C_STAND132: i32 = 280;
    /// Frame `c_stand133`.
    pub const C_STAND133: i32 = 281;
    /// Frame `c_stand134`.
    pub const C_STAND134: i32 = 282;
    /// Frame `c_stand135`.
    pub const C_STAND135: i32 = 283;
    /// Frame `c_stand136`.
    pub const C_STAND136: i32 = 284;
    /// Frame `c_stand137`.
    pub const C_STAND137: i32 = 285;
    /// Frame `c_stand138`.
    pub const C_STAND138: i32 = 286;
    /// Frame `c_stand139`.
    pub const C_STAND139: i32 = 287;
    /// Frame `c_stand140`.
    pub const C_STAND140: i32 = 288;
    /// Frame `c_stand201`.
    pub const C_STAND201: i32 = 289;
    /// Frame `c_stand202`.
    pub const C_STAND202: i32 = 290;
    /// Frame `c_stand203`.
    pub const C_STAND203: i32 = 291;
    /// Frame `c_stand204`.
    pub const C_STAND204: i32 = 292;
    /// Frame `c_stand205`.
    pub const C_STAND205: i32 = 293;
    /// Frame `c_stand206`.
    pub const C_STAND206: i32 = 294;
    /// Frame `c_stand207`.
    pub const C_STAND207: i32 = 295;
    /// Frame `c_stand208`.
    pub const C_STAND208: i32 = 296;
    /// Frame `c_stand209`.
    pub const C_STAND209: i32 = 297;
    /// Frame `c_stand210`.
    pub const C_STAND210: i32 = 298;
    /// Frame `c_stand211`.
    pub const C_STAND211: i32 = 299;
    /// Frame `c_stand212`.
    pub const C_STAND212: i32 = 300;
    /// Frame `c_stand213`.
    pub const C_STAND213: i32 = 301;
    /// Frame `c_stand214`.
    pub const C_STAND214: i32 = 302;
    /// Frame `c_stand215`.
    pub const C_STAND215: i32 = 303;
    /// Frame `c_stand216`.
    pub const C_STAND216: i32 = 304;
    /// Frame `c_stand217`.
    pub const C_STAND217: i32 = 305;
    /// Frame `c_stand218`.
    pub const C_STAND218: i32 = 306;
    /// Frame `c_stand219`.
    pub const C_STAND219: i32 = 307;
    /// Frame `c_stand220`.
    pub const C_STAND220: i32 = 308;
    /// Frame `c_stand221`.
    pub const C_STAND221: i32 = 309;
    /// Frame `c_stand222`.
    pub const C_STAND222: i32 = 310;
    /// Frame `c_stand223`.
    pub const C_STAND223: i32 = 311;
    /// Frame `c_stand224`.
    pub const C_STAND224: i32 = 312;
    /// Frame `c_stand225`.
    pub const C_STAND225: i32 = 313;
    /// Frame `c_stand226`.
    pub const C_STAND226: i32 = 314;
    /// Frame `c_stand227`.
    pub const C_STAND227: i32 = 315;
    /// Frame `c_stand228`.
    pub const C_STAND228: i32 = 316;
    /// Frame `c_stand229`.
    pub const C_STAND229: i32 = 317;
    /// Frame `c_stand230`.
    pub const C_STAND230: i32 = 318;
    /// Frame `c_stand231`.
    pub const C_STAND231: i32 = 319;
    /// Frame `c_stand232`.
    pub const C_STAND232: i32 = 320;
    /// Frame `c_stand233`.
    pub const C_STAND233: i32 = 321;
    /// Frame `c_stand234`.
    pub const C_STAND234: i32 = 322;
    /// Frame `c_stand235`.
    pub const C_STAND235: i32 = 323;
    /// Frame `c_stand236`.
    pub const C_STAND236: i32 = 324;
    /// Frame `c_stand237`.
    pub const C_STAND237: i32 = 325;
    /// Frame `c_stand238`.
    pub const C_STAND238: i32 = 326;
    /// Frame `c_stand239`.
    pub const C_STAND239: i32 = 327;
    /// Frame `c_stand240`.
    pub const C_STAND240: i32 = 328;
    /// Frame `c_stand241`.
    pub const C_STAND241: i32 = 329;
    /// Frame `c_stand242`.
    pub const C_STAND242: i32 = 330;
    /// Frame `c_stand243`.
    pub const C_STAND243: i32 = 331;
    /// Frame `c_stand244`.
    pub const C_STAND244: i32 = 332;
    /// Frame `c_stand245`.
    pub const C_STAND245: i32 = 333;
    /// Frame `c_stand246`.
    pub const C_STAND246: i32 = 334;
    /// Frame `c_stand247`.
    pub const C_STAND247: i32 = 335;
    /// Frame `c_stand248`.
    pub const C_STAND248: i32 = 336;
    /// Frame `c_stand249`.
    pub const C_STAND249: i32 = 337;
    /// Frame `c_stand250`.
    pub const C_STAND250: i32 = 338;
    /// Frame `c_stand251`.
    pub const C_STAND251: i32 = 339;
    /// Frame `c_stand252`.
    pub const C_STAND252: i32 = 340;
    /// Frame `c_stand253`.
    pub const C_STAND253: i32 = 341;
    /// Frame `c_stand254`.
    pub const C_STAND254: i32 = 342;
    /// Frame `c_attack101`.
    pub const C_ATTACK101: i32 = 343;
    /// Frame `c_attack102`.
    pub const C_ATTACK102: i32 = 344;
    /// Frame `c_attack103`.
    pub const C_ATTACK103: i32 = 345;
    /// Frame `c_attack104`.
    pub const C_ATTACK104: i32 = 346;
    /// Frame `c_attack105`.
    pub const C_ATTACK105: i32 = 347;
    /// Frame `c_attack106`.
    pub const C_ATTACK106: i32 = 348;
    /// Frame `c_attack107`.
    pub const C_ATTACK107: i32 = 349;
    /// Frame `c_attack108`.
    pub const C_ATTACK108: i32 = 350;
    /// Frame `c_attack109`.
    pub const C_ATTACK109: i32 = 351;
    /// Frame `c_attack110`.
    pub const C_ATTACK110: i32 = 352;
    /// Frame `c_attack111`.
    pub const C_ATTACK111: i32 = 353;
    /// Frame `c_attack112`.
    pub const C_ATTACK112: i32 = 354;
    /// Frame `c_attack113`.
    pub const C_ATTACK113: i32 = 355;
    /// Frame `c_attack114`.
    pub const C_ATTACK114: i32 = 356;
    /// Frame `c_attack115`.
    pub const C_ATTACK115: i32 = 357;
    /// Frame `c_attack116`.
    pub const C_ATTACK116: i32 = 358;
    /// Frame `c_attack117`.
    pub const C_ATTACK117: i32 = 359;
    /// Frame `c_attack118`.
    pub const C_ATTACK118: i32 = 360;
    /// Frame `c_attack119`.
    pub const C_ATTACK119: i32 = 361;
    /// Frame `c_attack120`.
    pub const C_ATTACK120: i32 = 362;
    /// Frame `c_attack121`.
    pub const C_ATTACK121: i32 = 363;
    /// Frame `c_attack122`.
    pub const C_ATTACK122: i32 = 364;
    /// Frame `c_attack123`.
    pub const C_ATTACK123: i32 = 365;
    /// Frame `c_attack124`.
    pub const C_ATTACK124: i32 = 366;
    /// Frame `c_jump01`.
    pub const C_JUMP01: i32 = 367;
    /// Frame `c_jump02`.
    pub const C_JUMP02: i32 = 368;
    /// Frame `c_jump03`.
    pub const C_JUMP03: i32 = 369;
    /// Frame `c_jump04`.
    pub const C_JUMP04: i32 = 370;
    /// Frame `c_jump05`.
    pub const C_JUMP05: i32 = 371;
    /// Frame `c_jump06`.
    pub const C_JUMP06: i32 = 372;
    /// Frame `c_jump07`.
    pub const C_JUMP07: i32 = 373;
    /// Frame `c_jump08`.
    pub const C_JUMP08: i32 = 374;
    /// Frame `c_jump09`.
    pub const C_JUMP09: i32 = 375;
    /// Frame `c_jump10`.
    pub const C_JUMP10: i32 = 376;
    /// Frame `c_attack201`.
    pub const C_ATTACK201: i32 = 377;
    /// Frame `c_attack202`.
    pub const C_ATTACK202: i32 = 378;
    /// Frame `c_attack203`.
    pub const C_ATTACK203: i32 = 379;
    /// Frame `c_attack204`.
    pub const C_ATTACK204: i32 = 380;
    /// Frame `c_attack205`.
    pub const C_ATTACK205: i32 = 381;
    /// Frame `c_attack206`.
    pub const C_ATTACK206: i32 = 382;
    /// Frame `c_attack207`.
    pub const C_ATTACK207: i32 = 383;
    /// Frame `c_attack208`.
    pub const C_ATTACK208: i32 = 384;
    /// Frame `c_attack209`.
    pub const C_ATTACK209: i32 = 385;
    /// Frame `c_attack210`.
    pub const C_ATTACK210: i32 = 386;
    /// Frame `c_attack211`.
    pub const C_ATTACK211: i32 = 387;
    /// Frame `c_attack212`.
    pub const C_ATTACK212: i32 = 388;
    /// Frame `c_attack213`.
    pub const C_ATTACK213: i32 = 389;
    /// Frame `c_attack214`.
    pub const C_ATTACK214: i32 = 390;
    /// Frame `c_attack215`.
    pub const C_ATTACK215: i32 = 391;
    /// Frame `c_attack216`.
    pub const C_ATTACK216: i32 = 392;
    /// Frame `c_attack217`.
    pub const C_ATTACK217: i32 = 393;
    /// Frame `c_attack218`.
    pub const C_ATTACK218: i32 = 394;
    /// Frame `c_attack219`.
    pub const C_ATTACK219: i32 = 395;
    /// Frame `c_attack220`.
    pub const C_ATTACK220: i32 = 396;
    /// Frame `c_attack221`.
    pub const C_ATTACK221: i32 = 397;
    /// Frame `c_attack301`.
    pub const C_ATTACK301: i32 = 398;
    /// Frame `c_attack302`.
    pub const C_ATTACK302: i32 = 399;
    /// Frame `c_attack303`.
    pub const C_ATTACK303: i32 = 400;
    /// Frame `c_attack304`.
    pub const C_ATTACK304: i32 = 401;
    /// Frame `c_attack305`.
    pub const C_ATTACK305: i32 = 402;
    /// Frame `c_attack306`.
    pub const C_ATTACK306: i32 = 403;
    /// Frame `c_attack307`.
    pub const C_ATTACK307: i32 = 404;
    /// Frame `c_attack308`.
    pub const C_ATTACK308: i32 = 405;
    /// Frame `c_attack309`.
    pub const C_ATTACK309: i32 = 406;
    /// Frame `c_attack310`.
    pub const C_ATTACK310: i32 = 407;
    /// Frame `c_attack311`.
    pub const C_ATTACK311: i32 = 408;
    /// Frame `c_attack312`.
    pub const C_ATTACK312: i32 = 409;
    /// Frame `c_attack313`.
    pub const C_ATTACK313: i32 = 410;
    /// Frame `c_attack314`.
    pub const C_ATTACK314: i32 = 411;
    /// Frame `c_attack315`.
    pub const C_ATTACK315: i32 = 412;
    /// Frame `c_attack316`.
    pub const C_ATTACK316: i32 = 413;
    /// Frame `c_attack317`.
    pub const C_ATTACK317: i32 = 414;
    /// Frame `c_attack318`.
    pub const C_ATTACK318: i32 = 415;
    /// Frame `c_attack319`.
    pub const C_ATTACK319: i32 = 416;
    /// Frame `c_attack320`.
    pub const C_ATTACK320: i32 = 417;
    /// Frame `c_attack321`.
    pub const C_ATTACK321: i32 = 418;
    /// Frame `c_attack401`.
    pub const C_ATTACK401: i32 = 419;
    /// Frame `c_attack402`.
    pub const C_ATTACK402: i32 = 420;
    /// Frame `c_attack403`.
    pub const C_ATTACK403: i32 = 421;
    /// Frame `c_attack404`.
    pub const C_ATTACK404: i32 = 422;
    /// Frame `c_attack405`.
    pub const C_ATTACK405: i32 = 423;
    /// Frame `c_attack501`.
    pub const C_ATTACK501: i32 = 424;
    /// Frame `c_attack502`.
    pub const C_ATTACK502: i32 = 425;
    /// Frame `c_attack503`.
    pub const C_ATTACK503: i32 = 426;
    /// Frame `c_attack504`.
    pub const C_ATTACK504: i32 = 427;
    /// Frame `c_attack505`.
    pub const C_ATTACK505: i32 = 428;
    /// Frame `c_attack601`.
    pub const C_ATTACK601: i32 = 429;
    /// Frame `c_attack602`.
    pub const C_ATTACK602: i32 = 430;
    /// Frame `c_attack603`.
    pub const C_ATTACK603: i32 = 431;
    /// Frame `c_attack604`.
    pub const C_ATTACK604: i32 = 432;
    /// Frame `c_attack605`.
    pub const C_ATTACK605: i32 = 433;
    /// Frame `c_attack701`.
    pub const C_ATTACK701: i32 = 434;
    /// Frame `c_attack702`.
    pub const C_ATTACK702: i32 = 435;
    /// Frame `c_attack703`.
    pub const C_ATTACK703: i32 = 436;
    /// Frame `c_attack704`.
    pub const C_ATTACK704: i32 = 437;
    /// Frame `c_attack705`.
    pub const C_ATTACK705: i32 = 438;
    /// Frame `c_pain101`.
    pub const C_PAIN101: i32 = 439;
    /// Frame `c_pain102`.
    pub const C_PAIN102: i32 = 440;
    /// Frame `c_pain103`.
    pub const C_PAIN103: i32 = 441;
    /// Frame `c_pain104`.
    pub const C_PAIN104: i32 = 442;
    /// Frame `c_pain201`.
    pub const C_PAIN201: i32 = 443;
    /// Frame `c_pain202`.
    pub const C_PAIN202: i32 = 444;
    /// Frame `c_pain203`.
    pub const C_PAIN203: i32 = 445;
    /// Frame `c_pain204`.
    pub const C_PAIN204: i32 = 446;
    /// Frame `c_pain301`.
    pub const C_PAIN301: i32 = 447;
    /// Frame `c_pain302`.
    pub const C_PAIN302: i32 = 448;
    /// Frame `c_pain303`.
    pub const C_PAIN303: i32 = 449;
    /// Frame `c_pain304`.
    pub const C_PAIN304: i32 = 450;
    /// Frame `c_pain401`.
    pub const C_PAIN401: i32 = 451;
    /// Frame `c_pain402`.
    pub const C_PAIN402: i32 = 452;
    /// Frame `c_pain403`.
    pub const C_PAIN403: i32 = 453;
    /// Frame `c_pain404`.
    pub const C_PAIN404: i32 = 454;
    /// Frame `c_pain405`.
    pub const C_PAIN405: i32 = 455;
    /// Frame `c_pain406`.
    pub const C_PAIN406: i32 = 456;
    /// Frame `c_pain407`.
    pub const C_PAIN407: i32 = 457;
    /// Frame `c_pain408`.
    pub const C_PAIN408: i32 = 458;
    /// Frame `c_pain409`.
    pub const C_PAIN409: i32 = 459;
    /// Frame `c_pain410`.
    pub const C_PAIN410: i32 = 460;
    /// Frame `c_pain411`.
    pub const C_PAIN411: i32 = 461;
    /// Frame `c_pain412`.
    pub const C_PAIN412: i32 = 462;
    /// Frame `c_pain413`.
    pub const C_PAIN413: i32 = 463;
    /// Frame `c_pain414`.
    pub const C_PAIN414: i32 = 464;
    /// Frame `c_pain415`.
    pub const C_PAIN415: i32 = 465;
    /// Frame `c_pain501`.
    pub const C_PAIN501: i32 = 466;
    /// Frame `c_pain502`.
    pub const C_PAIN502: i32 = 467;
    /// Frame `c_pain503`.
    pub const C_PAIN503: i32 = 468;
    /// Frame `c_pain504`.
    pub const C_PAIN504: i32 = 469;
    /// Frame `c_pain505`.
    pub const C_PAIN505: i32 = 470;
    /// Frame `c_pain506`.
    pub const C_PAIN506: i32 = 471;
    /// Frame `c_pain507`.
    pub const C_PAIN507: i32 = 472;
    /// Frame `c_pain508`.
    pub const C_PAIN508: i32 = 473;
    /// Frame `c_pain509`.
    pub const C_PAIN509: i32 = 474;
    /// Frame `c_pain510`.
    pub const C_PAIN510: i32 = 475;
    /// Frame `c_pain511`.
    pub const C_PAIN511: i32 = 476;
    /// Frame `c_pain512`.
    pub const C_PAIN512: i32 = 477;
    /// Frame `c_pain513`.
    pub const C_PAIN513: i32 = 478;
    /// Frame `c_pain514`.
    pub const C_PAIN514: i32 = 479;
    /// Frame `c_pain515`.
    pub const C_PAIN515: i32 = 480;
    /// Frame `c_pain516`.
    pub const C_PAIN516: i32 = 481;
    /// Frame `c_pain517`.
    pub const C_PAIN517: i32 = 482;
    /// Frame `c_pain518`.
    pub const C_PAIN518: i32 = 483;
    /// Frame `c_pain519`.
    pub const C_PAIN519: i32 = 484;
    /// Frame `c_pain520`.
    pub const C_PAIN520: i32 = 485;
    /// Frame `c_pain521`.
    pub const C_PAIN521: i32 = 486;
    /// Frame `c_pain522`.
    pub const C_PAIN522: i32 = 487;
    /// Frame `c_pain523`.
    pub const C_PAIN523: i32 = 488;
    /// Frame `c_pain524`.
    pub const C_PAIN524: i32 = 489;
    /// Frame `c_death101`.
    pub const C_DEATH101: i32 = 490;
    /// Frame `c_death102`.
    pub const C_DEATH102: i32 = 491;
    /// Frame `c_death103`.
    pub const C_DEATH103: i32 = 492;
    /// Frame `c_death104`.
    pub const C_DEATH104: i32 = 493;
    /// Frame `c_death105`.
    pub const C_DEATH105: i32 = 494;
    /// Frame `c_death106`.
    pub const C_DEATH106: i32 = 495;
    /// Frame `c_death107`.
    pub const C_DEATH107: i32 = 496;
    /// Frame `c_death108`.
    pub const C_DEATH108: i32 = 497;
    /// Frame `c_death109`.
    pub const C_DEATH109: i32 = 498;
    /// Frame `c_death110`.
    pub const C_DEATH110: i32 = 499;
    /// Frame `c_death111`.
    pub const C_DEATH111: i32 = 500;
    /// Frame `c_death112`.
    pub const C_DEATH112: i32 = 501;
    /// Frame `c_death113`.
    pub const C_DEATH113: i32 = 502;
    /// Frame `c_death114`.
    pub const C_DEATH114: i32 = 503;
    /// Frame `c_death115`.
    pub const C_DEATH115: i32 = 504;
    /// Frame `c_death116`.
    pub const C_DEATH116: i32 = 505;
    /// Frame `c_death117`.
    pub const C_DEATH117: i32 = 506;
    /// Frame `c_death118`.
    pub const C_DEATH118: i32 = 507;
    /// Frame `c_death201`.
    pub const C_DEATH201: i32 = 508;
    /// Frame `c_death202`.
    pub const C_DEATH202: i32 = 509;
    /// Frame `c_death203`.
    pub const C_DEATH203: i32 = 510;
    /// Frame `c_death204`.
    pub const C_DEATH204: i32 = 511;
    /// Frame `c_death301`.
    pub const C_DEATH301: i32 = 512;
    /// Frame `c_death302`.
    pub const C_DEATH302: i32 = 513;
    /// Frame `c_death303`.
    pub const C_DEATH303: i32 = 514;
    /// Frame `c_death304`.
    pub const C_DEATH304: i32 = 515;
    /// Frame `c_death305`.
    pub const C_DEATH305: i32 = 516;
    /// Frame `c_death306`.
    pub const C_DEATH306: i32 = 517;
    /// Frame `c_death307`.
    pub const C_DEATH307: i32 = 518;
    /// Frame `c_death308`.
    pub const C_DEATH308: i32 = 519;
    /// Frame `c_death309`.
    pub const C_DEATH309: i32 = 520;
    /// Frame `c_death310`.
    pub const C_DEATH310: i32 = 521;
    /// Frame `c_death311`.
    pub const C_DEATH311: i32 = 522;
    /// Frame `c_death312`.
    pub const C_DEATH312: i32 = 523;
    /// Frame `c_death313`.
    pub const C_DEATH313: i32 = 524;
    /// Frame `c_death314`.
    pub const C_DEATH314: i32 = 525;
    /// Frame `c_death315`.
    pub const C_DEATH315: i32 = 526;
    /// Frame `c_death316`.
    pub const C_DEATH316: i32 = 527;
    /// Frame `c_death317`.
    pub const C_DEATH317: i32 = 528;
    /// Frame `c_death318`.
    pub const C_DEATH318: i32 = 529;
    /// Frame `c_death319`.
    pub const C_DEATH319: i32 = 530;
    /// Frame `c_death320`.
    pub const C_DEATH320: i32 = 531;
    /// Frame `c_death321`.
    pub const C_DEATH321: i32 = 532;
    /// Frame `c_death401`.
    pub const C_DEATH401: i32 = 533;
    /// Frame `c_death402`.
    pub const C_DEATH402: i32 = 534;
    /// Frame `c_death403`.
    pub const C_DEATH403: i32 = 535;
    /// Frame `c_death404`.
    pub const C_DEATH404: i32 = 536;
    /// Frame `c_death405`.
    pub const C_DEATH405: i32 = 537;
    /// Frame `c_death406`.
    pub const C_DEATH406: i32 = 538;
    /// Frame `c_death407`.
    pub const C_DEATH407: i32 = 539;
    /// Frame `c_death408`.
    pub const C_DEATH408: i32 = 540;
    /// Frame `c_death409`.
    pub const C_DEATH409: i32 = 541;
    /// Frame `c_death410`.
    pub const C_DEATH410: i32 = 542;
    /// Frame `c_death411`.
    pub const C_DEATH411: i32 = 543;
    /// Frame `c_death412`.
    pub const C_DEATH412: i32 = 544;
    /// Frame `c_death413`.
    pub const C_DEATH413: i32 = 545;
    /// Frame `c_death414`.
    pub const C_DEATH414: i32 = 546;
    /// Frame `c_death415`.
    pub const C_DEATH415: i32 = 547;
    /// Frame `c_death416`.
    pub const C_DEATH416: i32 = 548;
    /// Frame `c_death417`.
    pub const C_DEATH417: i32 = 549;
    /// Frame `c_death418`.
    pub const C_DEATH418: i32 = 550;
    /// Frame `c_death419`.
    pub const C_DEATH419: i32 = 551;
    /// Frame `c_death420`.
    pub const C_DEATH420: i32 = 552;
    /// Frame `c_death421`.
    pub const C_DEATH421: i32 = 553;
    /// Frame `c_death422`.
    pub const C_DEATH422: i32 = 554;
    /// Frame `c_death423`.
    pub const C_DEATH423: i32 = 555;
    /// Frame `c_death424`.
    pub const C_DEATH424: i32 = 556;
    /// Frame `c_death425`.
    pub const C_DEATH425: i32 = 557;
    /// Frame `c_death426`.
    pub const C_DEATH426: i32 = 558;
    /// Frame `c_death427`.
    pub const C_DEATH427: i32 = 559;
    /// Frame `c_death428`.
    pub const C_DEATH428: i32 = 560;
    /// Frame `c_death429`.
    pub const C_DEATH429: i32 = 561;
    /// Frame `c_death430`.
    pub const C_DEATH430: i32 = 562;
    /// Frame `c_death431`.
    pub const C_DEATH431: i32 = 563;
    /// Frame `c_death432`.
    pub const C_DEATH432: i32 = 564;
    /// Frame `c_death433`.
    pub const C_DEATH433: i32 = 565;
    /// Frame `c_death434`.
    pub const C_DEATH434: i32 = 566;
    /// Frame `c_death435`.
    pub const C_DEATH435: i32 = 567;
    /// Frame `c_death436`.
    pub const C_DEATH436: i32 = 568;
    /// Frame `c_death501`.
    pub const C_DEATH501: i32 = 569;
    /// Frame `c_death502`.
    pub const C_DEATH502: i32 = 570;
    /// Frame `c_death503`.
    pub const C_DEATH503: i32 = 571;
    /// Frame `c_death504`.
    pub const C_DEATH504: i32 = 572;
    /// Frame `c_death505`.
    pub const C_DEATH505: i32 = 573;
    /// Frame `c_death506`.
    pub const C_DEATH506: i32 = 574;
    /// Frame `c_death507`.
    pub const C_DEATH507: i32 = 575;
    /// Frame `c_death508`.
    pub const C_DEATH508: i32 = 576;
    /// Frame `c_death509`.
    pub const C_DEATH509: i32 = 577;
    /// Frame `c_death510`.
    pub const C_DEATH510: i32 = 578;
    /// Frame `c_death511`.
    pub const C_DEATH511: i32 = 579;
    /// Frame `c_death512`.
    pub const C_DEATH512: i32 = 580;
    /// Frame `c_death513`.
    pub const C_DEATH513: i32 = 581;
    /// Frame `c_death514`.
    pub const C_DEATH514: i32 = 582;
    /// Frame `c_death515`.
    pub const C_DEATH515: i32 = 583;
    /// Frame `c_death516`.
    pub const C_DEATH516: i32 = 584;
    /// Frame `c_death517`.
    pub const C_DEATH517: i32 = 585;
    /// Frame `c_death518`.
    pub const C_DEATH518: i32 = 586;
    /// Frame `c_death519`.
    pub const C_DEATH519: i32 = 587;
    /// Frame `c_death520`.
    pub const C_DEATH520: i32 = 588;
    /// Frame `c_death521`.
    pub const C_DEATH521: i32 = 589;
    /// Frame `c_death522`.
    pub const C_DEATH522: i32 = 590;
    /// Frame `c_death523`.
    pub const C_DEATH523: i32 = 591;
    /// Frame `c_death524`.
    pub const C_DEATH524: i32 = 592;
    /// Frame `c_death525`.
    pub const C_DEATH525: i32 = 593;
    /// Frame `c_death526`.
    pub const C_DEATH526: i32 = 594;
    /// Frame `c_death527`.
    pub const C_DEATH527: i32 = 595;
    /// Frame `c_death528`.
    pub const C_DEATH528: i32 = 596;
    /// Frame `c_run101`.
    pub const C_RUN101: i32 = 597;
    /// Frame `c_run102`.
    pub const C_RUN102: i32 = 598;
    /// Frame `c_run103`.
    pub const C_RUN103: i32 = 599;
    /// Frame `c_run104`.
    pub const C_RUN104: i32 = 600;
    /// Frame `c_run105`.
    pub const C_RUN105: i32 = 601;
    /// Frame `c_run106`.
    pub const C_RUN106: i32 = 602;
    /// Frame `c_run201`.
    pub const C_RUN201: i32 = 603;
    /// Frame `c_run202`.
    pub const C_RUN202: i32 = 604;
    /// Frame `c_run203`.
    pub const C_RUN203: i32 = 605;
    /// Frame `c_run204`.
    pub const C_RUN204: i32 = 606;
    /// Frame `c_run205`.
    pub const C_RUN205: i32 = 607;
    /// Frame `c_run206`.
    pub const C_RUN206: i32 = 608;
    /// Frame `c_run301`.
    pub const C_RUN301: i32 = 609;
    /// Frame `c_run302`.
    pub const C_RUN302: i32 = 610;
    /// Frame `c_run303`.
    pub const C_RUN303: i32 = 611;
    /// Frame `c_run304`.
    pub const C_RUN304: i32 = 612;
    /// Frame `c_run305`.
    pub const C_RUN305: i32 = 613;
    /// Frame `c_run306`.
    pub const C_RUN306: i32 = 614;
    /// Frame `c_walk101`.
    pub const C_WALK101: i32 = 615;
    /// Frame `c_walk102`.
    pub const C_WALK102: i32 = 616;
    /// Frame `c_walk103`.
    pub const C_WALK103: i32 = 617;
    /// Frame `c_walk104`.
    pub const C_WALK104: i32 = 618;
    /// Frame `c_walk105`.
    pub const C_WALK105: i32 = 619;
    /// Frame `c_walk106`.
    pub const C_WALK106: i32 = 620;
    /// Frame `c_walk107`.
    pub const C_WALK107: i32 = 621;
    /// Frame `c_walk108`.
    pub const C_WALK108: i32 = 622;
    /// Frame `c_walk109`.
    pub const C_WALK109: i32 = 623;
    /// Frame `c_walk110`.
    pub const C_WALK110: i32 = 624;
    /// Frame `c_walk111`.
    pub const C_WALK111: i32 = 625;
    /// Frame `c_walk112`.
    pub const C_WALK112: i32 = 626;
    /// Frame `c_walk113`.
    pub const C_WALK113: i32 = 627;
    /// Frame `c_walk114`.
    pub const C_WALK114: i32 = 628;
    /// Frame `c_walk115`.
    pub const C_WALK115: i32 = 629;
    /// Frame `c_walk116`.
    pub const C_WALK116: i32 = 630;
    /// Frame `c_walk117`.
    pub const C_WALK117: i32 = 631;
    /// Frame `c_walk118`.
    pub const C_WALK118: i32 = 632;
    /// Frame `c_walk119`.
    pub const C_WALK119: i32 = 633;
    /// Frame `c_walk120`.
    pub const C_WALK120: i32 = 634;
    /// Frame `c_walk121`.
    pub const C_WALK121: i32 = 635;
    /// Frame `c_walk122`.
    pub const C_WALK122: i32 = 636;
    /// Frame `c_walk123`.
    pub const C_WALK123: i32 = 637;
    /// Frame `c_walk124`.
    pub const C_WALK124: i32 = 638;
    /// Frame `c_pain601`.
    pub const C_PAIN601: i32 = 639;
    /// Frame `c_pain602`.
    pub const C_PAIN602: i32 = 640;
    /// Frame `c_pain603`.
    pub const C_PAIN603: i32 = 641;
    /// Frame `c_pain604`.
    pub const C_PAIN604: i32 = 642;
    /// Frame `c_pain605`.
    pub const C_PAIN605: i32 = 643;
    /// Frame `c_pain606`.
    pub const C_PAIN606: i32 = 644;
    /// Frame `c_pain607`.
    pub const C_PAIN607: i32 = 645;
    /// Frame `c_pain608`.
    pub const C_PAIN608: i32 = 646;
    /// Frame `c_pain609`.
    pub const C_PAIN609: i32 = 647;
    /// Frame `c_pain610`.
    pub const C_PAIN610: i32 = 648;
    /// Frame `c_pain611`.
    pub const C_PAIN611: i32 = 649;
    /// Frame `c_pain612`.
    pub const C_PAIN612: i32 = 650;
    /// Frame `c_pain613`.
    pub const C_PAIN613: i32 = 651;
    /// Frame `c_pain614`.
    pub const C_PAIN614: i32 = 652;
    /// Frame `c_pain615`.
    pub const C_PAIN615: i32 = 653;
    /// Frame `c_pain616`.
    pub const C_PAIN616: i32 = 654;
    /// Frame `c_pain617`.
    pub const C_PAIN617: i32 = 655;
    /// Frame `c_pain618`.
    pub const C_PAIN618: i32 = 656;
    /// Frame `c_pain619`.
    pub const C_PAIN619: i32 = 657;
    /// Frame `c_pain620`.
    pub const C_PAIN620: i32 = 658;
    /// Frame `c_pain621`.
    pub const C_PAIN621: i32 = 659;
    /// Frame `c_pain622`.
    pub const C_PAIN622: i32 = 660;
    /// Frame `c_pain623`.
    pub const C_PAIN623: i32 = 661;
    /// Frame `c_pain624`.
    pub const C_PAIN624: i32 = 662;
    /// Frame `c_pain625`.
    pub const C_PAIN625: i32 = 663;
    /// Frame `c_pain626`.
    pub const C_PAIN626: i32 = 664;
    /// Frame `c_pain627`.
    pub const C_PAIN627: i32 = 665;
    /// Frame `c_pain628`.
    pub const C_PAIN628: i32 = 666;
    /// Frame `c_pain629`.
    pub const C_PAIN629: i32 = 667;
    /// Frame `c_pain630`.
    pub const C_PAIN630: i32 = 668;
    /// Frame `c_pain631`.
    pub const C_PAIN631: i32 = 669;
    /// Frame `c_pain632`.
    pub const C_PAIN632: i32 = 670;
    /// Frame `c_death601`.
    pub const C_DEATH601: i32 = 671;
    /// Frame `c_death602`.
    pub const C_DEATH602: i32 = 672;
    /// Frame `c_death603`.
    pub const C_DEATH603: i32 = 673;
    /// Frame `c_death604`.
    pub const C_DEATH604: i32 = 674;
    /// Frame `c_death605`.
    pub const C_DEATH605: i32 = 675;
    /// Frame `c_death606`.
    pub const C_DEATH606: i32 = 676;
    /// Frame `c_death607`.
    pub const C_DEATH607: i32 = 677;
    /// Frame `c_death608`.
    pub const C_DEATH608: i32 = 678;
    /// Frame `c_death609`.
    pub const C_DEATH609: i32 = 679;
    /// Frame `c_death610`.
    pub const C_DEATH610: i32 = 680;
    /// Frame `c_death611`.
    pub const C_DEATH611: i32 = 681;
    /// Frame `c_death612`.
    pub const C_DEATH612: i32 = 682;
    /// Frame `c_death613`.
    pub const C_DEATH613: i32 = 683;
    /// Frame `c_death614`.
    pub const C_DEATH614: i32 = 684;
    /// Frame `c_death701`.
    pub const C_DEATH701: i32 = 685;
    /// Frame `c_death702`.
    pub const C_DEATH702: i32 = 686;
    /// Frame `c_death703`.
    pub const C_DEATH703: i32 = 687;
    /// Frame `c_death704`.
    pub const C_DEATH704: i32 = 688;
    /// Frame `c_death705`.
    pub const C_DEATH705: i32 = 689;
    /// Frame `c_death706`.
    pub const C_DEATH706: i32 = 690;
    /// Frame `c_death707`.
    pub const C_DEATH707: i32 = 691;
    /// Frame `c_death708`.
    pub const C_DEATH708: i32 = 692;
    /// Frame `c_death709`.
    pub const C_DEATH709: i32 = 693;
    /// Frame `c_death710`.
    pub const C_DEATH710: i32 = 694;
    /// Frame `c_death711`.
    pub const C_DEATH711: i32 = 695;
    /// Frame `c_death712`.
    pub const C_DEATH712: i32 = 696;
    /// Frame `c_death713`.
    pub const C_DEATH713: i32 = 697;
    /// Frame `c_death714`.
    pub const C_DEATH714: i32 = 698;
    /// Frame `c_death715`.
    pub const C_DEATH715: i32 = 699;
    /// Frame `c_death716`.
    pub const C_DEATH716: i32 = 700;
    /// Frame `c_death717`.
    pub const C_DEATH717: i32 = 701;
    /// Frame `c_death718`.
    pub const C_DEATH718: i32 = 702;
    /// Frame `c_death719`.
    pub const C_DEATH719: i32 = 703;
    /// Frame `c_death720`.
    pub const C_DEATH720: i32 = 704;
    /// Frame `c_death721`.
    pub const C_DEATH721: i32 = 705;
    /// Frame `c_death722`.
    pub const C_DEATH722: i32 = 706;
    /// Frame `c_death723`.
    pub const C_DEATH723: i32 = 707;
    /// Frame `c_death724`.
    pub const C_DEATH724: i32 = 708;
    /// Frame `c_death725`.
    pub const C_DEATH725: i32 = 709;
    /// Frame `c_death726`.
    pub const C_DEATH726: i32 = 710;
    /// Frame `c_death727`.
    pub const C_DEATH727: i32 = 711;
    /// Frame `c_death728`.
    pub const C_DEATH728: i32 = 712;
    /// Frame `c_death729`.
    pub const C_DEATH729: i32 = 713;
    /// Frame `c_death730`.
    pub const C_DEATH730: i32 = 714;
    /// Frame `c_pain701`.
    pub const C_PAIN701: i32 = 715;
    /// Frame `c_pain702`.
    pub const C_PAIN702: i32 = 716;
    /// Frame `c_pain703`.
    pub const C_PAIN703: i32 = 717;
    /// Frame `c_pain704`.
    pub const C_PAIN704: i32 = 718;
    /// Frame `c_pain705`.
    pub const C_PAIN705: i32 = 719;
    /// Frame `c_pain706`.
    pub const C_PAIN706: i32 = 720;
    /// Frame `c_pain707`.
    pub const C_PAIN707: i32 = 721;
    /// Frame `c_pain708`.
    pub const C_PAIN708: i32 = 722;
    /// Frame `c_pain709`.
    pub const C_PAIN709: i32 = 723;
    /// Frame `c_pain710`.
    pub const C_PAIN710: i32 = 724;
    /// Frame `c_pain711`.
    pub const C_PAIN711: i32 = 725;
    /// Frame `c_pain712`.
    pub const C_PAIN712: i32 = 726;
    /// Frame `c_pain713`.
    pub const C_PAIN713: i32 = 727;
    /// Frame `c_pain714`.
    pub const C_PAIN714: i32 = 728;
    /// Frame `c_attack801`.
    pub const C_ATTACK801: i32 = 729;
    /// Frame `c_attack802`.
    pub const C_ATTACK802: i32 = 730;
    /// Frame `c_attack803`.
    pub const C_ATTACK803: i32 = 731;
    /// Frame `c_attack804`.
    pub const C_ATTACK804: i32 = 732;
    /// Frame `c_attack805`.
    pub const C_ATTACK805: i32 = 733;
    /// Frame `c_attack806`.
    pub const C_ATTACK806: i32 = 734;
    /// Frame `c_attack807`.
    pub const C_ATTACK807: i32 = 735;
    /// Frame `c_attack808`.
    pub const C_ATTACK808: i32 = 736;
    /// Frame `c_attack809`.
    pub const C_ATTACK809: i32 = 737;
    /// Frame `c_attack901`.
    pub const C_ATTACK901: i32 = 738;
    /// Frame `c_attack902`.
    pub const C_ATTACK902: i32 = 739;
    /// Frame `c_attack903`.
    pub const C_ATTACK903: i32 = 740;
    /// Frame `c_attack904`.
    pub const C_ATTACK904: i32 = 741;
    /// Frame `c_attack905`.
    pub const C_ATTACK905: i32 = 742;
    /// Frame `c_attack906`.
    pub const C_ATTACK906: i32 = 743;
    /// Frame `c_attack907`.
    pub const C_ATTACK907: i32 = 744;
    /// Frame `c_attack908`.
    pub const C_ATTACK908: i32 = 745;
    /// Frame `c_attack909`.
    pub const C_ATTACK909: i32 = 746;
    /// Frame `c_attack910`.
    pub const C_ATTACK910: i32 = 747;
    /// Frame `c_attack911`.
    pub const C_ATTACK911: i32 = 748;
    /// Frame `c_attack912`.
    pub const C_ATTACK912: i32 = 749;
    /// Frame `c_attack913`.
    pub const C_ATTACK913: i32 = 750;
    /// Frame `c_attack914`.
    pub const C_ATTACK914: i32 = 751;
    /// Frame `c_attack915`.
    pub const C_ATTACK915: i32 = 752;
    /// Frame `c_attack916`.
    pub const C_ATTACK916: i32 = 753;
    /// Frame `c_attack917`.
    pub const C_ATTACK917: i32 = 754;
    /// Frame `c_attack918`.
    pub const C_ATTACK918: i32 = 755;
    /// Frame `c_attack919`.
    pub const C_ATTACK919: i32 = 756;
    /// Frame `c_duck01`.
    pub const C_DUCK01: i32 = 757;
    /// Frame `c_duck02`.
    pub const C_DUCK02: i32 = 758;
    /// Frame `c_duckstep01`.
    pub const C_DUCKSTEP01: i32 = 759;
    /// Frame `c_duckstep02`.
    pub const C_DUCKSTEP02: i32 = 760;
    /// Frame `c_duckstep03`.
    pub const C_DUCKSTEP03: i32 = 761;
    /// Frame `c_duckstep04`.
    pub const C_DUCKSTEP04: i32 = 762;
    /// Frame `c_duckstep05`.
    pub const C_DUCKSTEP05: i32 = 763;
    /// Frame `c_duckstep06`.
    pub const C_DUCKSTEP06: i32 = 764;
    /// Frame `c_duckpain01`.
    pub const C_DUCKPAIN01: i32 = 765;
    /// Frame `c_duckpain02`.
    pub const C_DUCKPAIN02: i32 = 766;
    /// Frame `c_duckpain03`.
    pub const C_DUCKPAIN03: i32 = 767;
    /// Frame `c_duckpain04`.
    pub const C_DUCKPAIN04: i32 = 768;
    /// Frame `c_duckpain05`.
    pub const C_DUCKPAIN05: i32 = 769;
    /// Frame `c_duckdeath01`.
    pub const C_DUCKDEATH01: i32 = 770;
    /// Frame `c_duckdeath02`.
    pub const C_DUCKDEATH02: i32 = 771;
    /// Frame `c_duckdeath03`.
    pub const C_DUCKDEATH03: i32 = 772;
    /// Frame `c_duckdeath04`.
    pub const C_DUCKDEATH04: i32 = 773;
    /// Frame `c_duckdeath05`.
    pub const C_DUCKDEATH05: i32 = 774;
    /// Frame `c_duckdeath06`.
    pub const C_DUCKDEATH06: i32 = 775;
    /// Frame `c_duckdeath07`.
    pub const C_DUCKDEATH07: i32 = 776;
    /// Frame `c_duckdeath08`.
    pub const C_DUCKDEATH08: i32 = 777;
    /// Frame `c_duckdeath09`.
    pub const C_DUCKDEATH09: i32 = 778;
    /// Frame `c_duckdeath10`.
    pub const C_DUCKDEATH10: i32 = 779;
    /// Frame `c_duckdeath11`.
    pub const C_DUCKDEATH11: i32 = 780;
    /// Frame `c_duckdeath12`.
    pub const C_DUCKDEATH12: i32 = 781;
    /// Frame `c_duckdeath13`.
    pub const C_DUCKDEATH13: i32 = 782;
    /// Frame `c_duckdeath14`.
    pub const C_DUCKDEATH14: i32 = 783;
    /// Frame `c_duckdeath15`.
    pub const C_DUCKDEATH15: i32 = 784;
    /// Frame `c_duckdeath16`.
    pub const C_DUCKDEATH16: i32 = 785;
    /// Frame `c_duckdeath17`.
    pub const C_DUCKDEATH17: i32 = 786;
    /// Frame `c_duckdeath18`.
    pub const C_DUCKDEATH18: i32 = 787;
    /// Frame `c_duckdeath19`.
    pub const C_DUCKDEATH19: i32 = 788;
    /// Frame `c_duckdeath20`.
    pub const C_DUCKDEATH20: i32 = 789;
    /// Frame `c_duckdeath21`.
    pub const C_DUCKDEATH21: i32 = 790;
    /// Frame `c_duckdeath22`.
    pub const C_DUCKDEATH22: i32 = 791;
    /// Frame `c_duckdeath23`.
    pub const C_DUCKDEATH23: i32 = 792;
    /// Frame `c_duckdeath24`.
    pub const C_DUCKDEATH24: i32 = 793;
    /// Frame `c_duckdeath25`.
    pub const C_DUCKDEATH25: i32 = 794;
    /// Frame `c_duckdeath26`.
    pub const C_DUCKDEATH26: i32 = 795;
    /// Frame `c_duckdeath27`.
    pub const C_DUCKDEATH27: i32 = 796;
    /// Frame `c_duckdeath28`.
    pub const C_DUCKDEATH28: i32 = 797;
    /// Frame `c_duckdeath29`.
    pub const C_DUCKDEATH29: i32 = 798;
}

/// `gunnerMoves` move tables.
pub fn gunner_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("gunner_move_fidget", 30, 69, Some("gunner_stand"), vec![
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![MonsterAction::name("gunner_idlesound")], -1),
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
        monster_move("gunner_move_stand", 0, 29, None, vec![
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![MonsterAction::name("gunner_fidget")], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![MonsterAction::name("gunner_fidget")], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![MonsterAction::name("gunner_fidget")], -1),
        ]),
        monster_move("gunner_move_walk", 76, 88, None, vec![
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (2f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
        ]),
        monster_move("gunner_move_run", 94, 101, None, vec![
            monster_frame(MonsterAi::Run, (26f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (9f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Run, (9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (9f32) as f64, vec![MonsterAction::name("monster_done_dodge")], -1),
            monster_frame(MonsterAi::Run, (15f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (10f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Run, (13f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_runandshoot", 102, 107, None, vec![
            monster_frame(MonsterAi::Run, (32f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (15f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (18f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (20f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_pain3", 185, 189, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_pain2", 177, 184, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (11f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (6f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
        ]),
        monster_move("gunner_move_pain1", 159, 176, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
        ]),
        monster_move("gunner_move_death", 190, 200, Some("gunner_dead"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![MonsterAction::name("gunner_shrink")], -1),
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_duck", 201, 208, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![MonsterAction::name("monster_duck_down")], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![MonsterAction::name("monster_duck_hold")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("monster_duck_up")], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_attack_chain", 137, 143, Some("gunner_fire_chain"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("gunner_opengun")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_fire_chain", 144, 151, Some("gunner_refire_chain"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerFire")], -1),
        ]),
        monster_move("gunner_move_endfire_chain", 152, 158, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("monster_footstep")], -1),
        ]),
        monster_move("gunner_move_attack_grenade", 108, 128, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("gunner_blind_check")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_attack_grenade2", 229, 248, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("gunner_blind_check")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_jump", 209, 218, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("gunner_jump_now")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("gunner_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("gunner_move_jump2", 209, 218, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, (-8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("gunner_jump2_now")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("gunner_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
    ]
}
