//! Rogue morph frames (src/content/q1/missionpacks/monsters/tables/morph.ts).
//!
//! quakec_rogue/morph.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation};

static OPS_MORPH_ATTACK01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK07: &[MonsterOperation] = &[MonsterOperation::Action { name: "morph_stab2" }];
static OPS_MORPH_ATTACK08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_ATTACK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK10: &[MonsterOperation] = &[MonsterOperation::Action { name: "morph_stab2" }];
static OPS_MORPH_BIGATTACK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_BIGATTACK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_DIE1: &[MonsterOperation] = &[];
static OPS_MORPH_DIE10: &[MonsterOperation] = &[];
static OPS_MORPH_DIE11: &[MonsterOperation] = &[];
static OPS_MORPH_DIE12: &[MonsterOperation] = &[];
static OPS_MORPH_DIE13: &[MonsterOperation] = &[];
static OPS_MORPH_DIE14: &[MonsterOperation] = &[];
static OPS_MORPH_DIE15: &[MonsterOperation] = &[];
static OPS_MORPH_DIE16: &[MonsterOperation] = &[];
static OPS_MORPH_DIE17: &[MonsterOperation] = &[];
static OPS_MORPH_DIE18: &[MonsterOperation] = &[];
static OPS_MORPH_DIE19: &[MonsterOperation] = &[];
static OPS_MORPH_DIE2: &[MonsterOperation] = &[];
static OPS_MORPH_DIE20: &[MonsterOperation] = &[];
static OPS_MORPH_DIE21: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "morph:morph_die21",
}];
static OPS_MORPH_DIE3: &[MonsterOperation] = &[];
static OPS_MORPH_DIE4: &[MonsterOperation] = &[];
static OPS_MORPH_DIE5: &[MonsterOperation] = &[];
static OPS_MORPH_DIE6: &[MonsterOperation] = &[];
static OPS_MORPH_DIE7: &[MonsterOperation] = &[];
static OPS_MORPH_DIE8: &[MonsterOperation] = &[];
static OPS_MORPH_DIE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "morph:morph_die9",
}];
static OPS_MORPH_FIRE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE4: &[MonsterOperation] = &[MonsterOperation::Action { name: "morph_fire" }];
static OPS_MORPH_FIRE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_FIRE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK09: &[MonsterOperation] = &[MonsterOperation::Action { name: "morph_smack" }];
static OPS_MORPH_KNOCKBACK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_KNOCKBACK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MORPH_PAINA1: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA10: &[MonsterOperation] = &[MonsterOperation::Action { name: "morph_teleport" }];
static OPS_MORPH_PAINA2: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA3: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA4: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA5: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA6: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA7: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA8: &[MonsterOperation] = &[];
static OPS_MORPH_PAINA9: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB1: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB2: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB3: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB4: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB5: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB6: &[MonsterOperation] = &[];
static OPS_MORPH_PAINB7: &[MonsterOperation] = &[MonsterOperation::Action { name: "morph_teleport" }];
static OPS_MORPH_RUN1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 7.0,
}];
static OPS_MORPH_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 15.0,
}];
static OPS_MORPH_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 11.0,
}];
static OPS_MORPH_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 11.0,
}];
static OPS_MORPH_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_MORPH_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_MORPH_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 11.0,
}];
static OPS_MORPH_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 7.0,
}];
static OPS_MORPH_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 11.0,
}];
static OPS_MORPH_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 15.0,
}];
static OPS_MORPH_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 19.0,
}];
static OPS_MORPH_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MORPH_WAKE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "morph:morph_wake1",
}];
static OPS_MORPH_WAKE10: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE11: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE12: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE13: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE14: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "morph:morph_wake15",
}];
static OPS_MORPH_WAKE16: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE17: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE18: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE2: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE20: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE21: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE22: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE23: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE24: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE25: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE26: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE27: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE28: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE29: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE3: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE30: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE31: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "morph:morph_wake31",
}];
static OPS_MORPH_WAKE4: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE5: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE6: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE7: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE8: &[MonsterOperation] = &[];
static OPS_MORPH_WAKE9: &[MonsterOperation] = &[];
static OPS_MORPH_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_MORPH_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_MORPH_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_MORPH_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_MORPH_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_MORPH_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_MORPH_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_MORPH_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_MORPH_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_MORPH_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_MORPH_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_MORPH_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_MORPH_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "morph_attack01",
        MonsterFrame {
            frame: 65,
            next: "morph_attack02",
            operations: OPS_MORPH_ATTACK01,
        },
    ),
    (
        "morph_attack02",
        MonsterFrame {
            frame: 66,
            next: "morph_attack03",
            operations: OPS_MORPH_ATTACK02,
        },
    ),
    (
        "morph_attack03",
        MonsterFrame {
            frame: 67,
            next: "morph_attack04",
            operations: OPS_MORPH_ATTACK03,
        },
    ),
    (
        "morph_attack04",
        MonsterFrame {
            frame: 68,
            next: "morph_attack05",
            operations: OPS_MORPH_ATTACK04,
        },
    ),
    (
        "morph_attack05",
        MonsterFrame {
            frame: 69,
            next: "morph_attack06",
            operations: OPS_MORPH_ATTACK05,
        },
    ),
    (
        "morph_attack06",
        MonsterFrame {
            frame: 70,
            next: "morph_attack07",
            operations: OPS_MORPH_ATTACK06,
        },
    ),
    (
        "morph_attack07",
        MonsterFrame {
            frame: 71,
            next: "morph_attack08",
            operations: OPS_MORPH_ATTACK07,
        },
    ),
    (
        "morph_attack08",
        MonsterFrame {
            frame: 72,
            next: "morph_attack09",
            operations: OPS_MORPH_ATTACK08,
        },
    ),
    (
        "morph_attack09",
        MonsterFrame {
            frame: 73,
            next: "morph_attack10",
            operations: OPS_MORPH_ATTACK09,
        },
    ),
    (
        "morph_attack10",
        MonsterFrame {
            frame: 74,
            next: "morph_attack11",
            operations: OPS_MORPH_ATTACK10,
        },
    ),
    (
        "morph_attack11",
        MonsterFrame {
            frame: 75,
            next: "morph_attack12",
            operations: OPS_MORPH_ATTACK11,
        },
    ),
    (
        "morph_attack12",
        MonsterFrame {
            frame: 65,
            next: "morph_run1",
            operations: OPS_MORPH_ATTACK12,
        },
    ),
    (
        "morph_bigattack01",
        MonsterFrame {
            frame: 76,
            next: "morph_bigattack02",
            operations: OPS_MORPH_BIGATTACK01,
        },
    ),
    (
        "morph_bigattack02",
        MonsterFrame {
            frame: 77,
            next: "morph_bigattack03",
            operations: OPS_MORPH_BIGATTACK02,
        },
    ),
    (
        "morph_bigattack03",
        MonsterFrame {
            frame: 78,
            next: "morph_bigattack04",
            operations: OPS_MORPH_BIGATTACK03,
        },
    ),
    (
        "morph_bigattack04",
        MonsterFrame {
            frame: 79,
            next: "morph_bigattack05",
            operations: OPS_MORPH_BIGATTACK04,
        },
    ),
    (
        "morph_bigattack05",
        MonsterFrame {
            frame: 80,
            next: "morph_bigattack06",
            operations: OPS_MORPH_BIGATTACK05,
        },
    ),
    (
        "morph_bigattack06",
        MonsterFrame {
            frame: 81,
            next: "morph_bigattack07",
            operations: OPS_MORPH_BIGATTACK06,
        },
    ),
    (
        "morph_bigattack07",
        MonsterFrame {
            frame: 82,
            next: "morph_bigattack08",
            operations: OPS_MORPH_BIGATTACK07,
        },
    ),
    (
        "morph_bigattack08",
        MonsterFrame {
            frame: 83,
            next: "morph_bigattack09",
            operations: OPS_MORPH_BIGATTACK08,
        },
    ),
    (
        "morph_bigattack09",
        MonsterFrame {
            frame: 84,
            next: "morph_bigattack10",
            operations: OPS_MORPH_BIGATTACK09,
        },
    ),
    (
        "morph_bigattack10",
        MonsterFrame {
            frame: 85,
            next: "morph_bigattack11",
            operations: OPS_MORPH_BIGATTACK10,
        },
    ),
    (
        "morph_bigattack11",
        MonsterFrame {
            frame: 86,
            next: "morph_bigattack12",
            operations: OPS_MORPH_BIGATTACK11,
        },
    ),
    (
        "morph_bigattack12",
        MonsterFrame {
            frame: 87,
            next: "morph_bigattack13",
            operations: OPS_MORPH_BIGATTACK12,
        },
    ),
    (
        "morph_bigattack13",
        MonsterFrame {
            frame: 88,
            next: "morph_bigattack14",
            operations: OPS_MORPH_BIGATTACK13,
        },
    ),
    (
        "morph_bigattack14",
        MonsterFrame {
            frame: 89,
            next: "morph_bigattack15",
            operations: OPS_MORPH_BIGATTACK14,
        },
    ),
    (
        "morph_bigattack15",
        MonsterFrame {
            frame: 90,
            next: "morph_bigattack16",
            operations: OPS_MORPH_BIGATTACK15,
        },
    ),
    (
        "morph_bigattack16",
        MonsterFrame {
            frame: 91,
            next: "morph_bigattack17",
            operations: OPS_MORPH_BIGATTACK16,
        },
    ),
    (
        "morph_bigattack17",
        MonsterFrame {
            frame: 76,
            next: "morph_run1",
            operations: OPS_MORPH_BIGATTACK17,
        },
    ),
    (
        "morph_die1",
        MonsterFrame {
            frame: 121,
            next: "morph_die2",
            operations: OPS_MORPH_DIE1,
        },
    ),
    (
        "morph_die10",
        MonsterFrame {
            frame: 130,
            next: "morph_die11",
            operations: OPS_MORPH_DIE10,
        },
    ),
    (
        "morph_die11",
        MonsterFrame {
            frame: 131,
            next: "morph_die12",
            operations: OPS_MORPH_DIE11,
        },
    ),
    (
        "morph_die12",
        MonsterFrame {
            frame: 132,
            next: "morph_die13",
            operations: OPS_MORPH_DIE12,
        },
    ),
    (
        "morph_die13",
        MonsterFrame {
            frame: 133,
            next: "morph_die14",
            operations: OPS_MORPH_DIE13,
        },
    ),
    (
        "morph_die14",
        MonsterFrame {
            frame: 134,
            next: "morph_die15",
            operations: OPS_MORPH_DIE14,
        },
    ),
    (
        "morph_die15",
        MonsterFrame {
            frame: 135,
            next: "morph_die16",
            operations: OPS_MORPH_DIE15,
        },
    ),
    (
        "morph_die16",
        MonsterFrame {
            frame: 136,
            next: "morph_die17",
            operations: OPS_MORPH_DIE16,
        },
    ),
    (
        "morph_die17",
        MonsterFrame {
            frame: 137,
            next: "morph_die18",
            operations: OPS_MORPH_DIE17,
        },
    ),
    (
        "morph_die18",
        MonsterFrame {
            frame: 138,
            next: "morph_die19",
            operations: OPS_MORPH_DIE18,
        },
    ),
    (
        "morph_die19",
        MonsterFrame {
            frame: 139,
            next: "morph_die20",
            operations: OPS_MORPH_DIE19,
        },
    ),
    (
        "morph_die2",
        MonsterFrame {
            frame: 122,
            next: "morph_die3",
            operations: OPS_MORPH_DIE2,
        },
    ),
    (
        "morph_die20",
        MonsterFrame {
            frame: 140,
            next: "morph_die21",
            operations: OPS_MORPH_DIE20,
        },
    ),
    (
        "morph_die21",
        MonsterFrame {
            frame: 141,
            next: "morph_die21",
            operations: OPS_MORPH_DIE21,
        },
    ),
    (
        "morph_die3",
        MonsterFrame {
            frame: 123,
            next: "morph_die4",
            operations: OPS_MORPH_DIE3,
        },
    ),
    (
        "morph_die4",
        MonsterFrame {
            frame: 124,
            next: "morph_die5",
            operations: OPS_MORPH_DIE4,
        },
    ),
    (
        "morph_die5",
        MonsterFrame {
            frame: 125,
            next: "morph_die6",
            operations: OPS_MORPH_DIE5,
        },
    ),
    (
        "morph_die6",
        MonsterFrame {
            frame: 126,
            next: "morph_die7",
            operations: OPS_MORPH_DIE6,
        },
    ),
    (
        "morph_die7",
        MonsterFrame {
            frame: 127,
            next: "morph_die8",
            operations: OPS_MORPH_DIE7,
        },
    ),
    (
        "morph_die8",
        MonsterFrame {
            frame: 128,
            next: "morph_die9",
            operations: OPS_MORPH_DIE8,
        },
    ),
    (
        "morph_die9",
        MonsterFrame {
            frame: 129,
            next: "morph_die10",
            operations: OPS_MORPH_DIE9,
        },
    ),
    (
        "morph_fire1",
        MonsterFrame {
            frame: 56,
            next: "morph_fire2",
            operations: OPS_MORPH_FIRE1,
        },
    ),
    (
        "morph_fire2",
        MonsterFrame {
            frame: 57,
            next: "morph_fire3",
            operations: OPS_MORPH_FIRE2,
        },
    ),
    (
        "morph_fire3",
        MonsterFrame {
            frame: 58,
            next: "morph_fire4",
            operations: OPS_MORPH_FIRE3,
        },
    ),
    (
        "morph_fire4",
        MonsterFrame {
            frame: 59,
            next: "morph_fire5",
            operations: OPS_MORPH_FIRE4,
        },
    ),
    (
        "morph_fire5",
        MonsterFrame {
            frame: 60,
            next: "morph_fire6",
            operations: OPS_MORPH_FIRE5,
        },
    ),
    (
        "morph_fire6",
        MonsterFrame {
            frame: 61,
            next: "morph_fire7",
            operations: OPS_MORPH_FIRE6,
        },
    ),
    (
        "morph_fire7",
        MonsterFrame {
            frame: 62,
            next: "morph_fire8",
            operations: OPS_MORPH_FIRE7,
        },
    ),
    (
        "morph_fire8",
        MonsterFrame {
            frame: 63,
            next: "morph_fire9",
            operations: OPS_MORPH_FIRE8,
        },
    ),
    (
        "morph_fire9",
        MonsterFrame {
            frame: 64,
            next: "morph_run1",
            operations: OPS_MORPH_FIRE9,
        },
    ),
    (
        "morph_knockback01",
        MonsterFrame {
            frame: 92,
            next: "morph_knockback02",
            operations: OPS_MORPH_KNOCKBACK01,
        },
    ),
    (
        "morph_knockback02",
        MonsterFrame {
            frame: 93,
            next: "morph_knockback03",
            operations: OPS_MORPH_KNOCKBACK02,
        },
    ),
    (
        "morph_knockback03",
        MonsterFrame {
            frame: 94,
            next: "morph_knockback04",
            operations: OPS_MORPH_KNOCKBACK03,
        },
    ),
    (
        "morph_knockback04",
        MonsterFrame {
            frame: 95,
            next: "morph_knockback05",
            operations: OPS_MORPH_KNOCKBACK04,
        },
    ),
    (
        "morph_knockback05",
        MonsterFrame {
            frame: 96,
            next: "morph_knockback06",
            operations: OPS_MORPH_KNOCKBACK05,
        },
    ),
    (
        "morph_knockback06",
        MonsterFrame {
            frame: 97,
            next: "morph_knockback07",
            operations: OPS_MORPH_KNOCKBACK06,
        },
    ),
    (
        "morph_knockback07",
        MonsterFrame {
            frame: 98,
            next: "morph_knockback08",
            operations: OPS_MORPH_KNOCKBACK07,
        },
    ),
    (
        "morph_knockback08",
        MonsterFrame {
            frame: 99,
            next: "morph_knockback09",
            operations: OPS_MORPH_KNOCKBACK08,
        },
    ),
    (
        "morph_knockback09",
        MonsterFrame {
            frame: 100,
            next: "morph_knockback10",
            operations: OPS_MORPH_KNOCKBACK09,
        },
    ),
    (
        "morph_knockback10",
        MonsterFrame {
            frame: 101,
            next: "morph_knockback11",
            operations: OPS_MORPH_KNOCKBACK10,
        },
    ),
    (
        "morph_knockback11",
        MonsterFrame {
            frame: 102,
            next: "morph_knockback12",
            operations: OPS_MORPH_KNOCKBACK11,
        },
    ),
    (
        "morph_knockback12",
        MonsterFrame {
            frame: 103,
            next: "morph_run1",
            operations: OPS_MORPH_KNOCKBACK12,
        },
    ),
    (
        "morph_painA1",
        MonsterFrame {
            frame: 104,
            next: "morph_painA2",
            operations: OPS_MORPH_PAINA1,
        },
    ),
    (
        "morph_painA10",
        MonsterFrame {
            frame: 113,
            next: "morph_run1",
            operations: OPS_MORPH_PAINA10,
        },
    ),
    (
        "morph_painA2",
        MonsterFrame {
            frame: 105,
            next: "morph_painA3",
            operations: OPS_MORPH_PAINA2,
        },
    ),
    (
        "morph_painA3",
        MonsterFrame {
            frame: 106,
            next: "morph_painA4",
            operations: OPS_MORPH_PAINA3,
        },
    ),
    (
        "morph_painA4",
        MonsterFrame {
            frame: 107,
            next: "morph_painA5",
            operations: OPS_MORPH_PAINA4,
        },
    ),
    (
        "morph_painA5",
        MonsterFrame {
            frame: 108,
            next: "morph_painA6",
            operations: OPS_MORPH_PAINA5,
        },
    ),
    (
        "morph_painA6",
        MonsterFrame {
            frame: 109,
            next: "morph_painA7",
            operations: OPS_MORPH_PAINA6,
        },
    ),
    (
        "morph_painA7",
        MonsterFrame {
            frame: 110,
            next: "morph_painA8",
            operations: OPS_MORPH_PAINA7,
        },
    ),
    (
        "morph_painA8",
        MonsterFrame {
            frame: 111,
            next: "morph_painA9",
            operations: OPS_MORPH_PAINA8,
        },
    ),
    (
        "morph_painA9",
        MonsterFrame {
            frame: 112,
            next: "morph_painA10",
            operations: OPS_MORPH_PAINA9,
        },
    ),
    (
        "morph_painB1",
        MonsterFrame {
            frame: 114,
            next: "morph_painB2",
            operations: OPS_MORPH_PAINB1,
        },
    ),
    (
        "morph_painB2",
        MonsterFrame {
            frame: 115,
            next: "morph_painB3",
            operations: OPS_MORPH_PAINB2,
        },
    ),
    (
        "morph_painB3",
        MonsterFrame {
            frame: 116,
            next: "morph_painB4",
            operations: OPS_MORPH_PAINB3,
        },
    ),
    (
        "morph_painB4",
        MonsterFrame {
            frame: 117,
            next: "morph_painB5",
            operations: OPS_MORPH_PAINB4,
        },
    ),
    (
        "morph_painB5",
        MonsterFrame {
            frame: 118,
            next: "morph_painB6",
            operations: OPS_MORPH_PAINB5,
        },
    ),
    (
        "morph_painB6",
        MonsterFrame {
            frame: 119,
            next: "morph_painB7",
            operations: OPS_MORPH_PAINB6,
        },
    ),
    (
        "morph_painB7",
        MonsterFrame {
            frame: 120,
            next: "morph_run1",
            operations: OPS_MORPH_PAINB7,
        },
    ),
    (
        "morph_run1",
        MonsterFrame {
            frame: 32,
            next: "morph_run2",
            operations: OPS_MORPH_RUN1,
        },
    ),
    (
        "morph_run10",
        MonsterFrame {
            frame: 41,
            next: "morph_run11",
            operations: OPS_MORPH_RUN10,
        },
    ),
    (
        "morph_run11",
        MonsterFrame {
            frame: 42,
            next: "morph_run1",
            operations: OPS_MORPH_RUN11,
        },
    ),
    (
        "morph_run2",
        MonsterFrame {
            frame: 33,
            next: "morph_run3",
            operations: OPS_MORPH_RUN2,
        },
    ),
    (
        "morph_run3",
        MonsterFrame {
            frame: 34,
            next: "morph_run4",
            operations: OPS_MORPH_RUN3,
        },
    ),
    (
        "morph_run4",
        MonsterFrame {
            frame: 35,
            next: "morph_run5",
            operations: OPS_MORPH_RUN4,
        },
    ),
    (
        "morph_run5",
        MonsterFrame {
            frame: 36,
            next: "morph_run6",
            operations: OPS_MORPH_RUN5,
        },
    ),
    (
        "morph_run6",
        MonsterFrame {
            frame: 37,
            next: "morph_run7",
            operations: OPS_MORPH_RUN6,
        },
    ),
    (
        "morph_run7",
        MonsterFrame {
            frame: 38,
            next: "morph_run8",
            operations: OPS_MORPH_RUN7,
        },
    ),
    (
        "morph_run8",
        MonsterFrame {
            frame: 39,
            next: "morph_run9",
            operations: OPS_MORPH_RUN8,
        },
    ),
    (
        "morph_run9",
        MonsterFrame {
            frame: 40,
            next: "morph_run10",
            operations: OPS_MORPH_RUN9,
        },
    ),
    (
        "morph_stand1",
        MonsterFrame {
            frame: 0,
            next: "morph_stand1",
            operations: OPS_MORPH_STAND1,
        },
    ),
    (
        "morph_wake1",
        MonsterFrame {
            frame: 1,
            next: "morph_wake2",
            operations: OPS_MORPH_WAKE1,
        },
    ),
    (
        "morph_wake10",
        MonsterFrame {
            frame: 10,
            next: "morph_wake11",
            operations: OPS_MORPH_WAKE10,
        },
    ),
    (
        "morph_wake11",
        MonsterFrame {
            frame: 11,
            next: "morph_wake12",
            operations: OPS_MORPH_WAKE11,
        },
    ),
    (
        "morph_wake12",
        MonsterFrame {
            frame: 12,
            next: "morph_wake13",
            operations: OPS_MORPH_WAKE12,
        },
    ),
    (
        "morph_wake13",
        MonsterFrame {
            frame: 13,
            next: "morph_wake14",
            operations: OPS_MORPH_WAKE13,
        },
    ),
    (
        "morph_wake14",
        MonsterFrame {
            frame: 14,
            next: "morph_wake15",
            operations: OPS_MORPH_WAKE14,
        },
    ),
    (
        "morph_wake15",
        MonsterFrame {
            frame: 15,
            next: "morph_wake16",
            operations: OPS_MORPH_WAKE15,
        },
    ),
    (
        "morph_wake16",
        MonsterFrame {
            frame: 16,
            next: "morph_wake17",
            operations: OPS_MORPH_WAKE16,
        },
    ),
    (
        "morph_wake17",
        MonsterFrame {
            frame: 17,
            next: "morph_wake18",
            operations: OPS_MORPH_WAKE17,
        },
    ),
    (
        "morph_wake18",
        MonsterFrame {
            frame: 18,
            next: "morph_wake20",
            operations: OPS_MORPH_WAKE18,
        },
    ),
    (
        "morph_wake2",
        MonsterFrame {
            frame: 2,
            next: "morph_wake3",
            operations: OPS_MORPH_WAKE2,
        },
    ),
    (
        "morph_wake20",
        MonsterFrame {
            frame: 20,
            next: "morph_wake21",
            operations: OPS_MORPH_WAKE20,
        },
    ),
    (
        "morph_wake21",
        MonsterFrame {
            frame: 21,
            next: "morph_wake22",
            operations: OPS_MORPH_WAKE21,
        },
    ),
    (
        "morph_wake22",
        MonsterFrame {
            frame: 22,
            next: "morph_wake23",
            operations: OPS_MORPH_WAKE22,
        },
    ),
    (
        "morph_wake23",
        MonsterFrame {
            frame: 23,
            next: "morph_wake24",
            operations: OPS_MORPH_WAKE23,
        },
    ),
    (
        "morph_wake24",
        MonsterFrame {
            frame: 24,
            next: "morph_wake25",
            operations: OPS_MORPH_WAKE24,
        },
    ),
    (
        "morph_wake25",
        MonsterFrame {
            frame: 25,
            next: "morph_wake26",
            operations: OPS_MORPH_WAKE25,
        },
    ),
    (
        "morph_wake26",
        MonsterFrame {
            frame: 26,
            next: "morph_wake27",
            operations: OPS_MORPH_WAKE26,
        },
    ),
    (
        "morph_wake27",
        MonsterFrame {
            frame: 27,
            next: "morph_wake28",
            operations: OPS_MORPH_WAKE27,
        },
    ),
    (
        "morph_wake28",
        MonsterFrame {
            frame: 28,
            next: "morph_wake29",
            operations: OPS_MORPH_WAKE28,
        },
    ),
    (
        "morph_wake29",
        MonsterFrame {
            frame: 29,
            next: "morph_wake30",
            operations: OPS_MORPH_WAKE29,
        },
    ),
    (
        "morph_wake3",
        MonsterFrame {
            frame: 3,
            next: "morph_wake4",
            operations: OPS_MORPH_WAKE3,
        },
    ),
    (
        "morph_wake30",
        MonsterFrame {
            frame: 30,
            next: "morph_wake31",
            operations: OPS_MORPH_WAKE30,
        },
    ),
    (
        "morph_wake31",
        MonsterFrame {
            frame: 31,
            next: "morph_stand1",
            operations: OPS_MORPH_WAKE31,
        },
    ),
    (
        "morph_wake4",
        MonsterFrame {
            frame: 4,
            next: "morph_wake5",
            operations: OPS_MORPH_WAKE4,
        },
    ),
    (
        "morph_wake5",
        MonsterFrame {
            frame: 5,
            next: "morph_wake6",
            operations: OPS_MORPH_WAKE5,
        },
    ),
    (
        "morph_wake6",
        MonsterFrame {
            frame: 6,
            next: "morph_wake7",
            operations: OPS_MORPH_WAKE6,
        },
    ),
    (
        "morph_wake7",
        MonsterFrame {
            frame: 7,
            next: "morph_wake8",
            operations: OPS_MORPH_WAKE7,
        },
    ),
    (
        "morph_wake8",
        MonsterFrame {
            frame: 8,
            next: "morph_wake9",
            operations: OPS_MORPH_WAKE8,
        },
    ),
    (
        "morph_wake9",
        MonsterFrame {
            frame: 9,
            next: "morph_wake10",
            operations: OPS_MORPH_WAKE9,
        },
    ),
    (
        "morph_walk1",
        MonsterFrame {
            frame: 43,
            next: "morph_walk2",
            operations: OPS_MORPH_WALK1,
        },
    ),
    (
        "morph_walk10",
        MonsterFrame {
            frame: 52,
            next: "morph_walk11",
            operations: OPS_MORPH_WALK10,
        },
    ),
    (
        "morph_walk11",
        MonsterFrame {
            frame: 53,
            next: "morph_walk12",
            operations: OPS_MORPH_WALK11,
        },
    ),
    (
        "morph_walk12",
        MonsterFrame {
            frame: 54,
            next: "morph_walk13",
            operations: OPS_MORPH_WALK12,
        },
    ),
    (
        "morph_walk13",
        MonsterFrame {
            frame: 55,
            next: "morph_walk1",
            operations: OPS_MORPH_WALK13,
        },
    ),
    (
        "morph_walk2",
        MonsterFrame {
            frame: 44,
            next: "morph_walk3",
            operations: OPS_MORPH_WALK2,
        },
    ),
    (
        "morph_walk3",
        MonsterFrame {
            frame: 45,
            next: "morph_walk4",
            operations: OPS_MORPH_WALK3,
        },
    ),
    (
        "morph_walk4",
        MonsterFrame {
            frame: 46,
            next: "morph_walk5",
            operations: OPS_MORPH_WALK4,
        },
    ),
    (
        "morph_walk5",
        MonsterFrame {
            frame: 47,
            next: "morph_walk6",
            operations: OPS_MORPH_WALK5,
        },
    ),
    (
        "morph_walk6",
        MonsterFrame {
            frame: 48,
            next: "morph_walk7",
            operations: OPS_MORPH_WALK6,
        },
    ),
    (
        "morph_walk7",
        MonsterFrame {
            frame: 49,
            next: "morph_walk8",
            operations: OPS_MORPH_WALK7,
        },
    ),
    (
        "morph_walk8",
        MonsterFrame {
            frame: 50,
            next: "morph_walk9",
            operations: OPS_MORPH_WALK8,
        },
    ),
    (
        "morph_walk9",
        MonsterFrame {
            frame: 51,
            next: "morph_walk10",
            operations: OPS_MORPH_WALK9,
        },
    ),
];

/// Look up a morph frame by name.
#[must_use]
pub fn frame(name: &str) -> Option<&'static MonsterFrame> {
    FRAMES
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| &FRAMES[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_sorted_with_expected_count() {
        assert_eq!(FRAMES.len(), 143);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("morph_attack01").expect("first frame");
        assert_eq!((head.frame, head.next), (65, "morph_attack02"));
        let tail = frame("morph_walk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (51, "morph_walk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
