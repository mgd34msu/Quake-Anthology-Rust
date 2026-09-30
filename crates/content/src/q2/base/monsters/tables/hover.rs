//! hover move tables (`src/content/q2/base/monsters/tables/hover.ts`).
//!
//! Original Quake II m_hover.c frame order and distances. id Software, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `hoverFrame`.
pub mod hover_frame {
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
    /// Frame `forwrd01`.
    pub const FORWRD01: i32 = 30;
    /// Frame `forwrd02`.
    pub const FORWRD02: i32 = 31;
    /// Frame `forwrd03`.
    pub const FORWRD03: i32 = 32;
    /// Frame `forwrd04`.
    pub const FORWRD04: i32 = 33;
    /// Frame `forwrd05`.
    pub const FORWRD05: i32 = 34;
    /// Frame `forwrd06`.
    pub const FORWRD06: i32 = 35;
    /// Frame `forwrd07`.
    pub const FORWRD07: i32 = 36;
    /// Frame `forwrd08`.
    pub const FORWRD08: i32 = 37;
    /// Frame `forwrd09`.
    pub const FORWRD09: i32 = 38;
    /// Frame `forwrd10`.
    pub const FORWRD10: i32 = 39;
    /// Frame `forwrd11`.
    pub const FORWRD11: i32 = 40;
    /// Frame `forwrd12`.
    pub const FORWRD12: i32 = 41;
    /// Frame `forwrd13`.
    pub const FORWRD13: i32 = 42;
    /// Frame `forwrd14`.
    pub const FORWRD14: i32 = 43;
    /// Frame `forwrd15`.
    pub const FORWRD15: i32 = 44;
    /// Frame `forwrd16`.
    pub const FORWRD16: i32 = 45;
    /// Frame `forwrd17`.
    pub const FORWRD17: i32 = 46;
    /// Frame `forwrd18`.
    pub const FORWRD18: i32 = 47;
    /// Frame `forwrd19`.
    pub const FORWRD19: i32 = 48;
    /// Frame `forwrd20`.
    pub const FORWRD20: i32 = 49;
    /// Frame `forwrd21`.
    pub const FORWRD21: i32 = 50;
    /// Frame `forwrd22`.
    pub const FORWRD22: i32 = 51;
    /// Frame `forwrd23`.
    pub const FORWRD23: i32 = 52;
    /// Frame `forwrd24`.
    pub const FORWRD24: i32 = 53;
    /// Frame `forwrd25`.
    pub const FORWRD25: i32 = 54;
    /// Frame `forwrd26`.
    pub const FORWRD26: i32 = 55;
    /// Frame `forwrd27`.
    pub const FORWRD27: i32 = 56;
    /// Frame `forwrd28`.
    pub const FORWRD28: i32 = 57;
    /// Frame `forwrd29`.
    pub const FORWRD29: i32 = 58;
    /// Frame `forwrd30`.
    pub const FORWRD30: i32 = 59;
    /// Frame `forwrd31`.
    pub const FORWRD31: i32 = 60;
    /// Frame `forwrd32`.
    pub const FORWRD32: i32 = 61;
    /// Frame `forwrd33`.
    pub const FORWRD33: i32 = 62;
    /// Frame `forwrd34`.
    pub const FORWRD34: i32 = 63;
    /// Frame `forwrd35`.
    pub const FORWRD35: i32 = 64;
    /// Frame `stop101`.
    pub const STOP101: i32 = 65;
    /// Frame `stop102`.
    pub const STOP102: i32 = 66;
    /// Frame `stop103`.
    pub const STOP103: i32 = 67;
    /// Frame `stop104`.
    pub const STOP104: i32 = 68;
    /// Frame `stop105`.
    pub const STOP105: i32 = 69;
    /// Frame `stop106`.
    pub const STOP106: i32 = 70;
    /// Frame `stop107`.
    pub const STOP107: i32 = 71;
    /// Frame `stop108`.
    pub const STOP108: i32 = 72;
    /// Frame `stop109`.
    pub const STOP109: i32 = 73;
    /// Frame `stop201`.
    pub const STOP201: i32 = 74;
    /// Frame `stop202`.
    pub const STOP202: i32 = 75;
    /// Frame `stop203`.
    pub const STOP203: i32 = 76;
    /// Frame `stop204`.
    pub const STOP204: i32 = 77;
    /// Frame `stop205`.
    pub const STOP205: i32 = 78;
    /// Frame `stop206`.
    pub const STOP206: i32 = 79;
    /// Frame `stop207`.
    pub const STOP207: i32 = 80;
    /// Frame `stop208`.
    pub const STOP208: i32 = 81;
    /// Frame `takeof01`.
    pub const TAKEOF01: i32 = 82;
    /// Frame `takeof02`.
    pub const TAKEOF02: i32 = 83;
    /// Frame `takeof03`.
    pub const TAKEOF03: i32 = 84;
    /// Frame `takeof04`.
    pub const TAKEOF04: i32 = 85;
    /// Frame `takeof05`.
    pub const TAKEOF05: i32 = 86;
    /// Frame `takeof06`.
    pub const TAKEOF06: i32 = 87;
    /// Frame `takeof07`.
    pub const TAKEOF07: i32 = 88;
    /// Frame `takeof08`.
    pub const TAKEOF08: i32 = 89;
    /// Frame `takeof09`.
    pub const TAKEOF09: i32 = 90;
    /// Frame `takeof10`.
    pub const TAKEOF10: i32 = 91;
    /// Frame `takeof11`.
    pub const TAKEOF11: i32 = 92;
    /// Frame `takeof12`.
    pub const TAKEOF12: i32 = 93;
    /// Frame `takeof13`.
    pub const TAKEOF13: i32 = 94;
    /// Frame `takeof14`.
    pub const TAKEOF14: i32 = 95;
    /// Frame `takeof15`.
    pub const TAKEOF15: i32 = 96;
    /// Frame `takeof16`.
    pub const TAKEOF16: i32 = 97;
    /// Frame `takeof17`.
    pub const TAKEOF17: i32 = 98;
    /// Frame `takeof18`.
    pub const TAKEOF18: i32 = 99;
    /// Frame `takeof19`.
    pub const TAKEOF19: i32 = 100;
    /// Frame `takeof20`.
    pub const TAKEOF20: i32 = 101;
    /// Frame `takeof21`.
    pub const TAKEOF21: i32 = 102;
    /// Frame `takeof22`.
    pub const TAKEOF22: i32 = 103;
    /// Frame `takeof23`.
    pub const TAKEOF23: i32 = 104;
    /// Frame `takeof24`.
    pub const TAKEOF24: i32 = 105;
    /// Frame `takeof25`.
    pub const TAKEOF25: i32 = 106;
    /// Frame `takeof26`.
    pub const TAKEOF26: i32 = 107;
    /// Frame `takeof27`.
    pub const TAKEOF27: i32 = 108;
    /// Frame `takeof28`.
    pub const TAKEOF28: i32 = 109;
    /// Frame `takeof29`.
    pub const TAKEOF29: i32 = 110;
    /// Frame `takeof30`.
    pub const TAKEOF30: i32 = 111;
    /// Frame `land01`.
    pub const LAND01: i32 = 112;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 113;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 114;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 115;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 116;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 117;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 118;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 119;
    /// Frame `pain108`.
    pub const PAIN108: i32 = 120;
    /// Frame `pain109`.
    pub const PAIN109: i32 = 121;
    /// Frame `pain110`.
    pub const PAIN110: i32 = 122;
    /// Frame `pain111`.
    pub const PAIN111: i32 = 123;
    /// Frame `pain112`.
    pub const PAIN112: i32 = 124;
    /// Frame `pain113`.
    pub const PAIN113: i32 = 125;
    /// Frame `pain114`.
    pub const PAIN114: i32 = 126;
    /// Frame `pain115`.
    pub const PAIN115: i32 = 127;
    /// Frame `pain116`.
    pub const PAIN116: i32 = 128;
    /// Frame `pain117`.
    pub const PAIN117: i32 = 129;
    /// Frame `pain118`.
    pub const PAIN118: i32 = 130;
    /// Frame `pain119`.
    pub const PAIN119: i32 = 131;
    /// Frame `pain120`.
    pub const PAIN120: i32 = 132;
    /// Frame `pain121`.
    pub const PAIN121: i32 = 133;
    /// Frame `pain122`.
    pub const PAIN122: i32 = 134;
    /// Frame `pain123`.
    pub const PAIN123: i32 = 135;
    /// Frame `pain124`.
    pub const PAIN124: i32 = 136;
    /// Frame `pain125`.
    pub const PAIN125: i32 = 137;
    /// Frame `pain126`.
    pub const PAIN126: i32 = 138;
    /// Frame `pain127`.
    pub const PAIN127: i32 = 139;
    /// Frame `pain128`.
    pub const PAIN128: i32 = 140;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 141;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 142;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 143;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 144;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 145;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 146;
    /// Frame `pain207`.
    pub const PAIN207: i32 = 147;
    /// Frame `pain208`.
    pub const PAIN208: i32 = 148;
    /// Frame `pain209`.
    pub const PAIN209: i32 = 149;
    /// Frame `pain210`.
    pub const PAIN210: i32 = 150;
    /// Frame `pain211`.
    pub const PAIN211: i32 = 151;
    /// Frame `pain212`.
    pub const PAIN212: i32 = 152;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 153;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 154;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 155;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 156;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 157;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 158;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 159;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 160;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 161;
    /// Frame `death101`.
    pub const DEATH101: i32 = 162;
    /// Frame `death102`.
    pub const DEATH102: i32 = 163;
    /// Frame `death103`.
    pub const DEATH103: i32 = 164;
    /// Frame `death104`.
    pub const DEATH104: i32 = 165;
    /// Frame `death105`.
    pub const DEATH105: i32 = 166;
    /// Frame `death106`.
    pub const DEATH106: i32 = 167;
    /// Frame `death107`.
    pub const DEATH107: i32 = 168;
    /// Frame `death108`.
    pub const DEATH108: i32 = 169;
    /// Frame `death109`.
    pub const DEATH109: i32 = 170;
    /// Frame `death110`.
    pub const DEATH110: i32 = 171;
    /// Frame `death111`.
    pub const DEATH111: i32 = 172;
    /// Frame `backwd01`.
    pub const BACKWD01: i32 = 173;
    /// Frame `backwd02`.
    pub const BACKWD02: i32 = 174;
    /// Frame `backwd03`.
    pub const BACKWD03: i32 = 175;
    /// Frame `backwd04`.
    pub const BACKWD04: i32 = 176;
    /// Frame `backwd05`.
    pub const BACKWD05: i32 = 177;
    /// Frame `backwd06`.
    pub const BACKWD06: i32 = 178;
    /// Frame `backwd07`.
    pub const BACKWD07: i32 = 179;
    /// Frame `backwd08`.
    pub const BACKWD08: i32 = 180;
    /// Frame `backwd09`.
    pub const BACKWD09: i32 = 181;
    /// Frame `backwd10`.
    pub const BACKWD10: i32 = 182;
    /// Frame `backwd11`.
    pub const BACKWD11: i32 = 183;
    /// Frame `backwd12`.
    pub const BACKWD12: i32 = 184;
    /// Frame `backwd13`.
    pub const BACKWD13: i32 = 185;
    /// Frame `backwd14`.
    pub const BACKWD14: i32 = 186;
    /// Frame `backwd15`.
    pub const BACKWD15: i32 = 187;
    /// Frame `backwd16`.
    pub const BACKWD16: i32 = 188;
    /// Frame `backwd17`.
    pub const BACKWD17: i32 = 189;
    /// Frame `backwd18`.
    pub const BACKWD18: i32 = 190;
    /// Frame `backwd19`.
    pub const BACKWD19: i32 = 191;
    /// Frame `backwd20`.
    pub const BACKWD20: i32 = 192;
    /// Frame `backwd21`.
    pub const BACKWD21: i32 = 193;
    /// Frame `backwd22`.
    pub const BACKWD22: i32 = 194;
    /// Frame `backwd23`.
    pub const BACKWD23: i32 = 195;
    /// Frame `backwd24`.
    pub const BACKWD24: i32 = 196;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 197;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 198;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 199;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 200;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 201;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 202;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 203;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 204;
}

