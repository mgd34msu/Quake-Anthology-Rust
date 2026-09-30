//! Q1 Mg3 rune-knight frames (`src/content/q1/addons/monsters/heavy/tables/mg3_rknight.ts`).
//!
//! quakec_mg3/monsters/mg3_rknight.qc source frame order. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation};
use crate::q1::foundation::types::Q1Solid;

static OPS_RKNIGHT_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_RKNIGHT_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "rk_idle_sound" },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 2.0,
    },
];
static OPS_RKNIGHT_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_RKNIGHT_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_RKNIGHT_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_RKNIGHT_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_RKNIGHT_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_RKNIGHT_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_RKNIGHT_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_RKNIGHT_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_RKNIGHT_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_RKNIGHT_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_RKNIGHT_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_RKNIGHT_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_RKNIGHT_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_RKNIGHT_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_RKNIGHT_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_RKNIGHT_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_RKNIGHT_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_RKNIGHT_WALK19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_RKNIGHT_WALK20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_RKNIGHT_RUNB1: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "rk_idle_sound" },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 2.0,
    },
];
static OPS_RKNIGHT_RUNB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 5.0,
}];
static OPS_RKNIGHT_RUNB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 5.0,
}];
static OPS_RKNIGHT_RUNB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_RKNIGHT_RUNB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_RKNIGHT_RUNB6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_RKNIGHT_RUNB7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_RKNIGHT_RUNB8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_RKNIGHT_RUNB9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_RKNIGHT_RUNB10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_RKNIGHT_RUNB11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_RKNIGHT_RUNB12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_RKNIGHT_RUNB13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 6.0,
}];
static OPS_RKNIGHT_RUNB14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_RKNIGHT_RUNB15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_RKNIGHT_RUNB16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_RKNIGHT_RUNB17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_RKNIGHT_RUNB18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_RKNIGHT_RUNB19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_RKNIGHT_RUNB20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_RKNIGHT_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "rk_idle_sound" },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 20.0,
    },
];
static OPS_RKNIGHT_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 25.0,
}];
static OPS_RKNIGHT_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 18.0,
}];
static OPS_RKNIGHT_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_RKNIGHT_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_RKNIGHT_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 25.0,
}];
static OPS_RKNIGHT_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 21.0,
}];
static OPS_RKNIGHT_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 13.0,
}];
static OPS_RKNIGHT_PAIN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "rknight_pain_sound",
}];
static OPS_RKNIGHT_DIE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 10.0,
}];
static OPS_RKNIGHT_DIE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 8.0,
}];
static OPS_RKNIGHT_DIE3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Ai {
        mode: MonsterAi::Forward,
        distance: 7.0,
    },
];
static OPS_RKNIGHT_DIE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 10.0,
}];
static OPS_RKNIGHT_DIE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 11.0,
}];
static OPS_RKNIGHT_DIEB3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_RKNIGHT_MAGICA1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magica8",
}];
static OPS_RKNIGHT_MAGICA9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magica9",
}];
static OPS_RKNIGHT_MAGICA10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magica10",
}];
static OPS_RKNIGHT_MAGICA11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magica11",
}];
static OPS_RKNIGHT_MAGICA12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magica12",
}];
static OPS_RKNIGHT_MAGICA13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICA14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICB1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICB6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb6",
}];
static OPS_RKNIGHT_MAGICB7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb7",
}];
static OPS_RKNIGHT_MAGICB8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb8",
}];
static OPS_RKNIGHT_MAGICB9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb9",
}];
static OPS_RKNIGHT_MAGICB10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb10",
}];
static OPS_RKNIGHT_MAGICB11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb11",
}];
static OPS_RKNIGHT_MAGICB12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicb12",
}];
static OPS_RKNIGHT_MAGICB13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICC1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICC3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICC4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICC5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_RKNIGHT_MAGICC6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicc6",
}];
static OPS_RKNIGHT_MAGICC7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicc7",
}];
static OPS_RKNIGHT_MAGICC8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicc8",
}];
static OPS_RKNIGHT_MAGICC9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicc9",
}];
static OPS_RKNIGHT_MAGICC10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicc10",
}];
static OPS_RKNIGHT_MAGICC11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mg3_rknight:rknight_magicc11",
}];
static OPS_RKNIGHT_SLICE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_RKNIGHT_SLICE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_RKNIGHT_SLICE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 13.0,
}];
static OPS_RKNIGHT_SLICE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_RKNIGHT_SLICE5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 7.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SLICE6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 15.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SLICE7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SLICE8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SLICE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_RKNIGHT_SLICE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_RKNIGHT_SMASH1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_RKNIGHT_SMASH2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 13.0,
}];
static OPS_RKNIGHT_SMASH3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_RKNIGHT_SMASH4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 11.0,
}];
static OPS_RKNIGHT_SMASH5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 10.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SMASH6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 7.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SMASH7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 12.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SMASH8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SMASH9: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_SMASH10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_SMASH11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 2.0,
}];
static OPS_RKNIGHT_WATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_RKNIGHT_WATK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_RKNIGHT_WATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_RKNIGHT_WATK10: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_WATK11: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_WATK12: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_WATK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_RKNIGHT_WATK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_RKNIGHT_WATK17: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 1.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_WATK18: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_WATK19: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 4.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_RKNIGHT_WATK20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_RKNIGHT_WATK21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_RKNIGHT_WATK22: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];

