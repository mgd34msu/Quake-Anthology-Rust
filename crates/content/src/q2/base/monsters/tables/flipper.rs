//! flipper move tables (`src/content/q2/base/monsters/tables/flipper.ts`).
//!
//! Original Quake II m_flipper.c frame order and distances. id Software, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `flipperFrame`.
pub mod flipper_frame {
    /// Frame `flpbit01`.
    pub const FLPBIT01: i32 = 0;
    /// Frame `flpbit02`.
    pub const FLPBIT02: i32 = 1;
    /// Frame `flpbit03`.
    pub const FLPBIT03: i32 = 2;
    /// Frame `flpbit04`.
    pub const FLPBIT04: i32 = 3;
    /// Frame `flpbit05`.
    pub const FLPBIT05: i32 = 4;
    /// Frame `flpbit06`.
    pub const FLPBIT06: i32 = 5;
    /// Frame `flpbit07`.
    pub const FLPBIT07: i32 = 6;
    /// Frame `flpbit08`.
    pub const FLPBIT08: i32 = 7;
    /// Frame `flpbit09`.
    pub const FLPBIT09: i32 = 8;
    /// Frame `flpbit10`.
    pub const FLPBIT10: i32 = 9;
    /// Frame `flpbit11`.
    pub const FLPBIT11: i32 = 10;
    /// Frame `flpbit12`.
    pub const FLPBIT12: i32 = 11;
    /// Frame `flpbit13`.
    pub const FLPBIT13: i32 = 12;
    /// Frame `flpbit14`.
    pub const FLPBIT14: i32 = 13;
    /// Frame `flpbit15`.
    pub const FLPBIT15: i32 = 14;
    /// Frame `flpbit16`.
    pub const FLPBIT16: i32 = 15;
    /// Frame `flpbit17`.
    pub const FLPBIT17: i32 = 16;
    /// Frame `flpbit18`.
    pub const FLPBIT18: i32 = 17;
    /// Frame `flpbit19`.
    pub const FLPBIT19: i32 = 18;
    /// Frame `flpbit20`.
    pub const FLPBIT20: i32 = 19;
    /// Frame `flptal01`.
    pub const FLPTAL01: i32 = 20;
    /// Frame `flptal02`.
    pub const FLPTAL02: i32 = 21;
    /// Frame `flptal03`.
    pub const FLPTAL03: i32 = 22;
    /// Frame `flptal04`.
    pub const FLPTAL04: i32 = 23;
    /// Frame `flptal05`.
    pub const FLPTAL05: i32 = 24;
    /// Frame `flptal06`.
    pub const FLPTAL06: i32 = 25;
    /// Frame `flptal07`.
    pub const FLPTAL07: i32 = 26;
    /// Frame `flptal08`.
    pub const FLPTAL08: i32 = 27;
    /// Frame `flptal09`.
    pub const FLPTAL09: i32 = 28;
    /// Frame `flptal10`.
    pub const FLPTAL10: i32 = 29;
    /// Frame `flptal11`.
    pub const FLPTAL11: i32 = 30;
    /// Frame `flptal12`.
    pub const FLPTAL12: i32 = 31;
    /// Frame `flptal13`.
    pub const FLPTAL13: i32 = 32;
    /// Frame `flptal14`.
    pub const FLPTAL14: i32 = 33;
    /// Frame `flptal15`.
    pub const FLPTAL15: i32 = 34;
    /// Frame `flptal16`.
    pub const FLPTAL16: i32 = 35;
    /// Frame `flptal17`.
    pub const FLPTAL17: i32 = 36;
    /// Frame `flptal18`.
    pub const FLPTAL18: i32 = 37;
    /// Frame `flptal19`.
    pub const FLPTAL19: i32 = 38;
    /// Frame `flptal20`.
    pub const FLPTAL20: i32 = 39;
    /// Frame `flptal21`.
    pub const FLPTAL21: i32 = 40;
    /// Frame `flphor01`.
    pub const FLPHOR01: i32 = 41;
    /// Frame `flphor02`.
    pub const FLPHOR02: i32 = 42;
    /// Frame `flphor03`.
    pub const FLPHOR03: i32 = 43;
    /// Frame `flphor04`.
    pub const FLPHOR04: i32 = 44;
    /// Frame `flphor05`.
    pub const FLPHOR05: i32 = 45;
    /// Frame `flphor06`.
    pub const FLPHOR06: i32 = 46;
    /// Frame `flphor07`.
    pub const FLPHOR07: i32 = 47;
    /// Frame `flphor08`.
    pub const FLPHOR08: i32 = 48;
    /// Frame `flphor09`.
    pub const FLPHOR09: i32 = 49;
    /// Frame `flphor10`.
    pub const FLPHOR10: i32 = 50;
    /// Frame `flphor11`.
    pub const FLPHOR11: i32 = 51;
    /// Frame `flphor12`.
    pub const FLPHOR12: i32 = 52;
    /// Frame `flphor13`.
    pub const FLPHOR13: i32 = 53;
    /// Frame `flphor14`.
    pub const FLPHOR14: i32 = 54;
    /// Frame `flphor15`.
    pub const FLPHOR15: i32 = 55;
    /// Frame `flphor16`.
    pub const FLPHOR16: i32 = 56;
    /// Frame `flphor17`.
    pub const FLPHOR17: i32 = 57;
    /// Frame `flphor18`.
    pub const FLPHOR18: i32 = 58;
    /// Frame `flphor19`.
    pub const FLPHOR19: i32 = 59;
    /// Frame `flphor20`.
    pub const FLPHOR20: i32 = 60;
    /// Frame `flphor21`.
    pub const FLPHOR21: i32 = 61;
    /// Frame `flphor22`.
    pub const FLPHOR22: i32 = 62;
    /// Frame `flphor23`.
    pub const FLPHOR23: i32 = 63;
    /// Frame `flphor24`.
    pub const FLPHOR24: i32 = 64;
    /// Frame `flpver01`.
    pub const FLPVER01: i32 = 65;
    /// Frame `flpver02`.
    pub const FLPVER02: i32 = 66;
    /// Frame `flpver03`.
    pub const FLPVER03: i32 = 67;
    /// Frame `flpver04`.
    pub const FLPVER04: i32 = 68;
    /// Frame `flpver05`.
    pub const FLPVER05: i32 = 69;
    /// Frame `flpver06`.
    pub const FLPVER06: i32 = 70;
    /// Frame `flpver07`.
    pub const FLPVER07: i32 = 71;
    /// Frame `flpver08`.
    pub const FLPVER08: i32 = 72;
    /// Frame `flpver09`.
    pub const FLPVER09: i32 = 73;
    /// Frame `flpver10`.
    pub const FLPVER10: i32 = 74;
    /// Frame `flpver11`.
    pub const FLPVER11: i32 = 75;
    /// Frame `flpver12`.
    pub const FLPVER12: i32 = 76;
    /// Frame `flpver13`.
    pub const FLPVER13: i32 = 77;
    /// Frame `flpver14`.
    pub const FLPVER14: i32 = 78;
    /// Frame `flpver15`.
    pub const FLPVER15: i32 = 79;
    /// Frame `flpver16`.
    pub const FLPVER16: i32 = 80;
    /// Frame `flpver17`.
    pub const FLPVER17: i32 = 81;
    /// Frame `flpver18`.
    pub const FLPVER18: i32 = 82;
    /// Frame `flpver19`.
    pub const FLPVER19: i32 = 83;
    /// Frame `flpver20`.
    pub const FLPVER20: i32 = 84;
    /// Frame `flpver21`.
    pub const FLPVER21: i32 = 85;
    /// Frame `flpver22`.
    pub const FLPVER22: i32 = 86;
    /// Frame `flpver23`.
    pub const FLPVER23: i32 = 87;
    /// Frame `flpver24`.
    pub const FLPVER24: i32 = 88;
    /// Frame `flpver25`.
    pub const FLPVER25: i32 = 89;
    /// Frame `flpver26`.
    pub const FLPVER26: i32 = 90;
    /// Frame `flpver27`.
    pub const FLPVER27: i32 = 91;
    /// Frame `flpver28`.
    pub const FLPVER28: i32 = 92;
    /// Frame `flpver29`.
    pub const FLPVER29: i32 = 93;
    /// Frame `flppn101`.
    pub const FLPPN101: i32 = 94;
    /// Frame `flppn102`.
    pub const FLPPN102: i32 = 95;
    /// Frame `flppn103`.
    pub const FLPPN103: i32 = 96;
    /// Frame `flppn104`.
    pub const FLPPN104: i32 = 97;
    /// Frame `flppn105`.
    pub const FLPPN105: i32 = 98;
    /// Frame `flppn201`.
    pub const FLPPN201: i32 = 99;
    /// Frame `flppn202`.
    pub const FLPPN202: i32 = 100;
    /// Frame `flppn203`.
    pub const FLPPN203: i32 = 101;
    /// Frame `flppn204`.
    pub const FLPPN204: i32 = 102;
    /// Frame `flppn205`.
    pub const FLPPN205: i32 = 103;
    /// Frame `flpdth01`.
    pub const FLPDTH01: i32 = 104;
    /// Frame `flpdth02`.
    pub const FLPDTH02: i32 = 105;
    /// Frame `flpdth03`.
    pub const FLPDTH03: i32 = 106;
    /// Frame `flpdth04`.
    pub const FLPDTH04: i32 = 107;
    /// Frame `flpdth05`.
    pub const FLPDTH05: i32 = 108;
    /// Frame `flpdth06`.
    pub const FLPDTH06: i32 = 109;
    /// Frame `flpdth07`.
    pub const FLPDTH07: i32 = 110;
    /// Frame `flpdth08`.
    pub const FLPDTH08: i32 = 111;
    /// Frame `flpdth09`.
    pub const FLPDTH09: i32 = 112;
    /// Frame `flpdth10`.
    pub const FLPDTH10: i32 = 113;
    /// Frame `flpdth11`.
    pub const FLPDTH11: i32 = 114;
    /// Frame `flpdth12`.
    pub const FLPDTH12: i32 = 115;
    /// Frame `flpdth13`.
    pub const FLPDTH13: i32 = 116;
    /// Frame `flpdth14`.
    pub const FLPDTH14: i32 = 117;
    /// Frame `flpdth15`.
    pub const FLPDTH15: i32 = 118;
    /// Frame `flpdth16`.
    pub const FLPDTH16: i32 = 119;
    /// Frame `flpdth17`.
    pub const FLPDTH17: i32 = 120;
    /// Frame `flpdth18`.
    pub const FLPDTH18: i32 = 121;
    /// Frame `flpdth19`.
    pub const FLPDTH19: i32 = 122;
    /// Frame `flpdth20`.
    pub const FLPDTH20: i32 = 123;
    /// Frame `flpdth21`.
    pub const FLPDTH21: i32 = 124;
    /// Frame `flpdth22`.
    pub const FLPDTH22: i32 = 125;
    /// Frame `flpdth23`.
    pub const FLPDTH23: i32 = 126;
    /// Frame `flpdth24`.
    pub const FLPDTH24: i32 = 127;
    /// Frame `flpdth25`.
    pub const FLPDTH25: i32 = 128;
    /// Frame `flpdth26`.
    pub const FLPDTH26: i32 = 129;
    /// Frame `flpdth27`.
    pub const FLPDTH27: i32 = 130;
    /// Frame `flpdth28`.
    pub const FLPDTH28: i32 = 131;
    /// Frame `flpdth29`.
    pub const FLPDTH29: i32 = 132;
    /// Frame `flpdth30`.
    pub const FLPDTH30: i32 = 133;
    /// Frame `flpdth31`.
    pub const FLPDTH31: i32 = 134;
    /// Frame `flpdth32`.
    pub const FLPDTH32: i32 = 135;
    /// Frame `flpdth33`.
    pub const FLPDTH33: i32 = 136;
    /// Frame `flpdth34`.
    pub const FLPDTH34: i32 = 137;
    /// Frame `flpdth35`.
    pub const FLPDTH35: i32 = 138;
    /// Frame `flpdth36`.
    pub const FLPDTH36: i32 = 139;
    /// Frame `flpdth37`.
    pub const FLPDTH37: i32 = 140;
    /// Frame `flpdth38`.
    pub const FLPDTH38: i32 = 141;
    /// Frame `flpdth39`.
    pub const FLPDTH39: i32 = 142;
    /// Frame `flpdth40`.
    pub const FLPDTH40: i32 = 143;
    /// Frame `flpdth41`.
    pub const FLPDTH41: i32 = 144;
    /// Frame `flpdth42`.
    pub const FLPDTH42: i32 = 145;
    /// Frame `flpdth43`.
    pub const FLPDTH43: i32 = 146;
    /// Frame `flpdth44`.
    pub const FLPDTH44: i32 = 147;
    /// Frame `flpdth45`.
    pub const FLPDTH45: i32 = 148;
    /// Frame `flpdth46`.
    pub const FLPDTH46: i32 = 149;
    /// Frame `flpdth47`.
    pub const FLPDTH47: i32 = 150;
    /// Frame `flpdth48`.
    pub const FLPDTH48: i32 = 151;
    /// Frame `flpdth49`.
    pub const FLPDTH49: i32 = 152;
    /// Frame `flpdth50`.
    pub const FLPDTH50: i32 = 153;
    /// Frame `flpdth51`.
    pub const FLPDTH51: i32 = 154;
    /// Frame `flpdth52`.
    pub const FLPDTH52: i32 = 155;
    /// Frame `flpdth53`.
    pub const FLPDTH53: i32 = 156;
    /// Frame `flpdth54`.
    pub const FLPDTH54: i32 = 157;
    /// Frame `flpdth55`.
    pub const FLPDTH55: i32 = 158;
    /// Frame `flpdth56`.
    pub const FLPDTH56: i32 = 159;
}

/// `flipperMoves` move tables.
pub fn flipper_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "flipper_move_stand",
            41,
            41,
            None,
            vec![monster_frame(MonsterAi::Stand, 0.0, vec![], -1)],
        ),
        monster_move(
            "flipper_move_run_loop",
            70,
            93,
            None,
            vec![
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
            ],
        ),
        monster_move(
            "flipper_move_run_start",
            65,
            70,
            Some("flipper_run_loop"),
            vec![
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
            ],
        ),
        monster_move(
            "flipper_move_walk",
            41,
            64,
            None,
            vec![
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
            ],
        ),
        monster_move(
            "flipper_move_start_run",
            41,
            45,
            None,
            vec![
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![MonsterAction::name("flipper_run")], -1),
            ],
        ),
        monster_move(
            "flipper_move_pain2",
            94,
            98,
            Some("flipper_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "flipper_move_pain1",
            99,
            103,
            Some("flipper_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "flipper_move_attack",
            0,
            19,
            Some("flipper_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("flipper_preattack")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("flipper_bite")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("flipper_bite")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "flipper_move_death",
            104,
            159,
            Some("flipper_dead"),
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
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
    ]
}