/// `hoverMoves` move tables.
pub fn hover_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("hover_move_stand", 0, 29, None, vec![
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
        monster_move("hover_move_stop1", 65, 73, None, vec![
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
        monster_move("hover_move_stop2", 74, 81, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("hover_move_takeoff", 82, 111, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, -9.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("hover_move_pain3", 153, 161, Some("hover_run"), vec![
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
        monster_move("hover_move_pain2", 141, 152, Some("hover_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
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
        monster_move("hover_move_pain1", 113, 140, Some("hover_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -8.0, vec![], -1),
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
            monster_frame(MonsterAi::Move, -6.0, vec![], -1),
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 7.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 4.0, vec![], -1),
        ]),
        monster_move("hover_move_land", 112, 112, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("hover_move_forward", 30, 64, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
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
        monster_move("hover_move_walk", 30, 64, None, vec![
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
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
        ]),
        monster_move("hover_move_run", 30, 64, None, vec![
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
        ]),
        monster_move("hover_move_death1", 162, 172, Some("hover_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -10.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 5.0, vec![], -1),
            monster_frame(MonsterAi::Move, 4.0, vec![], -1),
            monster_frame(MonsterAi::Move, 7.0, vec![], -1),
        ]),
        monster_move("hover_move_backward", 173, 196, None, vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
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
        monster_move("hover_move_start_attack", 197, 199, Some("hover_attack"), vec![
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
        ]),
        monster_move("hover_move_attack1", 200, 202, None, vec![
            monster_frame(MonsterAi::Charge, -10.0, vec![MonsterAction::name("hover_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, -10.0, vec![MonsterAction::name("hover_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("hover_reattack")], -1),
        ]),
        monster_move("hover_move_end_attack", 203, 204, Some("hover_run"), vec![
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
        ]),
    ]
}
