//! flyer move tables (`src/content/q2/rerelease/monsters/tables/flyer.ts`).

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `flyerFrame`.
pub mod flyer_frame {
    /// Frame `start01`.
    pub const START01: i32 = 0;
    /// Frame `start02`.
    pub const START02: i32 = 1;
    /// Frame `start03`.
    pub const START03: i32 = 2;
    /// Frame `start04`.
    pub const START04: i32 = 3;
    /// Frame `start05`.
    pub const START05: i32 = 4;
    /// Frame `start06`.
    pub const START06: i32 = 5;
    /// Frame `stop01`.
    pub const STOP01: i32 = 6;
    /// Frame `stop02`.
    pub const STOP02: i32 = 7;
    /// Frame `stop03`.
    pub const STOP03: i32 = 8;
    /// Frame `stop04`.
    pub const STOP04: i32 = 9;
    /// Frame `stop05`.
    pub const STOP05: i32 = 10;
    /// Frame `stop06`.
    pub const STOP06: i32 = 11;
    /// Frame `stop07`.
    pub const STOP07: i32 = 12;
    /// Frame `stand01`.
    pub const STAND01: i32 = 13;
    /// Frame `stand02`.
    pub const STAND02: i32 = 14;
    /// Frame `stand03`.
    pub const STAND03: i32 = 15;
    /// Frame `stand04`.
    pub const STAND04: i32 = 16;
    /// Frame `stand05`.
    pub const STAND05: i32 = 17;
    /// Frame `stand06`.
    pub const STAND06: i32 = 18;
    /// Frame `stand07`.
    pub const STAND07: i32 = 19;
    /// Frame `stand08`.
    pub const STAND08: i32 = 20;
    /// Frame `stand09`.
    pub const STAND09: i32 = 21;
    /// Frame `stand10`.
    pub const STAND10: i32 = 22;
    /// Frame `stand11`.
    pub const STAND11: i32 = 23;
    /// Frame `stand12`.
    pub const STAND12: i32 = 24;
    /// Frame `stand13`.
    pub const STAND13: i32 = 25;
    /// Frame `stand14`.
    pub const STAND14: i32 = 26;
    /// Frame `stand15`.
    pub const STAND15: i32 = 27;
    /// Frame `stand16`.
    pub const STAND16: i32 = 28;
    /// Frame `stand17`.
    pub const STAND17: i32 = 29;
    /// Frame `stand18`.
    pub const STAND18: i32 = 30;
    /// Frame `stand19`.
    pub const STAND19: i32 = 31;
    /// Frame `stand20`.
    pub const STAND20: i32 = 32;
    /// Frame `stand21`.
    pub const STAND21: i32 = 33;
    /// Frame `stand22`.
    pub const STAND22: i32 = 34;
    /// Frame `stand23`.
    pub const STAND23: i32 = 35;
    /// Frame `stand24`.
    pub const STAND24: i32 = 36;
    /// Frame `stand25`.
    pub const STAND25: i32 = 37;
    /// Frame `stand26`.
    pub const STAND26: i32 = 38;
    /// Frame `stand27`.
    pub const STAND27: i32 = 39;
    /// Frame `stand28`.
    pub const STAND28: i32 = 40;
    /// Frame `stand29`.
    pub const STAND29: i32 = 41;
    /// Frame `stand30`.
    pub const STAND30: i32 = 42;
    /// Frame `stand31`.
    pub const STAND31: i32 = 43;
    /// Frame `stand32`.
    pub const STAND32: i32 = 44;
    /// Frame `stand33`.
    pub const STAND33: i32 = 45;
    /// Frame `stand34`.
    pub const STAND34: i32 = 46;
    /// Frame `stand35`.
    pub const STAND35: i32 = 47;
    /// Frame `stand36`.
    pub const STAND36: i32 = 48;
    /// Frame `stand37`.
    pub const STAND37: i32 = 49;
    /// Frame `stand38`.
    pub const STAND38: i32 = 50;
    /// Frame `stand39`.
    pub const STAND39: i32 = 51;
    /// Frame `stand40`.
    pub const STAND40: i32 = 52;
    /// Frame `stand41`.
    pub const STAND41: i32 = 53;
    /// Frame `stand42`.
    pub const STAND42: i32 = 54;
    /// Frame `stand43`.
    pub const STAND43: i32 = 55;
    /// Frame `stand44`.
    pub const STAND44: i32 = 56;
    /// Frame `stand45`.
    pub const STAND45: i32 = 57;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 58;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 59;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 60;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 61;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 62;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 63;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 64;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 65;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 66;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 67;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 68;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 69;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 70;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 71;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 72;
    /// Frame `attak116`.
    pub const ATTAK116: i32 = 73;
    /// Frame `attak117`.
    pub const ATTAK117: i32 = 74;
    /// Frame `attak118`.
    pub const ATTAK118: i32 = 75;
    /// Frame `attak119`.
    pub const ATTAK119: i32 = 76;
    /// Frame `attak120`.
    pub const ATTAK120: i32 = 77;
    /// Frame `attak121`.
    pub const ATTAK121: i32 = 78;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 79;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 80;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 81;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 82;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 83;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 84;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 85;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 86;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 87;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 88;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 89;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 90;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 91;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 92;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 93;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 94;
    /// Frame `attak217`.
    pub const ATTAK217: i32 = 95;
    /// Frame `bankl01`.
    pub const BANKL01: i32 = 96;
    /// Frame `bankl02`.
    pub const BANKL02: i32 = 97;
    /// Frame `bankl03`.
    pub const BANKL03: i32 = 98;
    /// Frame `bankl04`.
    pub const BANKL04: i32 = 99;
    /// Frame `bankl05`.
    pub const BANKL05: i32 = 100;
    /// Frame `bankl06`.
    pub const BANKL06: i32 = 101;
    /// Frame `bankl07`.
    pub const BANKL07: i32 = 102;
    /// Frame `bankr01`.
    pub const BANKR01: i32 = 103;
    /// Frame `bankr02`.
    pub const BANKR02: i32 = 104;
    /// Frame `bankr03`.
    pub const BANKR03: i32 = 105;
    /// Frame `bankr04`.
    pub const BANKR04: i32 = 106;
    /// Frame `bankr05`.
    pub const BANKR05: i32 = 107;
    /// Frame `bankr06`.
    pub const BANKR06: i32 = 108;
    /// Frame `bankr07`.
    pub const BANKR07: i32 = 109;
    /// Frame `rollf01`.
    pub const ROLLF01: i32 = 110;
    /// Frame `rollf02`.
    pub const ROLLF02: i32 = 111;
    /// Frame `rollf03`.
    pub const ROLLF03: i32 = 112;
    /// Frame `rollf04`.
    pub const ROLLF04: i32 = 113;
    /// Frame `rollf05`.
    pub const ROLLF05: i32 = 114;
    /// Frame `rollf06`.
    pub const ROLLF06: i32 = 115;
    /// Frame `rollf07`.
    pub const ROLLF07: i32 = 116;
    /// Frame `rollf08`.
    pub const ROLLF08: i32 = 117;
    /// Frame `rollf09`.
    pub const ROLLF09: i32 = 118;
    /// Frame `rollr01`.
    pub const ROLLR01: i32 = 119;
    /// Frame `rollr02`.
    pub const ROLLR02: i32 = 120;
    /// Frame `rollr03`.
    pub const ROLLR03: i32 = 121;
    /// Frame `rollr04`.
    pub const ROLLR04: i32 = 122;
    /// Frame `rollr05`.
    pub const ROLLR05: i32 = 123;
    /// Frame `rollr06`.
    pub const ROLLR06: i32 = 124;
    /// Frame `rollr07`.
    pub const ROLLR07: i32 = 125;
    /// Frame `rollr08`.
    pub const ROLLR08: i32 = 126;
    /// Frame `rollr09`.
    pub const ROLLR09: i32 = 127;
    /// Frame `defens01`.
    pub const DEFENS01: i32 = 128;
    /// Frame `defens02`.
    pub const DEFENS02: i32 = 129;
    /// Frame `defens03`.
    pub const DEFENS03: i32 = 130;
    /// Frame `defens04`.
    pub const DEFENS04: i32 = 131;
    /// Frame `defens05`.
    pub const DEFENS05: i32 = 132;
    /// Frame `defens06`.
    pub const DEFENS06: i32 = 133;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 134;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 135;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 136;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 137;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 138;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 139;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 140;
    /// Frame `pain108`.
    pub const PAIN108: i32 = 141;
    /// Frame `pain109`.
    pub const PAIN109: i32 = 142;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 143;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 144;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 145;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 146;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 147;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 148;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 149;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 150;
}

