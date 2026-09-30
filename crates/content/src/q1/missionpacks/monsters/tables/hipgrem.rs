//! Hipnotic hipgrem frames (src/content/q1/missionpacks/monsters/tables/hipgrem.ts).
//!
//! quakec_hipnotic/hipgrem.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

static OPS_GREMLIN_CLAW1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "Gremlin_Melee(200)",
    },
];
static OPS_GREMLIN_CLAW7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 15.0,
}];
static OPS_GREMLIN_CLAW8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_CLAW9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_DIE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "grem/death.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_GREMLIN_DIE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 1.0,
}];
static OPS_GREMLIN_DIE11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 2.0,
}];
static OPS_GREMLIN_DIE12: &[MonsterOperation] = &[];
static OPS_GREMLIN_DIE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 2.0,
}];
static OPS_GREMLIN_DIE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 1.0,
}];
static OPS_GREMLIN_DIE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 2.0,
}];
static OPS_GREMLIN_DIE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 1.0,
}];
static OPS_GREMLIN_DIE6: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_GREMLIN_DIE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 2.0,
}];
static OPS_GREMLIN_DIE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 1.0,
}];
static OPS_GREMLIN_DIE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 2.0,
}];
static OPS_GREMLIN_FLIP1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_flip1",
}];
static OPS_GREMLIN_FLIP2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_GREMLIN_FLIP3: &[MonsterOperation] = &[];
static OPS_GREMLIN_FLIP4: &[MonsterOperation] = &[];
static OPS_GREMLIN_FLIP5: &[MonsterOperation] = &[];
static OPS_GREMLIN_FLIP6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_flip6",
}];
static OPS_GREMLIN_FLIP7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_jump11",
}];
static OPS_GREMLIN_FLIP8: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_GREMLIN_GLOOK1: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK10: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK11: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK12: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK13: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK14: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK15: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK16: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK17: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK18: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK19: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK2: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_glook20",
}];
static OPS_GREMLIN_GLOOK3: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK4: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK5: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK6: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK7: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK8: &[MonsterOperation] = &[];
static OPS_GREMLIN_GLOOK9: &[MonsterOperation] = &[];
static OPS_GREMLIN_GORGE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_GREMLIN_GORGE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_GREMLIN_GORGE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 2.0,
}];
static OPS_GREMLIN_GORGE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "Gremlin_Gorge(200)",
    },
];
static OPS_GREMLIN_GORGE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GORGE8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "Gremlin_Gorge(-200)",
    },
];
static OPS_GREMLIN_GORGE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_GUNPAIN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(4)" }];
static OPS_GREMLIN_GUNPAIN2: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(2)" }];
static OPS_GREMLIN_GUNPAIN3: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_GREMLIN_JUMP10: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_jump11",
}];
static OPS_GREMLIN_JUMP12: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP13: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP14: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP15: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP16: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_GREMLIN_JUMP3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_GREMLIN_JUMP4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_GREMLIN_JUMP5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_jump5",
}];
static OPS_GREMLIN_JUMP6: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP7: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP8: &[MonsterOperation] = &[];
static OPS_GREMLIN_JUMP9: &[MonsterOperation] = &[];
static OPS_GREMLIN_LASER1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_laser1",
}];
static OPS_GREMLIN_LASER2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_laser1",
}];
static OPS_GREMLIN_LASER3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_laser1",
}];
static OPS_GREMLIN_LASER4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_laser1",
}];
static OPS_GREMLIN_LASER5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_laser1",
}];
static OPS_GREMLIN_LASER6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_laser1",
}];
static OPS_GREMLIN_LASER7: &[MonsterOperation] = &[];
static OPS_GREMLIN_LIGHT1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "Gremlin_FireLightningGun",
}];
static OPS_GREMLIN_LIGHT2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "Gremlin_FireLightningGun",
}];
static OPS_GREMLIN_LIGHT3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "Gremlin_FireLightningGun",
}];
static OPS_GREMLIN_LIGHT4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "Gremlin_FireLightningGun",
}];
static OPS_GREMLIN_LIGHT5: &[MonsterOperation] = &[];
static OPS_GREMLIN_LOOK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look1",
}];
static OPS_GREMLIN_LOOK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_look9",
}];
static OPS_GREMLIN_LUNGE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_LUNGE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 15.0,
}];
static OPS_GREMLIN_LUNGE8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 0.0,
    },
    MonsterOperation::Action {
        name: "Gremlin_Melee(0)",
    },
];
static OPS_GREMLIN_LUNGE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_GREMLIN_NAIL1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_nail1",
}];
static OPS_GREMLIN_NAIL2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_nail1",
}];
static OPS_GREMLIN_NAIL3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_nail1",
}];
static OPS_GREMLIN_NAIL4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_nail1",
}];
static OPS_GREMLIN_NAIL5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_nail1",
}];
static OPS_GREMLIN_NAIL6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_nail1",
}];
static OPS_GREMLIN_NAIL7: &[MonsterOperation] = &[];
static OPS_GREMLIN_PAIN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(4)" }];
static OPS_GREMLIN_PAIN2: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(4)" }];
static OPS_GREMLIN_PAIN3: &[MonsterOperation] = &[MonsterOperation::Action { name: "ai_back(2)" }];
static OPS_GREMLIN_PAIN4: &[MonsterOperation] = &[];
static OPS_GREMLIN_ROCKET1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_shot1",
}];
static OPS_GREMLIN_ROCKET2: &[MonsterOperation] = &[];
static OPS_GREMLIN_ROCKET3: &[MonsterOperation] = &[];
static OPS_GREMLIN_ROCKET4: &[MonsterOperation] = &[];
static OPS_GREMLIN_ROCKET5: &[MonsterOperation] = &[];
static OPS_GREMLIN_ROCKET6: &[MonsterOperation] = &[];
static OPS_GREMLIN_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "grem/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.1),
    },
    MonsterOperation::Action { name: "gremlin_run(0)" },
];
static OPS_GREMLIN_RUN10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(12)",
}];
static OPS_GREMLIN_RUN11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(16)",
}];
static OPS_GREMLIN_RUN12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(16)",
}];
static OPS_GREMLIN_RUN13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(12)",
}];
static OPS_GREMLIN_RUN14: &[MonsterOperation] = &[MonsterOperation::Action { name: "gremlin_run(8)" }];
static OPS_GREMLIN_RUN15: &[MonsterOperation] = &[MonsterOperation::Action { name: "gremlin_run(0)" }];
static OPS_GREMLIN_RUN2: &[MonsterOperation] = &[MonsterOperation::Action { name: "gremlin_run(8)" }];
static OPS_GREMLIN_RUN3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(12)",
}];
static OPS_GREMLIN_RUN4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(16)",
}];
static OPS_GREMLIN_RUN5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(16)",
}];
static OPS_GREMLIN_RUN6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_run(12)",
}];
static OPS_GREMLIN_RUN7: &[MonsterOperation] = &[MonsterOperation::Action { name: "gremlin_run(8)" }];
static OPS_GREMLIN_RUN8: &[MonsterOperation] = &[MonsterOperation::Action { name: "gremlin_run(0)" }];
static OPS_GREMLIN_RUN9: &[MonsterOperation] = &[MonsterOperation::Action { name: "gremlin_run(8)" }];
static OPS_GREMLIN_SHOT1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_shot1",
}];
static OPS_GREMLIN_SHOT2: &[MonsterOperation] = &[];
static OPS_GREMLIN_SHOT3: &[MonsterOperation] = &[];
static OPS_GREMLIN_SHOT4: &[MonsterOperation] = &[];
static OPS_GREMLIN_SHOT5: &[MonsterOperation] = &[];
static OPS_GREMLIN_SHOT6: &[MonsterOperation] = &[];
static OPS_GREMLIN_SPAWN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_spawn1",
}];
static OPS_GREMLIN_SPAWN2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_spawn2",
}];
static OPS_GREMLIN_SPAWN3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_spawn2",
}];
static OPS_GREMLIN_SPAWN4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_spawn2",
}];
static OPS_GREMLIN_SPAWN5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_spawn2",
}];
static OPS_GREMLIN_SPAWN6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_spawn6",
}];
static OPS_GREMLIN_STAND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_STAND9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hipgrem:gremlin_stand1",
}];
static OPS_GREMLIN_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "grem/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.1),
    },
    MonsterOperation::Action {
        name: "gremlin_walk(8)",
    },
];
static OPS_GREMLIN_WALK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];
static OPS_GREMLIN_WALK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "gremlin_walk(8)",
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "gremlin_claw1",
        MonsterFrame {
            frame: 60,
            next: "gremlin_claw2",
            operations: OPS_GREMLIN_CLAW1,
        },
    ),
    (
        "gremlin_claw10",
        MonsterFrame {
            frame: 69,
            next: "gremlin_claw11",
            operations: OPS_GREMLIN_CLAW10,
        },
    ),
    (
        "gremlin_claw11",
        MonsterFrame {
            frame: 70,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_CLAW11,
        },
    ),
    (
        "gremlin_claw2",
        MonsterFrame {
            frame: 61,
            next: "gremlin_claw3",
            operations: OPS_GREMLIN_CLAW2,
        },
    ),
    (
        "gremlin_claw3",
        MonsterFrame {
            frame: 62,
            next: "gremlin_claw4",
            operations: OPS_GREMLIN_CLAW3,
        },
    ),
    (
        "gremlin_claw4",
        MonsterFrame {
            frame: 63,
            next: "gremlin_claw5",
            operations: OPS_GREMLIN_CLAW4,
        },
    ),
    (
        "gremlin_claw5",
        MonsterFrame {
            frame: 64,
            next: "gremlin_claw6",
            operations: OPS_GREMLIN_CLAW5,
        },
    ),
    (
        "gremlin_claw6",
        MonsterFrame {
            frame: 65,
            next: "gremlin_claw7",
            operations: OPS_GREMLIN_CLAW6,
        },
    ),
    (
        "gremlin_claw7",
        MonsterFrame {
            frame: 66,
            next: "gremlin_claw8",
            operations: OPS_GREMLIN_CLAW7,
        },
    ),
    (
        "gremlin_claw8",
        MonsterFrame {
            frame: 67,
            next: "gremlin_claw9",
            operations: OPS_GREMLIN_CLAW8,
        },
    ),
    (
        "gremlin_claw9",
        MonsterFrame {
            frame: 68,
            next: "gremlin_claw10",
            operations: OPS_GREMLIN_CLAW9,
        },
    ),
    (
        "gremlin_die1",
        MonsterFrame {
            frame: 104,
            next: "gremlin_die2",
            operations: OPS_GREMLIN_DIE1,
        },
    ),
    (
        "gremlin_die10",
        MonsterFrame {
            frame: 113,
            next: "gremlin_die11",
            operations: OPS_GREMLIN_DIE10,
        },
    ),
    (
        "gremlin_die11",
        MonsterFrame {
            frame: 114,
            next: "gremlin_die12",
            operations: OPS_GREMLIN_DIE11,
        },
    ),
    (
        "gremlin_die12",
        MonsterFrame {
            frame: 115,
            next: "gremlin_die12",
            operations: OPS_GREMLIN_DIE12,
        },
    ),
    (
        "gremlin_die2",
        MonsterFrame {
            frame: 105,
            next: "gremlin_die3",
            operations: OPS_GREMLIN_DIE2,
        },
    ),
    (
        "gremlin_die3",
        MonsterFrame {
            frame: 106,
            next: "gremlin_die4",
            operations: OPS_GREMLIN_DIE3,
        },
    ),
    (
        "gremlin_die4",
        MonsterFrame {
            frame: 107,
            next: "gremlin_die5",
            operations: OPS_GREMLIN_DIE4,
        },
    ),
    (
        "gremlin_die5",
        MonsterFrame {
            frame: 108,
            next: "gremlin_die6",
            operations: OPS_GREMLIN_DIE5,
        },
    ),
    (
        "gremlin_die6",
        MonsterFrame {
            frame: 109,
            next: "gremlin_die7",
            operations: OPS_GREMLIN_DIE6,
        },
    ),
    (
        "gremlin_die7",
        MonsterFrame {
            frame: 110,
            next: "gremlin_die8",
            operations: OPS_GREMLIN_DIE7,
        },
    ),
    (
        "gremlin_die8",
        MonsterFrame {
            frame: 111,
            next: "gremlin_die9",
            operations: OPS_GREMLIN_DIE8,
        },
    ),
    (
        "gremlin_die9",
        MonsterFrame {
            frame: 112,
            next: "gremlin_die10",
            operations: OPS_GREMLIN_DIE9,
        },
    ),
    (
        "gremlin_flip1",
        MonsterFrame {
            frame: 116,
            next: "gremlin_flip2",
            operations: OPS_GREMLIN_FLIP1,
        },
    ),
    (
        "gremlin_flip2",
        MonsterFrame {
            frame: 117,
            next: "gremlin_flip3",
            operations: OPS_GREMLIN_FLIP2,
        },
    ),
    (
        "gremlin_flip3",
        MonsterFrame {
            frame: 118,
            next: "gremlin_flip4",
            operations: OPS_GREMLIN_FLIP3,
        },
    ),
    (
        "gremlin_flip4",
        MonsterFrame {
            frame: 119,
            next: "gremlin_flip5",
            operations: OPS_GREMLIN_FLIP4,
        },
    ),
    (
        "gremlin_flip5",
        MonsterFrame {
            frame: 120,
            next: "gremlin_flip6",
            operations: OPS_GREMLIN_FLIP5,
        },
    ),
    (
        "gremlin_flip6",
        MonsterFrame {
            frame: 121,
            next: "gremlin_flip7",
            operations: OPS_GREMLIN_FLIP6,
        },
    ),
    (
        "gremlin_flip7",
        MonsterFrame {
            frame: 122,
            next: "gremlin_gib",
            operations: OPS_GREMLIN_FLIP7,
        },
    ),
    (
        "gremlin_flip8",
        MonsterFrame {
            frame: 123,
            next: "gremlin_flip8",
            operations: OPS_GREMLIN_FLIP8,
        },
    ),
    (
        "gremlin_glook1",
        MonsterFrame {
            frame: 141,
            next: "gremlin_glook2",
            operations: OPS_GREMLIN_GLOOK1,
        },
    ),
    (
        "gremlin_glook10",
        MonsterFrame {
            frame: 150,
            next: "gremlin_glook11",
            operations: OPS_GREMLIN_GLOOK10,
        },
    ),
    (
        "gremlin_glook11",
        MonsterFrame {
            frame: 151,
            next: "gremlin_glook12",
            operations: OPS_GREMLIN_GLOOK11,
        },
    ),
    (
        "gremlin_glook12",
        MonsterFrame {
            frame: 152,
            next: "gremlin_glook13",
            operations: OPS_GREMLIN_GLOOK12,
        },
    ),
    (
        "gremlin_glook13",
        MonsterFrame {
            frame: 153,
            next: "gremlin_glook14",
            operations: OPS_GREMLIN_GLOOK13,
        },
    ),
    (
        "gremlin_glook14",
        MonsterFrame {
            frame: 154,
            next: "gremlin_glook15",
            operations: OPS_GREMLIN_GLOOK14,
        },
    ),
    (
        "gremlin_glook15",
        MonsterFrame {
            frame: 155,
            next: "gremlin_glook16",
            operations: OPS_GREMLIN_GLOOK15,
        },
    ),
    (
        "gremlin_glook16",
        MonsterFrame {
            frame: 156,
            next: "gremlin_glook17",
            operations: OPS_GREMLIN_GLOOK16,
        },
    ),
    (
        "gremlin_glook17",
        MonsterFrame {
            frame: 157,
            next: "gremlin_glook18",
            operations: OPS_GREMLIN_GLOOK17,
        },
    ),
    (
        "gremlin_glook18",
        MonsterFrame {
            frame: 158,
            next: "gremlin_glook19",
            operations: OPS_GREMLIN_GLOOK18,
        },
    ),
    (
        "gremlin_glook19",
        MonsterFrame {
            frame: 159,
            next: "gremlin_glook20",
            operations: OPS_GREMLIN_GLOOK19,
        },
    ),
    (
        "gremlin_glook2",
        MonsterFrame {
            frame: 142,
            next: "gremlin_glook3",
            operations: OPS_GREMLIN_GLOOK2,
        },
    ),
    (
        "gremlin_glook20",
        MonsterFrame {
            frame: 160,
            next: "gremlin_glook20",
            operations: OPS_GREMLIN_GLOOK20,
        },
    ),
    (
        "gremlin_glook3",
        MonsterFrame {
            frame: 143,
            next: "gremlin_glook4",
            operations: OPS_GREMLIN_GLOOK3,
        },
    ),
    (
        "gremlin_glook4",
        MonsterFrame {
            frame: 144,
            next: "gremlin_glook5",
            operations: OPS_GREMLIN_GLOOK4,
        },
    ),
    (
        "gremlin_glook5",
        MonsterFrame {
            frame: 145,
            next: "gremlin_glook6",
            operations: OPS_GREMLIN_GLOOK5,
        },
    ),
    (
        "gremlin_glook6",
        MonsterFrame {
            frame: 146,
            next: "gremlin_glook7",
            operations: OPS_GREMLIN_GLOOK6,
        },
    ),
    (
        "gremlin_glook7",
        MonsterFrame {
            frame: 147,
            next: "gremlin_glook8",
            operations: OPS_GREMLIN_GLOOK7,
        },
    ),
    (
        "gremlin_glook8",
        MonsterFrame {
            frame: 148,
            next: "gremlin_glook9",
            operations: OPS_GREMLIN_GLOOK8,
        },
    ),
    (
        "gremlin_glook9",
        MonsterFrame {
            frame: 149,
            next: "gremlin_glook10",
            operations: OPS_GREMLIN_GLOOK9,
        },
    ),
    (
        "gremlin_gorge1",
        MonsterFrame {
            frame: 71,
            next: "gremlin_gorge2",
            operations: OPS_GREMLIN_GORGE1,
        },
    ),
    (
        "gremlin_gorge10",
        MonsterFrame {
            frame: 80,
            next: "gremlin_gorge11",
            operations: OPS_GREMLIN_GORGE10,
        },
    ),
    (
        "gremlin_gorge11",
        MonsterFrame {
            frame: 81,
            next: "gremlin_gorge12",
            operations: OPS_GREMLIN_GORGE11,
        },
    ),
    (
        "gremlin_gorge12",
        MonsterFrame {
            frame: 82,
            next: "gremlin_gorge13",
            operations: OPS_GREMLIN_GORGE12,
        },
    ),
    (
        "gremlin_gorge13",
        MonsterFrame {
            frame: 83,
            next: "gremlin_gorge1",
            operations: OPS_GREMLIN_GORGE13,
        },
    ),
    (
        "gremlin_gorge2",
        MonsterFrame {
            frame: 72,
            next: "gremlin_gorge3",
            operations: OPS_GREMLIN_GORGE2,
        },
    ),
    (
        "gremlin_gorge3",
        MonsterFrame {
            frame: 73,
            next: "gremlin_gorge4",
            operations: OPS_GREMLIN_GORGE3,
        },
    ),
    (
        "gremlin_gorge4",
        MonsterFrame {
            frame: 74,
            next: "gremlin_gorge5",
            operations: OPS_GREMLIN_GORGE4,
        },
    ),
    (
        "gremlin_gorge5",
        MonsterFrame {
            frame: 75,
            next: "gremlin_gorge6",
            operations: OPS_GREMLIN_GORGE5,
        },
    ),
    (
        "gremlin_gorge6",
        MonsterFrame {
            frame: 76,
            next: "gremlin_gorge7",
            operations: OPS_GREMLIN_GORGE6,
        },
    ),
    (
        "gremlin_gorge7",
        MonsterFrame {
            frame: 77,
            next: "gremlin_gorge8",
            operations: OPS_GREMLIN_GORGE7,
        },
    ),
    (
        "gremlin_gorge8",
        MonsterFrame {
            frame: 78,
            next: "gremlin_gorge9",
            operations: OPS_GREMLIN_GORGE8,
        },
    ),
    (
        "gremlin_gorge9",
        MonsterFrame {
            frame: 79,
            next: "gremlin_gorge10",
            operations: OPS_GREMLIN_GORGE9,
        },
    ),
    (
        "gremlin_gunpain1",
        MonsterFrame {
            frame: 161,
            next: "gremlin_gunpain2",
            operations: OPS_GREMLIN_GUNPAIN1,
        },
    ),
    (
        "gremlin_gunpain2",
        MonsterFrame {
            frame: 162,
            next: "gremlin_gunpain3",
            operations: OPS_GREMLIN_GUNPAIN2,
        },
    ),
    (
        "gremlin_gunpain3",
        MonsterFrame {
            frame: 163,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_GUNPAIN3,
        },
    ),
    (
        "gremlin_jump1",
        MonsterFrame {
            frame: 44,
            next: "gremlin_jump2",
            operations: OPS_GREMLIN_JUMP1,
        },
    ),
    (
        "gremlin_jump10",
        MonsterFrame {
            frame: 53,
            next: "gremlin_jump11",
            operations: OPS_GREMLIN_JUMP10,
        },
    ),
    (
        "gremlin_jump11",
        MonsterFrame {
            frame: 54,
            next: "gremlin_jump1",
            operations: OPS_GREMLIN_JUMP11,
        },
    ),
    (
        "gremlin_jump12",
        MonsterFrame {
            frame: 55,
            next: "gremlin_jump13",
            operations: OPS_GREMLIN_JUMP12,
        },
    ),
    (
        "gremlin_jump13",
        MonsterFrame {
            frame: 56,
            next: "gremlin_jump14",
            operations: OPS_GREMLIN_JUMP13,
        },
    ),
    (
        "gremlin_jump14",
        MonsterFrame {
            frame: 57,
            next: "gremlin_jump15",
            operations: OPS_GREMLIN_JUMP14,
        },
    ),
    (
        "gremlin_jump15",
        MonsterFrame {
            frame: 58,
            next: "gremlin_jump16",
            operations: OPS_GREMLIN_JUMP15,
        },
    ),
    (
        "gremlin_jump16",
        MonsterFrame {
            frame: 59,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_JUMP16,
        },
    ),
    (
        "gremlin_jump2",
        MonsterFrame {
            frame: 45,
            next: "gremlin_jump3",
            operations: OPS_GREMLIN_JUMP2,
        },
    ),
    (
        "gremlin_jump3",
        MonsterFrame {
            frame: 46,
            next: "gremlin_jump4",
            operations: OPS_GREMLIN_JUMP3,
        },
    ),
    (
        "gremlin_jump4",
        MonsterFrame {
            frame: 47,
            next: "gremlin_jump5",
            operations: OPS_GREMLIN_JUMP4,
        },
    ),
    (
        "gremlin_jump5",
        MonsterFrame {
            frame: 48,
            next: "gremlin_jump6",
            operations: OPS_GREMLIN_JUMP5,
        },
    ),
    (
        "gremlin_jump6",
        MonsterFrame {
            frame: 49,
            next: "gremlin_jump7",
            operations: OPS_GREMLIN_JUMP6,
        },
    ),
    (
        "gremlin_jump7",
        MonsterFrame {
            frame: 50,
            next: "gremlin_jump8",
            operations: OPS_GREMLIN_JUMP7,
        },
    ),
    (
        "gremlin_jump8",
        MonsterFrame {
            frame: 51,
            next: "gremlin_jump9",
            operations: OPS_GREMLIN_JUMP8,
        },
    ),
    (
        "gremlin_jump9",
        MonsterFrame {
            frame: 52,
            next: "gremlin_jump10",
            operations: OPS_GREMLIN_JUMP9,
        },
    ),
    (
        "gremlin_laser1",
        MonsterFrame {
            frame: 135,
            next: "gremlin_laser2",
            operations: OPS_GREMLIN_LASER1,
        },
    ),
    (
        "gremlin_laser2",
        MonsterFrame {
            frame: 135,
            next: "gremlin_laser3",
            operations: OPS_GREMLIN_LASER2,
        },
    ),
    (
        "gremlin_laser3",
        MonsterFrame {
            frame: 135,
            next: "gremlin_laser4",
            operations: OPS_GREMLIN_LASER3,
        },
    ),
    (
        "gremlin_laser4",
        MonsterFrame {
            frame: 135,
            next: "gremlin_laser5",
            operations: OPS_GREMLIN_LASER4,
        },
    ),
    (
        "gremlin_laser5",
        MonsterFrame {
            frame: 135,
            next: "gremlin_laser6",
            operations: OPS_GREMLIN_LASER5,
        },
    ),
    (
        "gremlin_laser6",
        MonsterFrame {
            frame: 135,
            next: "gremlin_laser7",
            operations: OPS_GREMLIN_LASER6,
        },
    ),
    (
        "gremlin_laser7",
        MonsterFrame {
            frame: 135,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_LASER7,
        },
    ),
    (
        "gremlin_light1",
        MonsterFrame {
            frame: 135,
            next: "gremlin_light2",
            operations: OPS_GREMLIN_LIGHT1,
        },
    ),
    (
        "gremlin_light2",
        MonsterFrame {
            frame: 135,
            next: "gremlin_light3",
            operations: OPS_GREMLIN_LIGHT2,
        },
    ),
    (
        "gremlin_light3",
        MonsterFrame {
            frame: 135,
            next: "gremlin_light4",
            operations: OPS_GREMLIN_LIGHT3,
        },
    ),
    (
        "gremlin_light4",
        MonsterFrame {
            frame: 135,
            next: "gremlin_light5",
            operations: OPS_GREMLIN_LIGHT4,
        },
    ),
    (
        "gremlin_light5",
        MonsterFrame {
            frame: 135,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_LIGHT5,
        },
    ),
    (
        "gremlin_look1",
        MonsterFrame {
            frame: 90,
            next: "gremlin_look2",
            operations: OPS_GREMLIN_LOOK1,
        },
    ),
    (
        "gremlin_look2",
        MonsterFrame {
            frame: 91,
            next: "gremlin_look3",
            operations: OPS_GREMLIN_LOOK2,
        },
    ),
    (
        "gremlin_look3",
        MonsterFrame {
            frame: 92,
            next: "gremlin_look4",
            operations: OPS_GREMLIN_LOOK3,
        },
    ),
    (
        "gremlin_look4",
        MonsterFrame {
            frame: 93,
            next: "gremlin_look5",
            operations: OPS_GREMLIN_LOOK4,
        },
    ),
    (
        "gremlin_look5",
        MonsterFrame {
            frame: 94,
            next: "gremlin_look6",
            operations: OPS_GREMLIN_LOOK5,
        },
    ),
    (
        "gremlin_look6",
        MonsterFrame {
            frame: 95,
            next: "gremlin_look7",
            operations: OPS_GREMLIN_LOOK6,
        },
    ),
    (
        "gremlin_look7",
        MonsterFrame {
            frame: 96,
            next: "gremlin_look8",
            operations: OPS_GREMLIN_LOOK7,
        },
    ),
    (
        "gremlin_look8",
        MonsterFrame {
            frame: 97,
            next: "gremlin_look9",
            operations: OPS_GREMLIN_LOOK8,
        },
    ),
    (
        "gremlin_look9",
        MonsterFrame {
            frame: 98,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_LOOK9,
        },
    ),
    (
        "gremlin_lunge1",
        MonsterFrame {
            frame: 124,
            next: "gremlin_lunge2",
            operations: OPS_GREMLIN_LUNGE1,
        },
    ),
    (
        "gremlin_lunge10",
        MonsterFrame {
            frame: 133,
            next: "gremlin_lunge11",
            operations: OPS_GREMLIN_LUNGE10,
        },
    ),
    (
        "gremlin_lunge11",
        MonsterFrame {
            frame: 134,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_LUNGE11,
        },
    ),
    (
        "gremlin_lunge2",
        MonsterFrame {
            frame: 125,
            next: "gremlin_lunge3",
            operations: OPS_GREMLIN_LUNGE2,
        },
    ),
    (
        "gremlin_lunge3",
        MonsterFrame {
            frame: 126,
            next: "gremlin_lunge4",
            operations: OPS_GREMLIN_LUNGE3,
        },
    ),
    (
        "gremlin_lunge4",
        MonsterFrame {
            frame: 127,
            next: "gremlin_lunge5",
            operations: OPS_GREMLIN_LUNGE4,
        },
    ),
    (
        "gremlin_lunge5",
        MonsterFrame {
            frame: 128,
            next: "gremlin_lunge6",
            operations: OPS_GREMLIN_LUNGE5,
        },
    ),
    (
        "gremlin_lunge6",
        MonsterFrame {
            frame: 129,
            next: "gremlin_lunge7",
            operations: OPS_GREMLIN_LUNGE6,
        },
    ),
    (
        "gremlin_lunge7",
        MonsterFrame {
            frame: 130,
            next: "gremlin_lunge8",
            operations: OPS_GREMLIN_LUNGE7,
        },
    ),
    (
        "gremlin_lunge8",
        MonsterFrame {
            frame: 131,
            next: "gremlin_lunge9",
            operations: OPS_GREMLIN_LUNGE8,
        },
    ),
    (
        "gremlin_lunge9",
        MonsterFrame {
            frame: 132,
            next: "gremlin_lunge10",
            operations: OPS_GREMLIN_LUNGE9,
        },
    ),
    (
        "gremlin_nail1",
        MonsterFrame {
            frame: 135,
            next: "gremlin_nail2",
            operations: OPS_GREMLIN_NAIL1,
        },
    ),
    (
        "gremlin_nail2",
        MonsterFrame {
            frame: 135,
            next: "gremlin_nail3",
            operations: OPS_GREMLIN_NAIL2,
        },
    ),
    (
        "gremlin_nail3",
        MonsterFrame {
            frame: 135,
            next: "gremlin_nail4",
            operations: OPS_GREMLIN_NAIL3,
        },
    ),
    (
        "gremlin_nail4",
        MonsterFrame {
            frame: 135,
            next: "gremlin_nail5",
            operations: OPS_GREMLIN_NAIL4,
        },
    ),
    (
        "gremlin_nail5",
        MonsterFrame {
            frame: 135,
            next: "gremlin_nail6",
            operations: OPS_GREMLIN_NAIL5,
        },
    ),
    (
        "gremlin_nail6",
        MonsterFrame {
            frame: 135,
            next: "gremlin_nail7",
            operations: OPS_GREMLIN_NAIL6,
        },
    ),
    (
        "gremlin_nail7",
        MonsterFrame {
            frame: 135,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_NAIL7,
        },
    ),
    (
        "gremlin_pain1",
        MonsterFrame {
            frame: 100,
            next: "gremlin_pain2",
            operations: OPS_GREMLIN_PAIN1,
        },
    ),
    (
        "gremlin_pain2",
        MonsterFrame {
            frame: 101,
            next: "gremlin_pain3",
            operations: OPS_GREMLIN_PAIN2,
        },
    ),
    (
        "gremlin_pain3",
        MonsterFrame {
            frame: 102,
            next: "gremlin_pain4",
            operations: OPS_GREMLIN_PAIN3,
        },
    ),
    (
        "gremlin_pain4",
        MonsterFrame {
            frame: 103,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_PAIN4,
        },
    ),
    (
        "gremlin_rocket1",
        MonsterFrame {
            frame: 135,
            next: "gremlin_rocket2",
            operations: OPS_GREMLIN_ROCKET1,
        },
    ),
    (
        "gremlin_rocket2",
        MonsterFrame {
            frame: 136,
            next: "gremlin_rocket3",
            operations: OPS_GREMLIN_ROCKET2,
        },
    ),
    (
        "gremlin_rocket3",
        MonsterFrame {
            frame: 137,
            next: "gremlin_rocket4",
            operations: OPS_GREMLIN_ROCKET3,
        },
    ),
    (
        "gremlin_rocket4",
        MonsterFrame {
            frame: 138,
            next: "gremlin_rocket5",
            operations: OPS_GREMLIN_ROCKET4,
        },
    ),
    (
        "gremlin_rocket5",
        MonsterFrame {
            frame: 139,
            next: "gremlin_rocket6",
            operations: OPS_GREMLIN_ROCKET5,
        },
    ),
    (
        "gremlin_rocket6",
        MonsterFrame {
            frame: 140,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_ROCKET6,
        },
    ),
    (
        "gremlin_run1",
        MonsterFrame {
            frame: 29,
            next: "gremlin_run2",
            operations: OPS_GREMLIN_RUN1,
        },
    ),
    (
        "gremlin_run10",
        MonsterFrame {
            frame: 38,
            next: "gremlin_run11",
            operations: OPS_GREMLIN_RUN10,
        },
    ),
    (
        "gremlin_run11",
        MonsterFrame {
            frame: 39,
            next: "gremlin_run12",
            operations: OPS_GREMLIN_RUN11,
        },
    ),
    (
        "gremlin_run12",
        MonsterFrame {
            frame: 40,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_RUN12,
        },
    ),
    (
        "gremlin_run13",
        MonsterFrame {
            frame: 41,
            next: "gremlin_run14",
            operations: OPS_GREMLIN_RUN13,
        },
    ),
    (
        "gremlin_run14",
        MonsterFrame {
            frame: 42,
            next: "gremlin_run15",
            operations: OPS_GREMLIN_RUN14,
        },
    ),
    (
        "gremlin_run15",
        MonsterFrame {
            frame: 43,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_RUN15,
        },
    ),
    (
        "gremlin_run2",
        MonsterFrame {
            frame: 30,
            next: "gremlin_run3",
            operations: OPS_GREMLIN_RUN2,
        },
    ),
    (
        "gremlin_run3",
        MonsterFrame {
            frame: 31,
            next: "gremlin_run4",
            operations: OPS_GREMLIN_RUN3,
        },
    ),
    (
        "gremlin_run4",
        MonsterFrame {
            frame: 32,
            next: "gremlin_run5",
            operations: OPS_GREMLIN_RUN4,
        },
    ),
    (
        "gremlin_run5",
        MonsterFrame {
            frame: 33,
            next: "gremlin_run6",
            operations: OPS_GREMLIN_RUN5,
        },
    ),
    (
        "gremlin_run6",
        MonsterFrame {
            frame: 34,
            next: "gremlin_run7",
            operations: OPS_GREMLIN_RUN6,
        },
    ),
    (
        "gremlin_run7",
        MonsterFrame {
            frame: 35,
            next: "gremlin_run8",
            operations: OPS_GREMLIN_RUN7,
        },
    ),
    (
        "gremlin_run8",
        MonsterFrame {
            frame: 36,
            next: "gremlin_run9",
            operations: OPS_GREMLIN_RUN8,
        },
    ),
    (
        "gremlin_run9",
        MonsterFrame {
            frame: 37,
            next: "gremlin_run10",
            operations: OPS_GREMLIN_RUN9,
        },
    ),
    (
        "gremlin_shot1",
        MonsterFrame {
            frame: 135,
            next: "gremlin_shot2",
            operations: OPS_GREMLIN_SHOT1,
        },
    ),
    (
        "gremlin_shot2",
        MonsterFrame {
            frame: 136,
            next: "gremlin_shot3",
            operations: OPS_GREMLIN_SHOT2,
        },
    ),
    (
        "gremlin_shot3",
        MonsterFrame {
            frame: 137,
            next: "gremlin_shot4",
            operations: OPS_GREMLIN_SHOT3,
        },
    ),
    (
        "gremlin_shot4",
        MonsterFrame {
            frame: 138,
            next: "gremlin_shot5",
            operations: OPS_GREMLIN_SHOT4,
        },
    ),
    (
        "gremlin_shot5",
        MonsterFrame {
            frame: 139,
            next: "gremlin_shot6",
            operations: OPS_GREMLIN_SHOT5,
        },
    ),
    (
        "gremlin_shot6",
        MonsterFrame {
            frame: 140,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_SHOT6,
        },
    ),
    (
        "gremlin_spawn1",
        MonsterFrame {
            frame: 84,
            next: "gremlin_spawn2",
            operations: OPS_GREMLIN_SPAWN1,
        },
    ),
    (
        "gremlin_spawn2",
        MonsterFrame {
            frame: 85,
            next: "gremlin_spawn3",
            operations: OPS_GREMLIN_SPAWN2,
        },
    ),
    (
        "gremlin_spawn3",
        MonsterFrame {
            frame: 86,
            next: "gremlin_spawn4",
            operations: OPS_GREMLIN_SPAWN3,
        },
    ),
    (
        "gremlin_spawn4",
        MonsterFrame {
            frame: 87,
            next: "gremlin_spawn5",
            operations: OPS_GREMLIN_SPAWN4,
        },
    ),
    (
        "gremlin_spawn5",
        MonsterFrame {
            frame: 88,
            next: "gremlin_spawn6",
            operations: OPS_GREMLIN_SPAWN5,
        },
    ),
    (
        "gremlin_spawn6",
        MonsterFrame {
            frame: 89,
            next: "gremlin_run1",
            operations: OPS_GREMLIN_SPAWN6,
        },
    ),
    (
        "gremlin_stand1",
        MonsterFrame {
            frame: 0,
            next: "gremlin_stand2",
            operations: OPS_GREMLIN_STAND1,
        },
    ),
    (
        "gremlin_stand10",
        MonsterFrame {
            frame: 9,
            next: "gremlin_stand11",
            operations: OPS_GREMLIN_STAND10,
        },
    ),
    (
        "gremlin_stand11",
        MonsterFrame {
            frame: 10,
            next: "gremlin_stand12",
            operations: OPS_GREMLIN_STAND11,
        },
    ),
    (
        "gremlin_stand12",
        MonsterFrame {
            frame: 11,
            next: "gremlin_stand13",
            operations: OPS_GREMLIN_STAND12,
        },
    ),
    (
        "gremlin_stand13",
        MonsterFrame {
            frame: 12,
            next: "gremlin_stand14",
            operations: OPS_GREMLIN_STAND13,
        },
    ),
    (
        "gremlin_stand14",
        MonsterFrame {
            frame: 13,
            next: "gremlin_stand15",
            operations: OPS_GREMLIN_STAND14,
        },
    ),
    (
        "gremlin_stand15",
        MonsterFrame {
            frame: 14,
            next: "gremlin_stand16",
            operations: OPS_GREMLIN_STAND15,
        },
    ),
    (
        "gremlin_stand16",
        MonsterFrame {
            frame: 15,
            next: "gremlin_stand17",
            operations: OPS_GREMLIN_STAND16,
        },
    ),
    (
        "gremlin_stand17",
        MonsterFrame {
            frame: 16,
            next: "gremlin_stand1",
            operations: OPS_GREMLIN_STAND17,
        },
    ),
    (
        "gremlin_stand2",
        MonsterFrame {
            frame: 1,
            next: "gremlin_stand3",
            operations: OPS_GREMLIN_STAND2,
        },
    ),
    (
        "gremlin_stand3",
        MonsterFrame {
            frame: 2,
            next: "gremlin_stand4",
            operations: OPS_GREMLIN_STAND3,
        },
    ),
    (
        "gremlin_stand4",
        MonsterFrame {
            frame: 3,
            next: "gremlin_stand5",
            operations: OPS_GREMLIN_STAND4,
        },
    ),
    (
        "gremlin_stand5",
        MonsterFrame {
            frame: 4,
            next: "gremlin_stand6",
            operations: OPS_GREMLIN_STAND5,
        },
    ),
    (
        "gremlin_stand6",
        MonsterFrame {
            frame: 5,
            next: "gremlin_stand7",
            operations: OPS_GREMLIN_STAND6,
        },
    ),
    (
        "gremlin_stand7",
        MonsterFrame {
            frame: 6,
            next: "gremlin_stand8",
            operations: OPS_GREMLIN_STAND7,
        },
    ),
    (
        "gremlin_stand8",
        MonsterFrame {
            frame: 7,
            next: "gremlin_stand9",
            operations: OPS_GREMLIN_STAND8,
        },
    ),
    (
        "gremlin_stand9",
        MonsterFrame {
            frame: 8,
            next: "gremlin_stand10",
            operations: OPS_GREMLIN_STAND9,
        },
    ),
    (
        "gremlin_walk1",
        MonsterFrame {
            frame: 17,
            next: "gremlin_walk2",
            operations: OPS_GREMLIN_WALK1,
        },
    ),
    (
        "gremlin_walk10",
        MonsterFrame {
            frame: 26,
            next: "gremlin_walk11",
            operations: OPS_GREMLIN_WALK10,
        },
    ),
    (
        "gremlin_walk11",
        MonsterFrame {
            frame: 27,
            next: "gremlin_walk12",
            operations: OPS_GREMLIN_WALK11,
        },
    ),
    (
        "gremlin_walk12",
        MonsterFrame {
            frame: 28,
            next: "gremlin_walk1",
            operations: OPS_GREMLIN_WALK12,
        },
    ),
    (
        "gremlin_walk2",
        MonsterFrame {
            frame: 18,
            next: "gremlin_walk3",
            operations: OPS_GREMLIN_WALK2,
        },
    ),
    (
        "gremlin_walk3",
        MonsterFrame {
            frame: 19,
            next: "gremlin_walk4",
            operations: OPS_GREMLIN_WALK3,
        },
    ),
    (
        "gremlin_walk4",
        MonsterFrame {
            frame: 20,
            next: "gremlin_walk5",
            operations: OPS_GREMLIN_WALK4,
        },
    ),
    (
        "gremlin_walk5",
        MonsterFrame {
            frame: 21,
            next: "gremlin_walk6",
            operations: OPS_GREMLIN_WALK5,
        },
    ),
    (
        "gremlin_walk6",
        MonsterFrame {
            frame: 22,
            next: "gremlin_walk7",
            operations: OPS_GREMLIN_WALK6,
        },
    ),
    (
        "gremlin_walk7",
        MonsterFrame {
            frame: 23,
            next: "gremlin_walk8",
            operations: OPS_GREMLIN_WALK7,
        },
    ),
    (
        "gremlin_walk8",
        MonsterFrame {
            frame: 24,
            next: "gremlin_walk9",
            operations: OPS_GREMLIN_WALK8,
        },
    ),
    (
        "gremlin_walk9",
        MonsterFrame {
            frame: 25,
            next: "gremlin_walk10",
            operations: OPS_GREMLIN_WALK9,
        },
    ),
];

/// Look up a hipgrem frame by name.
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
        assert_eq!(FRAMES.len(), 188);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("gremlin_claw1").expect("first frame");
        assert_eq!((head.frame, head.next), (60, "gremlin_claw2"));
        let tail = frame("gremlin_walk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (25, "gremlin_walk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
