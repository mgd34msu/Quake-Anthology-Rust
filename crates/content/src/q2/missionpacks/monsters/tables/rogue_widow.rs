//! widow move tables (`src/content/q2/missionpacks/monsters/tables/rogue-widow.ts`).
//!
//! Original Quake II rogue/m_widow.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `widowFrame`.
pub mod widow_frame {
    /// Frame `idle01`.
    pub const IDLE01: i32 = 0;
    /// Frame `idle02`.
    pub const IDLE02: i32 = 1;
    /// Frame `idle03`.
    pub const IDLE03: i32 = 2;
    /// Frame `idle04`.
    pub const IDLE04: i32 = 3;
    /// Frame `idle05`.
    pub const IDLE05: i32 = 4;
    /// Frame `idle06`.
    pub const IDLE06: i32 = 5;
    /// Frame `idle07`.
    pub const IDLE07: i32 = 6;
    /// Frame `idle08`.
    pub const IDLE08: i32 = 7;
    /// Frame `idle09`.
    pub const IDLE09: i32 = 8;
    /// Frame `idle10`.
    pub const IDLE10: i32 = 9;
    /// Frame `idle11`.
    pub const IDLE11: i32 = 10;
    /// Frame `walk01`.
    pub const WALK01: i32 = 11;
    /// Frame `walk02`.
    pub const WALK02: i32 = 12;
    /// Frame `walk03`.
    pub const WALK03: i32 = 13;
    /// Frame `walk04`.
    pub const WALK04: i32 = 14;
    /// Frame `walk05`.
    pub const WALK05: i32 = 15;
    /// Frame `walk06`.
    pub const WALK06: i32 = 16;
    /// Frame `walk07`.
    pub const WALK07: i32 = 17;
    /// Frame `walk08`.
    pub const WALK08: i32 = 18;
    /// Frame `walk09`.
    pub const WALK09: i32 = 19;
    /// Frame `walk10`.
    pub const WALK10: i32 = 20;
    /// Frame `walk11`.
    pub const WALK11: i32 = 21;
    /// Frame `walk12`.
    pub const WALK12: i32 = 22;
    /// Frame `walk13`.
    pub const WALK13: i32 = 23;
    /// Frame `run01`.
    pub const RUN01: i32 = 24;
    /// Frame `run02`.
    pub const RUN02: i32 = 25;
    /// Frame `run03`.
    pub const RUN03: i32 = 26;
    /// Frame `run04`.
    pub const RUN04: i32 = 27;
    /// Frame `run05`.
    pub const RUN05: i32 = 28;
    /// Frame `run06`.
    pub const RUN06: i32 = 29;
    /// Frame `run07`.
    pub const RUN07: i32 = 30;
    /// Frame `run08`.
    pub const RUN08: i32 = 31;
    /// Frame `firea01`.
    pub const FIREA01: i32 = 32;
    /// Frame `firea02`.
    pub const FIREA02: i32 = 33;
    /// Frame `firea03`.
    pub const FIREA03: i32 = 34;
    /// Frame `firea04`.
    pub const FIREA04: i32 = 35;
    /// Frame `firea05`.
    pub const FIREA05: i32 = 36;
    /// Frame `firea06`.
    pub const FIREA06: i32 = 37;
    /// Frame `firea07`.
    pub const FIREA07: i32 = 38;
    /// Frame `firea08`.
    pub const FIREA08: i32 = 39;
    /// Frame `firea09`.
    pub const FIREA09: i32 = 40;
    /// Frame `fireb01`.
    pub const FIREB01: i32 = 41;
    /// Frame `fireb02`.
    pub const FIREB02: i32 = 42;
    /// Frame `fireb03`.
    pub const FIREB03: i32 = 43;
    /// Frame `fireb04`.
    pub const FIREB04: i32 = 44;
    /// Frame `fireb05`.
    pub const FIREB05: i32 = 45;
    /// Frame `fireb06`.
    pub const FIREB06: i32 = 46;
    /// Frame `fireb07`.
    pub const FIREB07: i32 = 47;
    /// Frame `fireb08`.
    pub const FIREB08: i32 = 48;
    /// Frame `fireb09`.
    pub const FIREB09: i32 = 49;
    /// Frame `firec01`.
    pub const FIREC01: i32 = 50;
    /// Frame `firec02`.
    pub const FIREC02: i32 = 51;
    /// Frame `firec03`.
    pub const FIREC03: i32 = 52;
    /// Frame `firec04`.
    pub const FIREC04: i32 = 53;
    /// Frame `firec05`.
    pub const FIREC05: i32 = 54;
    /// Frame `firec06`.
    pub const FIREC06: i32 = 55;
    /// Frame `firec07`.
    pub const FIREC07: i32 = 56;
    /// Frame `firec08`.
    pub const FIREC08: i32 = 57;
    /// Frame `firec09`.
    pub const FIREC09: i32 = 58;
    /// Frame `fired01`.
    pub const FIRED01: i32 = 59;
    /// Frame `fired02`.
    pub const FIRED02: i32 = 60;
    /// Frame `fired02a`.
    pub const FIRED02A: i32 = 61;
    /// Frame `fired03`.
    pub const FIRED03: i32 = 62;
    /// Frame `fired04`.
    pub const FIRED04: i32 = 63;
    /// Frame `fired05`.
    pub const FIRED05: i32 = 64;
    /// Frame `fired06`.
    pub const FIRED06: i32 = 65;
    /// Frame `fired07`.
    pub const FIRED07: i32 = 66;
    /// Frame `fired08`.
    pub const FIRED08: i32 = 67;
    /// Frame `fired09`.
    pub const FIRED09: i32 = 68;
    /// Frame `fired10`.
    pub const FIRED10: i32 = 69;
    /// Frame `fired11`.
    pub const FIRED11: i32 = 70;
    /// Frame `fired12`.
    pub const FIRED12: i32 = 71;
    /// Frame `fired13`.
    pub const FIRED13: i32 = 72;
    /// Frame `fired14`.
    pub const FIRED14: i32 = 73;
    /// Frame `fired15`.
    pub const FIRED15: i32 = 74;
    /// Frame `fired16`.
    pub const FIRED16: i32 = 75;
    /// Frame `fired17`.
    pub const FIRED17: i32 = 76;
    /// Frame `fired18`.
    pub const FIRED18: i32 = 77;
    /// Frame `fired19`.
    pub const FIRED19: i32 = 78;
    /// Frame `fired20`.
    pub const FIRED20: i32 = 79;
    /// Frame `fired21`.
    pub const FIRED21: i32 = 80;
    /// Frame `fired22`.
    pub const FIRED22: i32 = 81;
    /// Frame `spawn01`.
    pub const SPAWN01: i32 = 82;
    /// Frame `spawn02`.
    pub const SPAWN02: i32 = 83;
    /// Frame `spawn03`.
    pub const SPAWN03: i32 = 84;
    /// Frame `spawn04`.
    pub const SPAWN04: i32 = 85;
    /// Frame `spawn05`.
    pub const SPAWN05: i32 = 86;
    /// Frame `spawn06`.
    pub const SPAWN06: i32 = 87;
    /// Frame `spawn07`.
    pub const SPAWN07: i32 = 88;
    /// Frame `spawn08`.
    pub const SPAWN08: i32 = 89;
    /// Frame `spawn09`.
    pub const SPAWN09: i32 = 90;
    /// Frame `spawn10`.
    pub const SPAWN10: i32 = 91;
    /// Frame `spawn11`.
    pub const SPAWN11: i32 = 92;
    /// Frame `spawn12`.
    pub const SPAWN12: i32 = 93;
    /// Frame `spawn13`.
    pub const SPAWN13: i32 = 94;
    /// Frame `spawn14`.
    pub const SPAWN14: i32 = 95;
    /// Frame `spawn15`.
    pub const SPAWN15: i32 = 96;
    /// Frame `spawn16`.
    pub const SPAWN16: i32 = 97;
    /// Frame `spawn17`.
    pub const SPAWN17: i32 = 98;
    /// Frame `spawn18`.
    pub const SPAWN18: i32 = 99;
    /// Frame `pain01`.
    pub const PAIN01: i32 = 100;
    /// Frame `pain02`.
    pub const PAIN02: i32 = 101;
    /// Frame `pain03`.
    pub const PAIN03: i32 = 102;
    /// Frame `pain04`.
    pub const PAIN04: i32 = 103;
    /// Frame `pain05`.
    pub const PAIN05: i32 = 104;
    /// Frame `pain06`.
    pub const PAIN06: i32 = 105;
    /// Frame `pain07`.
    pub const PAIN07: i32 = 106;
    /// Frame `pain08`.
    pub const PAIN08: i32 = 107;
    /// Frame `pain09`.
    pub const PAIN09: i32 = 108;
    /// Frame `pain10`.
    pub const PAIN10: i32 = 109;
    /// Frame `pain11`.
    pub const PAIN11: i32 = 110;
    /// Frame `pain12`.
    pub const PAIN12: i32 = 111;
    /// Frame `pain13`.
    pub const PAIN13: i32 = 112;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 113;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 114;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 115;
    /// Frame `transa01`.
    pub const TRANSA01: i32 = 116;
    /// Frame `transa02`.
    pub const TRANSA02: i32 = 117;
    /// Frame `transa03`.
    pub const TRANSA03: i32 = 118;
    /// Frame `transa04`.
    pub const TRANSA04: i32 = 119;
    /// Frame `transa05`.
    pub const TRANSA05: i32 = 120;
    /// Frame `transb01`.
    pub const TRANSB01: i32 = 121;
    /// Frame `transb02`.
    pub const TRANSB02: i32 = 122;
    /// Frame `transb03`.
    pub const TRANSB03: i32 = 123;
    /// Frame `transb04`.
    pub const TRANSB04: i32 = 124;
    /// Frame `transb05`.
    pub const TRANSB05: i32 = 125;
    /// Frame `transc01`.
    pub const TRANSC01: i32 = 126;
    /// Frame `transc02`.
    pub const TRANSC02: i32 = 127;
    /// Frame `transc03`.
    pub const TRANSC03: i32 = 128;
    /// Frame `transc04`.
    pub const TRANSC04: i32 = 129;
    /// Frame `death01`.
    pub const DEATH01: i32 = 130;
    /// Frame `death02`.
    pub const DEATH02: i32 = 131;
    /// Frame `death03`.
    pub const DEATH03: i32 = 132;
    /// Frame `death04`.
    pub const DEATH04: i32 = 133;
    /// Frame `death05`.
    pub const DEATH05: i32 = 134;
    /// Frame `death06`.
    pub const DEATH06: i32 = 135;
    /// Frame `death07`.
    pub const DEATH07: i32 = 136;
    /// Frame `death08`.
    pub const DEATH08: i32 = 137;
    /// Frame `death09`.
    pub const DEATH09: i32 = 138;
    /// Frame `death10`.
    pub const DEATH10: i32 = 139;
    /// Frame `death11`.
    pub const DEATH11: i32 = 140;
    /// Frame `death12`.
    pub const DEATH12: i32 = 141;
    /// Frame `death13`.
    pub const DEATH13: i32 = 142;
    /// Frame `death14`.
    pub const DEATH14: i32 = 143;
    /// Frame `death15`.
    pub const DEATH15: i32 = 144;
    /// Frame `death16`.
    pub const DEATH16: i32 = 145;
    /// Frame `death17`.
    pub const DEATH17: i32 = 146;
    /// Frame `death18`.
    pub const DEATH18: i32 = 147;
    /// Frame `death19`.
    pub const DEATH19: i32 = 148;
    /// Frame `death20`.
    pub const DEATH20: i32 = 149;
    /// Frame `death21`.
    pub const DEATH21: i32 = 150;
    /// Frame `death22`.
    pub const DEATH22: i32 = 151;
    /// Frame `death23`.
    pub const DEATH23: i32 = 152;
    /// Frame `death24`.
    pub const DEATH24: i32 = 153;
    /// Frame `death25`.
    pub const DEATH25: i32 = 154;
    /// Frame `death26`.
    pub const DEATH26: i32 = 155;
    /// Frame `death27`.
    pub const DEATH27: i32 = 156;
    /// Frame `death28`.
    pub const DEATH28: i32 = 157;
    /// Frame `death29`.
    pub const DEATH29: i32 = 158;
    /// Frame `death30`.
    pub const DEATH30: i32 = 159;
    /// Frame `death31`.
    pub const DEATH31: i32 = 160;
    /// Frame `kick01`.
    pub const KICK01: i32 = 161;
    /// Frame `kick02`.
    pub const KICK02: i32 = 162;
    /// Frame `kick03`.
    pub const KICK03: i32 = 163;
    /// Frame `kick04`.
    pub const KICK04: i32 = 164;
    /// Frame `kick05`.
    pub const KICK05: i32 = 165;
    /// Frame `kick06`.
    pub const KICK06: i32 = 166;
    /// Frame `kick07`.
    pub const KICK07: i32 = 167;
    /// Frame `kick08`.
    pub const KICK08: i32 = 168;
}

