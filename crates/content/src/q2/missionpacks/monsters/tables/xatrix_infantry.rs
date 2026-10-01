//! infantry move tables (`src/content/q2/missionpacks/monsters/tables/xatrix-infantry.ts`).
//!
//! Original Quake II xatrix/m_infantry.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `infantryFrame`.
pub mod infantry_frame {
    /// Frame `gun02`.
    pub const GUN02: i32 = 0;
    /// Frame `stand01`.
    pub const STAND01: i32 = 1;
    /// Frame `stand02`.
    pub const STAND02: i32 = 2;
    /// Frame `stand03`.
    pub const STAND03: i32 = 3;
    /// Frame `stand04`.
    pub const STAND04: i32 = 4;
    /// Frame `stand05`.
    pub const STAND05: i32 = 5;
    /// Frame `stand06`.
    pub const STAND06: i32 = 6;
    /// Frame `stand07`.
    pub const STAND07: i32 = 7;
    /// Frame `stand08`.
    pub const STAND08: i32 = 8;
    /// Frame `stand09`.
    pub const STAND09: i32 = 9;
    /// Frame `stand10`.
    pub const STAND10: i32 = 10;
    /// Frame `stand11`.
    pub const STAND11: i32 = 11;
    /// Frame `stand12`.
    pub const STAND12: i32 = 12;
    /// Frame `stand13`.
    pub const STAND13: i32 = 13;
    /// Frame `stand14`.
    pub const STAND14: i32 = 14;
    /// Frame `stand15`.
    pub const STAND15: i32 = 15;
    /// Frame `stand16`.
    pub const STAND16: i32 = 16;
    /// Frame `stand17`.
    pub const STAND17: i32 = 17;
    /// Frame `stand18`.
    pub const STAND18: i32 = 18;
    /// Frame `stand19`.
    pub const STAND19: i32 = 19;
    /// Frame `stand20`.
    pub const STAND20: i32 = 20;
    /// Frame `stand21`.
    pub const STAND21: i32 = 21;
    /// Frame `stand22`.
    pub const STAND22: i32 = 22;
    /// Frame `stand23`.
    pub const STAND23: i32 = 23;
    /// Frame `stand24`.
    pub const STAND24: i32 = 24;
    /// Frame `stand25`.
    pub const STAND25: i32 = 25;
    /// Frame `stand26`.
    pub const STAND26: i32 = 26;
    /// Frame `stand27`.
    pub const STAND27: i32 = 27;
    /// Frame `stand28`.
    pub const STAND28: i32 = 28;
    /// Frame `stand29`.
    pub const STAND29: i32 = 29;
    /// Frame `stand30`.
    pub const STAND30: i32 = 30;
    /// Frame `stand31`.
    pub const STAND31: i32 = 31;
    /// Frame `stand32`.
    pub const STAND32: i32 = 32;
    /// Frame `stand33`.
    pub const STAND33: i32 = 33;
    /// Frame `stand34`.
    pub const STAND34: i32 = 34;
    /// Frame `stand35`.
    pub const STAND35: i32 = 35;
    /// Frame `stand36`.
    pub const STAND36: i32 = 36;
    /// Frame `stand37`.
    pub const STAND37: i32 = 37;
    /// Frame `stand38`.
    pub const STAND38: i32 = 38;
    /// Frame `stand39`.
    pub const STAND39: i32 = 39;
    /// Frame `stand40`.
    pub const STAND40: i32 = 40;
    /// Frame `stand41`.
    pub const STAND41: i32 = 41;
    /// Frame `stand42`.
    pub const STAND42: i32 = 42;
    /// Frame `stand43`.
    pub const STAND43: i32 = 43;
    /// Frame `stand44`.
    pub const STAND44: i32 = 44;
    /// Frame `stand45`.
    pub const STAND45: i32 = 45;
    /// Frame `stand46`.
    pub const STAND46: i32 = 46;
    /// Frame `stand47`.
    pub const STAND47: i32 = 47;
    /// Frame `stand48`.
    pub const STAND48: i32 = 48;
    /// Frame `stand49`.
    pub const STAND49: i32 = 49;
    /// Frame `stand50`.
    pub const STAND50: i32 = 50;
    /// Frame `stand51`.
    pub const STAND51: i32 = 51;
    /// Frame `stand52`.
    pub const STAND52: i32 = 52;
    /// Frame `stand53`.
    pub const STAND53: i32 = 53;
    /// Frame `stand54`.
    pub const STAND54: i32 = 54;
    /// Frame `stand55`.
    pub const STAND55: i32 = 55;
    /// Frame `stand56`.
    pub const STAND56: i32 = 56;
    /// Frame `stand57`.
    pub const STAND57: i32 = 57;
    /// Frame `stand58`.
    pub const STAND58: i32 = 58;
    /// Frame `stand59`.
    pub const STAND59: i32 = 59;
    /// Frame `stand60`.
    pub const STAND60: i32 = 60;
    /// Frame `stand61`.
    pub const STAND61: i32 = 61;
    /// Frame `stand62`.
    pub const STAND62: i32 = 62;
    /// Frame `stand63`.
    pub const STAND63: i32 = 63;
    /// Frame `stand64`.
    pub const STAND64: i32 = 64;
    /// Frame `stand65`.
    pub const STAND65: i32 = 65;
    /// Frame `stand66`.
    pub const STAND66: i32 = 66;
    /// Frame `stand67`.
    pub const STAND67: i32 = 67;
    /// Frame `stand68`.
    pub const STAND68: i32 = 68;
    /// Frame `stand69`.
    pub const STAND69: i32 = 69;
    /// Frame `stand70`.
    pub const STAND70: i32 = 70;
    /// Frame `stand71`.
    pub const STAND71: i32 = 71;
    /// Frame `walk01`.
    pub const WALK01: i32 = 72;
    /// Frame `walk02`.
    pub const WALK02: i32 = 73;
    /// Frame `walk03`.
    pub const WALK03: i32 = 74;
    /// Frame `walk04`.
    pub const WALK04: i32 = 75;
    /// Frame `walk05`.
    pub const WALK05: i32 = 76;
    /// Frame `walk06`.
    pub const WALK06: i32 = 77;
    /// Frame `walk07`.
    pub const WALK07: i32 = 78;
    /// Frame `walk08`.
    pub const WALK08: i32 = 79;
    /// Frame `walk09`.
    pub const WALK09: i32 = 80;
    /// Frame `walk10`.
    pub const WALK10: i32 = 81;
    /// Frame `walk11`.
    pub const WALK11: i32 = 82;
    /// Frame `walk12`.
    pub const WALK12: i32 = 83;
    /// Frame `walk13`.
    pub const WALK13: i32 = 84;
    /// Frame `walk14`.
    pub const WALK14: i32 = 85;
    /// Frame `walk15`.
    pub const WALK15: i32 = 86;
    /// Frame `walk16`.
    pub const WALK16: i32 = 87;
    /// Frame `walk17`.
    pub const WALK17: i32 = 88;
    /// Frame `walk18`.
    pub const WALK18: i32 = 89;
    /// Frame `walk19`.
    pub const WALK19: i32 = 90;
    /// Frame `walk20`.
    pub const WALK20: i32 = 91;
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
    /// Frame `pain101`.
    pub const PAIN101: i32 = 100;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 101;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 102;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 103;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 104;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 105;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 106;
    /// Frame `pain108`.
    pub const PAIN108: i32 = 107;
    /// Frame `pain109`.
    pub const PAIN109: i32 = 108;
    /// Frame `pain110`.
    pub const PAIN110: i32 = 109;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 110;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 111;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 112;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 113;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 114;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 115;
    /// Frame `pain207`.
    pub const PAIN207: i32 = 116;
    /// Frame `pain208`.
    pub const PAIN208: i32 = 117;
    /// Frame `pain209`.
    pub const PAIN209: i32 = 118;
    /// Frame `pain210`.
    pub const PAIN210: i32 = 119;
    /// Frame `duck01`.
    pub const DUCK01: i32 = 120;
    /// Frame `duck02`.
    pub const DUCK02: i32 = 121;
    /// Frame `duck03`.
    pub const DUCK03: i32 = 122;
    /// Frame `duck04`.
    pub const DUCK04: i32 = 123;
    /// Frame `duck05`.
    pub const DUCK05: i32 = 124;
    /// Frame `death101`.
    pub const DEATH101: i32 = 125;
    /// Frame `death102`.
    pub const DEATH102: i32 = 126;
    /// Frame `death103`.
    pub const DEATH103: i32 = 127;
    /// Frame `death104`.
    pub const DEATH104: i32 = 128;
    /// Frame `death105`.
    pub const DEATH105: i32 = 129;
    /// Frame `death106`.
    pub const DEATH106: i32 = 130;
    /// Frame `death107`.
    pub const DEATH107: i32 = 131;
    /// Frame `death108`.
    pub const DEATH108: i32 = 132;
    /// Frame `death109`.
    pub const DEATH109: i32 = 133;
    /// Frame `death110`.
    pub const DEATH110: i32 = 134;
    /// Frame `death111`.
    pub const DEATH111: i32 = 135;
    /// Frame `death112`.
    pub const DEATH112: i32 = 136;
    /// Frame `death113`.
    pub const DEATH113: i32 = 137;
    /// Frame `death114`.
    pub const DEATH114: i32 = 138;
    /// Frame `death115`.
    pub const DEATH115: i32 = 139;
    /// Frame `death116`.
    pub const DEATH116: i32 = 140;
    /// Frame `death117`.
    pub const DEATH117: i32 = 141;
    /// Frame `death118`.
    pub const DEATH118: i32 = 142;
    /// Frame `death119`.
    pub const DEATH119: i32 = 143;
    /// Frame `death120`.
    pub const DEATH120: i32 = 144;
    /// Frame `death201`.
    pub const DEATH201: i32 = 145;
    /// Frame `death202`.
    pub const DEATH202: i32 = 146;
    /// Frame `death203`.
    pub const DEATH203: i32 = 147;
    /// Frame `death204`.
    pub const DEATH204: i32 = 148;
    /// Frame `death205`.
    pub const DEATH205: i32 = 149;
    /// Frame `death206`.
    pub const DEATH206: i32 = 150;
    /// Frame `death207`.
    pub const DEATH207: i32 = 151;
    /// Frame `death208`.
    pub const DEATH208: i32 = 152;
    /// Frame `death209`.
    pub const DEATH209: i32 = 153;
    /// Frame `death210`.
    pub const DEATH210: i32 = 154;
    /// Frame `death211`.
    pub const DEATH211: i32 = 155;
    /// Frame `death212`.
    pub const DEATH212: i32 = 156;
    /// Frame `death213`.
    pub const DEATH213: i32 = 157;
    /// Frame `death214`.
    pub const DEATH214: i32 = 158;
    /// Frame `death215`.
    pub const DEATH215: i32 = 159;
    /// Frame `death216`.
    pub const DEATH216: i32 = 160;
    /// Frame `death217`.
    pub const DEATH217: i32 = 161;
    /// Frame `death218`.
    pub const DEATH218: i32 = 162;
    /// Frame `death219`.
    pub const DEATH219: i32 = 163;
    /// Frame `death220`.
    pub const DEATH220: i32 = 164;
    /// Frame `death221`.
    pub const DEATH221: i32 = 165;
    /// Frame `death222`.
    pub const DEATH222: i32 = 166;
    /// Frame `death223`.
    pub const DEATH223: i32 = 167;
    /// Frame `death224`.
    pub const DEATH224: i32 = 168;
    /// Frame `death225`.
    pub const DEATH225: i32 = 169;
    /// Frame `death301`.
    pub const DEATH301: i32 = 170;
    /// Frame `death302`.
    pub const DEATH302: i32 = 171;
    /// Frame `death303`.
    pub const DEATH303: i32 = 172;
    /// Frame `death304`.
    pub const DEATH304: i32 = 173;
    /// Frame `death305`.
    pub const DEATH305: i32 = 174;
    /// Frame `death306`.
    pub const DEATH306: i32 = 175;
    /// Frame `death307`.
    pub const DEATH307: i32 = 176;
    /// Frame `death308`.
    pub const DEATH308: i32 = 177;
    /// Frame `death309`.
    pub const DEATH309: i32 = 178;
    /// Frame `block01`.
    pub const BLOCK01: i32 = 179;
    /// Frame `block02`.
    pub const BLOCK02: i32 = 180;
    /// Frame `block03`.
    pub const BLOCK03: i32 = 181;
    /// Frame `block04`.
    pub const BLOCK04: i32 = 182;
    /// Frame `block05`.
    pub const BLOCK05: i32 = 183;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 184;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 185;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 186;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 187;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 188;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 189;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 190;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 191;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 192;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 193;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 194;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 195;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 196;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 197;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 198;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 199;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 200;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 201;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 202;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 203;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 204;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 205;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 206;
}

