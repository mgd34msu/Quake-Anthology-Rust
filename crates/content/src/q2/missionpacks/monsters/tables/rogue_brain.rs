//! brain move tables (`src/content/q2/missionpacks/monsters/tables/rogue-brain.ts`).
//!
//! Original Quake II rogue/m_brain.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `brainFrame`.
pub mod brain_frame {
    /// Frame `walk101`.
    pub const WALK101: i32 = 0;
    /// Frame `walk102`.
    pub const WALK102: i32 = 1;
    /// Frame `walk103`.
    pub const WALK103: i32 = 2;
    /// Frame `walk104`.
    pub const WALK104: i32 = 3;
    /// Frame `walk105`.
    pub const WALK105: i32 = 4;
    /// Frame `walk106`.
    pub const WALK106: i32 = 5;
    /// Frame `walk107`.
    pub const WALK107: i32 = 6;
    /// Frame `walk108`.
    pub const WALK108: i32 = 7;
    /// Frame `walk109`.
    pub const WALK109: i32 = 8;
    /// Frame `walk110`.
    pub const WALK110: i32 = 9;
    /// Frame `walk111`.
    pub const WALK111: i32 = 10;
    /// Frame `walk112`.
    pub const WALK112: i32 = 11;
    /// Frame `walk113`.
    pub const WALK113: i32 = 12;
    /// Frame `walk201`.
    pub const WALK201: i32 = 13;
    /// Frame `walk202`.
    pub const WALK202: i32 = 14;
    /// Frame `walk203`.
    pub const WALK203: i32 = 15;
    /// Frame `walk204`.
    pub const WALK204: i32 = 16;
    /// Frame `walk205`.
    pub const WALK205: i32 = 17;
    /// Frame `walk206`.
    pub const WALK206: i32 = 18;
    /// Frame `walk207`.
    pub const WALK207: i32 = 19;
    /// Frame `walk208`.
    pub const WALK208: i32 = 20;
    /// Frame `walk209`.
    pub const WALK209: i32 = 21;
    /// Frame `walk210`.
    pub const WALK210: i32 = 22;
    /// Frame `walk211`.
    pub const WALK211: i32 = 23;
    /// Frame `walk212`.
    pub const WALK212: i32 = 24;
    /// Frame `walk213`.
    pub const WALK213: i32 = 25;
    /// Frame `walk214`.
    pub const WALK214: i32 = 26;
    /// Frame `walk215`.
    pub const WALK215: i32 = 27;
    /// Frame `walk216`.
    pub const WALK216: i32 = 28;
    /// Frame `walk217`.
    pub const WALK217: i32 = 29;
    /// Frame `walk218`.
    pub const WALK218: i32 = 30;
    /// Frame `walk219`.
    pub const WALK219: i32 = 31;
    /// Frame `walk220`.
    pub const WALK220: i32 = 32;
    /// Frame `walk221`.
    pub const WALK221: i32 = 33;
    /// Frame `walk222`.
    pub const WALK222: i32 = 34;
    /// Frame `walk223`.
    pub const WALK223: i32 = 35;
    /// Frame `walk224`.
    pub const WALK224: i32 = 36;
    /// Frame `walk225`.
    pub const WALK225: i32 = 37;
    /// Frame `walk226`.
    pub const WALK226: i32 = 38;
    /// Frame `walk227`.
    pub const WALK227: i32 = 39;
    /// Frame `walk228`.
    pub const WALK228: i32 = 40;
    /// Frame `walk229`.
    pub const WALK229: i32 = 41;
    /// Frame `walk230`.
    pub const WALK230: i32 = 42;
    /// Frame `walk231`.
    pub const WALK231: i32 = 43;
    /// Frame `walk232`.
    pub const WALK232: i32 = 44;
    /// Frame `walk233`.
    pub const WALK233: i32 = 45;
    /// Frame `walk234`.
    pub const WALK234: i32 = 46;
    /// Frame `walk235`.
    pub const WALK235: i32 = 47;
    /// Frame `walk236`.
    pub const WALK236: i32 = 48;
    /// Frame `walk237`.
    pub const WALK237: i32 = 49;
    /// Frame `walk238`.
    pub const WALK238: i32 = 50;
    /// Frame `walk239`.
    pub const WALK239: i32 = 51;
    /// Frame `walk240`.
    pub const WALK240: i32 = 52;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 53;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 54;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 55;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 56;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 57;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 58;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 59;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 60;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 61;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 62;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 63;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 64;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 65;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 66;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 67;
    /// Frame `attak116`.
    pub const ATTAK116: i32 = 68;
    /// Frame `attak117`.
    pub const ATTAK117: i32 = 69;
    /// Frame `attak118`.
    pub const ATTAK118: i32 = 70;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 71;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 72;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 73;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 74;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 75;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 76;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 77;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 78;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 79;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 80;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 81;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 82;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 83;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 84;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 85;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 86;
    /// Frame `attak217`.
    pub const ATTAK217: i32 = 87;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 88;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 89;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 90;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 91;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 92;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 93;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 94;
    /// Frame `pain108`.
    pub const PAIN108: i32 = 95;
    /// Frame `pain109`.
    pub const PAIN109: i32 = 96;
    /// Frame `pain110`.
    pub const PAIN110: i32 = 97;
    /// Frame `pain111`.
    pub const PAIN111: i32 = 98;
    /// Frame `pain112`.
    pub const PAIN112: i32 = 99;
    /// Frame `pain113`.
    pub const PAIN113: i32 = 100;
    /// Frame `pain114`.
    pub const PAIN114: i32 = 101;
    /// Frame `pain115`.
    pub const PAIN115: i32 = 102;
    /// Frame `pain116`.
    pub const PAIN116: i32 = 103;
    /// Frame `pain117`.
    pub const PAIN117: i32 = 104;
    /// Frame `pain118`.
    pub const PAIN118: i32 = 105;
    /// Frame `pain119`.
    pub const PAIN119: i32 = 106;
    /// Frame `pain120`.
    pub const PAIN120: i32 = 107;
    /// Frame `pain121`.
    pub const PAIN121: i32 = 108;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 109;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 110;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 111;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 112;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 113;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 114;
    /// Frame `pain207`.
    pub const PAIN207: i32 = 115;
    /// Frame `pain208`.
    pub const PAIN208: i32 = 116;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 117;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 118;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 119;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 120;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 121;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 122;
    /// Frame `death101`.
    pub const DEATH101: i32 = 123;
    /// Frame `death102`.
    pub const DEATH102: i32 = 124;
    /// Frame `death103`.
    pub const DEATH103: i32 = 125;
    /// Frame `death104`.
    pub const DEATH104: i32 = 126;
    /// Frame `death105`.
    pub const DEATH105: i32 = 127;
    /// Frame `death106`.
    pub const DEATH106: i32 = 128;
    /// Frame `death107`.
    pub const DEATH107: i32 = 129;
    /// Frame `death108`.
    pub const DEATH108: i32 = 130;
    /// Frame `death109`.
    pub const DEATH109: i32 = 131;
    /// Frame `death110`.
    pub const DEATH110: i32 = 132;
    /// Frame `death111`.
    pub const DEATH111: i32 = 133;
    /// Frame `death112`.
    pub const DEATH112: i32 = 134;
    /// Frame `death113`.
    pub const DEATH113: i32 = 135;
    /// Frame `death114`.
    pub const DEATH114: i32 = 136;
    /// Frame `death115`.
    pub const DEATH115: i32 = 137;
    /// Frame `death116`.
    pub const DEATH116: i32 = 138;
    /// Frame `death117`.
    pub const DEATH117: i32 = 139;
    /// Frame `death118`.
    pub const DEATH118: i32 = 140;
    /// Frame `death201`.
    pub const DEATH201: i32 = 141;
    /// Frame `death202`.
    pub const DEATH202: i32 = 142;
    /// Frame `death203`.
    pub const DEATH203: i32 = 143;
    /// Frame `death204`.
    pub const DEATH204: i32 = 144;
    /// Frame `death205`.
    pub const DEATH205: i32 = 145;
    /// Frame `duck01`.
    pub const DUCK01: i32 = 146;
    /// Frame `duck02`.
    pub const DUCK02: i32 = 147;
    /// Frame `duck03`.
    pub const DUCK03: i32 = 148;
    /// Frame `duck04`.
    pub const DUCK04: i32 = 149;
    /// Frame `duck05`.
    pub const DUCK05: i32 = 150;
    /// Frame `duck06`.
    pub const DUCK06: i32 = 151;
    /// Frame `duck07`.
    pub const DUCK07: i32 = 152;
    /// Frame `duck08`.
    pub const DUCK08: i32 = 153;
    /// Frame `defens01`.
    pub const DEFENS01: i32 = 154;
    /// Frame `defens02`.
    pub const DEFENS02: i32 = 155;
    /// Frame `defens03`.
    pub const DEFENS03: i32 = 156;
    /// Frame `defens04`.
    pub const DEFENS04: i32 = 157;
    /// Frame `defens05`.
    pub const DEFENS05: i32 = 158;
    /// Frame `defens06`.
    pub const DEFENS06: i32 = 159;
    /// Frame `defens07`.
    pub const DEFENS07: i32 = 160;
    /// Frame `defens08`.
    pub const DEFENS08: i32 = 161;
    /// Frame `stand01`.
    pub const STAND01: i32 = 162;
    /// Frame `stand02`.
    pub const STAND02: i32 = 163;
    /// Frame `stand03`.
    pub const STAND03: i32 = 164;
    /// Frame `stand04`.
    pub const STAND04: i32 = 165;
    /// Frame `stand05`.
    pub const STAND05: i32 = 166;
    /// Frame `stand06`.
    pub const STAND06: i32 = 167;
    /// Frame `stand07`.
    pub const STAND07: i32 = 168;
    /// Frame `stand08`.
    pub const STAND08: i32 = 169;
    /// Frame `stand09`.
    pub const STAND09: i32 = 170;
    /// Frame `stand10`.
    pub const STAND10: i32 = 171;
    /// Frame `stand11`.
    pub const STAND11: i32 = 172;
    /// Frame `stand12`.
    pub const STAND12: i32 = 173;
    /// Frame `stand13`.
    pub const STAND13: i32 = 174;
    /// Frame `stand14`.
    pub const STAND14: i32 = 175;
    /// Frame `stand15`.
    pub const STAND15: i32 = 176;
    /// Frame `stand16`.
    pub const STAND16: i32 = 177;
    /// Frame `stand17`.
    pub const STAND17: i32 = 178;
    /// Frame `stand18`.
    pub const STAND18: i32 = 179;
    /// Frame `stand19`.
    pub const STAND19: i32 = 180;
    /// Frame `stand20`.
    pub const STAND20: i32 = 181;
    /// Frame `stand21`.
    pub const STAND21: i32 = 182;
    /// Frame `stand22`.
    pub const STAND22: i32 = 183;
    /// Frame `stand23`.
    pub const STAND23: i32 = 184;
    /// Frame `stand24`.
    pub const STAND24: i32 = 185;
    /// Frame `stand25`.
    pub const STAND25: i32 = 186;
    /// Frame `stand26`.
    pub const STAND26: i32 = 187;
    /// Frame `stand27`.
    pub const STAND27: i32 = 188;
    /// Frame `stand28`.
    pub const STAND28: i32 = 189;
    /// Frame `stand29`.
    pub const STAND29: i32 = 190;
    /// Frame `stand30`.
    pub const STAND30: i32 = 191;
    /// Frame `stand31`.
    pub const STAND31: i32 = 192;
    /// Frame `stand32`.
    pub const STAND32: i32 = 193;
    /// Frame `stand33`.
    pub const STAND33: i32 = 194;
    /// Frame `stand34`.
    pub const STAND34: i32 = 195;
    /// Frame `stand35`.
    pub const STAND35: i32 = 196;
    /// Frame `stand36`.
    pub const STAND36: i32 = 197;
    /// Frame `stand37`.
    pub const STAND37: i32 = 198;
    /// Frame `stand38`.
    pub const STAND38: i32 = 199;
    /// Frame `stand39`.
    pub const STAND39: i32 = 200;
    /// Frame `stand40`.
    pub const STAND40: i32 = 201;
    /// Frame `stand41`.
    pub const STAND41: i32 = 202;
    /// Frame `stand42`.
    pub const STAND42: i32 = 203;
    /// Frame `stand43`.
    pub const STAND43: i32 = 204;
    /// Frame `stand44`.
    pub const STAND44: i32 = 205;
    /// Frame `stand45`.
    pub const STAND45: i32 = 206;
    /// Frame `stand46`.
    pub const STAND46: i32 = 207;
    /// Frame `stand47`.
    pub const STAND47: i32 = 208;
    /// Frame `stand48`.
    pub const STAND48: i32 = 209;
    /// Frame `stand49`.
    pub const STAND49: i32 = 210;
    /// Frame `stand50`.
    pub const STAND50: i32 = 211;
    /// Frame `stand51`.
    pub const STAND51: i32 = 212;
    /// Frame `stand52`.
    pub const STAND52: i32 = 213;
    /// Frame `stand53`.
    pub const STAND53: i32 = 214;
    /// Frame `stand54`.
    pub const STAND54: i32 = 215;
    /// Frame `stand55`.
    pub const STAND55: i32 = 216;
    /// Frame `stand56`.
    pub const STAND56: i32 = 217;
    /// Frame `stand57`.
    pub const STAND57: i32 = 218;
    /// Frame `stand58`.
    pub const STAND58: i32 = 219;
    /// Frame `stand59`.
    pub const STAND59: i32 = 220;
    /// Frame `stand60`.
    pub const STAND60: i32 = 221;
}

