//! boss32 move tables (`src/content/q2/rerelease/monsters/tables/boss32.ts`).

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `boss32Frame`.
pub mod boss32_frame {
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 0;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 1;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 2;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 3;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 4;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 5;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 6;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 7;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 8;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 9;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 10;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 11;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 12;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 13;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 14;
    /// Frame `attak116`.
    pub const ATTAK116: i32 = 15;
    /// Frame `attak117`.
    pub const ATTAK117: i32 = 16;
    /// Frame `attak118`.
    pub const ATTAK118: i32 = 17;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 18;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 19;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 20;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 21;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 22;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 23;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 24;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 25;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 26;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 27;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 28;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 29;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 30;
    /// Frame `death01`.
    pub const DEATH01: i32 = 31;
    /// Frame `death02`.
    pub const DEATH02: i32 = 32;
    /// Frame `death03`.
    pub const DEATH03: i32 = 33;
    /// Frame `death04`.
    pub const DEATH04: i32 = 34;
    /// Frame `death05`.
    pub const DEATH05: i32 = 35;
    /// Frame `death06`.
    pub const DEATH06: i32 = 36;
    /// Frame `death07`.
    pub const DEATH07: i32 = 37;
    /// Frame `death08`.
    pub const DEATH08: i32 = 38;
    /// Frame `death09`.
    pub const DEATH09: i32 = 39;
    /// Frame `death10`.
    pub const DEATH10: i32 = 40;
    /// Frame `death11`.
    pub const DEATH11: i32 = 41;
    /// Frame `death12`.
    pub const DEATH12: i32 = 42;
    /// Frame `death13`.
    pub const DEATH13: i32 = 43;
    /// Frame `death14`.
    pub const DEATH14: i32 = 44;
    /// Frame `death15`.
    pub const DEATH15: i32 = 45;
    /// Frame `death16`.
    pub const DEATH16: i32 = 46;
    /// Frame `death17`.
    pub const DEATH17: i32 = 47;
    /// Frame `death18`.
    pub const DEATH18: i32 = 48;
    /// Frame `death19`.
    pub const DEATH19: i32 = 49;
    /// Frame `death20`.
    pub const DEATH20: i32 = 50;
    /// Frame `death21`.
    pub const DEATH21: i32 = 51;
    /// Frame `death22`.
    pub const DEATH22: i32 = 52;
    /// Frame `death23`.
    pub const DEATH23: i32 = 53;
    /// Frame `death24`.
    pub const DEATH24: i32 = 54;
    /// Frame `death25`.
    pub const DEATH25: i32 = 55;
    /// Frame `death26`.
    pub const DEATH26: i32 = 56;
    /// Frame `death27`.
    pub const DEATH27: i32 = 57;
    /// Frame `death28`.
    pub const DEATH28: i32 = 58;
    /// Frame `death29`.
    pub const DEATH29: i32 = 59;
    /// Frame `death30`.
    pub const DEATH30: i32 = 60;
    /// Frame `death31`.
    pub const DEATH31: i32 = 61;
    /// Frame `death32`.
    pub const DEATH32: i32 = 62;
    /// Frame `death33`.
    pub const DEATH33: i32 = 63;
    /// Frame `death34`.
    pub const DEATH34: i32 = 64;
    /// Frame `death35`.
    pub const DEATH35: i32 = 65;
    /// Frame `death36`.
    pub const DEATH36: i32 = 66;
    /// Frame `death37`.
    pub const DEATH37: i32 = 67;
    /// Frame `death38`.
    pub const DEATH38: i32 = 68;
    /// Frame `death39`.
    pub const DEATH39: i32 = 69;
    /// Frame `death40`.
    pub const DEATH40: i32 = 70;
    /// Frame `death41`.
    pub const DEATH41: i32 = 71;
    /// Frame `death42`.
    pub const DEATH42: i32 = 72;
    /// Frame `death43`.
    pub const DEATH43: i32 = 73;
    /// Frame `death44`.
    pub const DEATH44: i32 = 74;
    /// Frame `death45`.
    pub const DEATH45: i32 = 75;
    /// Frame `death46`.
    pub const DEATH46: i32 = 76;
    /// Frame `death47`.
    pub const DEATH47: i32 = 77;
    /// Frame `death48`.
    pub const DEATH48: i32 = 78;
    /// Frame `death49`.
    pub const DEATH49: i32 = 79;
    /// Frame `death50`.
    pub const DEATH50: i32 = 80;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 81;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 82;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 83;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 84;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 85;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 86;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 87;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 88;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 89;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 90;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 91;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 92;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 93;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 94;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 95;
    /// Frame `pain310`.
    pub const PAIN310: i32 = 96;
    /// Frame `pain311`.
    pub const PAIN311: i32 = 97;
    /// Frame `pain312`.
    pub const PAIN312: i32 = 98;
    /// Frame `pain313`.
    pub const PAIN313: i32 = 99;
    /// Frame `pain314`.
    pub const PAIN314: i32 = 100;
    /// Frame `pain315`.
    pub const PAIN315: i32 = 101;
    /// Frame `pain316`.
    pub const PAIN316: i32 = 102;
    /// Frame `pain317`.
    pub const PAIN317: i32 = 103;
    /// Frame `pain318`.
    pub const PAIN318: i32 = 104;
    /// Frame `pain319`.
    pub const PAIN319: i32 = 105;
    /// Frame `pain320`.
    pub const PAIN320: i32 = 106;
    /// Frame `pain321`.
    pub const PAIN321: i32 = 107;
    /// Frame `pain322`.
    pub const PAIN322: i32 = 108;
    /// Frame `pain323`.
    pub const PAIN323: i32 = 109;
    /// Frame `pain324`.
    pub const PAIN324: i32 = 110;
    /// Frame `pain325`.
    pub const PAIN325: i32 = 111;
    /// Frame `stand01`.
    pub const STAND01: i32 = 112;
    /// Frame `stand02`.
    pub const STAND02: i32 = 113;
    /// Frame `stand03`.
    pub const STAND03: i32 = 114;
    /// Frame `stand04`.
    pub const STAND04: i32 = 115;
    /// Frame `stand05`.
    pub const STAND05: i32 = 116;
    /// Frame `stand06`.
    pub const STAND06: i32 = 117;
    /// Frame `stand07`.
    pub const STAND07: i32 = 118;
    /// Frame `stand08`.
    pub const STAND08: i32 = 119;
    /// Frame `stand09`.
    pub const STAND09: i32 = 120;
    /// Frame `stand10`.
    pub const STAND10: i32 = 121;
    /// Frame `stand11`.
    pub const STAND11: i32 = 122;
    /// Frame `stand12`.
    pub const STAND12: i32 = 123;
    /// Frame `stand13`.
    pub const STAND13: i32 = 124;
    /// Frame `stand14`.
    pub const STAND14: i32 = 125;
    /// Frame `stand15`.
    pub const STAND15: i32 = 126;
    /// Frame `stand16`.
    pub const STAND16: i32 = 127;
    /// Frame `stand17`.
    pub const STAND17: i32 = 128;
    /// Frame `stand18`.
    pub const STAND18: i32 = 129;
    /// Frame `stand19`.
    pub const STAND19: i32 = 130;
    /// Frame `stand20`.
    pub const STAND20: i32 = 131;
    /// Frame `stand21`.
    pub const STAND21: i32 = 132;
    /// Frame `stand22`.
    pub const STAND22: i32 = 133;
    /// Frame `stand23`.
    pub const STAND23: i32 = 134;
    /// Frame `stand24`.
    pub const STAND24: i32 = 135;
    /// Frame `stand25`.
    pub const STAND25: i32 = 136;
    /// Frame `stand26`.
    pub const STAND26: i32 = 137;
    /// Frame `stand27`.
    pub const STAND27: i32 = 138;
    /// Frame `stand28`.
    pub const STAND28: i32 = 139;
    /// Frame `stand29`.
    pub const STAND29: i32 = 140;
    /// Frame `stand30`.
    pub const STAND30: i32 = 141;
    /// Frame `stand31`.
    pub const STAND31: i32 = 142;
    /// Frame `stand32`.
    pub const STAND32: i32 = 143;
    /// Frame `stand33`.
    pub const STAND33: i32 = 144;
    /// Frame `stand34`.
    pub const STAND34: i32 = 145;
    /// Frame `stand35`.
    pub const STAND35: i32 = 146;
    /// Frame `stand36`.
    pub const STAND36: i32 = 147;
    /// Frame `stand37`.
    pub const STAND37: i32 = 148;
    /// Frame `stand38`.
    pub const STAND38: i32 = 149;
    /// Frame `stand39`.
    pub const STAND39: i32 = 150;
    /// Frame `stand40`.
    pub const STAND40: i32 = 151;
    /// Frame `stand41`.
    pub const STAND41: i32 = 152;
    /// Frame `stand42`.
    pub const STAND42: i32 = 153;
    /// Frame `stand43`.
    pub const STAND43: i32 = 154;
    /// Frame `stand44`.
    pub const STAND44: i32 = 155;
    /// Frame `stand45`.
    pub const STAND45: i32 = 156;
    /// Frame `stand46`.
    pub const STAND46: i32 = 157;
    /// Frame `stand47`.
    pub const STAND47: i32 = 158;
    /// Frame `stand48`.
    pub const STAND48: i32 = 159;
    /// Frame `stand49`.
    pub const STAND49: i32 = 160;
    /// Frame `stand50`.
    pub const STAND50: i32 = 161;
    /// Frame `stand51`.
    pub const STAND51: i32 = 162;
    /// Frame `walk01`.
    pub const WALK01: i32 = 163;
    /// Frame `walk02`.
    pub const WALK02: i32 = 164;
    /// Frame `walk03`.
    pub const WALK03: i32 = 165;
    /// Frame `walk04`.
    pub const WALK04: i32 = 166;
    /// Frame `walk05`.
    pub const WALK05: i32 = 167;
    /// Frame `walk06`.
    pub const WALK06: i32 = 168;
    /// Frame `walk07`.
    pub const WALK07: i32 = 169;
    /// Frame `walk08`.
    pub const WALK08: i32 = 170;
    /// Frame `walk09`.
    pub const WALK09: i32 = 171;
    /// Frame `walk10`.
    pub const WALK10: i32 = 172;
    /// Frame `walk11`.
    pub const WALK11: i32 = 173;
    /// Frame `walk12`.
    pub const WALK12: i32 = 174;
    /// Frame `walk13`.
    pub const WALK13: i32 = 175;
    /// Frame `walk14`.
    pub const WALK14: i32 = 176;
    /// Frame `walk15`.
    pub const WALK15: i32 = 177;
    /// Frame `walk16`.
    pub const WALK16: i32 = 178;
    /// Frame `walk17`.
    pub const WALK17: i32 = 179;
    /// Frame `walk18`.
    pub const WALK18: i32 = 180;
    /// Frame `walk19`.
    pub const WALK19: i32 = 181;
    /// Frame `walk20`.
    pub const WALK20: i32 = 182;
    /// Frame `walk21`.
    pub const WALK21: i32 = 183;
    /// Frame `walk22`.
    pub const WALK22: i32 = 184;
    /// Frame `walk23`.
    pub const WALK23: i32 = 185;
    /// Frame `walk24`.
    pub const WALK24: i32 = 186;
    /// Frame `walk25`.
    pub const WALK25: i32 = 187;
    /// Frame `active01`.
    pub const ACTIVE01: i32 = 188;
    /// Frame `active02`.
    pub const ACTIVE02: i32 = 189;
    /// Frame `active03`.
    pub const ACTIVE03: i32 = 190;
    /// Frame `active04`.
    pub const ACTIVE04: i32 = 191;
    /// Frame `active05`.
    pub const ACTIVE05: i32 = 192;
    /// Frame `active06`.
    pub const ACTIVE06: i32 = 193;
    /// Frame `active07`.
    pub const ACTIVE07: i32 = 194;
    /// Frame `active08`.
    pub const ACTIVE08: i32 = 195;
    /// Frame `active09`.
    pub const ACTIVE09: i32 = 196;
    /// Frame `active10`.
    pub const ACTIVE10: i32 = 197;
    /// Frame `active11`.
    pub const ACTIVE11: i32 = 198;
    /// Frame `active12`.
    pub const ACTIVE12: i32 = 199;
    /// Frame `active13`.
    pub const ACTIVE13: i32 = 200;
    /// Frame `attak301`.
    pub const ATTAK301: i32 = 201;
    /// Frame `attak302`.
    pub const ATTAK302: i32 = 202;
    /// Frame `attak303`.
    pub const ATTAK303: i32 = 203;
    /// Frame `attak304`.
    pub const ATTAK304: i32 = 204;
    /// Frame `attak305`.
    pub const ATTAK305: i32 = 205;
    /// Frame `attak306`.
    pub const ATTAK306: i32 = 206;
    /// Frame `attak307`.
    pub const ATTAK307: i32 = 207;
    /// Frame `attak308`.
    pub const ATTAK308: i32 = 208;
    /// Frame `attak401`.
    pub const ATTAK401: i32 = 209;
    /// Frame `attak402`.
    pub const ATTAK402: i32 = 210;
    /// Frame `attak403`.
    pub const ATTAK403: i32 = 211;
    /// Frame `attak404`.
    pub const ATTAK404: i32 = 212;
    /// Frame `attak405`.
    pub const ATTAK405: i32 = 213;
    /// Frame `attak406`.
    pub const ATTAK406: i32 = 214;
    /// Frame `attak407`.
    pub const ATTAK407: i32 = 215;
    /// Frame `attak408`.
    pub const ATTAK408: i32 = 216;
    /// Frame `attak409`.
    pub const ATTAK409: i32 = 217;
    /// Frame `attak410`.
    pub const ATTAK410: i32 = 218;
    /// Frame `attak411`.
    pub const ATTAK411: i32 = 219;
    /// Frame `attak412`.
    pub const ATTAK412: i32 = 220;
    /// Frame `attak413`.
    pub const ATTAK413: i32 = 221;
    /// Frame `attak414`.
    pub const ATTAK414: i32 = 222;
    /// Frame `attak415`.
    pub const ATTAK415: i32 = 223;
    /// Frame `attak416`.
    pub const ATTAK416: i32 = 224;
    /// Frame `attak417`.
    pub const ATTAK417: i32 = 225;
    /// Frame `attak418`.
    pub const ATTAK418: i32 = 226;
    /// Frame `attak419`.
    pub const ATTAK419: i32 = 227;
    /// Frame `attak420`.
    pub const ATTAK420: i32 = 228;
    /// Frame `attak421`.
    pub const ATTAK421: i32 = 229;
    /// Frame `attak422`.
    pub const ATTAK422: i32 = 230;
    /// Frame `attak423`.
    pub const ATTAK423: i32 = 231;
    /// Frame `attak424`.
    pub const ATTAK424: i32 = 232;
    /// Frame `attak425`.
    pub const ATTAK425: i32 = 233;
    /// Frame `attak426`.
    pub const ATTAK426: i32 = 234;
    /// Frame `attak501`.
    pub const ATTAK501: i32 = 235;
    /// Frame `attak502`.
    pub const ATTAK502: i32 = 236;
    /// Frame `attak503`.
    pub const ATTAK503: i32 = 237;
    /// Frame `attak504`.
    pub const ATTAK504: i32 = 238;
    /// Frame `attak505`.
    pub const ATTAK505: i32 = 239;
    /// Frame `attak506`.
    pub const ATTAK506: i32 = 240;
    /// Frame `attak507`.
    pub const ATTAK507: i32 = 241;
    /// Frame `attak508`.
    pub const ATTAK508: i32 = 242;
    /// Frame `attak509`.
    pub const ATTAK509: i32 = 243;
    /// Frame `attak510`.
    pub const ATTAK510: i32 = 244;
    /// Frame `attak511`.
    pub const ATTAK511: i32 = 245;
    /// Frame `attak512`.
    pub const ATTAK512: i32 = 246;
    /// Frame `attak513`.
    pub const ATTAK513: i32 = 247;
    /// Frame `attak514`.
    pub const ATTAK514: i32 = 248;
    /// Frame `attak515`.
    pub const ATTAK515: i32 = 249;
    /// Frame `attak516`.
    pub const ATTAK516: i32 = 250;
    /// Frame `death201`.
    pub const DEATH201: i32 = 251;
    /// Frame `death202`.
    pub const DEATH202: i32 = 252;
    /// Frame `death203`.
    pub const DEATH203: i32 = 253;
    /// Frame `death204`.
    pub const DEATH204: i32 = 254;
    /// Frame `death205`.
    pub const DEATH205: i32 = 255;
    /// Frame `death206`.
    pub const DEATH206: i32 = 256;
    /// Frame `death207`.
    pub const DEATH207: i32 = 257;
    /// Frame `death208`.
    pub const DEATH208: i32 = 258;
    /// Frame `death209`.
    pub const DEATH209: i32 = 259;
    /// Frame `death210`.
    pub const DEATH210: i32 = 260;
    /// Frame `death211`.
    pub const DEATH211: i32 = 261;
    /// Frame `death212`.
    pub const DEATH212: i32 = 262;
    /// Frame `death213`.
    pub const DEATH213: i32 = 263;
    /// Frame `death214`.
    pub const DEATH214: i32 = 264;
    /// Frame `death215`.
    pub const DEATH215: i32 = 265;
    /// Frame `death216`.
    pub const DEATH216: i32 = 266;
    /// Frame `death217`.
    pub const DEATH217: i32 = 267;
    /// Frame `death218`.
    pub const DEATH218: i32 = 268;
    /// Frame `death219`.
    pub const DEATH219: i32 = 269;
    /// Frame `death220`.
    pub const DEATH220: i32 = 270;
    /// Frame `death221`.
    pub const DEATH221: i32 = 271;
    /// Frame `death222`.
    pub const DEATH222: i32 = 272;
    /// Frame `death223`.
    pub const DEATH223: i32 = 273;
    /// Frame `death224`.
    pub const DEATH224: i32 = 274;
    /// Frame `death225`.
    pub const DEATH225: i32 = 275;
    /// Frame `death226`.
    pub const DEATH226: i32 = 276;
    /// Frame `death227`.
    pub const DEATH227: i32 = 277;
    /// Frame `death228`.
    pub const DEATH228: i32 = 278;
    /// Frame `death229`.
    pub const DEATH229: i32 = 279;
    /// Frame `death230`.
    pub const DEATH230: i32 = 280;
    /// Frame `death231`.
    pub const DEATH231: i32 = 281;
    /// Frame `death232`.
    pub const DEATH232: i32 = 282;
    /// Frame `death233`.
    pub const DEATH233: i32 = 283;
    /// Frame `death234`.
    pub const DEATH234: i32 = 284;
    /// Frame `death235`.
    pub const DEATH235: i32 = 285;
    /// Frame `death236`.
    pub const DEATH236: i32 = 286;
    /// Frame `death237`.
    pub const DEATH237: i32 = 287;
    /// Frame `death238`.
    pub const DEATH238: i32 = 288;
    /// Frame `death239`.
    pub const DEATH239: i32 = 289;
    /// Frame `death240`.
    pub const DEATH240: i32 = 290;
    /// Frame `death241`.
    pub const DEATH241: i32 = 291;
    /// Frame `death242`.
    pub const DEATH242: i32 = 292;
    /// Frame `death243`.
    pub const DEATH243: i32 = 293;
    /// Frame `death244`.
    pub const DEATH244: i32 = 294;
    /// Frame `death245`.
    pub const DEATH245: i32 = 295;
    /// Frame `death246`.
    pub const DEATH246: i32 = 296;
    /// Frame `death247`.
    pub const DEATH247: i32 = 297;
    /// Frame `death248`.
    pub const DEATH248: i32 = 298;
    /// Frame `death249`.
    pub const DEATH249: i32 = 299;
    /// Frame `death250`.
    pub const DEATH250: i32 = 300;
    /// Frame `death251`.
    pub const DEATH251: i32 = 301;
    /// Frame `death252`.
    pub const DEATH252: i32 = 302;
    /// Frame `death253`.
    pub const DEATH253: i32 = 303;
    /// Frame `death254`.
    pub const DEATH254: i32 = 304;
    /// Frame `death255`.
    pub const DEATH255: i32 = 305;
    /// Frame `death256`.
    pub const DEATH256: i32 = 306;
    /// Frame `death257`.
    pub const DEATH257: i32 = 307;
    /// Frame `death258`.
    pub const DEATH258: i32 = 308;
    /// Frame `death259`.
    pub const DEATH259: i32 = 309;
    /// Frame `death260`.
    pub const DEATH260: i32 = 310;
    /// Frame `death261`.
    pub const DEATH261: i32 = 311;
    /// Frame `death262`.
    pub const DEATH262: i32 = 312;
    /// Frame `death263`.
    pub const DEATH263: i32 = 313;
    /// Frame `death264`.
    pub const DEATH264: i32 = 314;
    /// Frame `death265`.
    pub const DEATH265: i32 = 315;
    /// Frame `death266`.
    pub const DEATH266: i32 = 316;
    /// Frame `death267`.
    pub const DEATH267: i32 = 317;
    /// Frame `death268`.
    pub const DEATH268: i32 = 318;
    /// Frame `death269`.
    pub const DEATH269: i32 = 319;
    /// Frame `death270`.
    pub const DEATH270: i32 = 320;
    /// Frame `death271`.
    pub const DEATH271: i32 = 321;
    /// Frame `death272`.
    pub const DEATH272: i32 = 322;
    /// Frame `death273`.
    pub const DEATH273: i32 = 323;
    /// Frame `death274`.
    pub const DEATH274: i32 = 324;
    /// Frame `death275`.
    pub const DEATH275: i32 = 325;
    /// Frame `death276`.
    pub const DEATH276: i32 = 326;
    /// Frame `death277`.
    pub const DEATH277: i32 = 327;
    /// Frame `death278`.
    pub const DEATH278: i32 = 328;
    /// Frame `death279`.
    pub const DEATH279: i32 = 329;
    /// Frame `death280`.
    pub const DEATH280: i32 = 330;
    /// Frame `death281`.
    pub const DEATH281: i32 = 331;
    /// Frame `death282`.
    pub const DEATH282: i32 = 332;
    /// Frame `death283`.
    pub const DEATH283: i32 = 333;
    /// Frame `death284`.
    pub const DEATH284: i32 = 334;
    /// Frame `death285`.
    pub const DEATH285: i32 = 335;
    /// Frame `death286`.
    pub const DEATH286: i32 = 336;
    /// Frame `death287`.
    pub const DEATH287: i32 = 337;
    /// Frame `death288`.
    pub const DEATH288: i32 = 338;
    /// Frame `death289`.
    pub const DEATH289: i32 = 339;
    /// Frame `death290`.
    pub const DEATH290: i32 = 340;
    /// Frame `death291`.
    pub const DEATH291: i32 = 341;
    /// Frame `death292`.
    pub const DEATH292: i32 = 342;
    /// Frame `death293`.
    pub const DEATH293: i32 = 343;
    /// Frame `death294`.
    pub const DEATH294: i32 = 344;
    /// Frame `death295`.
    pub const DEATH295: i32 = 345;
    /// Frame `death301`.
    pub const DEATH301: i32 = 346;
    /// Frame `death302`.
    pub const DEATH302: i32 = 347;
    /// Frame `death303`.
    pub const DEATH303: i32 = 348;
    /// Frame `death304`.
    pub const DEATH304: i32 = 349;
    /// Frame `death305`.
    pub const DEATH305: i32 = 350;
    /// Frame `death306`.
    pub const DEATH306: i32 = 351;
    /// Frame `death307`.
    pub const DEATH307: i32 = 352;
    /// Frame `death308`.
    pub const DEATH308: i32 = 353;
    /// Frame `death309`.
    pub const DEATH309: i32 = 354;
    /// Frame `death310`.
    pub const DEATH310: i32 = 355;
    /// Frame `death311`.
    pub const DEATH311: i32 = 356;
    /// Frame `death312`.
    pub const DEATH312: i32 = 357;
    /// Frame `death313`.
    pub const DEATH313: i32 = 358;
    /// Frame `death314`.
    pub const DEATH314: i32 = 359;
    /// Frame `death315`.
    pub const DEATH315: i32 = 360;
    /// Frame `death316`.
    pub const DEATH316: i32 = 361;
    /// Frame `death317`.
    pub const DEATH317: i32 = 362;
    /// Frame `death318`.
    pub const DEATH318: i32 = 363;
    /// Frame `death319`.
    pub const DEATH319: i32 = 364;
    /// Frame `death320`.
    pub const DEATH320: i32 = 365;
    /// Frame `jump01`.
    pub const JUMP01: i32 = 366;
    /// Frame `jump02`.
    pub const JUMP02: i32 = 367;
    /// Frame `jump03`.
    pub const JUMP03: i32 = 368;
    /// Frame `jump04`.
    pub const JUMP04: i32 = 369;
    /// Frame `jump05`.
    pub const JUMP05: i32 = 370;
    /// Frame `jump06`.
    pub const JUMP06: i32 = 371;
    /// Frame `jump07`.
    pub const JUMP07: i32 = 372;
    /// Frame `jump08`.
    pub const JUMP08: i32 = 373;
    /// Frame `jump09`.
    pub const JUMP09: i32 = 374;
    /// Frame `jump10`.
    pub const JUMP10: i32 = 375;
    /// Frame `jump11`.
    pub const JUMP11: i32 = 376;
    /// Frame `jump12`.
    pub const JUMP12: i32 = 377;
    /// Frame `jump13`.
    pub const JUMP13: i32 = 378;
    /// Frame `pain401`.
    pub const PAIN401: i32 = 379;
    /// Frame `pain402`.
    pub const PAIN402: i32 = 380;
    /// Frame `pain403`.
    pub const PAIN403: i32 = 381;
    /// Frame `pain404`.
    pub const PAIN404: i32 = 382;
    /// Frame `pain501`.
    pub const PAIN501: i32 = 383;
    /// Frame `pain502`.
    pub const PAIN502: i32 = 384;
    /// Frame `pain503`.
    pub const PAIN503: i32 = 385;
    /// Frame `pain504`.
    pub const PAIN504: i32 = 386;
    /// Frame `pain601`.
    pub const PAIN601: i32 = 387;
    /// Frame `pain602`.
    pub const PAIN602: i32 = 388;
    /// Frame `pain603`.
    pub const PAIN603: i32 = 389;
    /// Frame `pain604`.
    pub const PAIN604: i32 = 390;
    /// Frame `pain605`.
    pub const PAIN605: i32 = 391;
    /// Frame `pain606`.
    pub const PAIN606: i32 = 392;
    /// Frame `pain607`.
    pub const PAIN607: i32 = 393;
    /// Frame `pain608`.
    pub const PAIN608: i32 = 394;
    /// Frame `pain609`.
    pub const PAIN609: i32 = 395;
    /// Frame `pain610`.
    pub const PAIN610: i32 = 396;
    /// Frame `pain611`.
    pub const PAIN611: i32 = 397;
    /// Frame `pain612`.
    pub const PAIN612: i32 = 398;
    /// Frame `pain613`.
    pub const PAIN613: i32 = 399;
    /// Frame `pain614`.
    pub const PAIN614: i32 = 400;
    /// Frame `pain615`.
    pub const PAIN615: i32 = 401;
    /// Frame `pain616`.
    pub const PAIN616: i32 = 402;
    /// Frame `pain617`.
    pub const PAIN617: i32 = 403;
    /// Frame `pain618`.
    pub const PAIN618: i32 = 404;
    /// Frame `pain619`.
    pub const PAIN619: i32 = 405;
    /// Frame `pain620`.
    pub const PAIN620: i32 = 406;
    /// Frame `pain621`.
    pub const PAIN621: i32 = 407;
    /// Frame `pain622`.
    pub const PAIN622: i32 = 408;
    /// Frame `pain623`.
    pub const PAIN623: i32 = 409;
    /// Frame `pain624`.
    pub const PAIN624: i32 = 410;
    /// Frame `pain625`.
    pub const PAIN625: i32 = 411;
    /// Frame `pain626`.
    pub const PAIN626: i32 = 412;
    /// Frame `pain627`.
    pub const PAIN627: i32 = 413;
    /// Frame `stand201`.
    pub const STAND201: i32 = 414;
    /// Frame `stand202`.
    pub const STAND202: i32 = 415;
    /// Frame `stand203`.
    pub const STAND203: i32 = 416;
    /// Frame `stand204`.
    pub const STAND204: i32 = 417;
    /// Frame `stand205`.
    pub const STAND205: i32 = 418;
    /// Frame `stand206`.
    pub const STAND206: i32 = 419;
    /// Frame `stand207`.
    pub const STAND207: i32 = 420;
    /// Frame `stand208`.
    pub const STAND208: i32 = 421;
    /// Frame `stand209`.
    pub const STAND209: i32 = 422;
    /// Frame `stand210`.
    pub const STAND210: i32 = 423;
    /// Frame `stand211`.
    pub const STAND211: i32 = 424;
    /// Frame `stand212`.
    pub const STAND212: i32 = 425;
    /// Frame `stand213`.
    pub const STAND213: i32 = 426;
    /// Frame `stand214`.
    pub const STAND214: i32 = 427;
    /// Frame `stand215`.
    pub const STAND215: i32 = 428;
    /// Frame `stand216`.
    pub const STAND216: i32 = 429;
    /// Frame `stand217`.
    pub const STAND217: i32 = 430;
    /// Frame `stand218`.
    pub const STAND218: i32 = 431;
    /// Frame `stand219`.
    pub const STAND219: i32 = 432;
    /// Frame `stand220`.
    pub const STAND220: i32 = 433;
    /// Frame `stand221`.
    pub const STAND221: i32 = 434;
    /// Frame `stand222`.
    pub const STAND222: i32 = 435;
    /// Frame `stand223`.
    pub const STAND223: i32 = 436;
    /// Frame `stand224`.
    pub const STAND224: i32 = 437;
    /// Frame `stand225`.
    pub const STAND225: i32 = 438;
    /// Frame `stand226`.
    pub const STAND226: i32 = 439;
    /// Frame `stand227`.
    pub const STAND227: i32 = 440;
    /// Frame `stand228`.
    pub const STAND228: i32 = 441;
    /// Frame `stand229`.
    pub const STAND229: i32 = 442;
    /// Frame `stand230`.
    pub const STAND230: i32 = 443;
    /// Frame `stand231`.
    pub const STAND231: i32 = 444;
    /// Frame `stand232`.
    pub const STAND232: i32 = 445;
    /// Frame `stand233`.
    pub const STAND233: i32 = 446;
    /// Frame `stand234`.
    pub const STAND234: i32 = 447;
    /// Frame `stand235`.
    pub const STAND235: i32 = 448;
    /// Frame `stand236`.
    pub const STAND236: i32 = 449;
    /// Frame `stand237`.
    pub const STAND237: i32 = 450;
    /// Frame `stand238`.
    pub const STAND238: i32 = 451;
    /// Frame `stand239`.
    pub const STAND239: i32 = 452;
    /// Frame `stand240`.
    pub const STAND240: i32 = 453;
    /// Frame `stand241`.
    pub const STAND241: i32 = 454;
    /// Frame `stand242`.
    pub const STAND242: i32 = 455;
    /// Frame `stand243`.
    pub const STAND243: i32 = 456;
    /// Frame `stand244`.
    pub const STAND244: i32 = 457;
    /// Frame `stand245`.
    pub const STAND245: i32 = 458;
    /// Frame `stand246`.
    pub const STAND246: i32 = 459;
    /// Frame `stand247`.
    pub const STAND247: i32 = 460;
    /// Frame `stand248`.
    pub const STAND248: i32 = 461;
    /// Frame `stand249`.
    pub const STAND249: i32 = 462;
    /// Frame `stand250`.
    pub const STAND250: i32 = 463;
    /// Frame `stand251`.
    pub const STAND251: i32 = 464;
    /// Frame `stand252`.
    pub const STAND252: i32 = 465;
    /// Frame `stand253`.
    pub const STAND253: i32 = 466;
    /// Frame `stand254`.
    pub const STAND254: i32 = 467;
    /// Frame `stand255`.
    pub const STAND255: i32 = 468;
    /// Frame `stand256`.
    pub const STAND256: i32 = 469;
    /// Frame `stand257`.
    pub const STAND257: i32 = 470;
    /// Frame `stand258`.
    pub const STAND258: i32 = 471;
    /// Frame `stand259`.
    pub const STAND259: i32 = 472;
    /// Frame `stand260`.
    pub const STAND260: i32 = 473;
    /// Frame `walk201`.
    pub const WALK201: i32 = 474;
    /// Frame `walk202`.
    pub const WALK202: i32 = 475;
    /// Frame `walk203`.
    pub const WALK203: i32 = 476;
    /// Frame `walk204`.
    pub const WALK204: i32 = 477;
    /// Frame `walk205`.
    pub const WALK205: i32 = 478;
    /// Frame `walk206`.
    pub const WALK206: i32 = 479;
    /// Frame `walk207`.
    pub const WALK207: i32 = 480;
    /// Frame `walk208`.
    pub const WALK208: i32 = 481;
    /// Frame `walk209`.
    pub const WALK209: i32 = 482;
    /// Frame `walk210`.
    pub const WALK210: i32 = 483;
    /// Frame `walk211`.
    pub const WALK211: i32 = 484;
    /// Frame `walk212`.
    pub const WALK212: i32 = 485;
    /// Frame `walk213`.
    pub const WALK213: i32 = 486;
    /// Frame `walk214`.
    pub const WALK214: i32 = 487;
    /// Frame `walk215`.
    pub const WALK215: i32 = 488;
    /// Frame `walk216`.
    pub const WALK216: i32 = 489;
    /// Frame `walk217`.
    pub const WALK217: i32 = 490;
}

