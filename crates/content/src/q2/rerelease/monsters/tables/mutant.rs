//! mutant move tables (`src/content/q2/rerelease/monsters/tables/mutant.ts`).

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `mutantFrame`.
pub mod mutant_frame {
    /// Frame `attack01`.
    pub const ATTACK01: i32 = 0;
    /// Frame `attack02`.
    pub const ATTACK02: i32 = 1;
    /// Frame `attack03`.
    pub const ATTACK03: i32 = 2;
    /// Frame `attack04`.
    pub const ATTACK04: i32 = 3;
    /// Frame `attack05`.
    pub const ATTACK05: i32 = 4;
    /// Frame `attack06`.
    pub const ATTACK06: i32 = 5;
    /// Frame `attack07`.
    pub const ATTACK07: i32 = 6;
    /// Frame `attack08`.
    pub const ATTACK08: i32 = 7;
    /// Frame `attack09`.
    pub const ATTACK09: i32 = 8;
    /// Frame `attack10`.
    pub const ATTACK10: i32 = 9;
    /// Frame `attack11`.
    pub const ATTACK11: i32 = 10;
    /// Frame `attack12`.
    pub const ATTACK12: i32 = 11;
    /// Frame `attack13`.
    pub const ATTACK13: i32 = 12;
    /// Frame `attack14`.
    pub const ATTACK14: i32 = 13;
    /// Frame `attack15`.
    pub const ATTACK15: i32 = 14;
    /// Frame `death101`.
    pub const DEATH101: i32 = 15;
    /// Frame `death102`.
    pub const DEATH102: i32 = 16;
    /// Frame `death103`.
    pub const DEATH103: i32 = 17;
    /// Frame `death104`.
    pub const DEATH104: i32 = 18;
    /// Frame `death105`.
    pub const DEATH105: i32 = 19;
    /// Frame `death106`.
    pub const DEATH106: i32 = 20;
    /// Frame `death107`.
    pub const DEATH107: i32 = 21;
    /// Frame `death108`.
    pub const DEATH108: i32 = 22;
    /// Frame `death109`.
    pub const DEATH109: i32 = 23;
    /// Frame `death201`.
    pub const DEATH201: i32 = 24;
    /// Frame `death202`.
    pub const DEATH202: i32 = 25;
    /// Frame `death203`.
    pub const DEATH203: i32 = 26;
    /// Frame `death204`.
    pub const DEATH204: i32 = 27;
    /// Frame `death205`.
    pub const DEATH205: i32 = 28;
    /// Frame `death206`.
    pub const DEATH206: i32 = 29;
    /// Frame `death207`.
    pub const DEATH207: i32 = 30;
    /// Frame `death208`.
    pub const DEATH208: i32 = 31;
    /// Frame `death209`.
    pub const DEATH209: i32 = 32;
    /// Frame `death210`.
    pub const DEATH210: i32 = 33;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 34;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 35;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 36;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 37;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 38;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 39;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 40;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 41;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 42;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 43;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 44;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 45;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 46;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 47;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 48;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 49;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 50;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 51;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 52;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 53;
    /// Frame `pain310`.
    pub const PAIN310: i32 = 54;
    /// Frame `pain311`.
    pub const PAIN311: i32 = 55;
    /// Frame `run03`.
    pub const RUN03: i32 = 56;
    /// Frame `run04`.
    pub const RUN04: i32 = 57;
    /// Frame `run05`.
    pub const RUN05: i32 = 58;
    /// Frame `run06`.
    pub const RUN06: i32 = 59;
    /// Frame `run07`.
    pub const RUN07: i32 = 60;
    /// Frame `run08`.
    pub const RUN08: i32 = 61;
    /// Frame `stand101`.
    pub const STAND101: i32 = 62;
    /// Frame `stand102`.
    pub const STAND102: i32 = 63;
    /// Frame `stand103`.
    pub const STAND103: i32 = 64;
    /// Frame `stand104`.
    pub const STAND104: i32 = 65;
    /// Frame `stand105`.
    pub const STAND105: i32 = 66;
    /// Frame `stand106`.
    pub const STAND106: i32 = 67;
    /// Frame `stand107`.
    pub const STAND107: i32 = 68;
    /// Frame `stand108`.
    pub const STAND108: i32 = 69;
    /// Frame `stand109`.
    pub const STAND109: i32 = 70;
    /// Frame `stand110`.
    pub const STAND110: i32 = 71;
    /// Frame `stand111`.
    pub const STAND111: i32 = 72;
    /// Frame `stand112`.
    pub const STAND112: i32 = 73;
    /// Frame `stand113`.
    pub const STAND113: i32 = 74;
    /// Frame `stand114`.
    pub const STAND114: i32 = 75;
    /// Frame `stand115`.
    pub const STAND115: i32 = 76;
    /// Frame `stand116`.
    pub const STAND116: i32 = 77;
    /// Frame `stand117`.
    pub const STAND117: i32 = 78;
    /// Frame `stand118`.
    pub const STAND118: i32 = 79;
    /// Frame `stand119`.
    pub const STAND119: i32 = 80;
    /// Frame `stand120`.
    pub const STAND120: i32 = 81;
    /// Frame `stand121`.
    pub const STAND121: i32 = 82;
    /// Frame `stand122`.
    pub const STAND122: i32 = 83;
    /// Frame `stand123`.
    pub const STAND123: i32 = 84;
    /// Frame `stand124`.
    pub const STAND124: i32 = 85;
    /// Frame `stand125`.
    pub const STAND125: i32 = 86;
    /// Frame `stand126`.
    pub const STAND126: i32 = 87;
    /// Frame `stand127`.
    pub const STAND127: i32 = 88;
    /// Frame `stand128`.
    pub const STAND128: i32 = 89;
    /// Frame `stand129`.
    pub const STAND129: i32 = 90;
    /// Frame `stand130`.
    pub const STAND130: i32 = 91;
    /// Frame `stand131`.
    pub const STAND131: i32 = 92;
    /// Frame `stand132`.
    pub const STAND132: i32 = 93;
    /// Frame `stand133`.
    pub const STAND133: i32 = 94;
    /// Frame `stand134`.
    pub const STAND134: i32 = 95;
    /// Frame `stand135`.
    pub const STAND135: i32 = 96;
    /// Frame `stand136`.
    pub const STAND136: i32 = 97;
    /// Frame `stand137`.
    pub const STAND137: i32 = 98;
    /// Frame `stand138`.
    pub const STAND138: i32 = 99;
    /// Frame `stand139`.
    pub const STAND139: i32 = 100;
    /// Frame `stand140`.
    pub const STAND140: i32 = 101;
    /// Frame `stand141`.
    pub const STAND141: i32 = 102;
    /// Frame `stand142`.
    pub const STAND142: i32 = 103;
    /// Frame `stand143`.
    pub const STAND143: i32 = 104;
    /// Frame `stand144`.
    pub const STAND144: i32 = 105;
    /// Frame `stand145`.
    pub const STAND145: i32 = 106;
    /// Frame `stand146`.
    pub const STAND146: i32 = 107;
    /// Frame `stand147`.
    pub const STAND147: i32 = 108;
    /// Frame `stand148`.
    pub const STAND148: i32 = 109;
    /// Frame `stand149`.
    pub const STAND149: i32 = 110;
    /// Frame `stand150`.
    pub const STAND150: i32 = 111;
    /// Frame `stand151`.
    pub const STAND151: i32 = 112;
    /// Frame `stand152`.
    pub const STAND152: i32 = 113;
    /// Frame `stand153`.
    pub const STAND153: i32 = 114;
    /// Frame `stand154`.
    pub const STAND154: i32 = 115;
    /// Frame `stand155`.
    pub const STAND155: i32 = 116;
    /// Frame `stand156`.
    pub const STAND156: i32 = 117;
    /// Frame `stand157`.
    pub const STAND157: i32 = 118;
    /// Frame `stand158`.
    pub const STAND158: i32 = 119;
    /// Frame `stand159`.
    pub const STAND159: i32 = 120;
    /// Frame `stand160`.
    pub const STAND160: i32 = 121;
    /// Frame `stand161`.
    pub const STAND161: i32 = 122;
    /// Frame `stand162`.
    pub const STAND162: i32 = 123;
    /// Frame `stand163`.
    pub const STAND163: i32 = 124;
    /// Frame `stand164`.
    pub const STAND164: i32 = 125;
    /// Frame `walk01`.
    pub const WALK01: i32 = 126;
    /// Frame `walk02`.
    pub const WALK02: i32 = 127;
    /// Frame `walk03`.
    pub const WALK03: i32 = 128;
    /// Frame `walk04`.
    pub const WALK04: i32 = 129;
    /// Frame `walk05`.
    pub const WALK05: i32 = 130;
    /// Frame `walk06`.
    pub const WALK06: i32 = 131;
    /// Frame `walk07`.
    pub const WALK07: i32 = 132;
    /// Frame `walk08`.
    pub const WALK08: i32 = 133;
    /// Frame `walk09`.
    pub const WALK09: i32 = 134;
    /// Frame `walk10`.
    pub const WALK10: i32 = 135;
    /// Frame `walk11`.
    pub const WALK11: i32 = 136;
    /// Frame `walk12`.
    pub const WALK12: i32 = 137;
    /// Frame `walk13`.
    pub const WALK13: i32 = 138;
    /// Frame `walk14`.
    pub const WALK14: i32 = 139;
    /// Frame `walk15`.
    pub const WALK15: i32 = 140;
    /// Frame `walk16`.
    pub const WALK16: i32 = 141;
    /// Frame `walk17`.
    pub const WALK17: i32 = 142;
    /// Frame `walk18`.
    pub const WALK18: i32 = 143;
    /// Frame `walk19`.
    pub const WALK19: i32 = 144;
    /// Frame `walk20`.
    pub const WALK20: i32 = 145;
    /// Frame `walk21`.
    pub const WALK21: i32 = 146;
    /// Frame `walk22`.
    pub const WALK22: i32 = 147;
    /// Frame `walk23`.
    pub const WALK23: i32 = 148;
    /// Frame `jump01`.
    pub const JUMP01: i32 = 149;
    /// Frame `jump02`.
    pub const JUMP02: i32 = 150;
    /// Frame `jump03`.
    pub const JUMP03: i32 = 151;
    /// Frame `jump04`.
    pub const JUMP04: i32 = 152;
    /// Frame `jump05`.
    pub const JUMP05: i32 = 153;
}

