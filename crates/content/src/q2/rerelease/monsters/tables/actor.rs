//! actor move tables (`src/content/q2/rerelease/monsters/tables/actor.ts`).

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `actorFrame`.
pub mod actor_frame {
    /// Frame `attak01`.
    pub const ATTAK01: i32 = 0;
    /// Frame `attak02`.
    pub const ATTAK02: i32 = 1;
    /// Frame `attak03`.
    pub const ATTAK03: i32 = 2;
    /// Frame `attak04`.
    pub const ATTAK04: i32 = 3;
    /// Frame `death101`.
    pub const DEATH101: i32 = 4;
    /// Frame `death102`.
    pub const DEATH102: i32 = 5;
    /// Frame `death103`.
    pub const DEATH103: i32 = 6;
    /// Frame `death104`.
    pub const DEATH104: i32 = 7;
    /// Frame `death105`.
    pub const DEATH105: i32 = 8;
    /// Frame `death106`.
    pub const DEATH106: i32 = 9;
    /// Frame `death107`.
    pub const DEATH107: i32 = 10;
    /// Frame `death201`.
    pub const DEATH201: i32 = 11;
    /// Frame `death202`.
    pub const DEATH202: i32 = 12;
    /// Frame `death203`.
    pub const DEATH203: i32 = 13;
    /// Frame `death204`.
    pub const DEATH204: i32 = 14;
    /// Frame `death205`.
    pub const DEATH205: i32 = 15;
    /// Frame `death206`.
    pub const DEATH206: i32 = 16;
    /// Frame `death207`.
    pub const DEATH207: i32 = 17;
    /// Frame `death208`.
    pub const DEATH208: i32 = 18;
    /// Frame `death209`.
    pub const DEATH209: i32 = 19;
    /// Frame `death210`.
    pub const DEATH210: i32 = 20;
    /// Frame `death211`.
    pub const DEATH211: i32 = 21;
    /// Frame `death212`.
    pub const DEATH212: i32 = 22;
    /// Frame `death213`.
    pub const DEATH213: i32 = 23;
    /// Frame `death301`.
    pub const DEATH301: i32 = 24;
    /// Frame `death302`.
    pub const DEATH302: i32 = 25;
    /// Frame `death303`.
    pub const DEATH303: i32 = 26;
    /// Frame `death304`.
    pub const DEATH304: i32 = 27;
    /// Frame `death305`.
    pub const DEATH305: i32 = 28;
    /// Frame `death306`.
    pub const DEATH306: i32 = 29;
    /// Frame `death307`.
    pub const DEATH307: i32 = 30;
    /// Frame `death308`.
    pub const DEATH308: i32 = 31;
    /// Frame `death309`.
    pub const DEATH309: i32 = 32;
    /// Frame `death310`.
    pub const DEATH310: i32 = 33;
    /// Frame `death311`.
    pub const DEATH311: i32 = 34;
    /// Frame `death312`.
    pub const DEATH312: i32 = 35;
    /// Frame `death313`.
    pub const DEATH313: i32 = 36;
    /// Frame `death314`.
    pub const DEATH314: i32 = 37;
    /// Frame `death315`.
    pub const DEATH315: i32 = 38;
    /// Frame `flip01`.
    pub const FLIP01: i32 = 39;
    /// Frame `flip02`.
    pub const FLIP02: i32 = 40;
    /// Frame `flip03`.
    pub const FLIP03: i32 = 41;
    /// Frame `flip04`.
    pub const FLIP04: i32 = 42;
    /// Frame `flip05`.
    pub const FLIP05: i32 = 43;
    /// Frame `flip06`.
    pub const FLIP06: i32 = 44;
    /// Frame `flip07`.
    pub const FLIP07: i32 = 45;
    /// Frame `flip08`.
    pub const FLIP08: i32 = 46;
    /// Frame `flip09`.
    pub const FLIP09: i32 = 47;
    /// Frame `flip10`.
    pub const FLIP10: i32 = 48;
    /// Frame `flip11`.
    pub const FLIP11: i32 = 49;
    /// Frame `flip12`.
    pub const FLIP12: i32 = 50;
    /// Frame `flip13`.
    pub const FLIP13: i32 = 51;
    /// Frame `flip14`.
    pub const FLIP14: i32 = 52;
    /// Frame `grenad01`.
    pub const GRENAD01: i32 = 53;
    /// Frame `grenad02`.
    pub const GRENAD02: i32 = 54;
    /// Frame `grenad03`.
    pub const GRENAD03: i32 = 55;
    /// Frame `grenad04`.
    pub const GRENAD04: i32 = 56;
    /// Frame `grenad05`.
    pub const GRENAD05: i32 = 57;
    /// Frame `grenad06`.
    pub const GRENAD06: i32 = 58;
    /// Frame `grenad07`.
    pub const GRENAD07: i32 = 59;
    /// Frame `grenad08`.
    pub const GRENAD08: i32 = 60;
    /// Frame `grenad09`.
    pub const GRENAD09: i32 = 61;
    /// Frame `grenad10`.
    pub const GRENAD10: i32 = 62;
    /// Frame `grenad11`.
    pub const GRENAD11: i32 = 63;
    /// Frame `grenad12`.
    pub const GRENAD12: i32 = 64;
    /// Frame `grenad13`.
    pub const GRENAD13: i32 = 65;
    /// Frame `grenad14`.
    pub const GRENAD14: i32 = 66;
    /// Frame `grenad15`.
    pub const GRENAD15: i32 = 67;
    /// Frame `jump01`.
    pub const JUMP01: i32 = 68;
    /// Frame `jump02`.
    pub const JUMP02: i32 = 69;
    /// Frame `jump03`.
    pub const JUMP03: i32 = 70;
    /// Frame `jump04`.
    pub const JUMP04: i32 = 71;
    /// Frame `jump05`.
    pub const JUMP05: i32 = 72;
    /// Frame `jump06`.
    pub const JUMP06: i32 = 73;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 74;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 75;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 76;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 77;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 78;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 79;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 80;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 81;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 82;
    /// Frame `push01`.
    pub const PUSH01: i32 = 83;
    /// Frame `push02`.
    pub const PUSH02: i32 = 84;
    /// Frame `push03`.
    pub const PUSH03: i32 = 85;
    /// Frame `push04`.
    pub const PUSH04: i32 = 86;
    /// Frame `push05`.
    pub const PUSH05: i32 = 87;
    /// Frame `push06`.
    pub const PUSH06: i32 = 88;
    /// Frame `push07`.
    pub const PUSH07: i32 = 89;
    /// Frame `push08`.
    pub const PUSH08: i32 = 90;
    /// Frame `push09`.
    pub const PUSH09: i32 = 91;
    /// Frame `run01`.
    pub const RUN01: i32 = 92;
    /// Frame `run02`.
    pub const RUN02: i32 = 93;
    /// Frame `run03`.
    pub const RUN03: i32 = 94;
    /// Frame `run04`.
    pub const RUN04: i32 = 95;
    /// Frame `run05`.
    pub const RUN05: i32 = 96;
    /// Frame `run06`.
    pub const RUN06: i32 = 97;
    /// Frame `run07`.
    pub const RUN07: i32 = 98;
    /// Frame `run08`.
    pub const RUN08: i32 = 99;
    /// Frame `run09`.
    pub const RUN09: i32 = 100;
    /// Frame `run10`.
    pub const RUN10: i32 = 101;
    /// Frame `run11`.
    pub const RUN11: i32 = 102;
    /// Frame `run12`.
    pub const RUN12: i32 = 103;
    /// Frame `runs01`.
    pub const RUNS01: i32 = 104;
    /// Frame `runs02`.
    pub const RUNS02: i32 = 105;
    /// Frame `runs03`.
    pub const RUNS03: i32 = 106;
    /// Frame `runs04`.
    pub const RUNS04: i32 = 107;
    /// Frame `runs05`.
    pub const RUNS05: i32 = 108;
    /// Frame `runs06`.
    pub const RUNS06: i32 = 109;
    /// Frame `runs07`.
    pub const RUNS07: i32 = 110;
    /// Frame `runs08`.
    pub const RUNS08: i32 = 111;
    /// Frame `runs09`.
    pub const RUNS09: i32 = 112;
    /// Frame `runs10`.
    pub const RUNS10: i32 = 113;
    /// Frame `runs11`.
    pub const RUNS11: i32 = 114;
    /// Frame `runs12`.
    pub const RUNS12: i32 = 115;
    /// Frame `salute01`.
    pub const SALUTE01: i32 = 116;
    /// Frame `salute02`.
    pub const SALUTE02: i32 = 117;
    /// Frame `salute03`.
    pub const SALUTE03: i32 = 118;
    /// Frame `salute04`.
    pub const SALUTE04: i32 = 119;
    /// Frame `salute05`.
    pub const SALUTE05: i32 = 120;
    /// Frame `salute06`.
    pub const SALUTE06: i32 = 121;
    /// Frame `salute07`.
    pub const SALUTE07: i32 = 122;
    /// Frame `salute08`.
    pub const SALUTE08: i32 = 123;
    /// Frame `salute09`.
    pub const SALUTE09: i32 = 124;
    /// Frame `salute10`.
    pub const SALUTE10: i32 = 125;
    /// Frame `salute11`.
    pub const SALUTE11: i32 = 126;
    /// Frame `salute12`.
    pub const SALUTE12: i32 = 127;
    /// Frame `stand101`.
    pub const STAND101: i32 = 128;
    /// Frame `stand102`.
    pub const STAND102: i32 = 129;
    /// Frame `stand103`.
    pub const STAND103: i32 = 130;
    /// Frame `stand104`.
    pub const STAND104: i32 = 131;
    /// Frame `stand105`.
    pub const STAND105: i32 = 132;
    /// Frame `stand106`.
    pub const STAND106: i32 = 133;
    /// Frame `stand107`.
    pub const STAND107: i32 = 134;
    /// Frame `stand108`.
    pub const STAND108: i32 = 135;
    /// Frame `stand109`.
    pub const STAND109: i32 = 136;
    /// Frame `stand110`.
    pub const STAND110: i32 = 137;
    /// Frame `stand111`.
    pub const STAND111: i32 = 138;
    /// Frame `stand112`.
    pub const STAND112: i32 = 139;
    /// Frame `stand113`.
    pub const STAND113: i32 = 140;
    /// Frame `stand114`.
    pub const STAND114: i32 = 141;
    /// Frame `stand115`.
    pub const STAND115: i32 = 142;
    /// Frame `stand116`.
    pub const STAND116: i32 = 143;
    /// Frame `stand117`.
    pub const STAND117: i32 = 144;
    /// Frame `stand118`.
    pub const STAND118: i32 = 145;
    /// Frame `stand119`.
    pub const STAND119: i32 = 146;
    /// Frame `stand120`.
    pub const STAND120: i32 = 147;
    /// Frame `stand121`.
    pub const STAND121: i32 = 148;
    /// Frame `stand122`.
    pub const STAND122: i32 = 149;
    /// Frame `stand123`.
    pub const STAND123: i32 = 150;
    /// Frame `stand124`.
    pub const STAND124: i32 = 151;
    /// Frame `stand125`.
    pub const STAND125: i32 = 152;
    /// Frame `stand126`.
    pub const STAND126: i32 = 153;
    /// Frame `stand127`.
    pub const STAND127: i32 = 154;
    /// Frame `stand128`.
    pub const STAND128: i32 = 155;
    /// Frame `stand129`.
    pub const STAND129: i32 = 156;
    /// Frame `stand130`.
    pub const STAND130: i32 = 157;
    /// Frame `stand131`.
    pub const STAND131: i32 = 158;
    /// Frame `stand132`.
    pub const STAND132: i32 = 159;
    /// Frame `stand133`.
    pub const STAND133: i32 = 160;
    /// Frame `stand134`.
    pub const STAND134: i32 = 161;
    /// Frame `stand135`.
    pub const STAND135: i32 = 162;
    /// Frame `stand136`.
    pub const STAND136: i32 = 163;
    /// Frame `stand137`.
    pub const STAND137: i32 = 164;
    /// Frame `stand138`.
    pub const STAND138: i32 = 165;
    /// Frame `stand139`.
    pub const STAND139: i32 = 166;
    /// Frame `stand140`.
    pub const STAND140: i32 = 167;
    /// Frame `stand201`.
    pub const STAND201: i32 = 168;
    /// Frame `stand202`.
    pub const STAND202: i32 = 169;
    /// Frame `stand203`.
    pub const STAND203: i32 = 170;
    /// Frame `stand204`.
    pub const STAND204: i32 = 171;
    /// Frame `stand205`.
    pub const STAND205: i32 = 172;
    /// Frame `stand206`.
    pub const STAND206: i32 = 173;
    /// Frame `stand207`.
    pub const STAND207: i32 = 174;
    /// Frame `stand208`.
    pub const STAND208: i32 = 175;
    /// Frame `stand209`.
    pub const STAND209: i32 = 176;
    /// Frame `stand210`.
    pub const STAND210: i32 = 177;
    /// Frame `stand211`.
    pub const STAND211: i32 = 178;
    /// Frame `stand212`.
    pub const STAND212: i32 = 179;
    /// Frame `stand213`.
    pub const STAND213: i32 = 180;
    /// Frame `stand214`.
    pub const STAND214: i32 = 181;
    /// Frame `stand215`.
    pub const STAND215: i32 = 182;
    /// Frame `stand216`.
    pub const STAND216: i32 = 183;
    /// Frame `stand217`.
    pub const STAND217: i32 = 184;
    /// Frame `stand218`.
    pub const STAND218: i32 = 185;
    /// Frame `stand219`.
    pub const STAND219: i32 = 186;
    /// Frame `stand220`.
    pub const STAND220: i32 = 187;
    /// Frame `stand221`.
    pub const STAND221: i32 = 188;
    /// Frame `stand222`.
    pub const STAND222: i32 = 189;
    /// Frame `stand223`.
    pub const STAND223: i32 = 190;
    /// Frame `swim01`.
    pub const SWIM01: i32 = 191;
    /// Frame `swim02`.
    pub const SWIM02: i32 = 192;
    /// Frame `swim03`.
    pub const SWIM03: i32 = 193;
    /// Frame `swim04`.
    pub const SWIM04: i32 = 194;
    /// Frame `swim05`.
    pub const SWIM05: i32 = 195;
    /// Frame `swim06`.
    pub const SWIM06: i32 = 196;
    /// Frame `swim07`.
    pub const SWIM07: i32 = 197;
    /// Frame `swim08`.
    pub const SWIM08: i32 = 198;
    /// Frame `swim09`.
    pub const SWIM09: i32 = 199;
    /// Frame `swim10`.
    pub const SWIM10: i32 = 200;
    /// Frame `swim11`.
    pub const SWIM11: i32 = 201;
    /// Frame `swim12`.
    pub const SWIM12: i32 = 202;
    /// Frame `sw_atk01`.
    pub const SW_ATK01: i32 = 203;
    /// Frame `sw_atk02`.
    pub const SW_ATK02: i32 = 204;
    /// Frame `sw_atk03`.
    pub const SW_ATK03: i32 = 205;
    /// Frame `sw_atk04`.
    pub const SW_ATK04: i32 = 206;
    /// Frame `sw_atk05`.
    pub const SW_ATK05: i32 = 207;
    /// Frame `sw_atk06`.
    pub const SW_ATK06: i32 = 208;
    /// Frame `sw_pan01`.
    pub const SW_PAN01: i32 = 209;
    /// Frame `sw_pan02`.
    pub const SW_PAN02: i32 = 210;
    /// Frame `sw_pan03`.
    pub const SW_PAN03: i32 = 211;
    /// Frame `sw_pan04`.
    pub const SW_PAN04: i32 = 212;
    /// Frame `sw_pan05`.
    pub const SW_PAN05: i32 = 213;
    /// Frame `sw_std01`.
    pub const SW_STD01: i32 = 214;
    /// Frame `sw_std02`.
    pub const SW_STD02: i32 = 215;
    /// Frame `sw_std03`.
    pub const SW_STD03: i32 = 216;
    /// Frame `sw_std04`.
    pub const SW_STD04: i32 = 217;
    /// Frame `sw_std05`.
    pub const SW_STD05: i32 = 218;
    /// Frame `sw_std06`.
    pub const SW_STD06: i32 = 219;
    /// Frame `sw_std07`.
    pub const SW_STD07: i32 = 220;
    /// Frame `sw_std08`.
    pub const SW_STD08: i32 = 221;
    /// Frame `sw_std09`.
    pub const SW_STD09: i32 = 222;
    /// Frame `sw_std10`.
    pub const SW_STD10: i32 = 223;
    /// Frame `sw_std11`.
    pub const SW_STD11: i32 = 224;
    /// Frame `sw_std12`.
    pub const SW_STD12: i32 = 225;
    /// Frame `sw_std13`.
    pub const SW_STD13: i32 = 226;
    /// Frame `sw_std14`.
    pub const SW_STD14: i32 = 227;
    /// Frame `sw_std15`.
    pub const SW_STD15: i32 = 228;
    /// Frame `sw_std16`.
    pub const SW_STD16: i32 = 229;
    /// Frame `sw_std17`.
    pub const SW_STD17: i32 = 230;
    /// Frame `sw_std18`.
    pub const SW_STD18: i32 = 231;
    /// Frame `sw_std19`.
    pub const SW_STD19: i32 = 232;
    /// Frame `sw_std20`.
    pub const SW_STD20: i32 = 233;
    /// Frame `taunt01`.
    pub const TAUNT01: i32 = 234;
    /// Frame `taunt02`.
    pub const TAUNT02: i32 = 235;
    /// Frame `taunt03`.
    pub const TAUNT03: i32 = 236;
    /// Frame `taunt04`.
    pub const TAUNT04: i32 = 237;
    /// Frame `taunt05`.
    pub const TAUNT05: i32 = 238;
    /// Frame `taunt06`.
    pub const TAUNT06: i32 = 239;
    /// Frame `taunt07`.
    pub const TAUNT07: i32 = 240;
    /// Frame `taunt08`.
    pub const TAUNT08: i32 = 241;
    /// Frame `taunt09`.
    pub const TAUNT09: i32 = 242;
    /// Frame `taunt10`.
    pub const TAUNT10: i32 = 243;
    /// Frame `taunt11`.
    pub const TAUNT11: i32 = 244;
    /// Frame `taunt12`.
    pub const TAUNT12: i32 = 245;
    /// Frame `taunt13`.
    pub const TAUNT13: i32 = 246;
    /// Frame `taunt14`.
    pub const TAUNT14: i32 = 247;
    /// Frame `taunt15`.
    pub const TAUNT15: i32 = 248;
    /// Frame `taunt16`.
    pub const TAUNT16: i32 = 249;
    /// Frame `taunt17`.
    pub const TAUNT17: i32 = 250;
    /// Frame `walk01`.
    pub const WALK01: i32 = 251;
    /// Frame `walk02`.
    pub const WALK02: i32 = 252;
    /// Frame `walk03`.
    pub const WALK03: i32 = 253;
    /// Frame `walk04`.
    pub const WALK04: i32 = 254;
    /// Frame `walk05`.
    pub const WALK05: i32 = 255;
    /// Frame `walk06`.
    pub const WALK06: i32 = 256;
    /// Frame `walk07`.
    pub const WALK07: i32 = 257;
    /// Frame `walk08`.
    pub const WALK08: i32 = 258;
    /// Frame `walk09`.
    pub const WALK09: i32 = 259;
    /// Frame `walk10`.
    pub const WALK10: i32 = 260;
    /// Frame `walk11`.
    pub const WALK11: i32 = 261;
    /// Frame `wave01`.
    pub const WAVE01: i32 = 262;
    /// Frame `wave02`.
    pub const WAVE02: i32 = 263;
    /// Frame `wave03`.
    pub const WAVE03: i32 = 264;
    /// Frame `wave04`.
    pub const WAVE04: i32 = 265;
    /// Frame `wave05`.
    pub const WAVE05: i32 = 266;
    /// Frame `wave06`.
    pub const WAVE06: i32 = 267;
    /// Frame `wave07`.
    pub const WAVE07: i32 = 268;
    /// Frame `wave08`.
    pub const WAVE08: i32 = 269;
    /// Frame `wave09`.
    pub const WAVE09: i32 = 270;
    /// Frame `wave10`.
    pub const WAVE10: i32 = 271;
    /// Frame `wave11`.
    pub const WAVE11: i32 = 272;
    /// Frame `wave12`.
    pub const WAVE12: i32 = 273;
    /// Frame `wave13`.
    pub const WAVE13: i32 = 274;
    /// Frame `wave14`.
    pub const WAVE14: i32 = 275;
    /// Frame `wave15`.
    pub const WAVE15: i32 = 276;
    /// Frame `wave16`.
    pub const WAVE16: i32 = 277;
    /// Frame `wave17`.
    pub const WAVE17: i32 = 278;
    /// Frame `wave18`.
    pub const WAVE18: i32 = 279;
    /// Frame `wave19`.
    pub const WAVE19: i32 = 280;
    /// Frame `wave20`.
    pub const WAVE20: i32 = 281;
    /// Frame `wave21`.
    pub const WAVE21: i32 = 282;
    /// Frame `bl_atk01`.
    pub const BL_ATK01: i32 = 283;
    /// Frame `bl_atk02`.
    pub const BL_ATK02: i32 = 284;
    /// Frame `bl_atk03`.
    pub const BL_ATK03: i32 = 285;
    /// Frame `bl_atk04`.
    pub const BL_ATK04: i32 = 286;
    /// Frame `bl_atk05`.
    pub const BL_ATK05: i32 = 287;
    /// Frame `bl_atk06`.
    pub const BL_ATK06: i32 = 288;
    /// Frame `bl_flp01`.
    pub const BL_FLP01: i32 = 289;
    /// Frame `bl_flp02`.
    pub const BL_FLP02: i32 = 290;
    /// Frame `bl_flp13`.
    pub const BL_FLP13: i32 = 291;
    /// Frame `bl_flp14`.
    pub const BL_FLP14: i32 = 292;
    /// Frame `bl_flp15`.
    pub const BL_FLP15: i32 = 293;
    /// Frame `bl_jmp01`.
    pub const BL_JMP01: i32 = 294;
    /// Frame `bl_jmp02`.
    pub const BL_JMP02: i32 = 295;
    /// Frame `bl_jmp03`.
    pub const BL_JMP03: i32 = 296;
    /// Frame `bl_jmp04`.
    pub const BL_JMP04: i32 = 297;
    /// Frame `bl_jmp05`.
    pub const BL_JMP05: i32 = 298;
    /// Frame `bl_jmp06`.
    pub const BL_JMP06: i32 = 299;
    /// Frame `bl_pn101`.
    pub const BL_PN101: i32 = 300;
    /// Frame `bl_pn102`.
    pub const BL_PN102: i32 = 301;
    /// Frame `bl_pn103`.
    pub const BL_PN103: i32 = 302;
    /// Frame `bl_pn201`.
    pub const BL_PN201: i32 = 303;
    /// Frame `bl_pn202`.
    pub const BL_PN202: i32 = 304;
    /// Frame `bl_pn203`.
    pub const BL_PN203: i32 = 305;
    /// Frame `bl_pn301`.
    pub const BL_PN301: i32 = 306;
    /// Frame `bl_pn302`.
    pub const BL_PN302: i32 = 307;
    /// Frame `bl_pn303`.
    pub const BL_PN303: i32 = 308;
    /// Frame `bl_psh08`.
    pub const BL_PSH08: i32 = 309;
    /// Frame `bl_psh09`.
    pub const BL_PSH09: i32 = 310;
    /// Frame `bl_run01`.
    pub const BL_RUN01: i32 = 311;
    /// Frame `bl_run02`.
    pub const BL_RUN02: i32 = 312;
    /// Frame `bl_run03`.
    pub const BL_RUN03: i32 = 313;
    /// Frame `bl_run04`.
    pub const BL_RUN04: i32 = 314;
    /// Frame `bl_run05`.
    pub const BL_RUN05: i32 = 315;
    /// Frame `bl_run06`.
    pub const BL_RUN06: i32 = 316;
    /// Frame `bl_run07`.
    pub const BL_RUN07: i32 = 317;
    /// Frame `bl_run08`.
    pub const BL_RUN08: i32 = 318;
    /// Frame `bl_run09`.
    pub const BL_RUN09: i32 = 319;
    /// Frame `bl_run10`.
    pub const BL_RUN10: i32 = 320;
    /// Frame `bl_run11`.
    pub const BL_RUN11: i32 = 321;
    /// Frame `bl_run12`.
    pub const BL_RUN12: i32 = 322;
    /// Frame `bl_rns03`.
    pub const BL_RNS03: i32 = 323;
    /// Frame `bl_rns04`.
    pub const BL_RNS04: i32 = 324;
    /// Frame `bl_rns05`.
    pub const BL_RNS05: i32 = 325;
    /// Frame `bl_rns06`.
    pub const BL_RNS06: i32 = 326;
    /// Frame `bl_rns07`.
    pub const BL_RNS07: i32 = 327;
    /// Frame `bl_rns08`.
    pub const BL_RNS08: i32 = 328;
    /// Frame `bl_rns09`.
    pub const BL_RNS09: i32 = 329;
    /// Frame `bl_sal10`.
    pub const BL_SAL10: i32 = 330;
    /// Frame `bl_sal11`.
    pub const BL_SAL11: i32 = 331;
    /// Frame `bl_sal12`.
    pub const BL_SAL12: i32 = 332;
    /// Frame `bl_std01`.
    pub const BL_STD01: i32 = 333;
    /// Frame `bl_std02`.
    pub const BL_STD02: i32 = 334;
    /// Frame `bl_std03`.
    pub const BL_STD03: i32 = 335;
    /// Frame `bl_std04`.
    pub const BL_STD04: i32 = 336;
    /// Frame `bl_std05`.
    pub const BL_STD05: i32 = 337;
    /// Frame `bl_std06`.
    pub const BL_STD06: i32 = 338;
    /// Frame `bl_std07`.
    pub const BL_STD07: i32 = 339;
    /// Frame `bl_std08`.
    pub const BL_STD08: i32 = 340;
    /// Frame `bl_std09`.
    pub const BL_STD09: i32 = 341;
    /// Frame `bl_std10`.
    pub const BL_STD10: i32 = 342;
    /// Frame `bl_std11`.
    pub const BL_STD11: i32 = 343;
    /// Frame `bl_std12`.
    pub const BL_STD12: i32 = 344;
    /// Frame `bl_std13`.
    pub const BL_STD13: i32 = 345;
    /// Frame `bl_std14`.
    pub const BL_STD14: i32 = 346;
    /// Frame `bl_std15`.
    pub const BL_STD15: i32 = 347;
    /// Frame `bl_std16`.
    pub const BL_STD16: i32 = 348;
    /// Frame `bl_std17`.
    pub const BL_STD17: i32 = 349;
    /// Frame `bl_std18`.
    pub const BL_STD18: i32 = 350;
    /// Frame `bl_std19`.
    pub const BL_STD19: i32 = 351;
    /// Frame `bl_std20`.
    pub const BL_STD20: i32 = 352;
    /// Frame `bl_std21`.
    pub const BL_STD21: i32 = 353;
    /// Frame `bl_std22`.
    pub const BL_STD22: i32 = 354;
    /// Frame `bl_std23`.
    pub const BL_STD23: i32 = 355;
    /// Frame `bl_std24`.
    pub const BL_STD24: i32 = 356;
    /// Frame `bl_std25`.
    pub const BL_STD25: i32 = 357;
    /// Frame `bl_std26`.
    pub const BL_STD26: i32 = 358;
    /// Frame `bl_std27`.
    pub const BL_STD27: i32 = 359;
    /// Frame `bl_std28`.
    pub const BL_STD28: i32 = 360;
    /// Frame `bl_std29`.
    pub const BL_STD29: i32 = 361;
    /// Frame `bl_std30`.
    pub const BL_STD30: i32 = 362;
    /// Frame `bl_std31`.
    pub const BL_STD31: i32 = 363;
    /// Frame `bl_std32`.
    pub const BL_STD32: i32 = 364;
    /// Frame `bl_std33`.
    pub const BL_STD33: i32 = 365;
    /// Frame `bl_std34`.
    pub const BL_STD34: i32 = 366;
    /// Frame `bl_std35`.
    pub const BL_STD35: i32 = 367;
    /// Frame `bl_std36`.
    pub const BL_STD36: i32 = 368;
    /// Frame `bl_std37`.
    pub const BL_STD37: i32 = 369;
    /// Frame `bl_std38`.
    pub const BL_STD38: i32 = 370;
    /// Frame `bl_std39`.
    pub const BL_STD39: i32 = 371;
    /// Frame `bl_std40`.
    pub const BL_STD40: i32 = 372;
    /// Frame `bl_swm01`.
    pub const BL_SWM01: i32 = 373;
    /// Frame `bl_swm02`.
    pub const BL_SWM02: i32 = 374;
    /// Frame `bl_swm03`.
    pub const BL_SWM03: i32 = 375;
    /// Frame `bl_swm04`.
    pub const BL_SWM04: i32 = 376;
    /// Frame `bl_swm05`.
    pub const BL_SWM05: i32 = 377;
    /// Frame `bl_swm06`.
    pub const BL_SWM06: i32 = 378;
    /// Frame `bl_swm07`.
    pub const BL_SWM07: i32 = 379;
    /// Frame `bl_swm08`.
    pub const BL_SWM08: i32 = 380;
    /// Frame `bl_swm09`.
    pub const BL_SWM09: i32 = 381;
    /// Frame `bl_swm10`.
    pub const BL_SWM10: i32 = 382;
    /// Frame `bl_swm11`.
    pub const BL_SWM11: i32 = 383;
    /// Frame `bl_swm12`.
    pub const BL_SWM12: i32 = 384;
    /// Frame `bl_swk01`.
    pub const BL_SWK01: i32 = 385;
    /// Frame `bl_swk02`.
    pub const BL_SWK02: i32 = 386;
    /// Frame `bl_swk03`.
    pub const BL_SWK03: i32 = 387;
    /// Frame `bl_swk04`.
    pub const BL_SWK04: i32 = 388;
    /// Frame `bl_swk05`.
    pub const BL_SWK05: i32 = 389;
    /// Frame `bl_swk06`.
    pub const BL_SWK06: i32 = 390;
    /// Frame `bl_swp01`.
    pub const BL_SWP01: i32 = 391;
    /// Frame `bl_swp02`.
    pub const BL_SWP02: i32 = 392;
    /// Frame `bl_swp03`.
    pub const BL_SWP03: i32 = 393;
    /// Frame `bl_swp04`.
    pub const BL_SWP04: i32 = 394;
    /// Frame `bl_swp05`.
    pub const BL_SWP05: i32 = 395;
    /// Frame `bl_sws01`.
    pub const BL_SWS01: i32 = 396;
    /// Frame `bl_sws02`.
    pub const BL_SWS02: i32 = 397;
    /// Frame `bl_sws03`.
    pub const BL_SWS03: i32 = 398;
    /// Frame `bl_sws04`.
    pub const BL_SWS04: i32 = 399;
    /// Frame `bl_sws05`.
    pub const BL_SWS05: i32 = 400;
    /// Frame `bl_sws06`.
    pub const BL_SWS06: i32 = 401;
    /// Frame `bl_sws07`.
    pub const BL_SWS07: i32 = 402;
    /// Frame `bl_sws08`.
    pub const BL_SWS08: i32 = 403;
    /// Frame `bl_sws09`.
    pub const BL_SWS09: i32 = 404;
    /// Frame `bl_sws10`.
    pub const BL_SWS10: i32 = 405;
    /// Frame `bl_sws11`.
    pub const BL_SWS11: i32 = 406;
    /// Frame `bl_sws12`.
    pub const BL_SWS12: i32 = 407;
    /// Frame `bl_sws13`.
    pub const BL_SWS13: i32 = 408;
    /// Frame `bl_sws14`.
    pub const BL_SWS14: i32 = 409;
    /// Frame `bl_tau14`.
    pub const BL_TAU14: i32 = 410;
    /// Frame `bl_tau15`.
    pub const BL_TAU15: i32 = 411;
    /// Frame `bl_tau16`.
    pub const BL_TAU16: i32 = 412;
    /// Frame `bl_tau17`.
    pub const BL_TAU17: i32 = 413;
    /// Frame `bl_wlk01`.
    pub const BL_WLK01: i32 = 414;
    /// Frame `bl_wlk02`.
    pub const BL_WLK02: i32 = 415;
    /// Frame `bl_wlk03`.
    pub const BL_WLK03: i32 = 416;
    /// Frame `bl_wlk04`.
    pub const BL_WLK04: i32 = 417;
    /// Frame `bl_wlk05`.
    pub const BL_WLK05: i32 = 418;
    /// Frame `bl_wlk06`.
    pub const BL_WLK06: i32 = 419;
    /// Frame `bl_wlk07`.
    pub const BL_WLK07: i32 = 420;
    /// Frame `bl_wlk08`.
    pub const BL_WLK08: i32 = 421;
    /// Frame `bl_wlk09`.
    pub const BL_WLK09: i32 = 422;
    /// Frame `bl_wlk10`.
    pub const BL_WLK10: i32 = 423;
    /// Frame `bl_wlk11`.
    pub const BL_WLK11: i32 = 424;
    /// Frame `bl_wav19`.
    pub const BL_WAV19: i32 = 425;
    /// Frame `bl_wav20`.
    pub const BL_WAV20: i32 = 426;
    /// Frame `bl_wav21`.
    pub const BL_WAV21: i32 = 427;
    /// Frame `cr_atk01`.
    pub const CR_ATK01: i32 = 428;
    /// Frame `cr_atk02`.
    pub const CR_ATK02: i32 = 429;
    /// Frame `cr_atk03`.
    pub const CR_ATK03: i32 = 430;
    /// Frame `cr_atk04`.
    pub const CR_ATK04: i32 = 431;
    /// Frame `cr_atk05`.
    pub const CR_ATK05: i32 = 432;
    /// Frame `cr_atk06`.
    pub const CR_ATK06: i32 = 433;
    /// Frame `cr_atk07`.
    pub const CR_ATK07: i32 = 434;
    /// Frame `cr_atk08`.
    pub const CR_ATK08: i32 = 435;
    /// Frame `cr_pan01`.
    pub const CR_PAN01: i32 = 436;
    /// Frame `cr_pan02`.
    pub const CR_PAN02: i32 = 437;
    /// Frame `cr_pan03`.
    pub const CR_PAN03: i32 = 438;
    /// Frame `cr_pan04`.
    pub const CR_PAN04: i32 = 439;
    /// Frame `cr_std01`.
    pub const CR_STD01: i32 = 440;
    /// Frame `cr_std02`.
    pub const CR_STD02: i32 = 441;
    /// Frame `cr_std03`.
    pub const CR_STD03: i32 = 442;
    /// Frame `cr_std04`.
    pub const CR_STD04: i32 = 443;
    /// Frame `cr_std05`.
    pub const CR_STD05: i32 = 444;
    /// Frame `cr_std06`.
    pub const CR_STD06: i32 = 445;
    /// Frame `cr_std07`.
    pub const CR_STD07: i32 = 446;
    /// Frame `cr_std08`.
    pub const CR_STD08: i32 = 447;
    /// Frame `cr_wlk01`.
    pub const CR_WLK01: i32 = 448;
    /// Frame `cr_wlk02`.
    pub const CR_WLK02: i32 = 449;
    /// Frame `cr_wlk03`.
    pub const CR_WLK03: i32 = 450;
    /// Frame `cr_wlk04`.
    pub const CR_WLK04: i32 = 451;
    /// Frame `cr_wlk05`.
    pub const CR_WLK05: i32 = 452;
    /// Frame `cr_wlk06`.
    pub const CR_WLK06: i32 = 453;
    /// Frame `cr_wlk07`.
    pub const CR_WLK07: i32 = 454;
    /// Frame `crbl_a01`.
    pub const CRBL_A01: i32 = 455;
    /// Frame `crbl_a02`.
    pub const CRBL_A02: i32 = 456;
    /// Frame `crbl_a03`.
    pub const CRBL_A03: i32 = 457;
    /// Frame `crbl_a04`.
    pub const CRBL_A04: i32 = 458;
    /// Frame `crbl_a05`.
    pub const CRBL_A05: i32 = 459;
    /// Frame `crbl_a06`.
    pub const CRBL_A06: i32 = 460;
    /// Frame `crbl_a07`.
    pub const CRBL_A07: i32 = 461;
    /// Frame `crbl_p01`.
    pub const CRBL_P01: i32 = 462;
    /// Frame `crbl_p02`.
    pub const CRBL_P02: i32 = 463;
    /// Frame `crbl_p03`.
    pub const CRBL_P03: i32 = 464;
    /// Frame `crbl_p04`.
    pub const CRBL_P04: i32 = 465;
    /// Frame `crbl_s01`.
    pub const CRBL_S01: i32 = 466;
    /// Frame `crbl_s02`.
    pub const CRBL_S02: i32 = 467;
    /// Frame `crbl_s03`.
    pub const CRBL_S03: i32 = 468;
    /// Frame `crbl_s04`.
    pub const CRBL_S04: i32 = 469;
    /// Frame `crbl_s05`.
    pub const CRBL_S05: i32 = 470;
    /// Frame `crbl_s06`.
    pub const CRBL_S06: i32 = 471;
    /// Frame `crbl_s07`.
    pub const CRBL_S07: i32 = 472;
    /// Frame `crbl_s08`.
    pub const CRBL_S08: i32 = 473;
    /// Frame `crbl_w01`.
    pub const CRBL_W01: i32 = 474;
    /// Frame `crbl_w02`.
    pub const CRBL_W02: i32 = 475;
    /// Frame `crbl_w03`.
    pub const CRBL_W03: i32 = 476;
    /// Frame `crbl_w04`.
    pub const CRBL_W04: i32 = 477;
    /// Frame `crbl_w05`.
    pub const CRBL_W05: i32 = 478;
    /// Frame `crbl_w06`.
    pub const CRBL_W06: i32 = 479;
    /// Frame `crbl_w07`.
    pub const CRBL_W07: i32 = 480;
}

/// `actorMoves` move tables.
pub fn actor_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("actor_move_stand", 128, 167, None, vec![
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
        monster_move("actor_move_walk", 251, 258, None, vec![
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (1f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_run", 93, 98, None, vec![
            monster_frame(MonsterAi::Run, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (15f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (15f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (20f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (15f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_pain1", 74, 76, Some("actor_run"), vec![
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_pain2", 77, 79, Some("actor_run"), vec![
            monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_pain3", 80, 82, Some("actor_run"), vec![
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_flipoff", 39, 52, Some("actor_run"), vec![
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_taunt", 234, 250, Some("actor_run"), vec![
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Turn, (0f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_death1", 4, 10, Some("actor_dead"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-13f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (14f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_death2", 11, 23, Some("actor_dead"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-13f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-13f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("actor_move_attack", 0, 3, Some("actor_run"), vec![
            monster_frame(MonsterAi::Charge, (-2f32) as f64, vec![MonsterAction::name("actor_fire")], -1),
            monster_frame(MonsterAi::Charge, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
        ]),
    ]
}
