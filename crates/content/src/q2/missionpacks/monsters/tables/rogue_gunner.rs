//! gunner move tables (`src/content/q2/missionpacks/monsters/tables/rogue-gunner.ts`).
//!
//! Original Quake II rogue/m_gunner.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

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
}

/// `gunnerMoves` move tables.
pub fn gunner_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("gunner_move_fidget", 30, 69, Some("gunner_stand"), vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("gunner_idlesound")], -1),
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
        monster_move("gunner_move_stand", 0, 29, None, vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("gunner_fidget")], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("gunner_fidget")], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("gunner_fidget")], -1),
        ]),
        monster_move("gunner_move_walk", 76, 88, None, vec![
            monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 3.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
        ]),
        monster_move("gunner_move_run", 94, 101, None, vec![
            monster_frame(MonsterAi::Run, 26.0, vec![], -1),
            monster_frame(MonsterAi::Run, 9.0, vec![], -1),
            monster_frame(MonsterAi::Run, 9.0, vec![], -1),
            monster_frame(MonsterAi::Run, 9.0, vec![MonsterAction::name("monster_done_dodge")], -1),
            monster_frame(MonsterAi::Run, 15.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 6.0, vec![], -1),
        ]),
        monster_move("gunner_move_runandshoot", 102, 107, None, vec![
            monster_frame(MonsterAi::Run, 32.0, vec![], -1),
            monster_frame(MonsterAi::Run, 15.0, vec![], -1),
            monster_frame(MonsterAi::Run, 10.0, vec![], -1),
            monster_frame(MonsterAi::Run, 18.0, vec![], -1),
            monster_frame(MonsterAi::Run, 8.0, vec![], -1),
            monster_frame(MonsterAi::Run, 20.0, vec![], -1),
        ]),
        monster_move("gunner_move_pain3", 185, 189, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
        ]),
        monster_move("gunner_move_pain2", 177, 184, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 11.0, vec![], -1),
            monster_frame(MonsterAi::Move, 6.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -7.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -7.0, vec![], -1),
        ]),
        monster_move("gunner_move_pain1", 159, 176, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![], -1),
            monster_frame(MonsterAi::Move, 3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("gunner_move_death", 190, 200, Some("gunner_dead"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -7.0, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -5.0, vec![], -1),
            monster_frame(MonsterAi::Move, 8.0, vec![], -1),
            monster_frame(MonsterAi::Move, 6.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("gunner_move_duck", 201, 208, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, 1.0, vec![MonsterAction::name("gunner_duck_down")], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 1.0, vec![MonsterAction::name("monster_duck_hold")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("monster_duck_up")], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
        ]),
        monster_move("gunner_move_attack_chain", 137, 143, Some("gunner_fire_chain"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("gunner_opengun")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
        ]),
        monster_move("gunner_move_fire_chain", 144, 151, Some("gunner_refire_chain"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerFire")], -1),
        ]),
        monster_move("gunner_move_endfire_chain", 152, 158, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
        ]),
        monster_move("gunner_move_attack_grenade", 108, 128, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("gunner_blind_check")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("GunnerGrenade")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
        ]),
        monster_move("gunner_move_jump", 209, 218, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("gunner_jump_now")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("gunner_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("gunner_move_jump2", 209, 218, Some("gunner_run"), vec![
            monster_frame(MonsterAi::Move, -8.0, vec![], -1),
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
            monster_frame(MonsterAi::Move, -4.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("gunner_jump_now")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("gunner_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
    ]
}
