//! Rogue dragon frames (src/content/q1/missionpacks/monsters/tables/dragon.ts).
//!
//! quakec_rogue/dragon.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_DRAGON_ATK_A1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_A2: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_fireball",
    },
];
static OPS_DRAGON_ATK_A3: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_stop_attack",
    },
];
static OPS_DRAGON_ATK_B1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_B2: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_fireball",
    },
];
static OPS_DRAGON_ATK_B3: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_stop_attack",
    },
];
static OPS_DRAGON_ATK_C1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_C2: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_fireball",
    },
];
static OPS_DRAGON_ATK_C3: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_stop_attack",
    },
];
static OPS_DRAGON_ATK_D1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_D2: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_fireball",
    },
];
static OPS_DRAGON_ATK_D3: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_stop_attack",
    },
];
static OPS_DRAGON_ATK_E1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_E2: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_fireball",
    },
];
static OPS_DRAGON_ATK_E3: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_stop_attack",
    },
];
static OPS_DRAGON_ATK_F1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_F2: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_fireball",
    },
];
static OPS_DRAGON_ATK_F3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(17)",
}];
static OPS_DRAGON_ATK_F4: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "dragon_move(17)",
    },
    MonsterOperation::Action {
        name: "dragon_stop_attack",
    },
];
static OPS_DRAGON_DEATH1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_death1",
}];
static OPS_DRAGON_DEATH10: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH11: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH12: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH13: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH14: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH15: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH16: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH17: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH18: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH19: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH2: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH20: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH21: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_death21",
}];
static OPS_DRAGON_DEATH3: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH4: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH5: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH6: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH7: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH8: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_DEATH9: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_explode" }];
static OPS_DRAGON_MELEE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE10: &[MonsterOperation] = &[MonsterOperation::Action { name: "dragon_tail" }];
static OPS_DRAGON_MELEE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(10)",
}];
static OPS_DRAGON_MELEE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(10)",
}];
static OPS_DRAGON_MELEE13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(10)",
}];
static OPS_DRAGON_MELEE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_MELEE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINA1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINA2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINA3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINB1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINB2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINB3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINC1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINC2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINC3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAIND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAIND2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAIND3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINF1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINF2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_PAINF3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon_move(12)",
}];
static OPS_DRAGON_STAND1: &[MonsterOperation] = &[];
static OPS_DRAGON_WALK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk1",
}];
static OPS_DRAGON_WALK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk2",
}];
static OPS_DRAGON_WALK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk11",
}];
static OPS_DRAGON_WALK12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk2",
}];
static OPS_DRAGON_WALK13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk13",
}];
static OPS_DRAGON_WALK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk2",
}];
static OPS_DRAGON_WALK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk3",
}];
static OPS_DRAGON_WALK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk2",
}];
static OPS_DRAGON_WALK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk5",
}];
static OPS_DRAGON_WALK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk2",
}];
static OPS_DRAGON_WALK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk7",
}];
static OPS_DRAGON_WALK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk2",
}];
static OPS_DRAGON_WALK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "dragon:dragon_walk9",
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "dragon_atk_a1",
        MonsterFrame {
            frame: 43,
            next: "dragon_atk_a2",
            operations: OPS_DRAGON_ATK_A1,
        },
    ),
    (
        "dragon_atk_a2",
        MonsterFrame {
            frame: 44,
            next: "dragon_atk_a3",
            operations: OPS_DRAGON_ATK_A2,
        },
    ),
    (
        "dragon_atk_a3",
        MonsterFrame {
            frame: 45,
            next: "dragon_walk5",
            operations: OPS_DRAGON_ATK_A3,
        },
    ),
    (
        "dragon_atk_b1",
        MonsterFrame {
            frame: 46,
            next: "dragon_atk_b2",
            operations: OPS_DRAGON_ATK_B1,
        },
    ),
    (
        "dragon_atk_b2",
        MonsterFrame {
            frame: 47,
            next: "dragon_atk_b3",
            operations: OPS_DRAGON_ATK_B2,
        },
    ),
    (
        "dragon_atk_b3",
        MonsterFrame {
            frame: 48,
            next: "dragon_walk7",
            operations: OPS_DRAGON_ATK_B3,
        },
    ),
    (
        "dragon_atk_c1",
        MonsterFrame {
            frame: 49,
            next: "dragon_atk_c2",
            operations: OPS_DRAGON_ATK_C1,
        },
    ),
    (
        "dragon_atk_c2",
        MonsterFrame {
            frame: 50,
            next: "dragon_atk_c3",
            operations: OPS_DRAGON_ATK_C2,
        },
    ),
    (
        "dragon_atk_c3",
        MonsterFrame {
            frame: 51,
            next: "dragon_walk9",
            operations: OPS_DRAGON_ATK_C3,
        },
    ),
    (
        "dragon_atk_d1",
        MonsterFrame {
            frame: 52,
            next: "dragon_atk_d2",
            operations: OPS_DRAGON_ATK_D1,
        },
    ),
    (
        "dragon_atk_d2",
        MonsterFrame {
            frame: 53,
            next: "dragon_atk_d3",
            operations: OPS_DRAGON_ATK_D2,
        },
    ),
    (
        "dragon_atk_d3",
        MonsterFrame {
            frame: 54,
            next: "dragon_walk11",
            operations: OPS_DRAGON_ATK_D3,
        },
    ),
    (
        "dragon_atk_e1",
        MonsterFrame {
            frame: 55,
            next: "dragon_atk_e2",
            operations: OPS_DRAGON_ATK_E1,
        },
    ),
    (
        "dragon_atk_e2",
        MonsterFrame {
            frame: 56,
            next: "dragon_atk_e3",
            operations: OPS_DRAGON_ATK_E2,
        },
    ),
    (
        "dragon_atk_e3",
        MonsterFrame {
            frame: 57,
            next: "dragon_walk13",
            operations: OPS_DRAGON_ATK_E3,
        },
    ),
    (
        "dragon_atk_f1",
        MonsterFrame {
            frame: 58,
            next: "dragon_atk_f2",
            operations: OPS_DRAGON_ATK_F1,
        },
    ),
    (
        "dragon_atk_f2",
        MonsterFrame {
            frame: 59,
            next: "dragon_atk_f3",
            operations: OPS_DRAGON_ATK_F2,
        },
    ),
    (
        "dragon_atk_f3",
        MonsterFrame {
            frame: 60,
            next: "dragon_atk_f4",
            operations: OPS_DRAGON_ATK_F3,
        },
    ),
    (
        "dragon_atk_f4",
        MonsterFrame {
            frame: 60,
            next: "dragon_walk3",
            operations: OPS_DRAGON_ATK_F4,
        },
    ),
    (
        "dragon_death1",
        MonsterFrame {
            frame: 80,
            next: "dragon_death2",
            operations: OPS_DRAGON_DEATH1,
        },
    ),
    (
        "dragon_death10",
        MonsterFrame {
            frame: 89,
            next: "dragon_death11",
            operations: OPS_DRAGON_DEATH10,
        },
    ),
    (
        "dragon_death11",
        MonsterFrame {
            frame: 90,
            next: "dragon_death12",
            operations: OPS_DRAGON_DEATH11,
        },
    ),
    (
        "dragon_death12",
        MonsterFrame {
            frame: 91,
            next: "dragon_death13",
            operations: OPS_DRAGON_DEATH12,
        },
    ),
    (
        "dragon_death13",
        MonsterFrame {
            frame: 92,
            next: "dragon_death14",
            operations: OPS_DRAGON_DEATH13,
        },
    ),
    (
        "dragon_death14",
        MonsterFrame {
            frame: 93,
            next: "dragon_death15",
            operations: OPS_DRAGON_DEATH14,
        },
    ),
    (
        "dragon_death15",
        MonsterFrame {
            frame: 94,
            next: "dragon_death16",
            operations: OPS_DRAGON_DEATH15,
        },
    ),
    (
        "dragon_death16",
        MonsterFrame {
            frame: 95,
            next: "dragon_death17",
            operations: OPS_DRAGON_DEATH16,
        },
    ),
    (
        "dragon_death17",
        MonsterFrame {
            frame: 96,
            next: "dragon_death18",
            operations: OPS_DRAGON_DEATH17,
        },
    ),
    (
        "dragon_death18",
        MonsterFrame {
            frame: 97,
            next: "dragon_death19",
            operations: OPS_DRAGON_DEATH18,
        },
    ),
    (
        "dragon_death19",
        MonsterFrame {
            frame: 98,
            next: "dragon_death20",
            operations: OPS_DRAGON_DEATH19,
        },
    ),
    (
        "dragon_death2",
        MonsterFrame {
            frame: 81,
            next: "dragon_death3",
            operations: OPS_DRAGON_DEATH2,
        },
    ),
    (
        "dragon_death20",
        MonsterFrame {
            frame: 99,
            next: "dragon_death21",
            operations: OPS_DRAGON_DEATH20,
        },
    ),
    (
        "dragon_death21",
        MonsterFrame {
            frame: 100,
            next: "dragon_death21",
            operations: OPS_DRAGON_DEATH21,
        },
    ),
    (
        "dragon_death3",
        MonsterFrame {
            frame: 82,
            next: "dragon_death4",
            operations: OPS_DRAGON_DEATH3,
        },
    ),
    (
        "dragon_death4",
        MonsterFrame {
            frame: 83,
            next: "dragon_death5",
            operations: OPS_DRAGON_DEATH4,
        },
    ),
    (
        "dragon_death5",
        MonsterFrame {
            frame: 84,
            next: "dragon_death6",
            operations: OPS_DRAGON_DEATH5,
        },
    ),
    (
        "dragon_death6",
        MonsterFrame {
            frame: 85,
            next: "dragon_death7",
            operations: OPS_DRAGON_DEATH6,
        },
    ),
    (
        "dragon_death7",
        MonsterFrame {
            frame: 86,
            next: "dragon_death8",
            operations: OPS_DRAGON_DEATH7,
        },
    ),
    (
        "dragon_death8",
        MonsterFrame {
            frame: 87,
            next: "dragon_death9",
            operations: OPS_DRAGON_DEATH8,
        },
    ),
    (
        "dragon_death9",
        MonsterFrame {
            frame: 88,
            next: "dragon_death10",
            operations: OPS_DRAGON_DEATH9,
        },
    ),
    (
        "dragon_melee1",
        MonsterFrame {
            frame: 20,
            next: "dragon_melee2",
            operations: OPS_DRAGON_MELEE1,
        },
    ),
    (
        "dragon_melee10",
        MonsterFrame {
            frame: 29,
            next: "dragon_melee11",
            operations: OPS_DRAGON_MELEE10,
        },
    ),
    (
        "dragon_melee11",
        MonsterFrame {
            frame: 30,
            next: "dragon_melee12",
            operations: OPS_DRAGON_MELEE11,
        },
    ),
    (
        "dragon_melee12",
        MonsterFrame {
            frame: 31,
            next: "dragon_melee13",
            operations: OPS_DRAGON_MELEE12,
        },
    ),
    (
        "dragon_melee13",
        MonsterFrame {
            frame: 32,
            next: "dragon_walk1",
            operations: OPS_DRAGON_MELEE13,
        },
    ),
    (
        "dragon_melee2",
        MonsterFrame {
            frame: 21,
            next: "dragon_melee3",
            operations: OPS_DRAGON_MELEE2,
        },
    ),
    (
        "dragon_melee3",
        MonsterFrame {
            frame: 22,
            next: "dragon_melee4",
            operations: OPS_DRAGON_MELEE3,
        },
    ),
    (
        "dragon_melee4",
        MonsterFrame {
            frame: 23,
            next: "dragon_melee5",
            operations: OPS_DRAGON_MELEE4,
        },
    ),
    (
        "dragon_melee5",
        MonsterFrame {
            frame: 24,
            next: "dragon_melee6",
            operations: OPS_DRAGON_MELEE5,
        },
    ),
    (
        "dragon_melee6",
        MonsterFrame {
            frame: 25,
            next: "dragon_melee7",
            operations: OPS_DRAGON_MELEE6,
        },
    ),
    (
        "dragon_melee7",
        MonsterFrame {
            frame: 26,
            next: "dragon_melee8",
            operations: OPS_DRAGON_MELEE7,
        },
    ),
    (
        "dragon_melee8",
        MonsterFrame {
            frame: 27,
            next: "dragon_melee9",
            operations: OPS_DRAGON_MELEE8,
        },
    ),
    (
        "dragon_melee9",
        MonsterFrame {
            frame: 28,
            next: "dragon_melee10",
            operations: OPS_DRAGON_MELEE9,
        },
    ),
    (
        "dragon_painA1",
        MonsterFrame {
            frame: 62,
            next: "dragon_painA2",
            operations: OPS_DRAGON_PAINA1,
        },
    ),
    (
        "dragon_painA2",
        MonsterFrame {
            frame: 63,
            next: "dragon_painA3",
            operations: OPS_DRAGON_PAINA2,
        },
    ),
    (
        "dragon_painA3",
        MonsterFrame {
            frame: 64,
            next: "dragon_walk5",
            operations: OPS_DRAGON_PAINA3,
        },
    ),
    (
        "dragon_painB1",
        MonsterFrame {
            frame: 65,
            next: "dragon_painB2",
            operations: OPS_DRAGON_PAINB1,
        },
    ),
    (
        "dragon_painB2",
        MonsterFrame {
            frame: 66,
            next: "dragon_painB3",
            operations: OPS_DRAGON_PAINB2,
        },
    ),
    (
        "dragon_painB3",
        MonsterFrame {
            frame: 67,
            next: "dragon_walk7",
            operations: OPS_DRAGON_PAINB3,
        },
    ),
    (
        "dragon_painC1",
        MonsterFrame {
            frame: 68,
            next: "dragon_painC2",
            operations: OPS_DRAGON_PAINC1,
        },
    ),
    (
        "dragon_painC2",
        MonsterFrame {
            frame: 69,
            next: "dragon_painC3",
            operations: OPS_DRAGON_PAINC2,
        },
    ),
    (
        "dragon_painC3",
        MonsterFrame {
            frame: 70,
            next: "dragon_walk9",
            operations: OPS_DRAGON_PAINC3,
        },
    ),
    (
        "dragon_painD1",
        MonsterFrame {
            frame: 71,
            next: "dragon_painD2",
            operations: OPS_DRAGON_PAIND1,
        },
    ),
    (
        "dragon_painD2",
        MonsterFrame {
            frame: 72,
            next: "dragon_painD3",
            operations: OPS_DRAGON_PAIND2,
        },
    ),
    (
        "dragon_painD3",
        MonsterFrame {
            frame: 73,
            next: "dragon_walk11",
            operations: OPS_DRAGON_PAIND3,
        },
    ),
    (
        "dragon_painE1",
        MonsterFrame {
            frame: 74,
            next: "dragon_painE2",
            operations: OPS_DRAGON_PAINE1,
        },
    ),
    (
        "dragon_painE2",
        MonsterFrame {
            frame: 75,
            next: "dragon_painE3",
            operations: OPS_DRAGON_PAINE2,
        },
    ),
    (
        "dragon_painE3",
        MonsterFrame {
            frame: 76,
            next: "dragon_walk13",
            operations: OPS_DRAGON_PAINE3,
        },
    ),
    (
        "dragon_painF1",
        MonsterFrame {
            frame: 77,
            next: "dragon_painF2",
            operations: OPS_DRAGON_PAINF1,
        },
    ),
    (
        "dragon_painF2",
        MonsterFrame {
            frame: 78,
            next: "dragon_painF3",
            operations: OPS_DRAGON_PAINF2,
        },
    ),
    (
        "dragon_painF3",
        MonsterFrame {
            frame: 79,
            next: "dragon_walk2",
            operations: OPS_DRAGON_PAINF3,
        },
    ),
    (
        "dragon_stand1",
        MonsterFrame {
            frame: 1,
            next: "dragon_walk1",
            operations: OPS_DRAGON_STAND1,
        },
    ),
    (
        "dragon_walk1",
        MonsterFrame {
            frame: 1,
            next: "dragon_walk2",
            operations: OPS_DRAGON_WALK1,
        },
    ),
    (
        "dragon_walk10",
        MonsterFrame {
            frame: 10,
            next: "dragon_walk11",
            operations: OPS_DRAGON_WALK10,
        },
    ),
    (
        "dragon_walk11",
        MonsterFrame {
            frame: 11,
            next: "dragon_walk12",
            operations: OPS_DRAGON_WALK11,
        },
    ),
    (
        "dragon_walk12",
        MonsterFrame {
            frame: 12,
            next: "dragon_walk13",
            operations: OPS_DRAGON_WALK12,
        },
    ),
    (
        "dragon_walk13",
        MonsterFrame {
            frame: 13,
            next: "dragon_walk1",
            operations: OPS_DRAGON_WALK13,
        },
    ),
    (
        "dragon_walk2",
        MonsterFrame {
            frame: 2,
            next: "dragon_walk3",
            operations: OPS_DRAGON_WALK2,
        },
    ),
    (
        "dragon_walk3",
        MonsterFrame {
            frame: 3,
            next: "dragon_walk4",
            operations: OPS_DRAGON_WALK3,
        },
    ),
    (
        "dragon_walk4",
        MonsterFrame {
            frame: 4,
            next: "dragon_walk5",
            operations: OPS_DRAGON_WALK4,
        },
    ),
    (
        "dragon_walk5",
        MonsterFrame {
            frame: 5,
            next: "dragon_walk6",
            operations: OPS_DRAGON_WALK5,
        },
    ),
    (
        "dragon_walk6",
        MonsterFrame {
            frame: 6,
            next: "dragon_walk7",
            operations: OPS_DRAGON_WALK6,
        },
    ),
    (
        "dragon_walk7",
        MonsterFrame {
            frame: 7,
            next: "dragon_walk8",
            operations: OPS_DRAGON_WALK7,
        },
    ),
    (
        "dragon_walk8",
        MonsterFrame {
            frame: 8,
            next: "dragon_walk9",
            operations: OPS_DRAGON_WALK8,
        },
    ),
    (
        "dragon_walk9",
        MonsterFrame {
            frame: 9,
            next: "dragon_walk10",
            operations: OPS_DRAGON_WALK9,
        },
    ),
];

/// Look up a dragon frame by name.
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
        assert_eq!(FRAMES.len(), 85);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("dragon_atk_a1").expect("first frame");
        assert_eq!((head.frame, head.next), (43, "dragon_atk_a2"));
        let tail = frame("dragon_walk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (9, "dragon_walk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
