//! Rogue s_wrath frames (src/content/q1/missionpacks/monsters/tables/s_wrath.ts).
//!
//! quakec_rogue/s_wrath.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation};

static OPS_OVERLORD_AT_A01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A07: &[MonsterOperation] = &[MonsterOperation::Action { name: "overlord_smash" }];
static OPS_OVERLORD_AT_A08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_A10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B08: &[MonsterOperation] = &[MonsterOperation::Action { name: "overlord_smash" }];
static OPS_OVERLORD_AT_B09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_B14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C06: &[MonsterOperation] = &[MonsterOperation::Action { name: "overlord_smash" }];
static OPS_OVERLORD_AT_C07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C11: &[MonsterOperation] = &[MonsterOperation::Action { name: "overlord_smash" }];
static OPS_OVERLORD_AT_C12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_AT_C14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_DIE01: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE02: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die02",
}];
static OPS_OVERLORD_DIE03: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE04: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE05: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE06: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE07: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE08: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE09: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die01",
}];
static OPS_OVERLORD_DIE17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die17",
}];
static OPS_OVERLORD_DIE18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die18",
}];
static OPS_OVERLORD_DIE19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "s_wrath:overlord_die19",
}];
static OPS_OVERLORD_MSL_A01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A06: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "WrathMissile(4)",
}];
static OPS_OVERLORD_MSL_A07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OVERLORD_MSL_A12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "overlord_teleport",
}];
static OPS_OVERLORD_PN_A01: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_A02: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_A03: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "overlord_teleport",
}];
static OPS_OVERLORD_PN_A04: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_A05: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_A06: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_A07: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_B01: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_B02: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_B03: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "overlord_teleport",
}];
static OPS_OVERLORD_PN_B04: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_B05: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_B06: &[MonsterOperation] = &[];
static OPS_OVERLORD_PN_B07: &[MonsterOperation] = &[];
static OPS_OVERLORD_RUN01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_RUN15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OVERLORD_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OVERLORD_WALK01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_OVERLORD_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "overlord_at_a01",
        MonsterFrame {
            frame: 16,
            next: "overlord_at_a02",
            operations: OPS_OVERLORD_AT_A01,
        },
    ),
    (
        "overlord_at_a02",
        MonsterFrame {
            frame: 17,
            next: "overlord_at_a03",
            operations: OPS_OVERLORD_AT_A02,
        },
    ),
    (
        "overlord_at_a03",
        MonsterFrame {
            frame: 18,
            next: "overlord_at_a04",
            operations: OPS_OVERLORD_AT_A03,
        },
    ),
    (
        "overlord_at_a04",
        MonsterFrame {
            frame: 19,
            next: "overlord_at_a05",
            operations: OPS_OVERLORD_AT_A04,
        },
    ),
    (
        "overlord_at_a05",
        MonsterFrame {
            frame: 20,
            next: "overlord_at_a06",
            operations: OPS_OVERLORD_AT_A05,
        },
    ),
    (
        "overlord_at_a06",
        MonsterFrame {
            frame: 21,
            next: "overlord_at_a07",
            operations: OPS_OVERLORD_AT_A06,
        },
    ),
    (
        "overlord_at_a07",
        MonsterFrame {
            frame: 22,
            next: "overlord_at_a08",
            operations: OPS_OVERLORD_AT_A07,
        },
    ),
    (
        "overlord_at_a08",
        MonsterFrame {
            frame: 23,
            next: "overlord_at_a09",
            operations: OPS_OVERLORD_AT_A08,
        },
    ),
    (
        "overlord_at_a09",
        MonsterFrame {
            frame: 24,
            next: "overlord_at_a10",
            operations: OPS_OVERLORD_AT_A09,
        },
    ),
    (
        "overlord_at_a10",
        MonsterFrame {
            frame: 25,
            next: "overlord_run01",
            operations: OPS_OVERLORD_AT_A10,
        },
    ),
    (
        "overlord_at_b01",
        MonsterFrame {
            frame: 26,
            next: "overlord_at_b02",
            operations: OPS_OVERLORD_AT_B01,
        },
    ),
    (
        "overlord_at_b02",
        MonsterFrame {
            frame: 27,
            next: "overlord_at_b03",
            operations: OPS_OVERLORD_AT_B02,
        },
    ),
    (
        "overlord_at_b03",
        MonsterFrame {
            frame: 28,
            next: "overlord_at_b04",
            operations: OPS_OVERLORD_AT_B03,
        },
    ),
    (
        "overlord_at_b04",
        MonsterFrame {
            frame: 29,
            next: "overlord_at_b05",
            operations: OPS_OVERLORD_AT_B04,
        },
    ),
    (
        "overlord_at_b05",
        MonsterFrame {
            frame: 30,
            next: "overlord_at_b06",
            operations: OPS_OVERLORD_AT_B05,
        },
    ),
    (
        "overlord_at_b06",
        MonsterFrame {
            frame: 31,
            next: "overlord_at_b07",
            operations: OPS_OVERLORD_AT_B06,
        },
    ),
    (
        "overlord_at_b07",
        MonsterFrame {
            frame: 32,
            next: "overlord_at_b08",
            operations: OPS_OVERLORD_AT_B07,
        },
    ),
    (
        "overlord_at_b08",
        MonsterFrame {
            frame: 33,
            next: "overlord_at_b09",
            operations: OPS_OVERLORD_AT_B08,
        },
    ),
    (
        "overlord_at_b09",
        MonsterFrame {
            frame: 34,
            next: "overlord_at_b10",
            operations: OPS_OVERLORD_AT_B09,
        },
    ),
    (
        "overlord_at_b10",
        MonsterFrame {
            frame: 35,
            next: "overlord_at_b11",
            operations: OPS_OVERLORD_AT_B10,
        },
    ),
    (
        "overlord_at_b11",
        MonsterFrame {
            frame: 36,
            next: "overlord_at_b12",
            operations: OPS_OVERLORD_AT_B11,
        },
    ),
    (
        "overlord_at_b12",
        MonsterFrame {
            frame: 37,
            next: "overlord_at_b13",
            operations: OPS_OVERLORD_AT_B12,
        },
    ),
    (
        "overlord_at_b13",
        MonsterFrame {
            frame: 38,
            next: "overlord_at_b14",
            operations: OPS_OVERLORD_AT_B13,
        },
    ),
    (
        "overlord_at_b14",
        MonsterFrame {
            frame: 39,
            next: "overlord_run01",
            operations: OPS_OVERLORD_AT_B14,
        },
    ),
    (
        "overlord_at_c01",
        MonsterFrame {
            frame: 40,
            next: "overlord_at_c02",
            operations: OPS_OVERLORD_AT_C01,
        },
    ),
    (
        "overlord_at_c02",
        MonsterFrame {
            frame: 41,
            next: "overlord_at_c03",
            operations: OPS_OVERLORD_AT_C02,
        },
    ),
    (
        "overlord_at_c03",
        MonsterFrame {
            frame: 42,
            next: "overlord_at_c04",
            operations: OPS_OVERLORD_AT_C03,
        },
    ),
    (
        "overlord_at_c04",
        MonsterFrame {
            frame: 43,
            next: "overlord_at_c05",
            operations: OPS_OVERLORD_AT_C04,
        },
    ),
    (
        "overlord_at_c05",
        MonsterFrame {
            frame: 44,
            next: "overlord_at_c06",
            operations: OPS_OVERLORD_AT_C05,
        },
    ),
    (
        "overlord_at_c06",
        MonsterFrame {
            frame: 45,
            next: "overlord_at_c07",
            operations: OPS_OVERLORD_AT_C06,
        },
    ),
    (
        "overlord_at_c07",
        MonsterFrame {
            frame: 46,
            next: "overlord_at_c08",
            operations: OPS_OVERLORD_AT_C07,
        },
    ),
    (
        "overlord_at_c08",
        MonsterFrame {
            frame: 47,
            next: "overlord_at_c09",
            operations: OPS_OVERLORD_AT_C08,
        },
    ),
    (
        "overlord_at_c09",
        MonsterFrame {
            frame: 48,
            next: "overlord_at_c10",
            operations: OPS_OVERLORD_AT_C09,
        },
    ),
    (
        "overlord_at_c10",
        MonsterFrame {
            frame: 49,
            next: "overlord_at_c11",
            operations: OPS_OVERLORD_AT_C10,
        },
    ),
    (
        "overlord_at_c11",
        MonsterFrame {
            frame: 50,
            next: "overlord_at_c12",
            operations: OPS_OVERLORD_AT_C11,
        },
    ),
    (
        "overlord_at_c12",
        MonsterFrame {
            frame: 51,
            next: "overlord_at_c13",
            operations: OPS_OVERLORD_AT_C12,
        },
    ),
    (
        "overlord_at_c13",
        MonsterFrame {
            frame: 52,
            next: "overlord_at_c14",
            operations: OPS_OVERLORD_AT_C13,
        },
    ),
    (
        "overlord_at_c14",
        MonsterFrame {
            frame: 53,
            next: "overlord_run01",
            operations: OPS_OVERLORD_AT_C14,
        },
    ),
    (
        "overlord_die01",
        MonsterFrame {
            frame: 91,
            next: "overlord_die02",
            operations: OPS_OVERLORD_DIE01,
        },
    ),
    (
        "overlord_die02",
        MonsterFrame {
            frame: 92,
            next: "overlord_die03",
            operations: OPS_OVERLORD_DIE02,
        },
    ),
    (
        "overlord_die03",
        MonsterFrame {
            frame: 93,
            next: "overlord_die04",
            operations: OPS_OVERLORD_DIE03,
        },
    ),
    (
        "overlord_die04",
        MonsterFrame {
            frame: 94,
            next: "overlord_die05",
            operations: OPS_OVERLORD_DIE04,
        },
    ),
    (
        "overlord_die05",
        MonsterFrame {
            frame: 95,
            next: "overlord_die06",
            operations: OPS_OVERLORD_DIE05,
        },
    ),
    (
        "overlord_die06",
        MonsterFrame {
            frame: 96,
            next: "overlord_die07",
            operations: OPS_OVERLORD_DIE06,
        },
    ),
    (
        "overlord_die07",
        MonsterFrame {
            frame: 97,
            next: "overlord_die08",
            operations: OPS_OVERLORD_DIE07,
        },
    ),
    (
        "overlord_die08",
        MonsterFrame {
            frame: 98,
            next: "overlord_die09",
            operations: OPS_OVERLORD_DIE08,
        },
    ),
    (
        "overlord_die09",
        MonsterFrame {
            frame: 99,
            next: "overlord_die10",
            operations: OPS_OVERLORD_DIE09,
        },
    ),
    (
        "overlord_die10",
        MonsterFrame {
            frame: 99,
            next: "overlord_die11",
            operations: OPS_OVERLORD_DIE10,
        },
    ),
    (
        "overlord_die11",
        MonsterFrame {
            frame: 100,
            next: "overlord_die12",
            operations: OPS_OVERLORD_DIE11,
        },
    ),
    (
        "overlord_die12",
        MonsterFrame {
            frame: 101,
            next: "overlord_die13",
            operations: OPS_OVERLORD_DIE12,
        },
    ),
    (
        "overlord_die13",
        MonsterFrame {
            frame: 102,
            next: "overlord_die14",
            operations: OPS_OVERLORD_DIE13,
        },
    ),
    (
        "overlord_die14",
        MonsterFrame {
            frame: 103,
            next: "overlord_die15",
            operations: OPS_OVERLORD_DIE14,
        },
    ),
    (
        "overlord_die15",
        MonsterFrame {
            frame: 104,
            next: "overlord_die16",
            operations: OPS_OVERLORD_DIE15,
        },
    ),
    (
        "overlord_die16",
        MonsterFrame {
            frame: 105,
            next: "overlord_die17",
            operations: OPS_OVERLORD_DIE16,
        },
    ),
    (
        "overlord_die17",
        MonsterFrame {
            frame: 106,
            next: "overlord_die18",
            operations: OPS_OVERLORD_DIE17,
        },
    ),
    (
        "overlord_die18",
        MonsterFrame {
            frame: 107,
            next: "overlord_die19",
            operations: OPS_OVERLORD_DIE18,
        },
    ),
    (
        "overlord_die19",
        MonsterFrame {
            frame: 107,
            next: "overlord_die19",
            operations: OPS_OVERLORD_DIE19,
        },
    ),
    (
        "overlord_msl_a01",
        MonsterFrame {
            frame: 54,
            next: "overlord_msl_a02",
            operations: OPS_OVERLORD_MSL_A01,
        },
    ),
    (
        "overlord_msl_a02",
        MonsterFrame {
            frame: 55,
            next: "overlord_msl_a03",
            operations: OPS_OVERLORD_MSL_A02,
        },
    ),
    (
        "overlord_msl_a03",
        MonsterFrame {
            frame: 56,
            next: "overlord_msl_a04",
            operations: OPS_OVERLORD_MSL_A03,
        },
    ),
    (
        "overlord_msl_a04",
        MonsterFrame {
            frame: 57,
            next: "overlord_msl_a05",
            operations: OPS_OVERLORD_MSL_A04,
        },
    ),
    (
        "overlord_msl_a05",
        MonsterFrame {
            frame: 58,
            next: "overlord_msl_a06",
            operations: OPS_OVERLORD_MSL_A05,
        },
    ),
    (
        "overlord_msl_a06",
        MonsterFrame {
            frame: 59,
            next: "overlord_msl_a07",
            operations: OPS_OVERLORD_MSL_A06,
        },
    ),
    (
        "overlord_msl_a07",
        MonsterFrame {
            frame: 60,
            next: "overlord_msl_a08",
            operations: OPS_OVERLORD_MSL_A07,
        },
    ),
    (
        "overlord_msl_a08",
        MonsterFrame {
            frame: 61,
            next: "overlord_msl_a09",
            operations: OPS_OVERLORD_MSL_A08,
        },
    ),
    (
        "overlord_msl_a09",
        MonsterFrame {
            frame: 62,
            next: "overlord_msl_a10",
            operations: OPS_OVERLORD_MSL_A09,
        },
    ),
    (
        "overlord_msl_a10",
        MonsterFrame {
            frame: 63,
            next: "overlord_msl_a11",
            operations: OPS_OVERLORD_MSL_A10,
        },
    ),
    (
        "overlord_msl_a11",
        MonsterFrame {
            frame: 64,
            next: "overlord_msl_a12",
            operations: OPS_OVERLORD_MSL_A11,
        },
    ),
    (
        "overlord_msl_a12",
        MonsterFrame {
            frame: 65,
            next: "overlord_run01",
            operations: OPS_OVERLORD_MSL_A12,
        },
    ),
    (
        "overlord_pn_a01",
        MonsterFrame {
            frame: 66,
            next: "overlord_pn_a02",
            operations: OPS_OVERLORD_PN_A01,
        },
    ),
    (
        "overlord_pn_a02",
        MonsterFrame {
            frame: 67,
            next: "overlord_pn_a03",
            operations: OPS_OVERLORD_PN_A02,
        },
    ),
    (
        "overlord_pn_a03",
        MonsterFrame {
            frame: 68,
            next: "overlord_pn_a04",
            operations: OPS_OVERLORD_PN_A03,
        },
    ),
    (
        "overlord_pn_a04",
        MonsterFrame {
            frame: 69,
            next: "overlord_pn_a05",
            operations: OPS_OVERLORD_PN_A04,
        },
    ),
    (
        "overlord_pn_a05",
        MonsterFrame {
            frame: 70,
            next: "overlord_pn_a06",
            operations: OPS_OVERLORD_PN_A05,
        },
    ),
    (
        "overlord_pn_a06",
        MonsterFrame {
            frame: 71,
            next: "overlord_pn_a07",
            operations: OPS_OVERLORD_PN_A06,
        },
    ),
    (
        "overlord_pn_a07",
        MonsterFrame {
            frame: 72,
            next: "overlord_run01",
            operations: OPS_OVERLORD_PN_A07,
        },
    ),
    (
        "overlord_pn_b01",
        MonsterFrame {
            frame: 80,
            next: "overlord_pn_b02",
            operations: OPS_OVERLORD_PN_B01,
        },
    ),
    (
        "overlord_pn_b02",
        MonsterFrame {
            frame: 81,
            next: "overlord_pn_b03",
            operations: OPS_OVERLORD_PN_B02,
        },
    ),
    (
        "overlord_pn_b03",
        MonsterFrame {
            frame: 82,
            next: "overlord_pn_b04",
            operations: OPS_OVERLORD_PN_B03,
        },
    ),
    (
        "overlord_pn_b04",
        MonsterFrame {
            frame: 83,
            next: "overlord_pn_b05",
            operations: OPS_OVERLORD_PN_B04,
        },
    ),
    (
        "overlord_pn_b05",
        MonsterFrame {
            frame: 84,
            next: "overlord_pn_b06",
            operations: OPS_OVERLORD_PN_B05,
        },
    ),
    (
        "overlord_pn_b06",
        MonsterFrame {
            frame: 85,
            next: "overlord_pn_b07",
            operations: OPS_OVERLORD_PN_B06,
        },
    ),
    (
        "overlord_pn_b07",
        MonsterFrame {
            frame: 86,
            next: "overlord_run01",
            operations: OPS_OVERLORD_PN_B07,
        },
    ),
    (
        "overlord_run01",
        MonsterFrame {
            frame: 1,
            next: "overlord_run02",
            operations: OPS_OVERLORD_RUN01,
        },
    ),
    (
        "overlord_run02",
        MonsterFrame {
            frame: 2,
            next: "overlord_run03",
            operations: OPS_OVERLORD_RUN02,
        },
    ),
    (
        "overlord_run03",
        MonsterFrame {
            frame: 3,
            next: "overlord_run04",
            operations: OPS_OVERLORD_RUN03,
        },
    ),
    (
        "overlord_run04",
        MonsterFrame {
            frame: 4,
            next: "overlord_run05",
            operations: OPS_OVERLORD_RUN04,
        },
    ),
    (
        "overlord_run05",
        MonsterFrame {
            frame: 5,
            next: "overlord_run06",
            operations: OPS_OVERLORD_RUN05,
        },
    ),
    (
        "overlord_run06",
        MonsterFrame {
            frame: 6,
            next: "overlord_run07",
            operations: OPS_OVERLORD_RUN06,
        },
    ),
    (
        "overlord_run07",
        MonsterFrame {
            frame: 7,
            next: "overlord_run08",
            operations: OPS_OVERLORD_RUN07,
        },
    ),
    (
        "overlord_run08",
        MonsterFrame {
            frame: 8,
            next: "overlord_run09",
            operations: OPS_OVERLORD_RUN08,
        },
    ),
    (
        "overlord_run09",
        MonsterFrame {
            frame: 9,
            next: "overlord_run10",
            operations: OPS_OVERLORD_RUN09,
        },
    ),
    (
        "overlord_run10",
        MonsterFrame {
            frame: 10,
            next: "overlord_run11",
            operations: OPS_OVERLORD_RUN10,
        },
    ),
    (
        "overlord_run11",
        MonsterFrame {
            frame: 11,
            next: "overlord_run12",
            operations: OPS_OVERLORD_RUN11,
        },
    ),
    (
        "overlord_run12",
        MonsterFrame {
            frame: 12,
            next: "overlord_run13",
            operations: OPS_OVERLORD_RUN12,
        },
    ),
    (
        "overlord_run13",
        MonsterFrame {
            frame: 13,
            next: "overlord_run14",
            operations: OPS_OVERLORD_RUN13,
        },
    ),
    (
        "overlord_run14",
        MonsterFrame {
            frame: 14,
            next: "overlord_run15",
            operations: OPS_OVERLORD_RUN14,
        },
    ),
    (
        "overlord_run15",
        MonsterFrame {
            frame: 15,
            next: "overlord_run01",
            operations: OPS_OVERLORD_RUN15,
        },
    ),
    (
        "overlord_stand1",
        MonsterFrame {
            frame: 1,
            next: "overlord_stand1",
            operations: OPS_OVERLORD_STAND1,
        },
    ),
    (
        "overlord_walk01",
        MonsterFrame {
            frame: 1,
            next: "overlord_walk02",
            operations: OPS_OVERLORD_WALK01,
        },
    ),
    (
        "overlord_walk02",
        MonsterFrame {
            frame: 2,
            next: "overlord_walk03",
            operations: OPS_OVERLORD_WALK02,
        },
    ),
    (
        "overlord_walk03",
        MonsterFrame {
            frame: 3,
            next: "overlord_walk04",
            operations: OPS_OVERLORD_WALK03,
        },
    ),
    (
        "overlord_walk04",
        MonsterFrame {
            frame: 4,
            next: "overlord_walk05",
            operations: OPS_OVERLORD_WALK04,
        },
    ),
    (
        "overlord_walk05",
        MonsterFrame {
            frame: 5,
            next: "overlord_walk06",
            operations: OPS_OVERLORD_WALK05,
        },
    ),
    (
        "overlord_walk06",
        MonsterFrame {
            frame: 6,
            next: "overlord_walk07",
            operations: OPS_OVERLORD_WALK06,
        },
    ),
    (
        "overlord_walk07",
        MonsterFrame {
            frame: 7,
            next: "overlord_walk08",
            operations: OPS_OVERLORD_WALK07,
        },
    ),
    (
        "overlord_walk08",
        MonsterFrame {
            frame: 8,
            next: "overlord_walk09",
            operations: OPS_OVERLORD_WALK08,
        },
    ),
    (
        "overlord_walk09",
        MonsterFrame {
            frame: 9,
            next: "overlord_walk10",
            operations: OPS_OVERLORD_WALK09,
        },
    ),
    (
        "overlord_walk10",
        MonsterFrame {
            frame: 10,
            next: "overlord_walk11",
            operations: OPS_OVERLORD_WALK10,
        },
    ),
    (
        "overlord_walk11",
        MonsterFrame {
            frame: 11,
            next: "overlord_walk12",
            operations: OPS_OVERLORD_WALK11,
        },
    ),
    (
        "overlord_walk12",
        MonsterFrame {
            frame: 12,
            next: "overlord_walk13",
            operations: OPS_OVERLORD_WALK12,
        },
    ),
    (
        "overlord_walk13",
        MonsterFrame {
            frame: 13,
            next: "overlord_walk14",
            operations: OPS_OVERLORD_WALK13,
        },
    ),
    (
        "overlord_walk14",
        MonsterFrame {
            frame: 14,
            next: "overlord_walk15",
            operations: OPS_OVERLORD_WALK14,
        },
    ),
    (
        "overlord_walk15",
        MonsterFrame {
            frame: 15,
            next: "overlord_walk01",
            operations: OPS_OVERLORD_WALK15,
        },
    ),
];

/// Look up a s_wrath frame by name.
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
        assert_eq!(FRAMES.len(), 114);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("overlord_at_a01").expect("first frame");
        assert_eq!((head.frame, head.next), (16, "overlord_at_a02"));
        let tail = frame("overlord_walk15").expect("last frame");
        assert_eq!((tail.frame, tail.next), (15, "overlord_walk01"));
        assert!(frame("no_such_frame").is_none());
    }
}