/// `flyerMoves` move tables.
pub fn flyer_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "flyer_move_stand",
            13,
            57,
            None,
            vec![
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
            ],
        ),
        monster_move(
            "flyer_move_walk",
            13,
            57,
            None,
            vec![
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_run",
            13,
            57,
            None,
            vec![
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (10f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_kamikaze",
            120,
            124,
            Some("flyer_kamikaze"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (40f32) as f64,
                    vec![MonsterAction::name("flyer_kamikaze_check")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (40f32) as f64,
                    vec![MonsterAction::name("flyer_kamikaze_check")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (40f32) as f64,
                    vec![MonsterAction::name("flyer_kamikaze_check")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (40f32) as f64,
                    vec![MonsterAction::name("flyer_kamikaze_check")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (40f32) as f64,
                    vec![MonsterAction::name("flyer_kamikaze_check")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "flyer_move_rollright",
            119,
            127,
            None,
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_rollleft",
            110,
            118,
            None,
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_pain3",
            147,
            150,
            Some("flyer_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_pain2",
            143,
            146,
            Some("flyer_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_pain1",
            134,
            142,
            Some("flyer_run"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_defense",
            128,
            133,
            None,
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_bankright",
            103,
            109,
            None,
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_bankleft",
            96,
            102,
            None,
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_attack2",
            79,
            95,
            Some("flyer_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (-10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_attack3",
            79,
            95,
            Some("flyer_run"),
            vec![
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireleft")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (10f32) as f64,
                    vec![MonsterAction::name("flyer_fireright")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (10f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_start_melee",
            58,
            63,
            Some("flyer_loop_melee"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("flyer_pop_blades")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_end_melee",
            76,
            78,
            Some("flyer_run"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "flyer_move_loop_melee",
            64,
            75,
            Some("flyer_check_melee"),
            vec![
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("flyer_slash_left")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("flyer_slash_right")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
            ],
        ),
    ]
}
