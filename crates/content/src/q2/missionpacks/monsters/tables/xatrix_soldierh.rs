//! soldierh move tables (`src/content/q2/missionpacks/monsters/tables/xatrix-soldierh.ts`).
//!
//! Original Quake II xatrix/m_soldier.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `soldierhFrame`.
pub mod soldierh_frame {
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
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 12;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 13;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 14;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 15;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 16;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 17;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 18;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 19;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 20;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 21;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 22;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 23;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 24;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 25;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 26;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 27;
    /// Frame `attak217`.
    pub const ATTAK217: i32 = 28;
    /// Frame `attak218`.
    pub const ATTAK218: i32 = 29;
    /// Frame `attak301`.
    pub const ATTAK301: i32 = 30;
    /// Frame `attak302`.
    pub const ATTAK302: i32 = 31;
    /// Frame `attak303`.
    pub const ATTAK303: i32 = 32;
    /// Frame `attak304`.
    pub const ATTAK304: i32 = 33;
    /// Frame `attak305`.
    pub const ATTAK305: i32 = 34;
    /// Frame `attak306`.
    pub const ATTAK306: i32 = 35;
    /// Frame `attak307`.
    pub const ATTAK307: i32 = 36;
    /// Frame `attak308`.
    pub const ATTAK308: i32 = 37;
    /// Frame `attak309`.
    pub const ATTAK309: i32 = 38;
    /// Frame `attak401`.
    pub const ATTAK401: i32 = 39;
    /// Frame `attak402`.
    pub const ATTAK402: i32 = 40;
    /// Frame `attak403`.
    pub const ATTAK403: i32 = 41;
    /// Frame `attak404`.
    pub const ATTAK404: i32 = 42;
    /// Frame `attak405`.
    pub const ATTAK405: i32 = 43;
    /// Frame `attak406`.
    pub const ATTAK406: i32 = 44;
    /// Frame `duck01`.
    pub const DUCK01: i32 = 45;
    /// Frame `duck02`.
    pub const DUCK02: i32 = 46;
    /// Frame `duck03`.
    pub const DUCK03: i32 = 47;
    /// Frame `duck04`.
    pub const DUCK04: i32 = 48;
    /// Frame `duck05`.
    pub const DUCK05: i32 = 49;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 50;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 51;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 52;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 53;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 54;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 55;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 56;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 57;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 58;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 59;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 60;
    /// Frame `pain207`.
    pub const PAIN207: i32 = 61;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 62;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 63;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 64;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 65;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 66;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 67;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 68;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 69;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 70;
    /// Frame `pain310`.
    pub const PAIN310: i32 = 71;
    /// Frame `pain311`.
    pub const PAIN311: i32 = 72;
    /// Frame `pain312`.
    pub const PAIN312: i32 = 73;
    /// Frame `pain313`.
    pub const PAIN313: i32 = 74;
    /// Frame `pain314`.
    pub const PAIN314: i32 = 75;
    /// Frame `pain315`.
    pub const PAIN315: i32 = 76;
    /// Frame `pain316`.
    pub const PAIN316: i32 = 77;
    /// Frame `pain317`.
    pub const PAIN317: i32 = 78;
    /// Frame `pain318`.
    pub const PAIN318: i32 = 79;
    /// Frame `pain401`.
    pub const PAIN401: i32 = 80;
    /// Frame `pain402`.
    pub const PAIN402: i32 = 81;
    /// Frame `pain403`.
    pub const PAIN403: i32 = 82;
    /// Frame `pain404`.
    pub const PAIN404: i32 = 83;
    /// Frame `pain405`.
    pub const PAIN405: i32 = 84;
    /// Frame `pain406`.
    pub const PAIN406: i32 = 85;
    /// Frame `pain407`.
    pub const PAIN407: i32 = 86;
    /// Frame `pain408`.
    pub const PAIN408: i32 = 87;
    /// Frame `pain409`.
    pub const PAIN409: i32 = 88;
    /// Frame `pain410`.
    pub const PAIN410: i32 = 89;
    /// Frame `pain411`.
    pub const PAIN411: i32 = 90;
    /// Frame `pain412`.
    pub const PAIN412: i32 = 91;
    /// Frame `pain413`.
    pub const PAIN413: i32 = 92;
    /// Frame `pain414`.
    pub const PAIN414: i32 = 93;
    /// Frame `pain415`.
    pub const PAIN415: i32 = 94;
    /// Frame `pain416`.
    pub const PAIN416: i32 = 95;
    /// Frame `pain417`.
    pub const PAIN417: i32 = 96;
    /// Frame `run01`.
    pub const RUN01: i32 = 97;
    /// Frame `run02`.
    pub const RUN02: i32 = 98;
    /// Frame `run03`.
    pub const RUN03: i32 = 99;
    /// Frame `run04`.
    pub const RUN04: i32 = 100;
    /// Frame `run05`.
    pub const RUN05: i32 = 101;
    /// Frame `run06`.
    pub const RUN06: i32 = 102;
    /// Frame `run07`.
    pub const RUN07: i32 = 103;
    /// Frame `run08`.
    pub const RUN08: i32 = 104;
    /// Frame `run09`.
    pub const RUN09: i32 = 105;
    /// Frame `run10`.
    pub const RUN10: i32 = 106;
    /// Frame `run11`.
    pub const RUN11: i32 = 107;
    /// Frame `run12`.
    pub const RUN12: i32 = 108;
    /// Frame `runs01`.
    pub const RUNS01: i32 = 109;
    /// Frame `runs02`.
    pub const RUNS02: i32 = 110;
    /// Frame `runs03`.
    pub const RUNS03: i32 = 111;
    /// Frame `runs04`.
    pub const RUNS04: i32 = 112;
    /// Frame `runs05`.
    pub const RUNS05: i32 = 113;
    /// Frame `runs06`.
    pub const RUNS06: i32 = 114;
    /// Frame `runs07`.
    pub const RUNS07: i32 = 115;
    /// Frame `runs08`.
    pub const RUNS08: i32 = 116;
    /// Frame `runs09`.
    pub const RUNS09: i32 = 117;
    /// Frame `runs10`.
    pub const RUNS10: i32 = 118;
    /// Frame `runs11`.
    pub const RUNS11: i32 = 119;
    /// Frame `runs12`.
    pub const RUNS12: i32 = 120;
    /// Frame `runs13`.
    pub const RUNS13: i32 = 121;
    /// Frame `runs14`.
    pub const RUNS14: i32 = 122;
    /// Frame `runs15`.
    pub const RUNS15: i32 = 123;
    /// Frame `runs16`.
    pub const RUNS16: i32 = 124;
    /// Frame `runs17`.
    pub const RUNS17: i32 = 125;
    /// Frame `runs18`.
    pub const RUNS18: i32 = 126;
    /// Frame `runt01`.
    pub const RUNT01: i32 = 127;
    /// Frame `runt02`.
    pub const RUNT02: i32 = 128;
    /// Frame `runt03`.
    pub const RUNT03: i32 = 129;
    /// Frame `runt04`.
    pub const RUNT04: i32 = 130;
    /// Frame `runt05`.
    pub const RUNT05: i32 = 131;
    /// Frame `runt06`.
    pub const RUNT06: i32 = 132;
    /// Frame `runt07`.
    pub const RUNT07: i32 = 133;
    /// Frame `runt08`.
    pub const RUNT08: i32 = 134;
    /// Frame `runt09`.
    pub const RUNT09: i32 = 135;
    /// Frame `runt10`.
    pub const RUNT10: i32 = 136;
    /// Frame `runt11`.
    pub const RUNT11: i32 = 137;
    /// Frame `runt12`.
    pub const RUNT12: i32 = 138;
    /// Frame `runt13`.
    pub const RUNT13: i32 = 139;
    /// Frame `runt14`.
    pub const RUNT14: i32 = 140;
    /// Frame `runt15`.
    pub const RUNT15: i32 = 141;
    /// Frame `runt16`.
    pub const RUNT16: i32 = 142;
    /// Frame `runt17`.
    pub const RUNT17: i32 = 143;
    /// Frame `runt18`.
    pub const RUNT18: i32 = 144;
    /// Frame `runt19`.
    pub const RUNT19: i32 = 145;
    /// Frame `stand101`.
    pub const STAND101: i32 = 146;
    /// Frame `stand102`.
    pub const STAND102: i32 = 147;
    /// Frame `stand103`.
    pub const STAND103: i32 = 148;
    /// Frame `stand104`.
    pub const STAND104: i32 = 149;
    /// Frame `stand105`.
    pub const STAND105: i32 = 150;
    /// Frame `stand106`.
    pub const STAND106: i32 = 151;
    /// Frame `stand107`.
    pub const STAND107: i32 = 152;
    /// Frame `stand108`.
    pub const STAND108: i32 = 153;
    /// Frame `stand109`.
    pub const STAND109: i32 = 154;
    /// Frame `stand110`.
    pub const STAND110: i32 = 155;
    /// Frame `stand111`.
    pub const STAND111: i32 = 156;
    /// Frame `stand112`.
    pub const STAND112: i32 = 157;
    /// Frame `stand113`.
    pub const STAND113: i32 = 158;
    /// Frame `stand114`.
    pub const STAND114: i32 = 159;
    /// Frame `stand115`.
    pub const STAND115: i32 = 160;
    /// Frame `stand116`.
    pub const STAND116: i32 = 161;
    /// Frame `stand117`.
    pub const STAND117: i32 = 162;
    /// Frame `stand118`.
    pub const STAND118: i32 = 163;
    /// Frame `stand119`.
    pub const STAND119: i32 = 164;
    /// Frame `stand120`.
    pub const STAND120: i32 = 165;
    /// Frame `stand121`.
    pub const STAND121: i32 = 166;
    /// Frame `stand122`.
    pub const STAND122: i32 = 167;
    /// Frame `stand123`.
    pub const STAND123: i32 = 168;
    /// Frame `stand124`.
    pub const STAND124: i32 = 169;
    /// Frame `stand125`.
    pub const STAND125: i32 = 170;
    /// Frame `stand126`.
    pub const STAND126: i32 = 171;
    /// Frame `stand127`.
    pub const STAND127: i32 = 172;
    /// Frame `stand128`.
    pub const STAND128: i32 = 173;
    /// Frame `stand129`.
    pub const STAND129: i32 = 174;
    /// Frame `stand130`.
    pub const STAND130: i32 = 175;
    /// Frame `stand301`.
    pub const STAND301: i32 = 176;
    /// Frame `stand302`.
    pub const STAND302: i32 = 177;
    /// Frame `stand303`.
    pub const STAND303: i32 = 178;
    /// Frame `stand304`.
    pub const STAND304: i32 = 179;
    /// Frame `stand305`.
    pub const STAND305: i32 = 180;
    /// Frame `stand306`.
    pub const STAND306: i32 = 181;
    /// Frame `stand307`.
    pub const STAND307: i32 = 182;
    /// Frame `stand308`.
    pub const STAND308: i32 = 183;
    /// Frame `stand309`.
    pub const STAND309: i32 = 184;
    /// Frame `stand310`.
    pub const STAND310: i32 = 185;
    /// Frame `stand311`.
    pub const STAND311: i32 = 186;
    /// Frame `stand312`.
    pub const STAND312: i32 = 187;
    /// Frame `stand313`.
    pub const STAND313: i32 = 188;
    /// Frame `stand314`.
    pub const STAND314: i32 = 189;
    /// Frame `stand315`.
    pub const STAND315: i32 = 190;
    /// Frame `stand316`.
    pub const STAND316: i32 = 191;
    /// Frame `stand317`.
    pub const STAND317: i32 = 192;
    /// Frame `stand318`.
    pub const STAND318: i32 = 193;
    /// Frame `stand319`.
    pub const STAND319: i32 = 194;
    /// Frame `stand320`.
    pub const STAND320: i32 = 195;
    /// Frame `stand321`.
    pub const STAND321: i32 = 196;
    /// Frame `stand322`.
    pub const STAND322: i32 = 197;
    /// Frame `stand323`.
    pub const STAND323: i32 = 198;
    /// Frame `stand324`.
    pub const STAND324: i32 = 199;
    /// Frame `stand325`.
    pub const STAND325: i32 = 200;
    /// Frame `stand326`.
    pub const STAND326: i32 = 201;
    /// Frame `stand327`.
    pub const STAND327: i32 = 202;
    /// Frame `stand328`.
    pub const STAND328: i32 = 203;
    /// Frame `stand329`.
    pub const STAND329: i32 = 204;
    /// Frame `stand330`.
    pub const STAND330: i32 = 205;
    /// Frame `stand331`.
    pub const STAND331: i32 = 206;
    /// Frame `stand332`.
    pub const STAND332: i32 = 207;
    /// Frame `stand333`.
    pub const STAND333: i32 = 208;
    /// Frame `stand334`.
    pub const STAND334: i32 = 209;
    /// Frame `stand335`.
    pub const STAND335: i32 = 210;
    /// Frame `stand336`.
    pub const STAND336: i32 = 211;
    /// Frame `stand337`.
    pub const STAND337: i32 = 212;
    /// Frame `stand338`.
    pub const STAND338: i32 = 213;
    /// Frame `stand339`.
    pub const STAND339: i32 = 214;
    /// Frame `walk101`.
    pub const WALK101: i32 = 215;
    /// Frame `walk102`.
    pub const WALK102: i32 = 216;
    /// Frame `walk103`.
    pub const WALK103: i32 = 217;
    /// Frame `walk104`.
    pub const WALK104: i32 = 218;
    /// Frame `walk105`.
    pub const WALK105: i32 = 219;
    /// Frame `walk106`.
    pub const WALK106: i32 = 220;
    /// Frame `walk107`.
    pub const WALK107: i32 = 221;
    /// Frame `walk108`.
    pub const WALK108: i32 = 222;
    /// Frame `walk109`.
    pub const WALK109: i32 = 223;
    /// Frame `walk110`.
    pub const WALK110: i32 = 224;
    /// Frame `walk111`.
    pub const WALK111: i32 = 225;
    /// Frame `walk112`.
    pub const WALK112: i32 = 226;
    /// Frame `walk113`.
    pub const WALK113: i32 = 227;
    /// Frame `walk114`.
    pub const WALK114: i32 = 228;
    /// Frame `walk115`.
    pub const WALK115: i32 = 229;
    /// Frame `walk116`.
    pub const WALK116: i32 = 230;
    /// Frame `walk117`.
    pub const WALK117: i32 = 231;
    /// Frame `walk118`.
    pub const WALK118: i32 = 232;
    /// Frame `walk119`.
    pub const WALK119: i32 = 233;
    /// Frame `walk120`.
    pub const WALK120: i32 = 234;
    /// Frame `walk121`.
    pub const WALK121: i32 = 235;
    /// Frame `walk122`.
    pub const WALK122: i32 = 236;
    /// Frame `walk123`.
    pub const WALK123: i32 = 237;
    /// Frame `walk124`.
    pub const WALK124: i32 = 238;
    /// Frame `walk125`.
    pub const WALK125: i32 = 239;
    /// Frame `walk126`.
    pub const WALK126: i32 = 240;
    /// Frame `walk127`.
    pub const WALK127: i32 = 241;
    /// Frame `walk128`.
    pub const WALK128: i32 = 242;
    /// Frame `walk129`.
    pub const WALK129: i32 = 243;
    /// Frame `walk130`.
    pub const WALK130: i32 = 244;
    /// Frame `walk131`.
    pub const WALK131: i32 = 245;
    /// Frame `walk132`.
    pub const WALK132: i32 = 246;
    /// Frame `walk133`.
    pub const WALK133: i32 = 247;
    /// Frame `walk201`.
    pub const WALK201: i32 = 248;
    /// Frame `walk202`.
    pub const WALK202: i32 = 249;
    /// Frame `walk203`.
    pub const WALK203: i32 = 250;
    /// Frame `walk204`.
    pub const WALK204: i32 = 251;
    /// Frame `walk205`.
    pub const WALK205: i32 = 252;
    /// Frame `walk206`.
    pub const WALK206: i32 = 253;
    /// Frame `walk207`.
    pub const WALK207: i32 = 254;
    /// Frame `walk208`.
    pub const WALK208: i32 = 255;
    /// Frame `walk209`.
    pub const WALK209: i32 = 256;
    /// Frame `walk210`.
    pub const WALK210: i32 = 257;
    /// Frame `walk211`.
    pub const WALK211: i32 = 258;
    /// Frame `walk212`.
    pub const WALK212: i32 = 259;
    /// Frame `walk213`.
    pub const WALK213: i32 = 260;
    /// Frame `walk214`.
    pub const WALK214: i32 = 261;
    /// Frame `walk215`.
    pub const WALK215: i32 = 262;
    /// Frame `walk216`.
    pub const WALK216: i32 = 263;
    /// Frame `walk217`.
    pub const WALK217: i32 = 264;
    /// Frame `walk218`.
    pub const WALK218: i32 = 265;
    /// Frame `walk219`.
    pub const WALK219: i32 = 266;
    /// Frame `walk220`.
    pub const WALK220: i32 = 267;
    /// Frame `walk221`.
    pub const WALK221: i32 = 268;
    /// Frame `walk222`.
    pub const WALK222: i32 = 269;
    /// Frame `walk223`.
    pub const WALK223: i32 = 270;
    /// Frame `walk224`.
    pub const WALK224: i32 = 271;
    /// Frame `death101`.
    pub const DEATH101: i32 = 272;
    /// Frame `death102`.
    pub const DEATH102: i32 = 273;
    /// Frame `death103`.
    pub const DEATH103: i32 = 274;
    /// Frame `death104`.
    pub const DEATH104: i32 = 275;
    /// Frame `death105`.
    pub const DEATH105: i32 = 276;
    /// Frame `death106`.
    pub const DEATH106: i32 = 277;
    /// Frame `death107`.
    pub const DEATH107: i32 = 278;
    /// Frame `death108`.
    pub const DEATH108: i32 = 279;
    /// Frame `death109`.
    pub const DEATH109: i32 = 280;
    /// Frame `death110`.
    pub const DEATH110: i32 = 281;
    /// Frame `death111`.
    pub const DEATH111: i32 = 282;
    /// Frame `death112`.
    pub const DEATH112: i32 = 283;
    /// Frame `death113`.
    pub const DEATH113: i32 = 284;
    /// Frame `death114`.
    pub const DEATH114: i32 = 285;
    /// Frame `death115`.
    pub const DEATH115: i32 = 286;
    /// Frame `death116`.
    pub const DEATH116: i32 = 287;
    /// Frame `death117`.
    pub const DEATH117: i32 = 288;
    /// Frame `death118`.
    pub const DEATH118: i32 = 289;
    /// Frame `death119`.
    pub const DEATH119: i32 = 290;
    /// Frame `death120`.
    pub const DEATH120: i32 = 291;
    /// Frame `death121`.
    pub const DEATH121: i32 = 292;
    /// Frame `death122`.
    pub const DEATH122: i32 = 293;
    /// Frame `death123`.
    pub const DEATH123: i32 = 294;
    /// Frame `death124`.
    pub const DEATH124: i32 = 295;
    /// Frame `death125`.
    pub const DEATH125: i32 = 296;
    /// Frame `death126`.
    pub const DEATH126: i32 = 297;
    /// Frame `death127`.
    pub const DEATH127: i32 = 298;
    /// Frame `death128`.
    pub const DEATH128: i32 = 299;
    /// Frame `death129`.
    pub const DEATH129: i32 = 300;
    /// Frame `death130`.
    pub const DEATH130: i32 = 301;
    /// Frame `death131`.
    pub const DEATH131: i32 = 302;
    /// Frame `death132`.
    pub const DEATH132: i32 = 303;
    /// Frame `death133`.
    pub const DEATH133: i32 = 304;
    /// Frame `death134`.
    pub const DEATH134: i32 = 305;
    /// Frame `death135`.
    pub const DEATH135: i32 = 306;
    /// Frame `death136`.
    pub const DEATH136: i32 = 307;
    /// Frame `death201`.
    pub const DEATH201: i32 = 308;
    /// Frame `death202`.
    pub const DEATH202: i32 = 309;
    /// Frame `death203`.
    pub const DEATH203: i32 = 310;
    /// Frame `death204`.
    pub const DEATH204: i32 = 311;
    /// Frame `death205`.
    pub const DEATH205: i32 = 312;
    /// Frame `death206`.
    pub const DEATH206: i32 = 313;
    /// Frame `death207`.
    pub const DEATH207: i32 = 314;
    /// Frame `death208`.
    pub const DEATH208: i32 = 315;
    /// Frame `death209`.
    pub const DEATH209: i32 = 316;
    /// Frame `death210`.
    pub const DEATH210: i32 = 317;
    /// Frame `death211`.
    pub const DEATH211: i32 = 318;
    /// Frame `death212`.
    pub const DEATH212: i32 = 319;
    /// Frame `death213`.
    pub const DEATH213: i32 = 320;
    /// Frame `death214`.
    pub const DEATH214: i32 = 321;
    /// Frame `death215`.
    pub const DEATH215: i32 = 322;
    /// Frame `death216`.
    pub const DEATH216: i32 = 323;
    /// Frame `death217`.
    pub const DEATH217: i32 = 324;
    /// Frame `death218`.
    pub const DEATH218: i32 = 325;
    /// Frame `death219`.
    pub const DEATH219: i32 = 326;
    /// Frame `death220`.
    pub const DEATH220: i32 = 327;
    /// Frame `death221`.
    pub const DEATH221: i32 = 328;
    /// Frame `death222`.
    pub const DEATH222: i32 = 329;
    /// Frame `death223`.
    pub const DEATH223: i32 = 330;
    /// Frame `death224`.
    pub const DEATH224: i32 = 331;
    /// Frame `death225`.
    pub const DEATH225: i32 = 332;
    /// Frame `death226`.
    pub const DEATH226: i32 = 333;
    /// Frame `death227`.
    pub const DEATH227: i32 = 334;
    /// Frame `death228`.
    pub const DEATH228: i32 = 335;
    /// Frame `death229`.
    pub const DEATH229: i32 = 336;
    /// Frame `death230`.
    pub const DEATH230: i32 = 337;
    /// Frame `death231`.
    pub const DEATH231: i32 = 338;
    /// Frame `death232`.
    pub const DEATH232: i32 = 339;
    /// Frame `death233`.
    pub const DEATH233: i32 = 340;
    /// Frame `death234`.
    pub const DEATH234: i32 = 341;
    /// Frame `death235`.
    pub const DEATH235: i32 = 342;
    /// Frame `death301`.
    pub const DEATH301: i32 = 343;
    /// Frame `death302`.
    pub const DEATH302: i32 = 344;
    /// Frame `death303`.
    pub const DEATH303: i32 = 345;
    /// Frame `death304`.
    pub const DEATH304: i32 = 346;
    /// Frame `death305`.
    pub const DEATH305: i32 = 347;
    /// Frame `death306`.
    pub const DEATH306: i32 = 348;
    /// Frame `death307`.
    pub const DEATH307: i32 = 349;
    /// Frame `death308`.
    pub const DEATH308: i32 = 350;
    /// Frame `death309`.
    pub const DEATH309: i32 = 351;
    /// Frame `death310`.
    pub const DEATH310: i32 = 352;
    /// Frame `death311`.
    pub const DEATH311: i32 = 353;
    /// Frame `death312`.
    pub const DEATH312: i32 = 354;
    /// Frame `death313`.
    pub const DEATH313: i32 = 355;
    /// Frame `death314`.
    pub const DEATH314: i32 = 356;
    /// Frame `death315`.
    pub const DEATH315: i32 = 357;
    /// Frame `death316`.
    pub const DEATH316: i32 = 358;
    /// Frame `death317`.
    pub const DEATH317: i32 = 359;
    /// Frame `death318`.
    pub const DEATH318: i32 = 360;
    /// Frame `death319`.
    pub const DEATH319: i32 = 361;
    /// Frame `death320`.
    pub const DEATH320: i32 = 362;
    /// Frame `death321`.
    pub const DEATH321: i32 = 363;
    /// Frame `death322`.
    pub const DEATH322: i32 = 364;
    /// Frame `death323`.
    pub const DEATH323: i32 = 365;
    /// Frame `death324`.
    pub const DEATH324: i32 = 366;
    /// Frame `death325`.
    pub const DEATH325: i32 = 367;
    /// Frame `death326`.
    pub const DEATH326: i32 = 368;
    /// Frame `death327`.
    pub const DEATH327: i32 = 369;
    /// Frame `death328`.
    pub const DEATH328: i32 = 370;
    /// Frame `death329`.
    pub const DEATH329: i32 = 371;
    /// Frame `death330`.
    pub const DEATH330: i32 = 372;
    /// Frame `death331`.
    pub const DEATH331: i32 = 373;
    /// Frame `death332`.
    pub const DEATH332: i32 = 374;
    /// Frame `death333`.
    pub const DEATH333: i32 = 375;
    /// Frame `death334`.
    pub const DEATH334: i32 = 376;
    /// Frame `death335`.
    pub const DEATH335: i32 = 377;
    /// Frame `death336`.
    pub const DEATH336: i32 = 378;
    /// Frame `death337`.
    pub const DEATH337: i32 = 379;
    /// Frame `death338`.
    pub const DEATH338: i32 = 380;
    /// Frame `death339`.
    pub const DEATH339: i32 = 381;
    /// Frame `death340`.
    pub const DEATH340: i32 = 382;
    /// Frame `death341`.
    pub const DEATH341: i32 = 383;
    /// Frame `death342`.
    pub const DEATH342: i32 = 384;
    /// Frame `death343`.
    pub const DEATH343: i32 = 385;
    /// Frame `death344`.
    pub const DEATH344: i32 = 386;
    /// Frame `death345`.
    pub const DEATH345: i32 = 387;
    /// Frame `death401`.
    pub const DEATH401: i32 = 388;
    /// Frame `death402`.
    pub const DEATH402: i32 = 389;
    /// Frame `death403`.
    pub const DEATH403: i32 = 390;
    /// Frame `death404`.
    pub const DEATH404: i32 = 391;
    /// Frame `death405`.
    pub const DEATH405: i32 = 392;
    /// Frame `death406`.
    pub const DEATH406: i32 = 393;
    /// Frame `death407`.
    pub const DEATH407: i32 = 394;
    /// Frame `death408`.
    pub const DEATH408: i32 = 395;
    /// Frame `death409`.
    pub const DEATH409: i32 = 396;
    /// Frame `death410`.
    pub const DEATH410: i32 = 397;
    /// Frame `death411`.
    pub const DEATH411: i32 = 398;
    /// Frame `death412`.
    pub const DEATH412: i32 = 399;
    /// Frame `death413`.
    pub const DEATH413: i32 = 400;
    /// Frame `death414`.
    pub const DEATH414: i32 = 401;
    /// Frame `death415`.
    pub const DEATH415: i32 = 402;
    /// Frame `death416`.
    pub const DEATH416: i32 = 403;
    /// Frame `death417`.
    pub const DEATH417: i32 = 404;
    /// Frame `death418`.
    pub const DEATH418: i32 = 405;
    /// Frame `death419`.
    pub const DEATH419: i32 = 406;
    /// Frame `death420`.
    pub const DEATH420: i32 = 407;
    /// Frame `death421`.
    pub const DEATH421: i32 = 408;
    /// Frame `death422`.
    pub const DEATH422: i32 = 409;
    /// Frame `death423`.
    pub const DEATH423: i32 = 410;
    /// Frame `death424`.
    pub const DEATH424: i32 = 411;
    /// Frame `death425`.
    pub const DEATH425: i32 = 412;
    /// Frame `death426`.
    pub const DEATH426: i32 = 413;
    /// Frame `death427`.
    pub const DEATH427: i32 = 414;
    /// Frame `death428`.
    pub const DEATH428: i32 = 415;
    /// Frame `death429`.
    pub const DEATH429: i32 = 416;
    /// Frame `death430`.
    pub const DEATH430: i32 = 417;
    /// Frame `death431`.
    pub const DEATH431: i32 = 418;
    /// Frame `death432`.
    pub const DEATH432: i32 = 419;
    /// Frame `death433`.
    pub const DEATH433: i32 = 420;
    /// Frame `death434`.
    pub const DEATH434: i32 = 421;
    /// Frame `death435`.
    pub const DEATH435: i32 = 422;
    /// Frame `death436`.
    pub const DEATH436: i32 = 423;
    /// Frame `death437`.
    pub const DEATH437: i32 = 424;
    /// Frame `death438`.
    pub const DEATH438: i32 = 425;
    /// Frame `death439`.
    pub const DEATH439: i32 = 426;
    /// Frame `death440`.
    pub const DEATH440: i32 = 427;
    /// Frame `death441`.
    pub const DEATH441: i32 = 428;
    /// Frame `death442`.
    pub const DEATH442: i32 = 429;
    /// Frame `death443`.
    pub const DEATH443: i32 = 430;
    /// Frame `death444`.
    pub const DEATH444: i32 = 431;
    /// Frame `death445`.
    pub const DEATH445: i32 = 432;
    /// Frame `death446`.
    pub const DEATH446: i32 = 433;
    /// Frame `death447`.
    pub const DEATH447: i32 = 434;
    /// Frame `death448`.
    pub const DEATH448: i32 = 435;
    /// Frame `death449`.
    pub const DEATH449: i32 = 436;
    /// Frame `death450`.
    pub const DEATH450: i32 = 437;
    /// Frame `death451`.
    pub const DEATH451: i32 = 438;
    /// Frame `death452`.
    pub const DEATH452: i32 = 439;
    /// Frame `death453`.
    pub const DEATH453: i32 = 440;
    /// Frame `death501`.
    pub const DEATH501: i32 = 441;
    /// Frame `death502`.
    pub const DEATH502: i32 = 442;
    /// Frame `death503`.
    pub const DEATH503: i32 = 443;
    /// Frame `death504`.
    pub const DEATH504: i32 = 444;
    /// Frame `death505`.
    pub const DEATH505: i32 = 445;
    /// Frame `death506`.
    pub const DEATH506: i32 = 446;
    /// Frame `death507`.
    pub const DEATH507: i32 = 447;
    /// Frame `death508`.
    pub const DEATH508: i32 = 448;
    /// Frame `death509`.
    pub const DEATH509: i32 = 449;
    /// Frame `death510`.
    pub const DEATH510: i32 = 450;
    /// Frame `death511`.
    pub const DEATH511: i32 = 451;
    /// Frame `death512`.
    pub const DEATH512: i32 = 452;
    /// Frame `death513`.
    pub const DEATH513: i32 = 453;
    /// Frame `death514`.
    pub const DEATH514: i32 = 454;
    /// Frame `death515`.
    pub const DEATH515: i32 = 455;
    /// Frame `death516`.
    pub const DEATH516: i32 = 456;
    /// Frame `death517`.
    pub const DEATH517: i32 = 457;
    /// Frame `death518`.
    pub const DEATH518: i32 = 458;
    /// Frame `death519`.
    pub const DEATH519: i32 = 459;
    /// Frame `death520`.
    pub const DEATH520: i32 = 460;
    /// Frame `death521`.
    pub const DEATH521: i32 = 461;
    /// Frame `death522`.
    pub const DEATH522: i32 = 462;
    /// Frame `death523`.
    pub const DEATH523: i32 = 463;
    /// Frame `death524`.
    pub const DEATH524: i32 = 464;
    /// Frame `death601`.
    pub const DEATH601: i32 = 465;
    /// Frame `death602`.
    pub const DEATH602: i32 = 466;
    /// Frame `death603`.
    pub const DEATH603: i32 = 467;
    /// Frame `death604`.
    pub const DEATH604: i32 = 468;
    /// Frame `death605`.
    pub const DEATH605: i32 = 469;
    /// Frame `death606`.
    pub const DEATH606: i32 = 470;
    /// Frame `death607`.
    pub const DEATH607: i32 = 471;
    /// Frame `death608`.
    pub const DEATH608: i32 = 472;
    /// Frame `death609`.
    pub const DEATH609: i32 = 473;
    /// Frame `death610`.
    pub const DEATH610: i32 = 474;
}

