//! parasite move tables (`src/content/q2/base/monsters/tables/parasite.ts`).
//!
//! Original Quake II m_parasite.c frame order and distances. id Software, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `parasiteFrame`.
pub mod parasite_frame {
    /// Frame `break01`.
    pub const BREAK01: i32 = 0;
    /// Frame `break02`.
    pub const BREAK02: i32 = 1;
    /// Frame `break03`.
    pub const BREAK03: i32 = 2;
    /// Frame `break04`.
    pub const BREAK04: i32 = 3;
    /// Frame `break05`.
    pub const BREAK05: i32 = 4;
    /// Frame `break06`.
    pub const BREAK06: i32 = 5;
    /// Frame `break07`.
    pub const BREAK07: i32 = 6;
    /// Frame `break08`.
    pub const BREAK08: i32 = 7;
    /// Frame `break09`.
    pub const BREAK09: i32 = 8;
    /// Frame `break10`.
    pub const BREAK10: i32 = 9;
    /// Frame `break11`.
    pub const BREAK11: i32 = 10;
    /// Frame `break12`.
    pub const BREAK12: i32 = 11;
    /// Frame `break13`.
    pub const BREAK13: i32 = 12;
    /// Frame `break14`.
    pub const BREAK14: i32 = 13;
    /// Frame `break15`.
    pub const BREAK15: i32 = 14;
    /// Frame `break16`.
    pub const BREAK16: i32 = 15;
    /// Frame `break17`.
    pub const BREAK17: i32 = 16;
    /// Frame `break18`.
    pub const BREAK18: i32 = 17;
    /// Frame `break19`.
    pub const BREAK19: i32 = 18;
    /// Frame `break20`.
    pub const BREAK20: i32 = 19;
    /// Frame `break21`.
    pub const BREAK21: i32 = 20;
    /// Frame `break22`.
    pub const BREAK22: i32 = 21;
    /// Frame `break23`.
    pub const BREAK23: i32 = 22;
    /// Frame `break24`.
    pub const BREAK24: i32 = 23;
    /// Frame `break25`.
    pub const BREAK25: i32 = 24;
    /// Frame `break26`.
    pub const BREAK26: i32 = 25;
    /// Frame `break27`.
    pub const BREAK27: i32 = 26;
    /// Frame `break28`.
    pub const BREAK28: i32 = 27;
    /// Frame `break29`.
    pub const BREAK29: i32 = 28;
    /// Frame `break30`.
    pub const BREAK30: i32 = 29;
    /// Frame `break31`.
    pub const BREAK31: i32 = 30;
    /// Frame `break32`.
    pub const BREAK32: i32 = 31;
    /// Frame `death101`.
    pub const DEATH101: i32 = 32;
    /// Frame `death102`.
    pub const DEATH102: i32 = 33;
    /// Frame `death103`.
    pub const DEATH103: i32 = 34;
    /// Frame `death104`.
    pub const DEATH104: i32 = 35;
    /// Frame `death105`.
    pub const DEATH105: i32 = 36;
    /// Frame `death106`.
    pub const DEATH106: i32 = 37;
    /// Frame `death107`.
    pub const DEATH107: i32 = 38;
    /// Frame `drain01`.
    pub const DRAIN01: i32 = 39;
    /// Frame `drain02`.
    pub const DRAIN02: i32 = 40;
    /// Frame `drain03`.
    pub const DRAIN03: i32 = 41;
    /// Frame `drain04`.
    pub const DRAIN04: i32 = 42;
    /// Frame `drain05`.
    pub const DRAIN05: i32 = 43;
    /// Frame `drain06`.
    pub const DRAIN06: i32 = 44;
    /// Frame `drain07`.
    pub const DRAIN07: i32 = 45;
    /// Frame `drain08`.
    pub const DRAIN08: i32 = 46;
    /// Frame `drain09`.
    pub const DRAIN09: i32 = 47;
    /// Frame `drain10`.
    pub const DRAIN10: i32 = 48;
    /// Frame `drain11`.
    pub const DRAIN11: i32 = 49;
    /// Frame `drain12`.
    pub const DRAIN12: i32 = 50;
    /// Frame `drain13`.
    pub const DRAIN13: i32 = 51;
    /// Frame `drain14`.
    pub const DRAIN14: i32 = 52;
    /// Frame `drain15`.
    pub const DRAIN15: i32 = 53;
    /// Frame `drain16`.
    pub const DRAIN16: i32 = 54;
    /// Frame `drain17`.
    pub const DRAIN17: i32 = 55;
    /// Frame `drain18`.
    pub const DRAIN18: i32 = 56;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 57;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 58;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 59;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 60;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 61;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 62;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 63;
    /// Frame `pain108`.
    pub const PAIN108: i32 = 64;
    /// Frame `pain109`.
    pub const PAIN109: i32 = 65;
    /// Frame `pain110`.
    pub const PAIN110: i32 = 66;
    /// Frame `pain111`.
    pub const PAIN111: i32 = 67;
    /// Frame `run01`.
    pub const RUN01: i32 = 68;
    /// Frame `run02`.
    pub const RUN02: i32 = 69;
    /// Frame `run03`.
    pub const RUN03: i32 = 70;
    /// Frame `run04`.
    pub const RUN04: i32 = 71;
    /// Frame `run05`.
    pub const RUN05: i32 = 72;
    /// Frame `run06`.
    pub const RUN06: i32 = 73;
    /// Frame `run07`.
    pub const RUN07: i32 = 74;
    /// Frame `run08`.
    pub const RUN08: i32 = 75;
    /// Frame `run09`.
    pub const RUN09: i32 = 76;
    /// Frame `run10`.
    pub const RUN10: i32 = 77;
    /// Frame `run11`.
    pub const RUN11: i32 = 78;
    /// Frame `run12`.
    pub const RUN12: i32 = 79;
    /// Frame `run13`.
    pub const RUN13: i32 = 80;
    /// Frame `run14`.
    pub const RUN14: i32 = 81;
    /// Frame `run15`.
    pub const RUN15: i32 = 82;
    /// Frame `stand01`.
    pub const STAND01: i32 = 83;
    /// Frame `stand02`.
    pub const STAND02: i32 = 84;
    /// Frame `stand03`.
    pub const STAND03: i32 = 85;
    /// Frame `stand04`.
    pub const STAND04: i32 = 86;
    /// Frame `stand05`.
    pub const STAND05: i32 = 87;
    /// Frame `stand06`.
    pub const STAND06: i32 = 88;
    /// Frame `stand07`.
    pub const STAND07: i32 = 89;
    /// Frame `stand08`.
    pub const STAND08: i32 = 90;
    /// Frame `stand09`.
    pub const STAND09: i32 = 91;
    /// Frame `stand10`.
    pub const STAND10: i32 = 92;
    /// Frame `stand11`.
    pub const STAND11: i32 = 93;
    /// Frame `stand12`.
    pub const STAND12: i32 = 94;
    /// Frame `stand13`.
    pub const STAND13: i32 = 95;
    /// Frame `stand14`.
    pub const STAND14: i32 = 96;
    /// Frame `stand15`.
    pub const STAND15: i32 = 97;
    /// Frame `stand16`.
    pub const STAND16: i32 = 98;
    /// Frame `stand17`.
    pub const STAND17: i32 = 99;
    /// Frame `stand18`.
    pub const STAND18: i32 = 100;
    /// Frame `stand19`.
    pub const STAND19: i32 = 101;
    /// Frame `stand20`.
    pub const STAND20: i32 = 102;
    /// Frame `stand21`.
    pub const STAND21: i32 = 103;
    /// Frame `stand22`.
    pub const STAND22: i32 = 104;
    /// Frame `stand23`.
    pub const STAND23: i32 = 105;
    /// Frame `stand24`.
    pub const STAND24: i32 = 106;
    /// Frame `stand25`.
    pub const STAND25: i32 = 107;
    /// Frame `stand26`.
    pub const STAND26: i32 = 108;
    /// Frame `stand27`.
    pub const STAND27: i32 = 109;
    /// Frame `stand28`.
    pub const STAND28: i32 = 110;
    /// Frame `stand29`.
    pub const STAND29: i32 = 111;
    /// Frame `stand30`.
    pub const STAND30: i32 = 112;
    /// Frame `stand31`.
    pub const STAND31: i32 = 113;
    /// Frame `stand32`.
    pub const STAND32: i32 = 114;
    /// Frame `stand33`.
    pub const STAND33: i32 = 115;
    /// Frame `stand34`.
    pub const STAND34: i32 = 116;
    /// Frame `stand35`.
    pub const STAND35: i32 = 117;
}

