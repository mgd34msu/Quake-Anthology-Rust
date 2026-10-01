//! tank move tables (`src/content/q2/rerelease/monsters/tables/tank.ts`).

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `tankFrame`.
pub mod tank_frame {
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
    /// Frame `walk01`.
    pub const WALK01: i32 = 30;
    /// Frame `walk02`.
    pub const WALK02: i32 = 31;
    /// Frame `walk03`.
    pub const WALK03: i32 = 32;
    /// Frame `walk04`.
    pub const WALK04: i32 = 33;
    /// Frame `walk05`.
    pub const WALK05: i32 = 34;
    /// Frame `walk06`.
    pub const WALK06: i32 = 35;
    /// Frame `walk07`.
    pub const WALK07: i32 = 36;
    /// Frame `walk08`.
    pub const WALK08: i32 = 37;
    /// Frame `walk09`.
    pub const WALK09: i32 = 38;
    /// Frame `walk10`.
    pub const WALK10: i32 = 39;
    /// Frame `walk11`.
    pub const WALK11: i32 = 40;
    /// Frame `walk12`.
    pub const WALK12: i32 = 41;
    /// Frame `walk13`.
    pub const WALK13: i32 = 42;
    /// Frame `walk14`.
    pub const WALK14: i32 = 43;
    /// Frame `walk15`.
    pub const WALK15: i32 = 44;
    /// Frame `walk16`.
    pub const WALK16: i32 = 45;
    /// Frame `walk17`.
    pub const WALK17: i32 = 46;
    /// Frame `walk18`.
    pub const WALK18: i32 = 47;
    /// Frame `walk19`.
    pub const WALK19: i32 = 48;
    /// Frame `walk20`.
    pub const WALK20: i32 = 49;
    /// Frame `walk21`.
    pub const WALK21: i32 = 50;
    /// Frame `walk22`.
    pub const WALK22: i32 = 51;
    /// Frame `walk23`.
    pub const WALK23: i32 = 52;
    /// Frame `walk24`.
    pub const WALK24: i32 = 53;
    /// Frame `walk25`.
    pub const WALK25: i32 = 54;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 55;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 56;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 57;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 58;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 59;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 60;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 61;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 62;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 63;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 64;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 65;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 66;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 67;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 68;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 69;
    /// Frame `attak116`.
    pub const ATTAK116: i32 = 70;
    /// Frame `attak117`.
    pub const ATTAK117: i32 = 71;
    /// Frame `attak118`.
    pub const ATTAK118: i32 = 72;
    /// Frame `attak119`.
    pub const ATTAK119: i32 = 73;
    /// Frame `attak120`.
    pub const ATTAK120: i32 = 74;
    /// Frame `attak121`.
    pub const ATTAK121: i32 = 75;
    /// Frame `attak122`.
    pub const ATTAK122: i32 = 76;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 77;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 78;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 79;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 80;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 81;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 82;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 83;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 84;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 85;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 86;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 87;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 88;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 89;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 90;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 91;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 92;
    /// Frame `attak217`.
    pub const ATTAK217: i32 = 93;
    /// Frame `attak218`.
    pub const ATTAK218: i32 = 94;
    /// Frame `attak219`.
    pub const ATTAK219: i32 = 95;
    /// Frame `attak220`.
    pub const ATTAK220: i32 = 96;
    /// Frame `attak221`.
    pub const ATTAK221: i32 = 97;
    /// Frame `attak222`.
    pub const ATTAK222: i32 = 98;
    /// Frame `attak223`.
    pub const ATTAK223: i32 = 99;
    /// Frame `attak224`.
    pub const ATTAK224: i32 = 100;
    /// Frame `attak225`.
    pub const ATTAK225: i32 = 101;
    /// Frame `attak226`.
    pub const ATTAK226: i32 = 102;
    /// Frame `attak227`.
    pub const ATTAK227: i32 = 103;
    /// Frame `attak228`.
    pub const ATTAK228: i32 = 104;
    /// Frame `attak229`.
    pub const ATTAK229: i32 = 105;
    /// Frame `attak230`.
    pub const ATTAK230: i32 = 106;
    /// Frame `attak231`.
    pub const ATTAK231: i32 = 107;
    /// Frame `attak232`.
    pub const ATTAK232: i32 = 108;
    /// Frame `attak233`.
    pub const ATTAK233: i32 = 109;
    /// Frame `attak234`.
    pub const ATTAK234: i32 = 110;
    /// Frame `attak235`.
    pub const ATTAK235: i32 = 111;
    /// Frame `attak236`.
    pub const ATTAK236: i32 = 112;
    /// Frame `attak237`.
    pub const ATTAK237: i32 = 113;
    /// Frame `attak238`.
    pub const ATTAK238: i32 = 114;
    /// Frame `attak301`.
    pub const ATTAK301: i32 = 115;
    /// Frame `attak302`.
    pub const ATTAK302: i32 = 116;
    /// Frame `attak303`.
    pub const ATTAK303: i32 = 117;
    /// Frame `attak304`.
    pub const ATTAK304: i32 = 118;
    /// Frame `attak305`.
    pub const ATTAK305: i32 = 119;
    /// Frame `attak306`.
    pub const ATTAK306: i32 = 120;
    /// Frame `attak307`.
    pub const ATTAK307: i32 = 121;
    /// Frame `attak308`.
    pub const ATTAK308: i32 = 122;
    /// Frame `attak309`.
    pub const ATTAK309: i32 = 123;
    /// Frame `attak310`.
    pub const ATTAK310: i32 = 124;
    /// Frame `attak311`.
    pub const ATTAK311: i32 = 125;
    /// Frame `attak312`.
    pub const ATTAK312: i32 = 126;
    /// Frame `attak313`.
    pub const ATTAK313: i32 = 127;
    /// Frame `attak314`.
    pub const ATTAK314: i32 = 128;
    /// Frame `attak315`.
    pub const ATTAK315: i32 = 129;
    /// Frame `attak316`.
    pub const ATTAK316: i32 = 130;
    /// Frame `attak317`.
    pub const ATTAK317: i32 = 131;
    /// Frame `attak318`.
    pub const ATTAK318: i32 = 132;
    /// Frame `attak319`.
    pub const ATTAK319: i32 = 133;
    /// Frame `attak320`.
    pub const ATTAK320: i32 = 134;
    /// Frame `attak321`.
    pub const ATTAK321: i32 = 135;
    /// Frame `attak322`.
    pub const ATTAK322: i32 = 136;
    /// Frame `attak323`.
    pub const ATTAK323: i32 = 137;
    /// Frame `attak324`.
    pub const ATTAK324: i32 = 138;
    /// Frame `attak325`.
    pub const ATTAK325: i32 = 139;
    /// Frame `attak326`.
    pub const ATTAK326: i32 = 140;
    /// Frame `attak327`.
    pub const ATTAK327: i32 = 141;
    /// Frame `attak328`.
    pub const ATTAK328: i32 = 142;
    /// Frame `attak329`.
    pub const ATTAK329: i32 = 143;
    /// Frame `attak330`.
    pub const ATTAK330: i32 = 144;
    /// Frame `attak331`.
    pub const ATTAK331: i32 = 145;
    /// Frame `attak332`.
    pub const ATTAK332: i32 = 146;
    /// Frame `attak333`.
    pub const ATTAK333: i32 = 147;
    /// Frame `attak334`.
    pub const ATTAK334: i32 = 148;
    /// Frame `attak335`.
    pub const ATTAK335: i32 = 149;
    /// Frame `attak336`.
    pub const ATTAK336: i32 = 150;
    /// Frame `attak337`.
    pub const ATTAK337: i32 = 151;
    /// Frame `attak338`.
    pub const ATTAK338: i32 = 152;
    /// Frame `attak339`.
    pub const ATTAK339: i32 = 153;
    /// Frame `attak340`.
    pub const ATTAK340: i32 = 154;
    /// Frame `attak341`.
    pub const ATTAK341: i32 = 155;
    /// Frame `attak342`.
    pub const ATTAK342: i32 = 156;
    /// Frame `attak343`.
    pub const ATTAK343: i32 = 157;
    /// Frame `attak344`.
    pub const ATTAK344: i32 = 158;
    /// Frame `attak345`.
    pub const ATTAK345: i32 = 159;
    /// Frame `attak346`.
    pub const ATTAK346: i32 = 160;
    /// Frame `attak347`.
    pub const ATTAK347: i32 = 161;
    /// Frame `attak348`.
    pub const ATTAK348: i32 = 162;
    /// Frame `attak349`.
    pub const ATTAK349: i32 = 163;
    /// Frame `attak350`.
    pub const ATTAK350: i32 = 164;
    /// Frame `attak351`.
    pub const ATTAK351: i32 = 165;
    /// Frame `attak352`.
    pub const ATTAK352: i32 = 166;
    /// Frame `attak353`.
    pub const ATTAK353: i32 = 167;
    /// Frame `attak401`.
    pub const ATTAK401: i32 = 168;
    /// Frame `attak402`.
    pub const ATTAK402: i32 = 169;
    /// Frame `attak403`.
    pub const ATTAK403: i32 = 170;
    /// Frame `attak404`.
    pub const ATTAK404: i32 = 171;
    /// Frame `attak405`.
    pub const ATTAK405: i32 = 172;
    /// Frame `attak406`.
    pub const ATTAK406: i32 = 173;
    /// Frame `attak407`.
    pub const ATTAK407: i32 = 174;
    /// Frame `attak408`.
    pub const ATTAK408: i32 = 175;
    /// Frame `attak409`.
    pub const ATTAK409: i32 = 176;
    /// Frame `attak410`.
    pub const ATTAK410: i32 = 177;
    /// Frame `attak411`.
    pub const ATTAK411: i32 = 178;
    /// Frame `attak412`.
    pub const ATTAK412: i32 = 179;
    /// Frame `attak413`.
    pub const ATTAK413: i32 = 180;
    /// Frame `attak414`.
    pub const ATTAK414: i32 = 181;
    /// Frame `attak415`.
    pub const ATTAK415: i32 = 182;
    /// Frame `attak416`.
    pub const ATTAK416: i32 = 183;
    /// Frame `attak417`.
    pub const ATTAK417: i32 = 184;
    /// Frame `attak418`.
    pub const ATTAK418: i32 = 185;
    /// Frame `attak419`.
    pub const ATTAK419: i32 = 186;
    /// Frame `attak420`.
    pub const ATTAK420: i32 = 187;
    /// Frame `attak421`.
    pub const ATTAK421: i32 = 188;
    /// Frame `attak422`.
    pub const ATTAK422: i32 = 189;
    /// Frame `attak423`.
    pub const ATTAK423: i32 = 190;
    /// Frame `attak424`.
    pub const ATTAK424: i32 = 191;
    /// Frame `attak425`.
    pub const ATTAK425: i32 = 192;
    /// Frame `attak426`.
    pub const ATTAK426: i32 = 193;
    /// Frame `attak427`.
    pub const ATTAK427: i32 = 194;
    /// Frame `attak428`.
    pub const ATTAK428: i32 = 195;
    /// Frame `attak429`.
    pub const ATTAK429: i32 = 196;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 197;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 198;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 199;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 200;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 201;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 202;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 203;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 204;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 205;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 206;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 207;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 208;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 209;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 210;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 211;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 212;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 213;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 214;
    /// Frame `pain310`.
    pub const PAIN310: i32 = 215;
    /// Frame `pain311`.
    pub const PAIN311: i32 = 216;
    /// Frame `pain312`.
    pub const PAIN312: i32 = 217;
    /// Frame `pain313`.
    pub const PAIN313: i32 = 218;
    /// Frame `pain314`.
    pub const PAIN314: i32 = 219;
    /// Frame `pain315`.
    pub const PAIN315: i32 = 220;
    /// Frame `pain316`.
    pub const PAIN316: i32 = 221;
    /// Frame `death101`.
    pub const DEATH101: i32 = 222;
    /// Frame `death102`.
    pub const DEATH102: i32 = 223;
    /// Frame `death103`.
    pub const DEATH103: i32 = 224;
    /// Frame `death104`.
    pub const DEATH104: i32 = 225;
    /// Frame `death105`.
    pub const DEATH105: i32 = 226;
    /// Frame `death106`.
    pub const DEATH106: i32 = 227;
    /// Frame `death107`.
    pub const DEATH107: i32 = 228;
    /// Frame `death108`.
    pub const DEATH108: i32 = 229;
    /// Frame `death109`.
    pub const DEATH109: i32 = 230;
    /// Frame `death110`.
    pub const DEATH110: i32 = 231;
    /// Frame `death111`.
    pub const DEATH111: i32 = 232;
    /// Frame `death112`.
    pub const DEATH112: i32 = 233;
    /// Frame `death113`.
    pub const DEATH113: i32 = 234;
    /// Frame `death114`.
    pub const DEATH114: i32 = 235;
    /// Frame `death115`.
    pub const DEATH115: i32 = 236;
    /// Frame `death116`.
    pub const DEATH116: i32 = 237;
    /// Frame `death117`.
    pub const DEATH117: i32 = 238;
    /// Frame `death118`.
    pub const DEATH118: i32 = 239;
    /// Frame `death119`.
    pub const DEATH119: i32 = 240;
    /// Frame `death120`.
    pub const DEATH120: i32 = 241;
    /// Frame `death121`.
    pub const DEATH121: i32 = 242;
    /// Frame `death122`.
    pub const DEATH122: i32 = 243;
    /// Frame `death123`.
    pub const DEATH123: i32 = 244;
    /// Frame `death124`.
    pub const DEATH124: i32 = 245;
    /// Frame `death125`.
    pub const DEATH125: i32 = 246;
    /// Frame `death126`.
    pub const DEATH126: i32 = 247;
    /// Frame `death127`.
    pub const DEATH127: i32 = 248;
    /// Frame `death128`.
    pub const DEATH128: i32 = 249;
    /// Frame `death129`.
    pub const DEATH129: i32 = 250;
    /// Frame `death130`.
    pub const DEATH130: i32 = 251;
    /// Frame `death131`.
    pub const DEATH131: i32 = 252;
    /// Frame `death132`.
    pub const DEATH132: i32 = 253;
    /// Frame `recln101`.
    pub const RECLN101: i32 = 254;
    /// Frame `recln102`.
    pub const RECLN102: i32 = 255;
    /// Frame `recln103`.
    pub const RECLN103: i32 = 256;
    /// Frame `recln104`.
    pub const RECLN104: i32 = 257;
    /// Frame `recln105`.
    pub const RECLN105: i32 = 258;
    /// Frame `recln106`.
    pub const RECLN106: i32 = 259;
    /// Frame `recln107`.
    pub const RECLN107: i32 = 260;
    /// Frame `recln108`.
    pub const RECLN108: i32 = 261;
    /// Frame `recln109`.
    pub const RECLN109: i32 = 262;
    /// Frame `recln110`.
    pub const RECLN110: i32 = 263;
    /// Frame `recln111`.
    pub const RECLN111: i32 = 264;
    /// Frame `recln112`.
    pub const RECLN112: i32 = 265;
    /// Frame `recln113`.
    pub const RECLN113: i32 = 266;
    /// Frame `recln114`.
    pub const RECLN114: i32 = 267;
    /// Frame `recln115`.
    pub const RECLN115: i32 = 268;
    /// Frame `recln116`.
    pub const RECLN116: i32 = 269;
    /// Frame `recln117`.
    pub const RECLN117: i32 = 270;
    /// Frame `recln118`.
    pub const RECLN118: i32 = 271;
    /// Frame `recln119`.
    pub const RECLN119: i32 = 272;
    /// Frame `recln120`.
    pub const RECLN120: i32 = 273;
    /// Frame `recln121`.
    pub const RECLN121: i32 = 274;
    /// Frame `recln122`.
    pub const RECLN122: i32 = 275;
    /// Frame `recln123`.
    pub const RECLN123: i32 = 276;
    /// Frame `recln124`.
    pub const RECLN124: i32 = 277;
    /// Frame `recln125`.
    pub const RECLN125: i32 = 278;
    /// Frame `recln126`.
    pub const RECLN126: i32 = 279;
    /// Frame `recln127`.
    pub const RECLN127: i32 = 280;
    /// Frame `recln128`.
    pub const RECLN128: i32 = 281;
    /// Frame `recln129`.
    pub const RECLN129: i32 = 282;
    /// Frame `recln130`.
    pub const RECLN130: i32 = 283;
    /// Frame `recln131`.
    pub const RECLN131: i32 = 284;
    /// Frame `recln132`.
    pub const RECLN132: i32 = 285;
    /// Frame `recln133`.
    pub const RECLN133: i32 = 286;
    /// Frame `recln134`.
    pub const RECLN134: i32 = 287;
    /// Frame `recln135`.
    pub const RECLN135: i32 = 288;
    /// Frame `recln136`.
    pub const RECLN136: i32 = 289;
    /// Frame `recln137`.
    pub const RECLN137: i32 = 290;
    /// Frame `recln138`.
    pub const RECLN138: i32 = 291;
    /// Frame `recln139`.
    pub const RECLN139: i32 = 292;
    /// Frame `recln140`.
    pub const RECLN140: i32 = 293;
}