/// `brainMoves` move tables.
pub fn brain_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("brain_move_stand", 162, 191, None, vec![
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
        monster_move("brain_move_idle", 192, 221, Some("brain_stand"), vec![
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
        monster_move("brain_move_walk1", 0, 10, None, vec![
            monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 9.0, vec![], -1),
            monster_frame(MonsterAi::Walk, -4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, -1.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
        ]),
        monster_move("brain_move_defense", 154, 161, None, vec![
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
        monster_move("brain_move_pain3", 117, 122, Some("brain_run"), vec![
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
        ]),
        monster_move("brain_move_pain2", 109, 116, Some("brain_run"), vec![
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
        ]),
        monster_move("brain_move_pain1", 88, 108, Some("brain_run"), vec![
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 7.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
        ]),
        monster_move("brain_move_duck", 146, 153, Some("brain_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![MonsterAction::name("monster_duck_down")], -1),
            monster_frame(MonsterAi::Move, 17.0, vec![MonsterAction::name("monster_duck_hold")], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![MonsterAction::name("monster_duck_up")], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
        ]),
        monster_move("brain_move_death2", 141, 145, Some("brain_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 9.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("brain_move_death1", 123, 140, Some("brain_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 9.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
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
        monster_move("brain_move_attack1", 53, 70, Some("brain_run"), vec![
            monster_frame(MonsterAi::Charge, 8.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -3.0, vec![MonsterAction::name("brain_swing_right")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -7.0, vec![MonsterAction::name("brain_hit_right")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![MonsterAction::name("brain_swing_left")], -1),
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![MonsterAction::name("brain_hit_left")], -1),
            monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -11.0, vec![], -1),
        ]),
        monster_move("brain_move_attack2", 71, 87, Some("brain_run"), vec![
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("brain_chest_open")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 13.0, vec![MonsterAction::name("brain_tentacle_attack")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -9.0, vec![MonsterAction::name("brain_chest_closed")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
        ]),
        monster_move("brain_move_run", 0, 10, None, vec![
            monster_frame(MonsterAi::Run, 9.0, vec![], -1),
            monster_frame(MonsterAi::Run, 2.0, vec![], -1),
            monster_frame(MonsterAi::Run, 3.0, vec![], -1),
            monster_frame(MonsterAi::Run, 3.0, vec![], -1),
            monster_frame(MonsterAi::Run, 1.0, vec![], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![], -1),
            monster_frame(MonsterAi::Run, 0.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, -4.0, vec![], -1),
            monster_frame(MonsterAi::Run, -1.0, vec![], -1),
            monster_frame(MonsterAi::Run, 2.0, vec![], -1),
        ]),
    ]
}