/// `parasiteMoves` move tables.
pub fn parasite_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "parasite_move_start_fidget",
            100,
            103,
            Some("parasite_do_fidget"),
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_fidget",
            104,
            109,
            Some("parasite_refidget"),
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_scratch")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_scratch")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_end_fidget",
            110,
            117,
            Some("parasite_stand"),
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_scratch")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_stand",
            83,
            99,
            Some("parasite_stand"),
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_tap")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_tap")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_tap")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_tap")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_tap")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("parasite_tap")], -1),
            ],
        ),
        monster_move(
            "parasite_move_run",
            70,
            76,
            None,
            vec![
                monster_frame(MonsterAi::Run, 30.0, vec![], -1),
                monster_frame(MonsterAi::Run, 30.0, vec![], -1),
                monster_frame(MonsterAi::Run, 22.0, vec![], -1),
                monster_frame(MonsterAi::Run, 19.0, vec![], -1),
                monster_frame(MonsterAi::Run, 24.0, vec![], -1),
                monster_frame(MonsterAi::Run, 28.0, vec![], -1),
                monster_frame(MonsterAi::Run, 25.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_start_run",
            68,
            69,
            Some("parasite_run"),
            vec![
                monster_frame(MonsterAi::Run, 0.0, vec![], -1),
                monster_frame(MonsterAi::Run, 30.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_stop_run",
            77,
            82,
            None,
            vec![
                monster_frame(MonsterAi::Run, 20.0, vec![], -1),
                monster_frame(MonsterAi::Run, 20.0, vec![], -1),
                monster_frame(MonsterAi::Run, 12.0, vec![], -1),
                monster_frame(MonsterAi::Run, 10.0, vec![], -1),
                monster_frame(MonsterAi::Run, 0.0, vec![], -1),
                monster_frame(MonsterAi::Run, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_walk",
            70,
            76,
            Some("parasite_walk"),
            vec![
                monster_frame(MonsterAi::Walk, 30.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 30.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 22.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 19.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 24.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 28.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 25.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_start_walk",
            68,
            69,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 30.0, vec![MonsterAction::name("parasite_walk")], -1),
            ],
        ),
        monster_move(
            "parasite_move_stop_walk",
            77,
            82,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 20.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 20.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 12.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 10.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_pain1",
            57,
            67,
            Some("parasite_start_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 6.0, vec![], -1),
                monster_frame(MonsterAi::Move, 16.0, vec![], -1),
                monster_frame(MonsterAi::Move, -6.0, vec![], -1),
                monster_frame(MonsterAi::Move, -7.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_drain",
            39,
            56,
            Some("parasite_start_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("parasite_launch")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    15.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -2.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -2.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -3.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -2.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    -1.0,
                    vec![MonsterAction::name("parasite_drain_attack")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("parasite_reel_in")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_break",
            0,
            31,
            Some("parasite_start_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -18.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 9.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -18.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 8.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 9.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -18.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 11.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -5.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
            ],
        ),
        monster_move(
            "parasite_move_death",
            32,
            38,
            Some("parasite_dead"),
            vec![
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
