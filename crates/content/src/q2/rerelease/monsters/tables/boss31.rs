//! boss31 move tables (`src/content/q2/rerelease/monsters/tables/boss31.ts`).

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `boss31Frame`.
pub mod boss31_frame {
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
}

/// `boss31Moves` move tables.
pub fn boss31_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("jorg_move_stand", 112, 162, None, vec![
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![MonsterAction::name("jorg_idle")], -1),
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
            monster_frame(MonsterAi::Stand, (19f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (11f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (9f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (-17f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (-12f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (-14f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
        ]),
        monster_move("jorg_move_run", 168, 181, None, vec![
            monster_frame(MonsterAi::Run, (17f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (12f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (33f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (9f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_start_walk", 163, 167, None, vec![
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (15f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_walk", 168, 181, None, vec![
            monster_frame(MonsterAi::Walk, (17f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (12f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (33f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_end_walk", 182, 187, None, vec![
            monster_frame(MonsterAi::Walk, (11f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (-8f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_pain3", 87, 111, Some("jorg_run"), vec![
            monster_frame(MonsterAi::Move, (-28f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Move, (-9f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-7f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-11f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (11f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (7f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Move, (17f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
        ]),
        monster_move("jorg_move_pain2", 84, 86, Some("jorg_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_pain1", 81, 83, Some("jorg_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_death", 31, 80, Some("jorg_dead"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("BossExplode")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-15f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-11f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-25f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-10f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-21f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-16f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (22f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (33f32) as f64, vec![MonsterAction::name("jorg_step_left")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (28f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (28f32) as f64, vec![MonsterAction::name("jorg_step_right")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-19f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("jorg_death_hit")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_attack2", 18, 30, Some("jorg_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorgBFG")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_start_attack1", 0, 7, Some("jorg_attack1"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("jorg_move_attack1", 8, 13, Some("jorg_reattack1"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorg_firebullet")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorg_firebullet")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorg_firebullet")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorg_firebullet")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorg_firebullet")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("jorg_firebullet")], -1),
        ]),
        monster_move("jorg_move_end_attack1", 14, 17, Some("jorg_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
    ]
}