/// `widowMoves` move tables.
pub fn widow_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "widow_move_stand",
            0,
            10,
            None,
            vec![
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
            ],
        ),
        monster_move(
            "widow_move_walk",
            11,
            23,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 2.79, vec![MonsterAction::name("widow_step")], -1),
                monster_frame(MonsterAi::Walk, 2.77, vec![], -1),
                monster_frame(MonsterAi::Walk, 3.53, vec![], -1),
                monster_frame(MonsterAi::Walk, 3.97, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.13, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.09, vec![], -1),
                monster_frame(MonsterAi::Walk, 3.84, vec![], -1),
                monster_frame(MonsterAi::Walk, 3.62, vec![MonsterAction::name("widow_step")], -1),
                monster_frame(MonsterAi::Walk, 3.29, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.08, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.94, vec![], -1),
                monster_frame(MonsterAi::Walk, 5.73, vec![], -1),
                monster_frame(MonsterAi::Walk, 2.85, vec![], -1),
            ],
        ),
        monster_move(
            "widow_move_run",
            11,
            23,
            None,
            vec![
                monster_frame(MonsterAi::Run, 2.79, vec![MonsterAction::name("widow_step")], -1),
                monster_frame(MonsterAi::Run, 2.77, vec![], -1),
                monster_frame(MonsterAi::Run, 3.53, vec![], -1),
                monster_frame(MonsterAi::Run, 3.97, vec![], -1),
                monster_frame(MonsterAi::Run, 4.13, vec![], -1),
                monster_frame(MonsterAi::Run, 4.09, vec![], -1),
                monster_frame(MonsterAi::Run, 3.84, vec![], -1),
                monster_frame(MonsterAi::Run, 3.62, vec![MonsterAction::name("widow_step")], -1),
                monster_frame(MonsterAi::Run, 3.29, vec![], -1),
                monster_frame(MonsterAi::Run, 6.08, vec![], -1),
                monster_frame(MonsterAi::Run, 6.94, vec![], -1),
                monster_frame(MonsterAi::Run, 5.73, vec![], -1),
                monster_frame(MonsterAi::Run, 2.85, vec![], -1),
            ],
        ),
        monster_move(
            "widow_move_run_attack",
            24,
            31,
            Some("widow_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    13.0,
                    vec![MonsterAction::name("widow_stepshoot")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 11.72, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 18.04, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 14.58, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    13.0,
                    vec![MonsterAction::name("widow_stepshoot")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 12.12, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 19.63, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 11.37, vec![MonsterAction::name("WidowBlaster")], -1),
            ],
        ),
        monster_move(
            "widow_move_attack_pre_blaster",
            59,
            61,
            None,
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_attack_blaster")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "widow_move_attack_blaster",
            61,
            79,
            None,
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_reattack_blaster")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "widow_move_attack_post_blaster",
            80,
            81,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "widow_move_attack_post_blaster_r",
            116,
            120,
            None,
            vec![
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_start_run_12")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "widow_move_attack_post_blaster_l",
            121,
            125,
            None,
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 14.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    10.0,
                    vec![MonsterAction::name("widow_start_run_12")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "widow_move_attack_pre_rail",
            126,
            129,
            None,
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_start_rail")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_attack_rail")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "widow_move_attack_rail",
            32,
            40,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowSaveLoc")], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![MonsterAction::name("WidowRail")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow_rail_done")], -1),
            ],
        ),
        monster_move(
            "widow_move_attack_rail_r",
            41,
            49,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowSaveLoc")], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![MonsterAction::name("WidowRail")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow_rail_done")], -1),
            ],
        ),
        monster_move(
            "widow_move_attack_rail_l",
            50,
            58,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowSaveLoc")], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![MonsterAction::name("WidowRail")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("widow_rail_done")], -1),
            ],
        ),
        monster_move(
            "widow_move_spawn",
            82,
            99,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_start_spawn")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_ready_spawn")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_spawn_check")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("WidowBlaster")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("widow_done_spawn")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "widow_move_pain_heavy",
            100,
            112,
            Some("widow_run"),
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
            ],
        ),
        monster_move(
            "widow_move_pain_light",
            113,
            115,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "widow_move_death",
            130,
            160,
            None,
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
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("spawn_out_start")], -1),
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
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("spawn_out_do")], -1),
            ],
        ),
        monster_move(
            "widow_move_attack_kick",
            161,
            168,
            Some("widow_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("widow_attack_kick")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
    ]
}