/// `soldierhMoves` move tables.
pub fn soldierh_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "soldierh_move_stand1",
            146,
            175,
            Some("soldierh_stand"),
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("soldierh_idle")], -1),
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
            ],
        ),
        monster_move(
            "soldierh_move_stand3",
            176,
            214,
            Some("soldierh_stand"),
            vec![
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
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("soldierh_cock")], -1),
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
            ],
        ),
        monster_move(
            "soldierh_move_walk1",
            215,
            247,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 1.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
                monster_frame(
                    MonsterAi::Walk,
                    -1.0,
                    vec![MonsterAction::name("soldierh_walk1_random")],
                    -1,
                ),
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
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_walk2",
            256,
            265,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 9.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 8.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 1.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_start_run",
            97,
            98,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Run, 7.0, vec![], -1),
                monster_frame(MonsterAi::Run, 5.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_run",
            99,
            104,
            None,
            vec![
                monster_frame(MonsterAi::Run, 10.0, vec![], -1),
                monster_frame(MonsterAi::Run, 11.0, vec![], -1),
                monster_frame(MonsterAi::Run, 11.0, vec![], -1),
                monster_frame(MonsterAi::Run, 16.0, vec![], -1),
                monster_frame(MonsterAi::Run, 10.0, vec![], -1),
                monster_frame(MonsterAi::Run, 15.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_pain1",
            50,
            54,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Move, -3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_pain2",
            55,
            61,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Move, -13.0, vec![], -1),
                monster_frame(MonsterAi::Move, -1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_pain3",
            62,
            79,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Move, -8.0, vec![], -1),
                monster_frame(MonsterAi::Move, 10.0, vec![], -1),
                monster_frame(MonsterAi::Move, -4.0, vec![], -1),
                monster_frame(MonsterAi::Move, -1.0, vec![], -1),
                monster_frame(MonsterAi::Move, -3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_pain4",
            80,
            96,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -10.0, vec![], -1),
                monster_frame(MonsterAi::Move, -6.0, vec![], -1),
                monster_frame(MonsterAi::Move, 8.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 5.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, -1.0, vec![], -1),
                monster_frame(MonsterAi::Move, -1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_attack1",
            0,
            11,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_hyper_sound")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("soldierh_fire1")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_ripper1")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_ripper1")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_attack1_refire1")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_hyper_refire1")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("soldierh_cock")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_attack1_refire2")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_attack2",
            12,
            29,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_hyper_sound")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("soldierh_fire2")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_ripper2")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_ripper2")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_attack2_refire1")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_hyper_refire2")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("soldierh_cock")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_attack2_refire2")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_attack3",
            30,
            38,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("soldierh_fire3")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_attack3_refire")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("soldierh_duck_up")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_attack4",
            39,
            44,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("soldierh_fire4")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_attack6",
            109,
            122,
            Some("soldierh_run"),
            vec![
                monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 12.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 11.0, vec![MonsterAction::name("soldierh_fire8")], -1),
                monster_frame(MonsterAi::Charge, 13.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 18.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 15.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 14.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 11.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 8.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 11.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 12.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 12.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    17.0,
                    vec![MonsterAction::name("soldierh_attack6_refire")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "soldierh_move_duck",
            45,
            49,
            Some("soldierh_run"),
            vec![
                monster_frame(
                    MonsterAi::Move,
                    5.0,
                    vec![MonsterAction::name("soldierh_duck_down")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    -1.0,
                    vec![MonsterAction::name("soldierh_duck_hold")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("soldierh_duck_up")], -1),
                monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            ],
        ),
        monster_move(
            "soldierh_move_death1",
            272,
            307,
            Some("soldierh_dead"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -10.0, vec![], -1),
                monster_frame(MonsterAi::Move, -10.0, vec![], -1),
                monster_frame(MonsterAi::Move, -10.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
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
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("soldierh_fire6")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("soldierh_fire7")], -1),
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
            "soldierh_move_death2",
            308,
            342,
            Some("soldierh_dead"),
            vec![
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
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
            "soldierh_move_death3",
            343,
            387,
            Some("soldierh_dead"),
            vec![
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
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
            "soldierh_move_death4",
            388,
            440,
            Some("soldierh_dead"),
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
            "soldierh_move_death5",
            441,
            464,
            Some("soldierh_dead"),
            vec![
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
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
            "soldierh_move_death6",
            465,
            474,
            Some("soldierh_dead"),
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
            ],
        ),
    ]
}