/// `boss32Moves` move tables.
pub fn boss32_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "makron_move_stand",
            414,
            473,
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
            ],
        ),
        monster_move(
            "makron_move_run",
            477,
            486,
            None,
            vec![
                monster_frame(
                    MonsterAi::Run,
                    (3f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Run,
                    (8f32) as f64,
                    vec![MonsterAction::name("makron_step_right")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (9f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_walk",
            477,
            486,
            None,
            vec![
                monster_frame(
                    MonsterAi::Run,
                    (3f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Run,
                    (8f32) as f64,
                    vec![MonsterAction::name("makron_step_right")],
                    -1,
                ),
                monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (9f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_pain6",
            387,
            413,
            Some("makron_run"),
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
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("makron_popup")],
                    -1,
                ),
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
                    vec![MonsterAction::name("makron_taunt")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_pain5",
            383,
            386,
            Some("makron_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_pain4",
            379,
            382,
            Some("makron_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_death2",
            251,
            345,
            Some("makron_dead"),
            vec![
                monster_frame(MonsterAi::Move, (-15f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-12f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
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
                monster_frame(MonsterAi::Move, (11f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (12f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (11f32) as f64,
                    vec![MonsterAction::name("makron_step_right")],
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
                monster_frame(MonsterAi::Move, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (7f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (6f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
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
                monster_frame(MonsterAi::Move, (-6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (-6f32) as f64,
                    vec![MonsterAction::name("makron_step_right")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (-4f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (-3f32) as f64,
                    vec![MonsterAction::name("makron_step_right")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (-8f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (-3f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (-7f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (-4f32) as f64,
                    vec![MonsterAction::name("makron_step_right")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (-6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (-7f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("makron_step_left")],
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
                monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (27f32) as f64,
                    vec![MonsterAction::name("makron_hit")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (26f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("makron_brainsplorch")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_death3",
            346,
            365,
            None,
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
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_sight",
            188,
            200,
            Some("makron_run"),
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
            ],
        ),
        monster_move(
            "makron_move_attack3",
            201,
            208,
            Some("makron_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("makronBFG")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_attack4",
            209,
            234,
            Some("makron_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronHyperblaster")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "makron_move_attack5",
            235,
            250,
            Some("makron_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("makron_prerailgun")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronSaveloc")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("MakronRailgun")],
                    -1,
                ),
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
