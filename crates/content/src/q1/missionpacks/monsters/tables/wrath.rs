//! Rogue wrath frames (src/content/q1/missionpacks/monsters/tables/wrath.ts).
//!
//! quakec_rogue/wrath.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::Q1SoundChannel;

static OPS_WRATH_AT_A01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "WrathMissile(1)",
}];
static OPS_WRATH_AT_A12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_A14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B06: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "WrathMissile(2)",
}];
static OPS_WRATH_AT_B07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_B13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C07: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "WrathMissile(3)",
}];
static OPS_WRATH_AT_C08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_AT_C15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_WRATH_DIE02: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "wrath/wdthc.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_WRATH_DIE03: &[MonsterOperation] = &[];
static OPS_WRATH_DIE04: &[MonsterOperation] = &[];
static OPS_WRATH_DIE05: &[MonsterOperation] = &[];
static OPS_WRATH_DIE07: &[MonsterOperation] = &[];
static OPS_WRATH_DIE09: &[MonsterOperation] = &[];
static OPS_WRATH_DIE11: &[MonsterOperation] = &[];
static OPS_WRATH_DIE13: &[MonsterOperation] = &[];
static OPS_WRATH_DIE15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "wrath:wrath_die15",
}];
static OPS_WRATH_PN_A01: &[MonsterOperation] = &[];
static OPS_WRATH_PN_A02: &[MonsterOperation] = &[];
static OPS_WRATH_PN_A03: &[MonsterOperation] = &[];
static OPS_WRATH_PN_A04: &[MonsterOperation] = &[];
static OPS_WRATH_PN_A05: &[MonsterOperation] = &[];
static OPS_WRATH_PN_A06: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B01: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B02: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B03: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B04: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B05: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B06: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B07: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B08: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B09: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B10: &[MonsterOperation] = &[];
static OPS_WRATH_PN_B11: &[MonsterOperation] = &[];
static OPS_WRATH_RUN01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_WRATH_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WRATH_WALK01: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK02: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK03: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK04: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK05: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK06: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK07: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK08: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK09: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WRATH_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "wrath_at_a01",
        MonsterFrame {
            frame: 13,
            next: "wrath_at_a02",
            operations: OPS_WRATH_AT_A01,
        },
    ),
    (
        "wrath_at_a02",
        MonsterFrame {
            frame: 14,
            next: "wrath_at_a03",
            operations: OPS_WRATH_AT_A02,
        },
    ),
    (
        "wrath_at_a03",
        MonsterFrame {
            frame: 15,
            next: "wrath_at_a04",
            operations: OPS_WRATH_AT_A03,
        },
    ),
    (
        "wrath_at_a04",
        MonsterFrame {
            frame: 16,
            next: "wrath_at_a05",
            operations: OPS_WRATH_AT_A04,
        },
    ),
    (
        "wrath_at_a05",
        MonsterFrame {
            frame: 17,
            next: "wrath_at_a06",
            operations: OPS_WRATH_AT_A05,
        },
    ),
    (
        "wrath_at_a06",
        MonsterFrame {
            frame: 18,
            next: "wrath_at_a07",
            operations: OPS_WRATH_AT_A06,
        },
    ),
    (
        "wrath_at_a07",
        MonsterFrame {
            frame: 19,
            next: "wrath_at_a08",
            operations: OPS_WRATH_AT_A07,
        },
    ),
    (
        "wrath_at_a08",
        MonsterFrame {
            frame: 20,
            next: "wrath_at_a09",
            operations: OPS_WRATH_AT_A08,
        },
    ),
    (
        "wrath_at_a09",
        MonsterFrame {
            frame: 21,
            next: "wrath_at_a10",
            operations: OPS_WRATH_AT_A09,
        },
    ),
    (
        "wrath_at_a10",
        MonsterFrame {
            frame: 22,
            next: "wrath_at_a11",
            operations: OPS_WRATH_AT_A10,
        },
    ),
    (
        "wrath_at_a11",
        MonsterFrame {
            frame: 23,
            next: "wrath_at_a12",
            operations: OPS_WRATH_AT_A11,
        },
    ),
    (
        "wrath_at_a12",
        MonsterFrame {
            frame: 24,
            next: "wrath_at_a13",
            operations: OPS_WRATH_AT_A12,
        },
    ),
    (
        "wrath_at_a13",
        MonsterFrame {
            frame: 25,
            next: "wrath_at_a14",
            operations: OPS_WRATH_AT_A13,
        },
    ),
    (
        "wrath_at_a14",
        MonsterFrame {
            frame: 26,
            next: "wrath_run01",
            operations: OPS_WRATH_AT_A14,
        },
    ),
    (
        "wrath_at_b01",
        MonsterFrame {
            frame: 27,
            next: "wrath_at_b02",
            operations: OPS_WRATH_AT_B01,
        },
    ),
    (
        "wrath_at_b02",
        MonsterFrame {
            frame: 28,
            next: "wrath_at_b03",
            operations: OPS_WRATH_AT_B02,
        },
    ),
    (
        "wrath_at_b03",
        MonsterFrame {
            frame: 29,
            next: "wrath_at_b04",
            operations: OPS_WRATH_AT_B03,
        },
    ),
    (
        "wrath_at_b04",
        MonsterFrame {
            frame: 30,
            next: "wrath_at_b05",
            operations: OPS_WRATH_AT_B04,
        },
    ),
    (
        "wrath_at_b05",
        MonsterFrame {
            frame: 31,
            next: "wrath_at_b06",
            operations: OPS_WRATH_AT_B05,
        },
    ),
    (
        "wrath_at_b06",
        MonsterFrame {
            frame: 32,
            next: "wrath_at_b07",
            operations: OPS_WRATH_AT_B06,
        },
    ),
    (
        "wrath_at_b07",
        MonsterFrame {
            frame: 33,
            next: "wrath_at_b08",
            operations: OPS_WRATH_AT_B07,
        },
    ),
    (
        "wrath_at_b08",
        MonsterFrame {
            frame: 34,
            next: "wrath_at_b09",
            operations: OPS_WRATH_AT_B08,
        },
    ),
    (
        "wrath_at_b09",
        MonsterFrame {
            frame: 35,
            next: "wrath_at_b10",
            operations: OPS_WRATH_AT_B09,
        },
    ),
    (
        "wrath_at_b10",
        MonsterFrame {
            frame: 36,
            next: "wrath_at_b11",
            operations: OPS_WRATH_AT_B10,
        },
    ),
    (
        "wrath_at_b11",
        MonsterFrame {
            frame: 37,
            next: "wrath_at_b12",
            operations: OPS_WRATH_AT_B11,
        },
    ),
    (
        "wrath_at_b12",
        MonsterFrame {
            frame: 38,
            next: "wrath_at_b13",
            operations: OPS_WRATH_AT_B12,
        },
    ),
    (
        "wrath_at_b13",
        MonsterFrame {
            frame: 39,
            next: "wrath_run01",
            operations: OPS_WRATH_AT_B13,
        },
    ),
    (
        "wrath_at_c01",
        MonsterFrame {
            frame: 40,
            next: "wrath_at_c02",
            operations: OPS_WRATH_AT_C01,
        },
    ),
    (
        "wrath_at_c02",
        MonsterFrame {
            frame: 41,
            next: "wrath_at_c03",
            operations: OPS_WRATH_AT_C02,
        },
    ),
    (
        "wrath_at_c03",
        MonsterFrame {
            frame: 42,
            next: "wrath_at_c04",
            operations: OPS_WRATH_AT_C03,
        },
    ),
    (
        "wrath_at_c04",
        MonsterFrame {
            frame: 43,
            next: "wrath_at_c05",
            operations: OPS_WRATH_AT_C04,
        },
    ),
    (
        "wrath_at_c05",
        MonsterFrame {
            frame: 44,
            next: "wrath_at_c06",
            operations: OPS_WRATH_AT_C05,
        },
    ),
    (
        "wrath_at_c06",
        MonsterFrame {
            frame: 45,
            next: "wrath_at_c07",
            operations: OPS_WRATH_AT_C06,
        },
    ),
    (
        "wrath_at_c07",
        MonsterFrame {
            frame: 46,
            next: "wrath_at_c08",
            operations: OPS_WRATH_AT_C07,
        },
    ),
    (
        "wrath_at_c08",
        MonsterFrame {
            frame: 47,
            next: "wrath_at_c09",
            operations: OPS_WRATH_AT_C08,
        },
    ),
    (
        "wrath_at_c09",
        MonsterFrame {
            frame: 48,
            next: "wrath_at_c10",
            operations: OPS_WRATH_AT_C09,
        },
    ),
    (
        "wrath_at_c10",
        MonsterFrame {
            frame: 49,
            next: "wrath_at_c11",
            operations: OPS_WRATH_AT_C10,
        },
    ),
    (
        "wrath_at_c11",
        MonsterFrame {
            frame: 50,
            next: "wrath_at_c12",
            operations: OPS_WRATH_AT_C11,
        },
    ),
    (
        "wrath_at_c12",
        MonsterFrame {
            frame: 51,
            next: "wrath_at_c13",
            operations: OPS_WRATH_AT_C12,
        },
    ),
    (
        "wrath_at_c13",
        MonsterFrame {
            frame: 52,
            next: "wrath_at_c14",
            operations: OPS_WRATH_AT_C13,
        },
    ),
    (
        "wrath_at_c14",
        MonsterFrame {
            frame: 53,
            next: "wrath_at_c15",
            operations: OPS_WRATH_AT_C14,
        },
    ),
    (
        "wrath_at_c15",
        MonsterFrame {
            frame: 54,
            next: "wrath_run01",
            operations: OPS_WRATH_AT_C15,
        },
    ),
    (
        "wrath_die02",
        MonsterFrame {
            frame: 73,
            next: "wrath_die03",
            operations: OPS_WRATH_DIE02,
        },
    ),
    (
        "wrath_die03",
        MonsterFrame {
            frame: 74,
            next: "wrath_die04",
            operations: OPS_WRATH_DIE03,
        },
    ),
    (
        "wrath_die04",
        MonsterFrame {
            frame: 75,
            next: "wrath_die05",
            operations: OPS_WRATH_DIE04,
        },
    ),
    (
        "wrath_die05",
        MonsterFrame {
            frame: 76,
            next: "wrath_die07",
            operations: OPS_WRATH_DIE05,
        },
    ),
    (
        "wrath_die07",
        MonsterFrame {
            frame: 78,
            next: "wrath_die09",
            operations: OPS_WRATH_DIE07,
        },
    ),
    (
        "wrath_die09",
        MonsterFrame {
            frame: 80,
            next: "wrath_die11",
            operations: OPS_WRATH_DIE09,
        },
    ),
    (
        "wrath_die11",
        MonsterFrame {
            frame: 82,
            next: "wrath_die13",
            operations: OPS_WRATH_DIE11,
        },
    ),
    (
        "wrath_die13",
        MonsterFrame {
            frame: 84,
            next: "wrath_die15",
            operations: OPS_WRATH_DIE13,
        },
    ),
    (
        "wrath_die15",
        MonsterFrame {
            frame: 86,
            next: "wrath_die15",
            operations: OPS_WRATH_DIE15,
        },
    ),
    (
        "wrath_pn_a01",
        MonsterFrame {
            frame: 55,
            next: "wrath_pn_a02",
            operations: OPS_WRATH_PN_A01,
        },
    ),
    (
        "wrath_pn_a02",
        MonsterFrame {
            frame: 56,
            next: "wrath_pn_a03",
            operations: OPS_WRATH_PN_A02,
        },
    ),
    (
        "wrath_pn_a03",
        MonsterFrame {
            frame: 57,
            next: "wrath_pn_a04",
            operations: OPS_WRATH_PN_A03,
        },
    ),
    (
        "wrath_pn_a04",
        MonsterFrame {
            frame: 58,
            next: "wrath_pn_a05",
            operations: OPS_WRATH_PN_A04,
        },
    ),
    (
        "wrath_pn_a05",
        MonsterFrame {
            frame: 59,
            next: "wrath_pn_a06",
            operations: OPS_WRATH_PN_A05,
        },
    ),
    (
        "wrath_pn_a06",
        MonsterFrame {
            frame: 60,
            next: "wrath_run01",
            operations: OPS_WRATH_PN_A06,
        },
    ),
    (
        "wrath_pn_b01",
        MonsterFrame {
            frame: 61,
            next: "wrath_pn_b02",
            operations: OPS_WRATH_PN_B01,
        },
    ),
    (
        "wrath_pn_b02",
        MonsterFrame {
            frame: 62,
            next: "wrath_pn_b03",
            operations: OPS_WRATH_PN_B02,
        },
    ),
    (
        "wrath_pn_b03",
        MonsterFrame {
            frame: 63,
            next: "wrath_pn_b04",
            operations: OPS_WRATH_PN_B03,
        },
    ),
    (
        "wrath_pn_b04",
        MonsterFrame {
            frame: 64,
            next: "wrath_pn_b05",
            operations: OPS_WRATH_PN_B04,
        },
    ),
    (
        "wrath_pn_b05",
        MonsterFrame {
            frame: 65,
            next: "wrath_pn_b06",
            operations: OPS_WRATH_PN_B05,
        },
    ),
    (
        "wrath_pn_b06",
        MonsterFrame {
            frame: 66,
            next: "wrath_pn_b07",
            operations: OPS_WRATH_PN_B06,
        },
    ),
    (
        "wrath_pn_b07",
        MonsterFrame {
            frame: 67,
            next: "wrath_pn_b08",
            operations: OPS_WRATH_PN_B07,
        },
    ),
    (
        "wrath_pn_b08",
        MonsterFrame {
            frame: 68,
            next: "wrath_pn_b09",
            operations: OPS_WRATH_PN_B08,
        },
    ),
    (
        "wrath_pn_b09",
        MonsterFrame {
            frame: 69,
            next: "wrath_pn_b10",
            operations: OPS_WRATH_PN_B09,
        },
    ),
    (
        "wrath_pn_b10",
        MonsterFrame {
            frame: 70,
            next: "wrath_pn_b11",
            operations: OPS_WRATH_PN_B10,
        },
    ),
    (
        "wrath_pn_b11",
        MonsterFrame {
            frame: 71,
            next: "wrath_run01",
            operations: OPS_WRATH_PN_B11,
        },
    ),
    (
        "wrath_run01",
        MonsterFrame {
            frame: 1,
            next: "wrath_run02",
            operations: OPS_WRATH_RUN01,
        },
    ),
    (
        "wrath_run02",
        MonsterFrame {
            frame: 2,
            next: "wrath_run03",
            operations: OPS_WRATH_RUN02,
        },
    ),
    (
        "wrath_run03",
        MonsterFrame {
            frame: 3,
            next: "wrath_run04",
            operations: OPS_WRATH_RUN03,
        },
    ),
    (
        "wrath_run04",
        MonsterFrame {
            frame: 4,
            next: "wrath_run05",
            operations: OPS_WRATH_RUN04,
        },
    ),
    (
        "wrath_run05",
        MonsterFrame {
            frame: 5,
            next: "wrath_run06",
            operations: OPS_WRATH_RUN05,
        },
    ),
    (
        "wrath_run06",
        MonsterFrame {
            frame: 6,
            next: "wrath_run07",
            operations: OPS_WRATH_RUN06,
        },
    ),
    (
        "wrath_run07",
        MonsterFrame {
            frame: 7,
            next: "wrath_run08",
            operations: OPS_WRATH_RUN07,
        },
    ),
    (
        "wrath_run08",
        MonsterFrame {
            frame: 8,
            next: "wrath_run09",
            operations: OPS_WRATH_RUN08,
        },
    ),
    (
        "wrath_run09",
        MonsterFrame {
            frame: 9,
            next: "wrath_run10",
            operations: OPS_WRATH_RUN09,
        },
    ),
    (
        "wrath_run10",
        MonsterFrame {
            frame: 10,
            next: "wrath_run11",
            operations: OPS_WRATH_RUN10,
        },
    ),
    (
        "wrath_run11",
        MonsterFrame {
            frame: 11,
            next: "wrath_run12",
            operations: OPS_WRATH_RUN11,
        },
    ),
    (
        "wrath_run12",
        MonsterFrame {
            frame: 12,
            next: "wrath_run01",
            operations: OPS_WRATH_RUN12,
        },
    ),
    (
        "wrath_stand1",
        MonsterFrame {
            frame: 1,
            next: "wrath_stand1",
            operations: OPS_WRATH_STAND1,
        },
    ),
    (
        "wrath_walk01",
        MonsterFrame {
            frame: 1,
            next: "wrath_walk02",
            operations: OPS_WRATH_WALK01,
        },
    ),
    (
        "wrath_walk02",
        MonsterFrame {
            frame: 2,
            next: "wrath_walk03",
            operations: OPS_WRATH_WALK02,
        },
    ),
    (
        "wrath_walk03",
        MonsterFrame {
            frame: 3,
            next: "wrath_walk04",
            operations: OPS_WRATH_WALK03,
        },
    ),
    (
        "wrath_walk04",
        MonsterFrame {
            frame: 4,
            next: "wrath_walk05",
            operations: OPS_WRATH_WALK04,
        },
    ),
    (
        "wrath_walk05",
        MonsterFrame {
            frame: 5,
            next: "wrath_walk06",
            operations: OPS_WRATH_WALK05,
        },
    ),
    (
        "wrath_walk06",
        MonsterFrame {
            frame: 6,
            next: "wrath_walk07",
            operations: OPS_WRATH_WALK06,
        },
    ),
    (
        "wrath_walk07",
        MonsterFrame {
            frame: 7,
            next: "wrath_walk08",
            operations: OPS_WRATH_WALK07,
        },
    ),
    (
        "wrath_walk08",
        MonsterFrame {
            frame: 8,
            next: "wrath_walk09",
            operations: OPS_WRATH_WALK08,
        },
    ),
    (
        "wrath_walk09",
        MonsterFrame {
            frame: 9,
            next: "wrath_walk10",
            operations: OPS_WRATH_WALK09,
        },
    ),
    (
        "wrath_walk10",
        MonsterFrame {
            frame: 10,
            next: "wrath_walk11",
            operations: OPS_WRATH_WALK10,
        },
    ),
    (
        "wrath_walk11",
        MonsterFrame {
            frame: 11,
            next: "wrath_walk12",
            operations: OPS_WRATH_WALK11,
        },
    ),
    (
        "wrath_walk12",
        MonsterFrame {
            frame: 12,
            next: "wrath_walk01",
            operations: OPS_WRATH_WALK12,
        },
    ),
];

/// Look up a wrath frame by name.
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
        assert_eq!(FRAMES.len(), 93);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("wrath_at_a01").expect("first frame");
        assert_eq!((head.frame, head.next), (13, "wrath_at_a02"));
        let tail = frame("wrath_walk12").expect("last frame");
        assert_eq!((tail.frame, tail.next), (12, "wrath_walk01"));
        assert!(frame("no_such_frame").is_none());
    }
}
