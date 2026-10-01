//! shambler move tables (`src/content/q2/rerelease/monsters/tables/shambler.ts`).

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `shamblerFrame`.
pub mod shambler_frame {
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
    /// Frame `walk01`.
    pub const WALK01: i32 = 17;
    /// Frame `walk02`.
    pub const WALK02: i32 = 18;
    /// Frame `walk03`.
    pub const WALK03: i32 = 19;
    /// Frame `walk04`.
    pub const WALK04: i32 = 20;
    /// Frame `walk05`.
    pub const WALK05: i32 = 21;
    /// Frame `walk06`.
    pub const WALK06: i32 = 22;
    /// Frame `walk07`.
    pub const WALK07: i32 = 23;
    /// Frame `walk08`.
    pub const WALK08: i32 = 24;
    /// Frame `walk09`.
    pub const WALK09: i32 = 25;
    /// Frame `walk10`.
    pub const WALK10: i32 = 26;
    /// Frame `walk11`.
    pub const WALK11: i32 = 27;
    /// Frame `walk12`.
    pub const WALK12: i32 = 28;
    /// Frame `run01`.
    pub const RUN01: i32 = 29;
    /// Frame `run02`.
    pub const RUN02: i32 = 30;
    /// Frame `run03`.
    pub const RUN03: i32 = 31;
    /// Frame `run04`.
    pub const RUN04: i32 = 32;
    /// Frame `run05`.
    pub const RUN05: i32 = 33;
    /// Frame `run06`.
    pub const RUN06: i32 = 34;
    /// Frame `smash01`.
    pub const SMASH01: i32 = 35;
    /// Frame `smash02`.
    pub const SMASH02: i32 = 36;
    /// Frame `smash03`.
    pub const SMASH03: i32 = 37;
    /// Frame `smash04`.
    pub const SMASH04: i32 = 38;
    /// Frame `smash05`.
    pub const SMASH05: i32 = 39;
    /// Frame `smash06`.
    pub const SMASH06: i32 = 40;
    /// Frame `smash07`.
    pub const SMASH07: i32 = 41;
    /// Frame `smash08`.
    pub const SMASH08: i32 = 42;
    /// Frame `smash09`.
    pub const SMASH09: i32 = 43;
    /// Frame `smash10`.
    pub const SMASH10: i32 = 44;
    /// Frame `smash11`.
    pub const SMASH11: i32 = 45;
    /// Frame `smash12`.
    pub const SMASH12: i32 = 46;
    /// Frame `swingr01`.
    pub const SWINGR01: i32 = 47;
    /// Frame `swingr02`.
    pub const SWINGR02: i32 = 48;
    /// Frame `swingr03`.
    pub const SWINGR03: i32 = 49;
    /// Frame `swingr04`.
    pub const SWINGR04: i32 = 50;
    /// Frame `swingr05`.
    pub const SWINGR05: i32 = 51;
    /// Frame `swingr06`.
    pub const SWINGR06: i32 = 52;
    /// Frame `swingr07`.
    pub const SWINGR07: i32 = 53;
    /// Frame `swingr08`.
    pub const SWINGR08: i32 = 54;
    /// Frame `swingr09`.
    pub const SWINGR09: i32 = 55;
    /// Frame `swingl01`.
    pub const SWINGL01: i32 = 56;
    /// Frame `swingl02`.
    pub const SWINGL02: i32 = 57;
    /// Frame `swingl03`.
    pub const SWINGL03: i32 = 58;
    /// Frame `swingl04`.
    pub const SWINGL04: i32 = 59;
    /// Frame `swingl05`.
    pub const SWINGL05: i32 = 60;
    /// Frame `swingl06`.
    pub const SWINGL06: i32 = 61;
    /// Frame `swingl07`.
    pub const SWINGL07: i32 = 62;
    /// Frame `swingl08`.
    pub const SWINGL08: i32 = 63;
    /// Frame `swingl09`.
    pub const SWINGL09: i32 = 64;
    /// Frame `magic01`.
    pub const MAGIC01: i32 = 65;
    /// Frame `magic02`.
    pub const MAGIC02: i32 = 66;
    /// Frame `magic03`.
    pub const MAGIC03: i32 = 67;
    /// Frame `magic04`.
    pub const MAGIC04: i32 = 68;
    /// Frame `magic05`.
    pub const MAGIC05: i32 = 69;
    /// Frame `magic06`.
    pub const MAGIC06: i32 = 70;
    /// Frame `magic07`.
    pub const MAGIC07: i32 = 71;
    /// Frame `magic08`.
    pub const MAGIC08: i32 = 72;
    /// Frame `magic09`.
    pub const MAGIC09: i32 = 73;
    /// Frame `magic10`.
    pub const MAGIC10: i32 = 74;
    /// Frame `magic11`.
    pub const MAGIC11: i32 = 75;
    /// Frame `magic12`.
    pub const MAGIC12: i32 = 76;
    /// Frame `pain01`.
    pub const PAIN01: i32 = 77;
    /// Frame `pain02`.
    pub const PAIN02: i32 = 78;
    /// Frame `pain03`.
    pub const PAIN03: i32 = 79;
    /// Frame `pain04`.
    pub const PAIN04: i32 = 80;
    /// Frame `pain05`.
    pub const PAIN05: i32 = 81;
    /// Frame `pain06`.
    pub const PAIN06: i32 = 82;
    /// Frame `death01`.
    pub const DEATH01: i32 = 83;
    /// Frame `death02`.
    pub const DEATH02: i32 = 84;
    /// Frame `death03`.
    pub const DEATH03: i32 = 85;
    /// Frame `death04`.
    pub const DEATH04: i32 = 86;
    /// Frame `death05`.
    pub const DEATH05: i32 = 87;
    /// Frame `death06`.
    pub const DEATH06: i32 = 88;
    /// Frame `death07`.
    pub const DEATH07: i32 = 89;
    /// Frame `death08`.
    pub const DEATH08: i32 = 90;
    /// Frame `death09`.
    pub const DEATH09: i32 = 91;
    /// Frame `death10`.
    pub const DEATH10: i32 = 92;
    /// Frame `death11`.
    pub const DEATH11: i32 = 93;
}