/// `mutantMoves` move tables.
pub fn mutant_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("mutant_move_stand", 62, 112, None, vec![
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
        ]),
        monster_move("mutant_move_idle", 113, 125, Some("mutant_stand"), vec![
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![MonsterAction::name("mutant_idle_loop")], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Stand, (0f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_walk", 130, 141, None, vec![
            monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (13f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (16f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (15f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_start_walk", 126, 129, Some("mutant_walk_loop"), vec![
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Walk, (1f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_run", 56, 61, None, vec![
            monster_frame(MonsterAi::Run, (40f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (40f32) as f64, vec![MonsterAction::name("mutant_step")], -1),
            monster_frame(MonsterAi::Run, (24f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (5f32) as f64, vec![MonsterAction::name("mutant_step")], -1),
            monster_frame(MonsterAi::Run, (17f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_attack", 8, 14, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("mutant_hit_left")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("mutant_hit_right")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![MonsterAction::name("mutant_check_refire")], -1),
        ]),
        monster_move("mutant_move_jump", 0, 7, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (17f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (15f32) as f64, vec![MonsterAction::name("mutant_jump_takeoff")], -1),
            monster_frame(MonsterAi::Charge, (15f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (15f32) as f64, vec![MonsterAction::name("mutant_check_landing")], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_pain1", 34, 38, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Move, (4f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (5f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_pain2", 39, 44, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Move, (-24f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (11f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (4f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_pain3", 45, 55, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Move, (-22f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (3f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (1f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_death1", 15, 23, Some("monster_dead"), vec![
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (7f32) as f64, vec![MonsterAction::name("mutant_shrink")], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_right".to_string()), (0f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_death2", 24, 33, Some("monster_dead"), vec![
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (1f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (3f32) as f64, vec![MonsterAction::name("mutant_shrink")], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (6f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (5f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (2f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Source("ai_move_slide_left".to_string()), (0f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_jump_up", 149, 153, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Move, (-8f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (-8f32) as f64, vec![MonsterAction::name("mutant_jump_up")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("mutant_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
        monster_move("mutant_move_jump_down", 149, 153, Some("mutant_run"), vec![
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("mutant_jump_down")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![MonsterAction::name("mutant_jump_wait_land")], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
        ]),
    ]
}