/// Addon monster frames (`frames`).
pub fn frames() -> &'static HashMap<String, MonsterFrame> {
    static MAP: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                String::from("rknight_stand1"),
                MonsterFrame {
                    frame: 0,
                    next: "rknight_stand2",
                    operations: OPS_RKNIGHT_STAND1,
                },
            ),
            (
                String::from("rknight_stand2"),
                MonsterFrame {
                    frame: 1,
                    next: "rknight_stand3",
                    operations: OPS_RKNIGHT_STAND2,
                },
            ),
            (
                String::from("rknight_stand3"),
                MonsterFrame {
                    frame: 2,
                    next: "rknight_stand4",
                    operations: OPS_RKNIGHT_STAND3,
                },
            ),
            (
                String::from("rknight_stand4"),
                MonsterFrame {
                    frame: 3,
                    next: "rknight_stand5",
                    operations: OPS_RKNIGHT_STAND4,
                },
            ),
            (
                String::from("rknight_stand5"),
                MonsterFrame {
                    frame: 4,
                    next: "rknight_stand6",
                    operations: OPS_RKNIGHT_STAND5,
                },
            ),
            (
                String::from("rknight_stand6"),
                MonsterFrame {
                    frame: 5,
                    next: "rknight_stand7",
                    operations: OPS_RKNIGHT_STAND6,
                },
            ),
            (
                String::from("rknight_stand7"),
                MonsterFrame {
                    frame: 6,
                    next: "rknight_stand8",
                    operations: OPS_RKNIGHT_STAND7,
                },
            ),
            (
                String::from("rknight_stand8"),
                MonsterFrame {
                    frame: 7,
                    next: "rknight_stand9",
                    operations: OPS_RKNIGHT_STAND8,
                },
            ),
            (
                String::from("rknight_stand9"),
                MonsterFrame {
                    frame: 8,
                    next: "rknight_stand1",
                    operations: OPS_RKNIGHT_STAND9,
                },
            ),
            (
                String::from("rknight_walk1"),
                MonsterFrame {
                    frame: 9,
                    next: "rknight_walk2",
                    operations: OPS_RKNIGHT_WALK1,
                },
            ),
            (
                String::from("rknight_walk2"),
                MonsterFrame {
                    frame: 10,
                    next: "rknight_walk3",
                    operations: OPS_RKNIGHT_WALK2,
                },
            ),
            (
                String::from("rknight_walk3"),
                MonsterFrame {
                    frame: 11,
                    next: "rknight_walk4",
                    operations: OPS_RKNIGHT_WALK3,
                },
            ),
            (
                String::from("rknight_walk4"),
                MonsterFrame {
                    frame: 12,
                    next: "rknight_walk5",
                    operations: OPS_RKNIGHT_WALK4,
                },
            ),
            (
                String::from("rknight_walk5"),
                MonsterFrame {
                    frame: 13,
                    next: "rknight_walk6",
                    operations: OPS_RKNIGHT_WALK5,
                },
            ),
            (
                String::from("rknight_walk6"),
                MonsterFrame {
                    frame: 14,
                    next: "rknight_walk7",
                    operations: OPS_RKNIGHT_WALK6,
                },
            ),
            (
                String::from("rknight_walk7"),
                MonsterFrame {
                    frame: 15,
                    next: "rknight_walk8",
                    operations: OPS_RKNIGHT_WALK7,
                },
            ),
            (
                String::from("rknight_walk8"),
                MonsterFrame {
                    frame: 16,
                    next: "rknight_walk9",
                    operations: OPS_RKNIGHT_WALK8,
                },
            ),
            (
                String::from("rknight_walk9"),
                MonsterFrame {
                    frame: 17,
                    next: "rknight_walk10",
                    operations: OPS_RKNIGHT_WALK9,
                },
            ),
            (
                String::from("rknight_walk10"),
                MonsterFrame {
                    frame: 18,
                    next: "rknight_walk11",
                    operations: OPS_RKNIGHT_WALK10,
                },
            ),
            (
                String::from("rknight_walk11"),
                MonsterFrame {
                    frame: 19,
                    next: "rknight_walk12",
                    operations: OPS_RKNIGHT_WALK11,
                },
            ),
            (
                String::from("rknight_walk12"),
                MonsterFrame {
                    frame: 20,
                    next: "rknight_walk13",
                    operations: OPS_RKNIGHT_WALK12,
                },
            ),
            (
                String::from("rknight_walk13"),
                MonsterFrame {
                    frame: 21,
                    next: "rknight_walk14",
                    operations: OPS_RKNIGHT_WALK13,
                },
            ),
            (
                String::from("rknight_walk14"),
                MonsterFrame {
                    frame: 22,
                    next: "rknight_walk15",
                    operations: OPS_RKNIGHT_WALK14,
                },
            ),
            (
                String::from("rknight_walk15"),
                MonsterFrame {
                    frame: 23,
                    next: "rknight_walk16",
                    operations: OPS_RKNIGHT_WALK15,
                },
            ),
            (
                String::from("rknight_walk16"),
                MonsterFrame {
                    frame: 24,
                    next: "rknight_walk17",
                    operations: OPS_RKNIGHT_WALK16,
                },
            ),
            (
                String::from("rknight_walk17"),
                MonsterFrame {
                    frame: 25,
                    next: "rknight_walk18",
                    operations: OPS_RKNIGHT_WALK17,
                },
            ),
            (
                String::from("rknight_walk18"),
                MonsterFrame {
                    frame: 26,
                    next: "rknight_walk19",
                    operations: OPS_RKNIGHT_WALK18,
                },
            ),
            (
                String::from("rknight_walk19"),
                MonsterFrame {
                    frame: 27,
                    next: "rknight_walk20",
                    operations: OPS_RKNIGHT_WALK19,
                },
            ),
            (
                String::from("rknight_walk20"),
                MonsterFrame {
                    frame: 28,
                    next: "rknight_walk1",
                    operations: OPS_RKNIGHT_WALK20,
                },
            ),
            (
                String::from("rknight_runb1"),
                MonsterFrame {
                    frame: 9,
                    next: "rknight_runb2",
                    operations: OPS_RKNIGHT_RUNB1,
                },
            ),
            (
                String::from("rknight_runb2"),
                MonsterFrame {
                    frame: 10,
                    next: "rknight_runb3",
                    operations: OPS_RKNIGHT_RUNB2,
                },
            ),
            (
                String::from("rknight_runb3"),
                MonsterFrame {
                    frame: 11,
                    next: "rknight_runb4",
                    operations: OPS_RKNIGHT_RUNB3,
                },
            ),
            (
                String::from("rknight_runb4"),
                MonsterFrame {
                    frame: 12,
                    next: "rknight_runb5",
                    operations: OPS_RKNIGHT_RUNB4,
                },
            ),
            (
                String::from("rknight_runb5"),
                MonsterFrame {
                    frame: 13,
                    next: "rknight_runb6",
                    operations: OPS_RKNIGHT_RUNB5,
                },
            ),
            (
                String::from("rknight_runb6"),
                MonsterFrame {
                    frame: 14,
                    next: "rknight_runb7",
                    operations: OPS_RKNIGHT_RUNB6,
                },
            ),
            (
                String::from("rknight_runb7"),
                MonsterFrame {
                    frame: 15,
                    next: "rknight_runb8",
                    operations: OPS_RKNIGHT_RUNB7,
                },
            ),
            (
                String::from("rknight_runb8"),
                MonsterFrame {
                    frame: 16,
                    next: "rknight_runb9",
                    operations: OPS_RKNIGHT_RUNB8,
                },
            ),
            (
                String::from("rknight_runb9"),
                MonsterFrame {
                    frame: 17,
                    next: "rknight_runb10",
                    operations: OPS_RKNIGHT_RUNB9,
                },
            ),
            (
                String::from("rknight_runb10"),
                MonsterFrame {
                    frame: 18,
                    next: "rknight_runb11",
                    operations: OPS_RKNIGHT_RUNB10,
                },
            ),
            (
                String::from("rknight_runb11"),
                MonsterFrame {
                    frame: 19,
                    next: "rknight_runb12",
                    operations: OPS_RKNIGHT_RUNB11,
                },
            ),
            (
                String::from("rknight_runb12"),
                MonsterFrame {
                    frame: 20,
                    next: "rknight_runb13",
                    operations: OPS_RKNIGHT_RUNB12,
                },
            ),
            (
                String::from("rknight_runb13"),
                MonsterFrame {
                    frame: 21,
                    next: "rknight_runb14",
                    operations: OPS_RKNIGHT_RUNB13,
                },
            ),
            (
                String::from("rknight_runb14"),
                MonsterFrame {
                    frame: 22,
                    next: "rknight_runb15",
                    operations: OPS_RKNIGHT_RUNB14,
                },
            ),
            (
                String::from("rknight_runb15"),
                MonsterFrame {
                    frame: 23,
                    next: "rknight_runb16",
                    operations: OPS_RKNIGHT_RUNB15,
                },
            ),
            (
                String::from("rknight_runb16"),
                MonsterFrame {
                    frame: 24,
                    next: "rknight_runb17",
                    operations: OPS_RKNIGHT_RUNB16,
                },
            ),
            (
                String::from("rknight_runb17"),
                MonsterFrame {
                    frame: 25,
                    next: "rknight_runb18",
                    operations: OPS_RKNIGHT_RUNB17,
                },
            ),
            (
                String::from("rknight_runb18"),
                MonsterFrame {
                    frame: 26,
                    next: "rknight_runb19",
                    operations: OPS_RKNIGHT_RUNB18,
                },
            ),
            (
                String::from("rknight_runb19"),
                MonsterFrame {
                    frame: 27,
                    next: "rknight_runb20",
                    operations: OPS_RKNIGHT_RUNB19,
                },
            ),
            (
                String::from("rknight_runb20"),
                MonsterFrame {
                    frame: 28,
                    next: "rknight_runb1",
                    operations: OPS_RKNIGHT_RUNB20,
                },
            ),
            (
                String::from("rknight_run1"),
                MonsterFrame {
                    frame: 29,
                    next: "rknight_run2",
                    operations: OPS_RKNIGHT_RUN1,
                },
            ),
            (
                String::from("rknight_run2"),
                MonsterFrame {
                    frame: 30,
                    next: "rknight_run3",
                    operations: OPS_RKNIGHT_RUN2,
                },
            ),
            (
                String::from("rknight_run3"),
                MonsterFrame {
                    frame: 31,
                    next: "rknight_run4",
                    operations: OPS_RKNIGHT_RUN3,
                },
            ),
            (
                String::from("rknight_run4"),
                MonsterFrame {
                    frame: 32,
                    next: "rknight_run5",
                    operations: OPS_RKNIGHT_RUN4,
                },
            ),
            (
                String::from("rknight_run5"),
                MonsterFrame {
                    frame: 33,
                    next: "rknight_run6",
                    operations: OPS_RKNIGHT_RUN5,
                },
            ),
            (
                String::from("rknight_run6"),
                MonsterFrame {
                    frame: 34,
                    next: "rknight_run7",
                    operations: OPS_RKNIGHT_RUN6,
                },
            ),
            (
                String::from("rknight_run7"),
                MonsterFrame {
                    frame: 35,
                    next: "rknight_run8",
                    operations: OPS_RKNIGHT_RUN7,
                },
            ),
            (
                String::from("rknight_run8"),
                MonsterFrame {
                    frame: 36,
                    next: "rknight_run1",
                    operations: OPS_RKNIGHT_RUN8,
                },
            ),
            (
                String::from("rknight_pain1"),
                MonsterFrame {
                    frame: 37,
                    next: "rknight_pain2",
                    operations: OPS_RKNIGHT_PAIN1,
                },
            ),
            (
                String::from("rknight_pain2"),
                MonsterFrame {
                    frame: 38,
                    next: "rknight_pain3",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_pain3"),
                MonsterFrame {
                    frame: 39,
                    next: "rknight_pain4",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_pain4"),
                MonsterFrame {
                    frame: 40,
                    next: "rknight_pain5",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_pain5"),
                MonsterFrame {
                    frame: 41,
                    next: "rknight_run",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die1"),
                MonsterFrame {
                    frame: 42,
                    next: "rknight_die2",
                    operations: OPS_RKNIGHT_DIE1,
                },
            ),
            (
                String::from("rknight_die2"),
                MonsterFrame {
                    frame: 43,
                    next: "rknight_die3",
                    operations: OPS_RKNIGHT_DIE2,
                },
            ),
            (
                String::from("rknight_die3"),
                MonsterFrame {
                    frame: 44,
                    next: "rknight_die4",
                    operations: OPS_RKNIGHT_DIE3,
                },
            ),
            (
                String::from("rknight_die4"),
                MonsterFrame {
                    frame: 45,
                    next: "rknight_die5",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die5"),
                MonsterFrame {
                    frame: 46,
                    next: "rknight_die6",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die6"),
                MonsterFrame {
                    frame: 47,
                    next: "rknight_die7",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die7"),
                MonsterFrame {
                    frame: 48,
                    next: "rknight_die8",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die8"),
                MonsterFrame {
                    frame: 49,
                    next: "rknight_die9",
                    operations: OPS_RKNIGHT_DIE8,
                },
            ),
            (
                String::from("rknight_die9"),
                MonsterFrame {
                    frame: 50,
                    next: "rknight_die10",
                    operations: OPS_RKNIGHT_DIE9,
                },
            ),
            (
                String::from("rknight_die10"),
                MonsterFrame {
                    frame: 51,
                    next: "rknight_die11",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die11"),
                MonsterFrame {
                    frame: 52,
                    next: "rknight_die12",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_die12"),
                MonsterFrame {
                    frame: 53,
                    next: "rknight_die12",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb1"),
                MonsterFrame {
                    frame: 54,
                    next: "rknight_dieb2",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb2"),
                MonsterFrame {
                    frame: 55,
                    next: "rknight_dieb3",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb3"),
                MonsterFrame {
                    frame: 56,
                    next: "rknight_dieb4",
                    operations: OPS_RKNIGHT_DIEB3,
                },
            ),
            (
                String::from("rknight_dieb4"),
                MonsterFrame {
                    frame: 57,
                    next: "rknight_dieb5",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb5"),
                MonsterFrame {
                    frame: 58,
                    next: "rknight_dieb6",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb6"),
                MonsterFrame {
                    frame: 59,
                    next: "rknight_dieb7",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb7"),
                MonsterFrame {
                    frame: 60,
                    next: "rknight_dieb8",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb8"),
                MonsterFrame {
                    frame: 61,
                    next: "rknight_dieb9",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_dieb9"),
                MonsterFrame {
                    frame: 62,
                    next: "rknight_dieb9",
                    operations: &[],
                },
            ),
            (
                String::from("rknight_magica1"),
                MonsterFrame {
                    frame: 79,
                    next: "rknight_magica2",
                    operations: OPS_RKNIGHT_MAGICA1,
                },
            ),
            (
                String::from("rknight_magica2"),
                MonsterFrame {
                    frame: 80,
                    next: "rknight_magica3",
                    operations: OPS_RKNIGHT_MAGICA2,
                },
            ),
            (
                String::from("rknight_magica3"),
                MonsterFrame {
                    frame: 81,
                    next: "rknight_magica4",
                    operations: OPS_RKNIGHT_MAGICA3,
                },
            ),
            (
                String::from("rknight_magica4"),
                MonsterFrame {
                    frame: 82,
                    next: "rknight_magica5",
                    operations: OPS_RKNIGHT_MAGICA4,
                },
            ),
            (
                String::from("rknight_magica5"),
                MonsterFrame {
                    frame: 83,
                    next: "rknight_magica6",
                    operations: OPS_RKNIGHT_MAGICA5,
                },
            ),
            (
                String::from("rknight_magica6"),
                MonsterFrame {
                    frame: 84,
                    next: "rknight_magica7",
                    operations: OPS_RKNIGHT_MAGICA6,
                },
            ),
            (
                String::from("rknight_magica7"),
                MonsterFrame {
                    frame: 85,
                    next: "rknight_magica8",
                    operations: OPS_RKNIGHT_MAGICA7,
                },
            ),
            (
                String::from("rknight_magica8"),
                MonsterFrame {
                    frame: 86,
                    next: "rknight_magica9",
                    operations: OPS_RKNIGHT_MAGICA8,
                },
            ),
            (
                String::from("rknight_magica9"),
                MonsterFrame {
                    frame: 87,
                    next: "rknight_magica10",
                    operations: OPS_RKNIGHT_MAGICA9,
                },
            ),
            (
                String::from("rknight_magica10"),
                MonsterFrame {
                    frame: 88,
                    next: "rknight_magica11",
                    operations: OPS_RKNIGHT_MAGICA10,
                },
            ),
            (
                String::from("rknight_magica11"),
                MonsterFrame {
                    frame: 89,
                    next: "rknight_magica12",
                    operations: OPS_RKNIGHT_MAGICA11,
                },
            ),
            (
                String::from("rknight_magica12"),
                MonsterFrame {
                    frame: 90,
                    next: "rknight_magica13",
                    operations: OPS_RKNIGHT_MAGICA12,
                },
            ),
            (
                String::from("rknight_magica13"),
                MonsterFrame {
                    frame: 91,
                    next: "rknight_magica14",
                    operations: OPS_RKNIGHT_MAGICA13,
                },
            ),
            (
                String::from("rknight_magica14"),
                MonsterFrame {
                    frame: 92,
                    next: "rknight_run",
                    operations: OPS_RKNIGHT_MAGICA14,
                },
            ),
            (
                String::from("rknight_magicb1"),
                MonsterFrame {
                    frame: 93,
                    next: "rknight_magicb2",
                    operations: OPS_RKNIGHT_MAGICB1,
                },
            ),
            (
                String::from("rknight_magicb2"),
                MonsterFrame {
                    frame: 94,
                    next: "rknight_magicb3",
                    operations: OPS_RKNIGHT_MAGICB2,
                },
            ),
            (
                String::from("rknight_magicb3"),
                MonsterFrame {
                    frame: 95,
                    next: "rknight_magicb4",
                    operations: OPS_RKNIGHT_MAGICB3,
                },
            ),
            (
                String::from("rknight_magicb4"),
                MonsterFrame {
                    frame: 96,
                    next: "rknight_magicb5",
                    operations: OPS_RKNIGHT_MAGICB4,
                },
            ),
            (
                String::from("rknight_magicb5"),
                MonsterFrame {
                    frame: 97,
                    next: "rknight_magicb6",
                    operations: OPS_RKNIGHT_MAGICB5,
                },
            ),
            (
                String::from("rknight_magicb6"),
                MonsterFrame {
                    frame: 98,
                    next: "rknight_magicb7",
                    operations: OPS_RKNIGHT_MAGICB6,
                },
            ),
            (
                String::from("rknight_magicb7"),
                MonsterFrame {
                    frame: 99,
                    next: "rknight_magicb8",
                    operations: OPS_RKNIGHT_MAGICB7,
                },
            ),
            (
                String::from("rknight_magicb8"),
                MonsterFrame {
                    frame: 100,
                    next: "rknight_magicb9",
                    operations: OPS_RKNIGHT_MAGICB8,
                },
            ),
            (
                String::from("rknight_magicb9"),
                MonsterFrame {
                    frame: 101,
                    next: "rknight_magicb10",
                    operations: OPS_RKNIGHT_MAGICB9,
                },
            ),
            (
                String::from("rknight_magicb10"),
                MonsterFrame {
                    frame: 102,
                    next: "rknight_magicb11",
                    operations: OPS_RKNIGHT_MAGICB10,
                },
            ),
            (
                String::from("rknight_magicb11"),
                MonsterFrame {
                    frame: 103,
                    next: "rknight_magicb12",
                    operations: OPS_RKNIGHT_MAGICB11,
                },
            ),
            (
                String::from("rknight_magicb12"),
                MonsterFrame {
                    frame: 104,
                    next: "rknight_magicb13",
                    operations: OPS_RKNIGHT_MAGICB12,
                },
            ),
            (
                String::from("rknight_magicb13"),
                MonsterFrame {
                    frame: 105,
                    next: "rknight_run",
                    operations: OPS_RKNIGHT_MAGICB13,
                },
            ),
            (
                String::from("rknight_magicc1"),
                MonsterFrame {
                    frame: 155,
                    next: "rknight_magicc2",
                    operations: OPS_RKNIGHT_MAGICC1,
                },
            ),
            (
                String::from("rknight_magicc2"),
                MonsterFrame {
                    frame: 156,
                    next: "rknight_magicc3",
                    operations: OPS_RKNIGHT_MAGICC2,
                },
            ),
            (
                String::from("rknight_magicc3"),
                MonsterFrame {
                    frame: 157,
                    next: "rknight_magicc4",
                    operations: OPS_RKNIGHT_MAGICC3,
                },
            ),
            (
                String::from("rknight_magicc4"),
                MonsterFrame {
                    frame: 158,
                    next: "rknight_magicc5",
                    operations: OPS_RKNIGHT_MAGICC4,
                },
            ),
            (
                String::from("rknight_magicc5"),
                MonsterFrame {
                    frame: 159,
                    next: "rknight_magicc6",
                    operations: OPS_RKNIGHT_MAGICC5,
                },
            ),
            (
                String::from("rknight_magicc6"),
                MonsterFrame {
                    frame: 160,
                    next: "rknight_magicc7",
                    operations: OPS_RKNIGHT_MAGICC6,
                },
            ),
            (
                String::from("rknight_magicc7"),
                MonsterFrame {
                    frame: 161,
                    next: "rknight_magicc8",
                    operations: OPS_RKNIGHT_MAGICC7,
                },
            ),
            (
                String::from("rknight_magicc8"),
                MonsterFrame {
                    frame: 162,
                    next: "rknight_magicc9",
                    operations: OPS_RKNIGHT_MAGICC8,
                },
            ),
            (
                String::from("rknight_magicc9"),
                MonsterFrame {
                    frame: 163,
                    next: "rknight_magicc10",
                    operations: OPS_RKNIGHT_MAGICC9,
                },
            ),
            (
                String::from("rknight_magicc10"),
                MonsterFrame {
                    frame: 164,
                    next: "rknight_magicc11",
                    operations: OPS_RKNIGHT_MAGICC10,
                },
            ),
            (
                String::from("rknight_magicc11"),
                MonsterFrame {
                    frame: 165,
                    next: "rknight_run",
                    operations: OPS_RKNIGHT_MAGICC11,
                },
            ),
            (
                String::from("rknight_slice1"),
                MonsterFrame {
                    frame: 112,
                    next: "rknight_slice2",
                    operations: OPS_RKNIGHT_SLICE1,
                },
            ),
            (
                String::from("rknight_slice2"),
                MonsterFrame {
                    frame: 113,
                    next: "rknight_slice3",
                    operations: OPS_RKNIGHT_SLICE2,
                },
            ),
            (
                String::from("rknight_slice3"),
                MonsterFrame {
                    frame: 114,
                    next: "rknight_slice4",
                    operations: OPS_RKNIGHT_SLICE3,
                },
            ),
            (
                String::from("rknight_slice4"),
                MonsterFrame {
                    frame: 115,
                    next: "rknight_slice5",
                    operations: OPS_RKNIGHT_SLICE4,
                },
            ),
            (
                String::from("rknight_slice5"),
                MonsterFrame {
                    frame: 116,
                    next: "rknight_slice6",
                    operations: OPS_RKNIGHT_SLICE5,
                },
            ),
            (
                String::from("rknight_slice6"),
                MonsterFrame {
                    frame: 117,
                    next: "rknight_slice7",
                    operations: OPS_RKNIGHT_SLICE6,
                },
            ),
            (
                String::from("rknight_slice7"),
                MonsterFrame {
                    frame: 118,
                    next: "rknight_slice8",
                    operations: OPS_RKNIGHT_SLICE7,
                },
            ),
            (
                String::from("rknight_slice8"),
                MonsterFrame {
                    frame: 119,
                    next: "rknight_slice9",
                    operations: OPS_RKNIGHT_SLICE8,
                },
            ),
            (
                String::from("rknight_slice9"),
                MonsterFrame {
                    frame: 120,
                    next: "rknight_slice10",
                    operations: OPS_RKNIGHT_SLICE9,
                },
            ),
            (
                String::from("rknight_slice10"),
                MonsterFrame {
                    frame: 121,
                    next: "rknight_run",
                    operations: OPS_RKNIGHT_SLICE10,
                },
            ),
            (
                String::from("rknight_smash1"),
                MonsterFrame {
                    frame: 122,
                    next: "rknight_smash2",
                    operations: OPS_RKNIGHT_SMASH1,
                },
            ),
            (
                String::from("rknight_smash2"),
                MonsterFrame {
                    frame: 123,
                    next: "rknight_smash3",
                    operations: OPS_RKNIGHT_SMASH2,
                },
            ),
            (
                String::from("rknight_smash3"),
                MonsterFrame {
                    frame: 124,
                    next: "rknight_smash4",
                    operations: OPS_RKNIGHT_SMASH3,
                },
            ),
            (
                String::from("rknight_smash4"),
                MonsterFrame {
                    frame: 125,
                    next: "rknight_smash5",
                    operations: OPS_RKNIGHT_SMASH4,
                },
            ),
            (
                String::from("rknight_smash5"),
                MonsterFrame {
                    frame: 126,
                    next: "rknight_smash6",
                    operations: OPS_RKNIGHT_SMASH5,
                },
            ),
            (
                String::from("rknight_smash6"),
                MonsterFrame {
                    frame: 127,
                    next: "rknight_smash7",
                    operations: OPS_RKNIGHT_SMASH6,
                },
            ),
            (
                String::from("rknight_smash7"),
                MonsterFrame {
                    frame: 128,
                    next: "rknight_smash8",
                    operations: OPS_RKNIGHT_SMASH7,
                },
            ),
            (
                String::from("rknight_smash8"),
                MonsterFrame {
                    frame: 129,
                    next: "rknight_smash9",
                    operations: OPS_RKNIGHT_SMASH8,
                },
            ),
            (
                String::from("rknight_smash9"),
                MonsterFrame {
                    frame: 130,
                    next: "rknight_smash10",
                    operations: OPS_RKNIGHT_SMASH9,
                },
            ),
            (
                String::from("rknight_smash10"),
                MonsterFrame {
                    frame: 131,
                    next: "rknight_smash11",
                    operations: OPS_RKNIGHT_SMASH10,
                },
            ),
            (
                String::from("rknight_smash11"),
                MonsterFrame {
                    frame: 132,
                    next: "rknight_run",
                    operations: OPS_RKNIGHT_SMASH11,
                },
            ),
            (
                String::from("rknight_watk1"),
                MonsterFrame {
                    frame: 133,
                    next: "rknight_watk2",
                    operations: OPS_RKNIGHT_WATK1,
                },
            ),
            (
                String::from("rknight_watk2"),
                MonsterFrame {
                    frame: 134,
                    next: "rknight_watk3",
                    operations: OPS_RKNIGHT_WATK2,
                },
            ),
            (
                String::from("rknight_watk3"),
                MonsterFrame {
                    frame: 135,
                    next: "rknight_watk4",
                    operations: OPS_RKNIGHT_WATK3,
                },
            ),
            (
                String::from("rknight_watk4"),
                MonsterFrame {
                    frame: 136,
                    next: "rknight_watk5",
                    operations: OPS_RKNIGHT_WATK4,
                },
            ),
            (
                String::from("rknight_watk5"),
                MonsterFrame {
                    frame: 137,
                    next: "rknight_watk6",
                    operations: OPS_RKNIGHT_WATK5,
                },
            ),
            (
                String::from("rknight_watk6"),
                MonsterFrame {
                    frame: 138,
                    next: "rknight_watk7",
                    operations: OPS_RKNIGHT_WATK6,
                },
            ),
            (
                String::from("rknight_watk7"),
                MonsterFrame {
                    frame: 139,
                    next: "rknight_watk8",
                    operations: OPS_RKNIGHT_WATK7,
                },
            ),
            (
                String::from("rknight_watk8"),
                MonsterFrame {
                    frame: 140,
                    next: "rknight_watk9",
                    operations: OPS_RKNIGHT_WATK8,
                },
            ),
            (
                String::from("rknight_watk9"),
                MonsterFrame {
                    frame: 141,
                    next: "rknight_watk10",
                    operations: OPS_RKNIGHT_WATK9,
                },
            ),
            (
                String::from("rknight_watk10"),
                MonsterFrame {
                    frame: 142,
                    next: "rknight_watk11",
                    operations: OPS_RKNIGHT_WATK10,
                },
            ),
            (
                String::from("rknight_watk11"),
                MonsterFrame {
                    frame: 143,
                    next: "rknight_watk12",
                    operations: OPS_RKNIGHT_WATK11,
                },
            ),
            (
                String::from("rknight_watk12"),
                MonsterFrame {
                    frame: 144,
                    next: "rknight_watk13",
                    operations: OPS_RKNIGHT_WATK12,
                },
            ),
            (
                String::from("rknight_watk13"),
                MonsterFrame {
                    frame: 145,
                    next: "rknight_watk14",
                    operations: OPS_RKNIGHT_WATK13,
                },
            ),
            (
                String::from("rknight_watk14"),
                MonsterFrame {
                    frame: 146,
                    next: "rknight_watk15",
                    operations: OPS_RKNIGHT_WATK14,
                },
            ),
            (
                String::from("rknight_watk15"),
                MonsterFrame {
                    frame: 147,
                    next: "rknight_watk16",
                    operations: OPS_RKNIGHT_WATK15,
                },
            ),
            (
                String::from("rknight_watk16"),
                MonsterFrame {
                    frame: 148,
                    next: "rknight_watk17",
                    operations: OPS_RKNIGHT_WATK16,
                },
            ),
            (
                String::from("rknight_watk17"),
                MonsterFrame {
                    frame: 149,
                    next: "rknight_watk18",
                    operations: OPS_RKNIGHT_WATK17,
                },
            ),
            (
                String::from("rknight_watk18"),
                MonsterFrame {
                    frame: 150,
                    next: "rknight_watk19",
                    operations: OPS_RKNIGHT_WATK18,
                },
            ),
            (
                String::from("rknight_watk19"),
                MonsterFrame {
                    frame: 151,
                    next: "rknight_watk20",
                    operations: OPS_RKNIGHT_WATK19,
                },
            ),
            (
                String::from("rknight_watk20"),
                MonsterFrame {
                    frame: 152,
                    next: "rknight_watk21",
                    operations: OPS_RKNIGHT_WATK20,
                },
            ),
            (
                String::from("rknight_watk21"),
                MonsterFrame {
                    frame: 153,
                    next: "rknight_watk22",
                    operations: OPS_RKNIGHT_WATK21,
                },
            ),
            (
                String::from("rknight_watk22"),
                MonsterFrame {
                    frame: 154,
                    next: "rknight_run",
                    operations: OPS_RKNIGHT_WATK22,
                },
            ),
        ])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuations_resolve() {
        assert_eq!(frames().len(), 164);
        for (name, frame) in frames() {
            // `rknight_run` is action-routed (see `super::super::rune_knight`).
            if frame.next == "rknight_run" {
                continue;
            }
            assert!(frames().contains_key(frame.next), "dangling {name} -> {}", frame.next);
        }
    }
}