/// `tankMoves` move tables.
pub fn tank_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("tank_move_stand", 0, 29, None, vec![
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
        monster_move("tank_move_start_walk", 30, 33, Some("tank_walk"), vec![
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (11f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_walk", 34, 49, None, vec![
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_stop_walk", 50, 54, Some("tank_stand"), vec![
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (4f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_start_run", 30, 33, Some("tank_run"), vec![
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (11f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_run", 34, 49, None, vec![
            monster_frame(MonsterAi::Run, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (4f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
            monster_frame(MonsterAi::Run, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (6f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_stop_run", 50, 54, Some("tank_walk"), vec![
            monster_frame(MonsterAi::Run, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (4f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_pain1", 197, 200, Some("tank_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("tank_move_pain2", 201, 205, Some("tank_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("tank_move_pain3", 206, 221, Some("tank_run"), vec![
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_attack_blast", 55, 70, Some("tank_reattack_blaster"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![MonsterAction::name("tank_blind_check")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankBlaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankBlaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankBlaster")], -1),
        ]),
        monster_move("tank_move_reattack_blast", 65, 70, Some("tank_reattack_blaster"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankBlaster")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankBlaster")], -1),
        ]),
        monster_move("tank_move_attack_post_blast", 71, 76, Some("tank_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_attack_strike", 77, 114, Some("tank_poststrike"), vec![
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (9f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("tank_windup")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("TankStrike")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
        ]),
        monster_move("tank_move_attack_pre_rocket", 115, 135, Some("tank_doattack_rocket"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (7f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-3f32) as f64, vec![], -1),
        ]),
        monster_move("tank_move_attack_fire_rocket", 136, 144, Some("tank_refire_rocket"), vec![
            monster_frame(MonsterAi::Charge, (-3f32) as f64, vec![MonsterAction::name("tank_blind_check")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankRocket")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("TankRocket")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![MonsterAction::name("TankRocket")], -1),
        ]),
        monster_move("tank_move_attack_post_rocket", 145, 167, Some("tank_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (-1f32) as f64, vec![MonsterAction::name("tank_footstep")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("tank_move_attack_chain", 168, 196, Some("tank_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::None, (0f32) as f64, vec![MonsterAction::name("TankMachineGun")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("tank_move_death", 222, 253, Some("tank_dead"), vec![
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![MonsterAction::name("tank_shrink")], -1),
            monster_frame(MonsterAi::Move, (-15f32) as f64, vec![MonsterAction::name("tank_thud")], -1),
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
    ]
}