/// `shamblerMoves` move tables.
pub fn shambler_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "shambler_move_stand",
            0,
            16,
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
            ],
        ),
        monster_move(
            "shambler_move_walk",
            17,
            28,
            None,
            vec![
                monster_frame(MonsterAi::Walk, (10f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (12f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (3f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (13f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Walk, (9f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Walk,
                    (7f32) as f64,
                    vec![MonsterAction::name("shambler_maybe_idle")],
                    -1,
                ),
                monster_frame(MonsterAi::Walk, (5f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "shambler_move_run",
            29,
            34,
            None,
            vec![
                monster_frame(MonsterAi::Run, (20f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (24f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (20f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (20f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Run, (24f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Run,
                    (20f32) as f64,
                    vec![MonsterAction::name("shambler_maybe_idle")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "shambler_move_pain",
            77,
            82,
            Some("shambler_run"),
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
            "shambler_attack_magic",
            65,
            76,
            Some("shambler_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("shambler_windup")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("shambler_lightning_update")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("shambler_lightning_update")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("shambler_lightning_update")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("shambler_lightning_update")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("ShamblerSaveLoc")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("ShamblerCastLightning")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("ShamblerCastLightning")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("ShamblerCastLightning")],
                    -1,
                ),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "shambler_attack_smash",
            35,
            46,
            Some("shambler_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (2f32) as f64,
                    vec![MonsterAction::name("shambler_melee1")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (6f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (4f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (1f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (0f32) as f64,
                    vec![MonsterAction::name("sham_smash10")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (5f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (4f32) as f64, vec![], -1),
            ],
        ),
        monster_move(
            "shambler_attack_swingl",
            56,
            64,
            Some("shambler_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (5f32) as f64,
                    vec![MonsterAction::name("shambler_melee1")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (7f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (7f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (9f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (5f32) as f64,
                    vec![MonsterAction::name("ShamClaw")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (4f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (8f32) as f64,
                    vec![MonsterAction::name("sham_swingl9")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "shambler_attack_swingr",
            47,
            55,
            Some("shambler_run"),
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    (1f32) as f64,
                    vec![MonsterAction::name("shambler_melee2")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (8f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (14f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (7f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Charge, (6f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (6f32) as f64,
                    vec![MonsterAction::name("ShamClaw")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, (3f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    (8f32) as f64,
                    vec![MonsterAction::name("sham_swingr9")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "shambler_move_death",
            83,
            93,
            Some("shambler_dead"),
            vec![
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(MonsterAi::Move, (0f32) as f64, vec![], -1),
                monster_frame(
                    MonsterAi::Move,
                    (0f32) as f64,
                    vec![MonsterAction::name("shambler_shrink")],
                    -1,
                ),
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
    ]
}