/// `infantryMoves` move tables.
pub fn infantry_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("infantry_move_stand", 50, 71, None, vec![
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
        monster_move("infantry_move_fidget", 1, 49, Some("infantry_stand"), vec![
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 3.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 6.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 3.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -2.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -1.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -3.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -2.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -3.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -3.0, vec![], -1),
            monster_frame(MonsterAi::Stand, -2.0, vec![], -1),
        ]),
        monster_move("infantry_move_walk", 74, 85, None, vec![
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
        ]),
        monster_move("infantry_move_run", 92, 99, None, vec![
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 20.0, vec![], -1),
            monster_frame(MonsterAi::Run, 5.0, vec![], -1),
            monster_frame(MonsterAi::Run, 7.0, vec![], -1),
            monster_frame(MonsterAi::Run, 30.0, vec![], -1),
            monster_frame(MonsterAi::Run, 35.0, vec![], -1),
            monster_frame(MonsterAi::Run, 2.0, vec![], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![], -1),
        ]),
        monster_move("infantry_move_pain1", 100, 109, Some("infantry_run"), vec![
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 6.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
        ]),
        monster_move("infantry_move_pain2", 110, 119, Some("infantry_run"), vec![
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
        ]),
        monster_move("infantry_move_death1", 125, 144, Some("infantry_dead"), vec![
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 9.0, vec![], -1),
            monster_frame(MonsterAi::Move, 9.0, vec![], -1),
            monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
        ]),
        monster_move("infantry_move_death2", 145, 169, Some("infantry_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 4.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -10.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -7.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -8.0, vec![MonsterAction::name("InfantryMachineGun")], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, 4.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("infantry_move_death3", 170, 178, Some("infantry_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, -11.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -11.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("infantry_move_duck", 120, 124, Some("infantry_run"), vec![
            monster_frame(MonsterAi::Move, -2.0, vec![MonsterAction::name("infantry_duck_down")], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![MonsterAction::name("infantry_duck_hold")], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 4.0, vec![MonsterAction::name("infantry_duck_up")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("infantry_move_attack1", 184, 198, Some("infantry_run"), vec![
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("infantry_set_firetime")], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("infantry_fire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -7.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("infantry_cock_gun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -1.0, vec![], -1),
        ]),
        monster_move("infantry_move_attack2", 199, 206, Some("infantry_run"), vec![
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("infantry_swing")], -1),
            monster_frame(MonsterAi::Charge, 8.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 8.0, vec![MonsterAction::name("infantry_smack")], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
        ]),
    ]
}
