//! Base monster animation frames (`src/content/q1/base/frames.ts`).
//!
//! Animation declarations from id Software Quake and rerelease QuakeC.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.
//!
//! This table is generated from the donor `frames.ts` (same entry order
//! would also work, but the sorted slice keeps lookups logarithmic).

use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

use super::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};

static OPS_BOSS_DEATH1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/death.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_BOSS_DEATH10: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_death10" }];
static OPS_BOSS_DEATH2: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH3: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH4: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH5: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH6: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH7: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH8: &[MonsterOperation] = &[];
static OPS_BOSS_DEATH9: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "boss1/out1.wav",
        channel: Q1SoundChannel::Body,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Action { name: "boss_death9" },
];
static OPS_BOSS_IDLE1: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle1" }];
static OPS_BOSS_IDLE10: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle10" }];
static OPS_BOSS_IDLE11: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle11" }];
static OPS_BOSS_IDLE12: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle12" }];
static OPS_BOSS_IDLE13: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle13" }];
static OPS_BOSS_IDLE14: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle14" }];
static OPS_BOSS_IDLE15: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle15" }];
static OPS_BOSS_IDLE16: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle16" }];
static OPS_BOSS_IDLE17: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle17" }];
static OPS_BOSS_IDLE18: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle18" }];
static OPS_BOSS_IDLE19: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle19" }];
static OPS_BOSS_IDLE2: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle2" }];
static OPS_BOSS_IDLE20: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle20" }];
static OPS_BOSS_IDLE21: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle21" }];
static OPS_BOSS_IDLE22: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle22" }];
static OPS_BOSS_IDLE23: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle23" }];
static OPS_BOSS_IDLE24: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle24" }];
static OPS_BOSS_IDLE25: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle25" }];
static OPS_BOSS_IDLE26: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle26" }];
static OPS_BOSS_IDLE27: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle27" }];
static OPS_BOSS_IDLE28: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle28" }];
static OPS_BOSS_IDLE29: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle29" }];
static OPS_BOSS_IDLE3: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle3" }];
static OPS_BOSS_IDLE30: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle30" }];
static OPS_BOSS_IDLE31: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle31" }];
static OPS_BOSS_IDLE4: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle4" }];
static OPS_BOSS_IDLE5: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle5" }];
static OPS_BOSS_IDLE6: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle6" }];
static OPS_BOSS_IDLE7: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle7" }];
static OPS_BOSS_IDLE8: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle8" }];
static OPS_BOSS_IDLE9: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_idle9" }];
static OPS_BOSS_MISSILE1: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile1" }];
static OPS_BOSS_MISSILE10: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile10" }];
static OPS_BOSS_MISSILE11: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile11" }];
static OPS_BOSS_MISSILE12: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile12" }];
static OPS_BOSS_MISSILE13: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile13" }];
static OPS_BOSS_MISSILE14: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile14" }];
static OPS_BOSS_MISSILE15: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile15" }];
static OPS_BOSS_MISSILE16: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile16" }];
static OPS_BOSS_MISSILE17: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile17" }];
static OPS_BOSS_MISSILE18: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile18" }];
static OPS_BOSS_MISSILE19: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile19" }];
static OPS_BOSS_MISSILE2: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile2" }];
static OPS_BOSS_MISSILE20: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile20" }];
static OPS_BOSS_MISSILE21: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile21" }];
static OPS_BOSS_MISSILE22: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile22" }];
static OPS_BOSS_MISSILE23: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile23" }];
static OPS_BOSS_MISSILE3: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile3" }];
static OPS_BOSS_MISSILE4: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile4" }];
static OPS_BOSS_MISSILE5: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile5" }];
static OPS_BOSS_MISSILE6: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile6" }];
static OPS_BOSS_MISSILE7: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile7" }];
static OPS_BOSS_MISSILE8: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile8" }];
static OPS_BOSS_MISSILE9: &[MonsterOperation] = &[MonsterOperation::Action { name: "boss_missile9" }];
static OPS_BOSS_RISE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/out1.wav",
    channel: Q1SoundChannel::Weapon,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_BOSS_RISE10: &[MonsterOperation] = &[];
static OPS_BOSS_RISE11: &[MonsterOperation] = &[];
static OPS_BOSS_RISE12: &[MonsterOperation] = &[];
static OPS_BOSS_RISE13: &[MonsterOperation] = &[];
static OPS_BOSS_RISE14: &[MonsterOperation] = &[];
static OPS_BOSS_RISE15: &[MonsterOperation] = &[];
static OPS_BOSS_RISE16: &[MonsterOperation] = &[];
static OPS_BOSS_RISE17: &[MonsterOperation] = &[];
static OPS_BOSS_RISE2: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "boss1/sight1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_BOSS_RISE3: &[MonsterOperation] = &[];
static OPS_BOSS_RISE4: &[MonsterOperation] = &[];
static OPS_BOSS_RISE5: &[MonsterOperation] = &[];
static OPS_BOSS_RISE6: &[MonsterOperation] = &[];
static OPS_BOSS_RISE7: &[MonsterOperation] = &[];
static OPS_BOSS_RISE8: &[MonsterOperation] = &[];
static OPS_BOSS_RISE9: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA1: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA10: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA2: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA3: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA4: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA5: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA6: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA7: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA8: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKA9: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB1: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB10: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB2: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB3: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB4: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB5: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB6: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB7: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB8: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKB9: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC1: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC10: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC2: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC3: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC4: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC5: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC6: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC7: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC8: &[MonsterOperation] = &[];
static OPS_BOSS_SHOCKC9: &[MonsterOperation] = &[];
static OPS_DEMON1_ATTA1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_DEMON1_ATTA10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 2.0,
}];
static OPS_DEMON1_ATTA11: &[MonsterOperation] = &[MonsterOperation::Action { name: "demon1_atta11" }];
static OPS_DEMON1_ATTA12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_DEMON1_ATTA13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 8.0,
}];
static OPS_DEMON1_ATTA14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_DEMON1_ATTA15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_DEMON1_ATTA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_DEMON1_ATTA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_DEMON1_ATTA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_DEMON1_ATTA5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Action { name: "demon1_atta5" },
];
static OPS_DEMON1_ATTA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_DEMON1_ATTA7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_DEMON1_ATTA8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 8.0,
}];
static OPS_DEMON1_ATTA9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_DEMON1_DIE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "demon/ddeath.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_DEMON1_DIE2: &[MonsterOperation] = &[];
static OPS_DEMON1_DIE3: &[MonsterOperation] = &[];
static OPS_DEMON1_DIE4: &[MonsterOperation] = &[];
static OPS_DEMON1_DIE5: &[MonsterOperation] = &[];
static OPS_DEMON1_DIE6: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_DEMON1_DIE7: &[MonsterOperation] = &[];
static OPS_DEMON1_DIE8: &[MonsterOperation] = &[];
static OPS_DEMON1_DIE9: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_DEMON1_JUMP10: &[MonsterOperation] = &[MonsterOperation::Action { name: "demon1_jump10" }];
static OPS_DEMON1_JUMP11: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP12: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_DEMON1_JUMP3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_DEMON1_JUMP4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "demon1_jump4" },
];
static OPS_DEMON1_JUMP5: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP6: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP7: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP8: &[MonsterOperation] = &[];
static OPS_DEMON1_JUMP9: &[MonsterOperation] = &[];
static OPS_DEMON1_PAIN1: &[MonsterOperation] = &[];
static OPS_DEMON1_PAIN2: &[MonsterOperation] = &[];
static OPS_DEMON1_PAIN3: &[MonsterOperation] = &[];
static OPS_DEMON1_PAIN4: &[MonsterOperation] = &[];
static OPS_DEMON1_PAIN5: &[MonsterOperation] = &[];
static OPS_DEMON1_PAIN6: &[MonsterOperation] = &[];
static OPS_DEMON1_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "demon/idle1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 20.0,
    },
];
static OPS_DEMON1_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 15.0,
}];
static OPS_DEMON1_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 36.0,
}];
static OPS_DEMON1_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_DEMON1_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 15.0,
}];
static OPS_DEMON1_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 36.0,
}];
static OPS_DEMON1_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_DEMON1_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "demon/idle1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 8.0,
    },
];
static OPS_DEMON1_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_DEMON1_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_DEMON1_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 7.0,
}];
static OPS_DEMON1_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_DEMON1_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_DEMON1_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 10.0,
}];
static OPS_DEMON1_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 10.0,
}];
static OPS_ENF_ATK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK10: &[MonsterOperation] = &[MonsterOperation::Action { name: "enf_atk10" }];
static OPS_ENF_ATK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK14: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "enf_atk14" },
];
static OPS_ENF_ATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK6: &[MonsterOperation] = &[MonsterOperation::Action { name: "enf_atk6" }];
static OPS_ENF_ATK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_ATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ENF_DIE1: &[MonsterOperation] = &[];
static OPS_ENF_DIE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 5.0,
}];
static OPS_ENF_DIE11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 5.0,
}];
static OPS_ENF_DIE12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 5.0,
}];
static OPS_ENF_DIE13: &[MonsterOperation] = &[];
static OPS_ENF_DIE14: &[MonsterOperation] = &[];
static OPS_ENF_DIE2: &[MonsterOperation] = &[];
static OPS_ENF_DIE3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Action { name: "enf_die3" },
];
static OPS_ENF_DIE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 14.0,
}];
static OPS_ENF_DIE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 2.0,
}];
static OPS_ENF_DIE6: &[MonsterOperation] = &[];
static OPS_ENF_DIE7: &[MonsterOperation] = &[];
static OPS_ENF_DIE8: &[MonsterOperation] = &[];
static OPS_ENF_DIE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 3.0,
}];
static OPS_ENF_FDIE1: &[MonsterOperation] = &[];
static OPS_ENF_FDIE10: &[MonsterOperation] = &[];
static OPS_ENF_FDIE11: &[MonsterOperation] = &[];
static OPS_ENF_FDIE2: &[MonsterOperation] = &[];
static OPS_ENF_FDIE3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Action { name: "enf_fdie3" },
];
static OPS_ENF_FDIE4: &[MonsterOperation] = &[];
static OPS_ENF_FDIE5: &[MonsterOperation] = &[];
static OPS_ENF_FDIE6: &[MonsterOperation] = &[];
static OPS_ENF_FDIE7: &[MonsterOperation] = &[];
static OPS_ENF_FDIE8: &[MonsterOperation] = &[];
static OPS_ENF_FDIE9: &[MonsterOperation] = &[];
static OPS_ENF_PAINA1: &[MonsterOperation] = &[];
static OPS_ENF_PAINA2: &[MonsterOperation] = &[];
static OPS_ENF_PAINA3: &[MonsterOperation] = &[];
static OPS_ENF_PAINA4: &[MonsterOperation] = &[];
static OPS_ENF_PAINB1: &[MonsterOperation] = &[];
static OPS_ENF_PAINB2: &[MonsterOperation] = &[];
static OPS_ENF_PAINB3: &[MonsterOperation] = &[];
static OPS_ENF_PAINB4: &[MonsterOperation] = &[];
static OPS_ENF_PAINB5: &[MonsterOperation] = &[];
static OPS_ENF_PAINC1: &[MonsterOperation] = &[];
static OPS_ENF_PAINC2: &[MonsterOperation] = &[];
static OPS_ENF_PAINC3: &[MonsterOperation] = &[];
static OPS_ENF_PAINC4: &[MonsterOperation] = &[];
static OPS_ENF_PAINC5: &[MonsterOperation] = &[];
static OPS_ENF_PAINC6: &[MonsterOperation] = &[];
static OPS_ENF_PAINC7: &[MonsterOperation] = &[];
static OPS_ENF_PAINC8: &[MonsterOperation] = &[];
static OPS_ENF_PAIND1: &[MonsterOperation] = &[];
static OPS_ENF_PAIND10: &[MonsterOperation] = &[];
static OPS_ENF_PAIND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ENF_PAIND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ENF_PAIND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ENF_PAIND14: &[MonsterOperation] = &[];
static OPS_ENF_PAIND15: &[MonsterOperation] = &[];
static OPS_ENF_PAIND16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ENF_PAIND17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ENF_PAIND18: &[MonsterOperation] = &[];
static OPS_ENF_PAIND19: &[MonsterOperation] = &[];
static OPS_ENF_PAIND2: &[MonsterOperation] = &[];
static OPS_ENF_PAIND3: &[MonsterOperation] = &[];
static OPS_ENF_PAIND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 2.0,
}];
static OPS_ENF_PAIND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ENF_PAIND6: &[MonsterOperation] = &[];
static OPS_ENF_PAIND7: &[MonsterOperation] = &[];
static OPS_ENF_PAIND8: &[MonsterOperation] = &[];
static OPS_ENF_PAIND9: &[MonsterOperation] = &[];
static OPS_ENF_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "enforcer/idle1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 18.0,
    },
];
static OPS_ENF_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_ENF_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 7.0,
}];
static OPS_ENF_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_ENF_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_ENF_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_ENF_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 7.0,
}];
static OPS_ENF_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 11.0,
}];
static OPS_ENF_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ENF_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "enforcer/idle1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 2.0,
    },
];
static OPS_ENF_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ENF_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ENF_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ENF_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ENF_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ENF_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ENF_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ENF_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ENF_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_ENF_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ENF_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ENF_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ENF_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ENF_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ENF_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_F_ATTACK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK15: &[MonsterOperation] = &[MonsterOperation::Action { name: "f_attack15" }];
static OPS_F_ATTACK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK3: &[MonsterOperation] = &[MonsterOperation::Action { name: "f_attack3" }];
static OPS_F_ATTACK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_F_ATTACK9: &[MonsterOperation] = &[MonsterOperation::Action { name: "f_attack9" }];
static OPS_F_DEATH1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "fish/death.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_F_DEATH10: &[MonsterOperation] = &[];
static OPS_F_DEATH11: &[MonsterOperation] = &[];
static OPS_F_DEATH12: &[MonsterOperation] = &[];
static OPS_F_DEATH13: &[MonsterOperation] = &[];
static OPS_F_DEATH14: &[MonsterOperation] = &[];
static OPS_F_DEATH15: &[MonsterOperation] = &[];
static OPS_F_DEATH16: &[MonsterOperation] = &[];
static OPS_F_DEATH17: &[MonsterOperation] = &[];
static OPS_F_DEATH18: &[MonsterOperation] = &[];
static OPS_F_DEATH19: &[MonsterOperation] = &[];
static OPS_F_DEATH2: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_F_DEATH20: &[MonsterOperation] = &[];
static OPS_F_DEATH21: &[MonsterOperation] = &[];
static OPS_F_DEATH3: &[MonsterOperation] = &[];
static OPS_F_DEATH4: &[MonsterOperation] = &[];
static OPS_F_DEATH5: &[MonsterOperation] = &[];
static OPS_F_DEATH6: &[MonsterOperation] = &[];
static OPS_F_DEATH7: &[MonsterOperation] = &[];
static OPS_F_DEATH8: &[MonsterOperation] = &[];
static OPS_F_DEATH9: &[MonsterOperation] = &[];
static OPS_F_PAIN1: &[MonsterOperation] = &[];
static OPS_F_PAIN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_PAIN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_F_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 12.0,
    },
    MonsterOperation::Sound {
        path: "fish/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: Some(0.5),
    },
];
static OPS_F_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_F_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_F_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_F_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_HKNIGHT_CHAR_A1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 20.0,
}];
static OPS_HKNIGHT_CHAR_A10: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 20.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_A11: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 18.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_A12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 16.0,
}];
static OPS_HKNIGHT_CHAR_A13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_HKNIGHT_CHAR_A14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 25.0,
}];
static OPS_HKNIGHT_CHAR_A15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 21.0,
}];
static OPS_HKNIGHT_CHAR_A16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 13.0,
}];
static OPS_HKNIGHT_CHAR_A2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 25.0,
}];
static OPS_HKNIGHT_CHAR_A3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 18.0,
}];
static OPS_HKNIGHT_CHAR_A4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 16.0,
}];
static OPS_HKNIGHT_CHAR_A5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_HKNIGHT_CHAR_A6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 20.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_A7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 21.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_A8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 13.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_A9: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 20.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_B1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_char_b1",
}];
static OPS_HKNIGHT_CHAR_B2: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 17.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_B3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 12.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_B4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 22.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_B5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 18.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_CHAR_B6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_DIE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 10.0,
}];
static OPS_HKNIGHT_DIE10: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE11: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE12: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 8.0,
}];
static OPS_HKNIGHT_DIE3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Ai {
        mode: MonsterAi::Forward,
        distance: 7.0,
    },
];
static OPS_HKNIGHT_DIE4: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE5: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE6: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE7: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 10.0,
}];
static OPS_HKNIGHT_DIE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 11.0,
}];
static OPS_HKNIGHT_DIEB1: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB2: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_HKNIGHT_DIEB4: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB5: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB6: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB7: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB8: &[MonsterOperation] = &[];
static OPS_HKNIGHT_DIEB9: &[MonsterOperation] = &[];
static OPS_HKNIGHT_MAGICA1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magica10",
}];
static OPS_HKNIGHT_MAGICA11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magica11",
}];
static OPS_HKNIGHT_MAGICA12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magica12",
}];
static OPS_HKNIGHT_MAGICA13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICA7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magica7",
}];
static OPS_HKNIGHT_MAGICA8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magica8",
}];
static OPS_HKNIGHT_MAGICA9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magica9",
}];
static OPS_HKNIGHT_MAGICB1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicb10",
}];
static OPS_HKNIGHT_MAGICB11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicb11",
}];
static OPS_HKNIGHT_MAGICB12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicb12",
}];
static OPS_HKNIGHT_MAGICB13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICB7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicb7",
}];
static OPS_HKNIGHT_MAGICB8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicb8",
}];
static OPS_HKNIGHT_MAGICB9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicb9",
}];
static OPS_HKNIGHT_MAGICC1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICC10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicc10",
}];
static OPS_HKNIGHT_MAGICC11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicc11",
}];
static OPS_HKNIGHT_MAGICC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICC3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICC4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICC5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_HKNIGHT_MAGICC6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicc6",
}];
static OPS_HKNIGHT_MAGICC7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicc7",
}];
static OPS_HKNIGHT_MAGICC8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicc8",
}];
static OPS_HKNIGHT_MAGICC9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hknight_magicc9",
}];
static OPS_HKNIGHT_PAIN1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "hknight/pain1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_HKNIGHT_PAIN2: &[MonsterOperation] = &[];
static OPS_HKNIGHT_PAIN3: &[MonsterOperation] = &[];
static OPS_HKNIGHT_PAIN4: &[MonsterOperation] = &[];
static OPS_HKNIGHT_PAIN5: &[MonsterOperation] = &[];
static OPS_HKNIGHT_RUN1: &[MonsterOperation] = &[MonsterOperation::Action { name: "hknight_run1" }];
static OPS_HKNIGHT_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 25.0,
}];
static OPS_HKNIGHT_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 18.0,
}];
static OPS_HKNIGHT_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_HKNIGHT_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_HKNIGHT_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 25.0,
}];
static OPS_HKNIGHT_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 21.0,
}];
static OPS_HKNIGHT_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 13.0,
}];
static OPS_HKNIGHT_SLICE1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_HKNIGHT_SLICE10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_HKNIGHT_SLICE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_HKNIGHT_SLICE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 13.0,
}];
static OPS_HKNIGHT_SLICE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_HKNIGHT_SLICE5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 7.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SLICE6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 15.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SLICE7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SLICE8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SLICE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_HKNIGHT_SMASH1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_HKNIGHT_SMASH10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_SMASH11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_SMASH2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 13.0,
}];
static OPS_HKNIGHT_SMASH3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_HKNIGHT_SMASH4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 11.0,
}];
static OPS_HKNIGHT_SMASH5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 10.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SMASH6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 7.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SMASH7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 12.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SMASH8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_SMASH9: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_HKNIGHT_WALK1: &[MonsterOperation] = &[MonsterOperation::Action { name: "hknight_walk1" }];
static OPS_HKNIGHT_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_HKNIGHT_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_HKNIGHT_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_HKNIGHT_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_HKNIGHT_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_HKNIGHT_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_HKNIGHT_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_HKNIGHT_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_HKNIGHT_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_HKNIGHT_WALK19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_HKNIGHT_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_HKNIGHT_WALK20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_HKNIGHT_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_HKNIGHT_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_HKNIGHT_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_HKNIGHT_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_HKNIGHT_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_HKNIGHT_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_HKNIGHT_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_HKNIGHT_WATK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 2.0,
}];
static OPS_HKNIGHT_WATK10: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_WATK11: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_WATK12: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_WATK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_HKNIGHT_WATK17: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 1.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_WATK18: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_WATK19: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 4.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_HKNIGHT_WATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_HKNIGHT_WATK21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_HKNIGHT_WATK22: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_HKNIGHT_WATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Melee,
    distance: 0.0,
}];
static OPS_HKNIGHT_WATK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_HKNIGHT_WATK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_HKNIGHT_WATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_KNIGHT_ATK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "knight/sword1.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 0.0,
    },
];
static OPS_KNIGHT_ATK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_KNIGHT_ATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_KNIGHT_ATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_KNIGHT_ATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_KNIGHT_ATK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_KNIGHT_ATK6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 4.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_KNIGHT_ATK7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 1.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_KNIGHT_ATK8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 3.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Melee,
        distance: 0.0,
    },
];
static OPS_KNIGHT_ATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_KNIGHT_BOW1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_BOW9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_KNIGHT_DIE1: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE10: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE2: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_KNIGHT_DIE4: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE5: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE6: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE7: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE8: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIE9: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB1: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB10: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB11: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB2: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_KNIGHT_DIEB4: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB5: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB6: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB7: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB8: &[MonsterOperation] = &[];
static OPS_KNIGHT_DIEB9: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAIN1: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAIN2: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAIN3: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAINB1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 0.0,
}];
static OPS_KNIGHT_PAINB10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 0.0,
}];
static OPS_KNIGHT_PAINB11: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAINB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 3.0,
}];
static OPS_KNIGHT_PAINB3: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAINB4: &[MonsterOperation] = &[];
static OPS_KNIGHT_PAINB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 2.0,
}];
static OPS_KNIGHT_PAINB6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 4.0,
}];
static OPS_KNIGHT_PAINB7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 2.0,
}];
static OPS_KNIGHT_PAINB8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 5.0,
}];
static OPS_KNIGHT_PAINB9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 5.0,
}];
static OPS_KNIGHT_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "knight/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 16.0,
    },
];
static OPS_KNIGHT_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_KNIGHT_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 13.0,
}];
static OPS_KNIGHT_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 7.0,
}];
static OPS_KNIGHT_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_KNIGHT_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_KNIGHT_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_KNIGHT_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 6.0,
}];
static OPS_KNIGHT_RUNATK1: &[MonsterOperation] = &[MonsterOperation::Action { name: "knight_runatk1" }];
static OPS_KNIGHT_RUNATK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::ChargeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 10.0,
}];
static OPS_KNIGHT_RUNATK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::ChargeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::ChargeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::ChargeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::MeleeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::MeleeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::MeleeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::MeleeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_RUNATK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::MeleeSide,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_KNIGHT_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "knight/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 3.0,
    },
];
static OPS_KNIGHT_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_KNIGHT_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_KNIGHT_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_KNIGHT_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_KNIGHT_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_KNIGHT_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_KNIGHT_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_BDIE1: &[MonsterOperation] = &[];
static OPS_OGRE_BDIE10: &[MonsterOperation] = &[];
static OPS_OGRE_BDIE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 5.0,
}];
static OPS_OGRE_BDIE3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Action { name: "ogre_bdie3" },
];
static OPS_OGRE_BDIE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 1.0,
}];
static OPS_OGRE_BDIE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 3.0,
}];
static OPS_OGRE_BDIE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 7.0,
}];
static OPS_OGRE_BDIE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Forward,
    distance: 25.0,
}];
static OPS_OGRE_BDIE8: &[MonsterOperation] = &[];
static OPS_OGRE_BDIE9: &[MonsterOperation] = &[];
static OPS_OGRE_DIE1: &[MonsterOperation] = &[];
static OPS_OGRE_DIE10: &[MonsterOperation] = &[];
static OPS_OGRE_DIE11: &[MonsterOperation] = &[];
static OPS_OGRE_DIE12: &[MonsterOperation] = &[];
static OPS_OGRE_DIE13: &[MonsterOperation] = &[];
static OPS_OGRE_DIE14: &[MonsterOperation] = &[];
static OPS_OGRE_DIE2: &[MonsterOperation] = &[];
static OPS_OGRE_DIE3: &[MonsterOperation] = &[
    MonsterOperation::Solid { solid: Q1Solid::None },
    MonsterOperation::Action { name: "ogre_die3" },
];
static OPS_OGRE_DIE4: &[MonsterOperation] = &[];
static OPS_OGRE_DIE5: &[MonsterOperation] = &[];
static OPS_OGRE_DIE6: &[MonsterOperation] = &[];
static OPS_OGRE_DIE7: &[MonsterOperation] = &[];
static OPS_OGRE_DIE8: &[MonsterOperation] = &[];
static OPS_OGRE_DIE9: &[MonsterOperation] = &[];
static OPS_OGRE_NAIL1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_OGRE_NAIL2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_OGRE_NAIL3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_OGRE_NAIL4: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "ogre_nail4" },
];
static OPS_OGRE_NAIL5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_OGRE_NAIL6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_OGRE_NAIL7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_OGRE_PAIN1: &[MonsterOperation] = &[];
static OPS_OGRE_PAIN2: &[MonsterOperation] = &[];
static OPS_OGRE_PAIN3: &[MonsterOperation] = &[];
static OPS_OGRE_PAIN4: &[MonsterOperation] = &[];
static OPS_OGRE_PAIN5: &[MonsterOperation] = &[];
static OPS_OGRE_PAINB1: &[MonsterOperation] = &[];
static OPS_OGRE_PAINB2: &[MonsterOperation] = &[];
static OPS_OGRE_PAINB3: &[MonsterOperation] = &[];
static OPS_OGRE_PAINC1: &[MonsterOperation] = &[];
static OPS_OGRE_PAINC2: &[MonsterOperation] = &[];
static OPS_OGRE_PAINC3: &[MonsterOperation] = &[];
static OPS_OGRE_PAINC4: &[MonsterOperation] = &[];
static OPS_OGRE_PAINC5: &[MonsterOperation] = &[];
static OPS_OGRE_PAINC6: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND1: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND10: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND11: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND12: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND13: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND14: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND15: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND16: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 10.0,
}];
static OPS_OGRE_PAIND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 9.0,
}];
static OPS_OGRE_PAIND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 4.0,
}];
static OPS_OGRE_PAIND5: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND6: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND7: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND8: &[MonsterOperation] = &[];
static OPS_OGRE_PAIND9: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE1: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE10: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE11: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE12: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE13: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE14: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE15: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 10.0,
}];
static OPS_OGRE_PAINE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 9.0,
}];
static OPS_OGRE_PAINE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 4.0,
}];
static OPS_OGRE_PAINE5: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE6: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE7: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE8: &[MonsterOperation] = &[];
static OPS_OGRE_PAINE9: &[MonsterOperation] = &[];
static OPS_OGRE_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 9.0,
    },
    MonsterOperation::Sound {
        path: "ogre/ogidle2.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
];
static OPS_OGRE_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_OGRE_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_OGRE_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 22.0,
}];
static OPS_OGRE_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_OGRE_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_OGRE_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 13.0,
}];
static OPS_OGRE_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 24.0,
}];
static OPS_OGRE_SMASH1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 6.0,
    },
    MonsterOperation::Sound {
        path: "ogre/ogsawatk.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
];
static OPS_OGRE_SMASH10: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_smash10" }];
static OPS_OGRE_SMASH11: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
    MonsterOperation::Action { name: "ogre_smash11" },
];
static OPS_OGRE_SMASH12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_OGRE_SMASH13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_OGRE_SMASH14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 12.0,
}];
static OPS_OGRE_SMASH2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_OGRE_SMASH3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_OGRE_SMASH4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_OGRE_SMASH5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_OGRE_SMASH6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 4.0,
    },
    MonsterOperation::Action { name: "ogre_smash6" },
];
static OPS_OGRE_SMASH7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 4.0,
    },
    MonsterOperation::Action { name: "ogre_smash7" },
];
static OPS_OGRE_SMASH8: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 10.0,
    },
    MonsterOperation::Action { name: "ogre_smash8" },
];
static OPS_OGRE_SMASH9: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 13.0,
    },
    MonsterOperation::Action { name: "ogre_smash9" },
];
static OPS_OGRE_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND5: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "ogre/ogidle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Stand,
        distance: 0.0,
    },
];
static OPS_OGRE_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_OGRE_SWING1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 11.0,
    },
    MonsterOperation::Sound {
        path: "ogre/ogsawatk.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
];
static OPS_OGRE_SWING10: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_swing10" }];
static OPS_OGRE_SWING11: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_swing11" }];
static OPS_OGRE_SWING12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_OGRE_SWING13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 8.0,
}];
static OPS_OGRE_SWING14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_OGRE_SWING2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_OGRE_SWING3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_OGRE_SWING4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 13.0,
}];
static OPS_OGRE_SWING5: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 9.0,
    },
    MonsterOperation::Action { name: "ogre_swing5" },
];
static OPS_OGRE_SWING6: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_swing6" }];
static OPS_OGRE_SWING7: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_swing7" }];
static OPS_OGRE_SWING8: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_swing8" }];
static OPS_OGRE_SWING9: &[MonsterOperation] = &[MonsterOperation::Action { name: "ogre_swing9" }];
static OPS_OGRE_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_OGRE_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_OGRE_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_OGRE_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_OGRE_WALK3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 2.0,
    },
    MonsterOperation::Sound {
        path: "ogre/ogidle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
];
static OPS_OGRE_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_OGRE_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_OGRE_WALK6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 5.0,
    },
    MonsterOperation::Sound {
        path: "ogre/ogdrag.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.1),
    },
];
static OPS_OGRE_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OGRE_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_OGRE_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_OLD_IDLE1: &[MonsterOperation] = &[];
static OPS_OLD_IDLE10: &[MonsterOperation] = &[];
static OPS_OLD_IDLE11: &[MonsterOperation] = &[];
static OPS_OLD_IDLE12: &[MonsterOperation] = &[];
static OPS_OLD_IDLE13: &[MonsterOperation] = &[];
static OPS_OLD_IDLE14: &[MonsterOperation] = &[];
static OPS_OLD_IDLE15: &[MonsterOperation] = &[];
static OPS_OLD_IDLE16: &[MonsterOperation] = &[];
static OPS_OLD_IDLE17: &[MonsterOperation] = &[];
static OPS_OLD_IDLE18: &[MonsterOperation] = &[];
static OPS_OLD_IDLE19: &[MonsterOperation] = &[];
static OPS_OLD_IDLE2: &[MonsterOperation] = &[];
static OPS_OLD_IDLE20: &[MonsterOperation] = &[];
static OPS_OLD_IDLE21: &[MonsterOperation] = &[];
static OPS_OLD_IDLE22: &[MonsterOperation] = &[];
static OPS_OLD_IDLE23: &[MonsterOperation] = &[];
static OPS_OLD_IDLE24: &[MonsterOperation] = &[];
static OPS_OLD_IDLE25: &[MonsterOperation] = &[];
static OPS_OLD_IDLE26: &[MonsterOperation] = &[];
static OPS_OLD_IDLE27: &[MonsterOperation] = &[];
static OPS_OLD_IDLE28: &[MonsterOperation] = &[];
static OPS_OLD_IDLE29: &[MonsterOperation] = &[];
static OPS_OLD_IDLE3: &[MonsterOperation] = &[];
static OPS_OLD_IDLE30: &[MonsterOperation] = &[];
static OPS_OLD_IDLE31: &[MonsterOperation] = &[];
static OPS_OLD_IDLE32: &[MonsterOperation] = &[];
static OPS_OLD_IDLE33: &[MonsterOperation] = &[];
static OPS_OLD_IDLE34: &[MonsterOperation] = &[];
static OPS_OLD_IDLE35: &[MonsterOperation] = &[];
static OPS_OLD_IDLE36: &[MonsterOperation] = &[];
static OPS_OLD_IDLE37: &[MonsterOperation] = &[];
static OPS_OLD_IDLE38: &[MonsterOperation] = &[];
static OPS_OLD_IDLE39: &[MonsterOperation] = &[];
static OPS_OLD_IDLE4: &[MonsterOperation] = &[];
static OPS_OLD_IDLE40: &[MonsterOperation] = &[];
static OPS_OLD_IDLE41: &[MonsterOperation] = &[];
static OPS_OLD_IDLE42: &[MonsterOperation] = &[];
static OPS_OLD_IDLE43: &[MonsterOperation] = &[];
static OPS_OLD_IDLE44: &[MonsterOperation] = &[];
static OPS_OLD_IDLE45: &[MonsterOperation] = &[];
static OPS_OLD_IDLE46: &[MonsterOperation] = &[];
static OPS_OLD_IDLE5: &[MonsterOperation] = &[];
static OPS_OLD_IDLE6: &[MonsterOperation] = &[];
static OPS_OLD_IDLE7: &[MonsterOperation] = &[];
static OPS_OLD_IDLE8: &[MonsterOperation] = &[];
static OPS_OLD_IDLE9: &[MonsterOperation] = &[];
static OPS_OLD_THRASH1: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "m" }];
static OPS_OLD_THRASH10: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "e" }];
static OPS_OLD_THRASH11: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "g" }];
static OPS_OLD_THRASH12: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "i" }];
static OPS_OLD_THRASH13: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "k" }];
static OPS_OLD_THRASH14: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "m" }];
static OPS_OLD_THRASH15: &[MonsterOperation] = &[
    MonsterOperation::Lightstyle { pattern: "m" },
    MonsterOperation::Action { name: "old_thrash15" },
];
static OPS_OLD_THRASH16: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "g" }];
static OPS_OLD_THRASH17: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "c" }];
static OPS_OLD_THRASH18: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "b" }];
static OPS_OLD_THRASH19: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "a" }];
static OPS_OLD_THRASH2: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "k" }];
static OPS_OLD_THRASH20: &[MonsterOperation] = &[MonsterOperation::Action { name: "old_thrash20" }];
static OPS_OLD_THRASH3: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "k" }];
static OPS_OLD_THRASH4: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "i" }];
static OPS_OLD_THRASH5: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "g" }];
static OPS_OLD_THRASH6: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "e" }];
static OPS_OLD_THRASH7: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "c" }];
static OPS_OLD_THRASH8: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "a" }];
static OPS_OLD_THRASH9: &[MonsterOperation] = &[MonsterOperation::Lightstyle { pattern: "c" }];
static OPS_SHAL_ATTACK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shalrath/attack.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
];
static OPS_SHAL_ATTACK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK11: &[MonsterOperation] = &[];
static OPS_SHAL_ATTACK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAL_ATTACK9: &[MonsterOperation] = &[MonsterOperation::Action { name: "shal_attack9" }];
static OPS_SHAL_DEATH1: &[MonsterOperation] = &[];
static OPS_SHAL_DEATH2: &[MonsterOperation] = &[];
static OPS_SHAL_DEATH3: &[MonsterOperation] = &[];
static OPS_SHAL_DEATH4: &[MonsterOperation] = &[];
static OPS_SHAL_DEATH5: &[MonsterOperation] = &[];
static OPS_SHAL_DEATH6: &[MonsterOperation] = &[];
static OPS_SHAL_DEATH7: &[MonsterOperation] = &[];
static OPS_SHAL_PAIN1: &[MonsterOperation] = &[];
static OPS_SHAL_PAIN2: &[MonsterOperation] = &[];
static OPS_SHAL_PAIN3: &[MonsterOperation] = &[];
static OPS_SHAL_PAIN4: &[MonsterOperation] = &[];
static OPS_SHAL_PAIN5: &[MonsterOperation] = &[];
static OPS_SHAL_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shalrath/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 6.0,
    },
];
static OPS_SHAL_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_SHAL_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_SHAL_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 5.0,
}];
static OPS_SHAL_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_SHAL_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_SHAL_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_SHAL_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_SHAL_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_SHAL_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 5.0,
}];
static OPS_SHAL_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 6.0,
}];
static OPS_SHAL_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 5.0,
}];
static OPS_SHAL_STAND: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAL_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shalrath/idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 6.0,
    },
];
static OPS_SHAL_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_SHAL_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_SHAL_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_SHAL_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_SHAL_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_SHAL_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_SHAL_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_SHAL_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_SHAL_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_SHAL_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_SHAL_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_SHAM_DEATH1: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH10: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH11: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH2: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_SHAM_DEATH4: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH5: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH6: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH7: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH8: &[MonsterOperation] = &[];
static OPS_SHAM_DEATH9: &[MonsterOperation] = &[];
static OPS_SHAM_MAGIC1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Sound {
        path: "shambler/sattck1.wav",
        channel: Q1SoundChannel::Weapon,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
];
static OPS_SHAM_MAGIC10: &[MonsterOperation] = &[MonsterOperation::Action { name: "sham_magic10" }];
static OPS_SHAM_MAGIC11: &[MonsterOperation] = &[];
static OPS_SHAM_MAGIC12: &[MonsterOperation] = &[];
static OPS_SHAM_MAGIC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_SHAM_MAGIC3: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "sham_magic3" },
];
static OPS_SHAM_MAGIC4: &[MonsterOperation] = &[MonsterOperation::Action { name: "sham_magic4" }];
static OPS_SHAM_MAGIC5: &[MonsterOperation] = &[MonsterOperation::Action { name: "sham_magic5" }];
static OPS_SHAM_MAGIC6: &[MonsterOperation] = &[MonsterOperation::Action { name: "sham_magic6" }];
static OPS_SHAM_MAGIC9: &[MonsterOperation] = &[MonsterOperation::Action { name: "sham_magic9" }];
static OPS_SHAM_PAIN1: &[MonsterOperation] = &[];
static OPS_SHAM_PAIN2: &[MonsterOperation] = &[];
static OPS_SHAM_PAIN3: &[MonsterOperation] = &[];
static OPS_SHAM_PAIN4: &[MonsterOperation] = &[];
static OPS_SHAM_PAIN5: &[MonsterOperation] = &[];
static OPS_SHAM_PAIN6: &[MonsterOperation] = &[];
static OPS_SHAM_RUN1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_SHAM_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 24.0,
}];
static OPS_SHAM_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_SHAM_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 20.0,
}];
static OPS_SHAM_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 24.0,
}];
static OPS_SHAM_RUN6: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 20.0,
    },
    MonsterOperation::Sound {
        path: "shambler/sidle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Greater,
        chance: Some(0.8),
    },
];
static OPS_SHAM_SMASH1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shambler/melee1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 2.0,
    },
];
static OPS_SHAM_SMASH10: &[MonsterOperation] = &[MonsterOperation::Action { name: "sham_smash10" }];
static OPS_SHAM_SMASH11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_SHAM_SMASH12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_SHAM_SMASH2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_SHAM_SMASH3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_SHAM_SMASH4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 5.0,
}];
static OPS_SHAM_SMASH5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_SHAM_SMASH6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 1.0,
}];
static OPS_SHAM_SMASH7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_SHAM_SMASH8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_SHAM_SMASH9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 0.0,
}];
static OPS_SHAM_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_SHAM_SWINGL1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shambler/melee2.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 5.0,
    },
];
static OPS_SHAM_SWINGL2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SHAM_SWINGL3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_SHAM_SWINGL4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SHAM_SWINGL5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_SHAM_SWINGL6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 9.0,
}];
static OPS_SHAM_SWINGL7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 5.0,
    },
    MonsterOperation::Action { name: "sham_swingl7" },
];
static OPS_SHAM_SWINGL8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 4.0,
}];
static OPS_SHAM_SWINGL9: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 8.0,
    },
    MonsterOperation::Action { name: "sham_swingl9" },
];
static OPS_SHAM_SWINGR1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "shambler/melee1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 1.0,
    },
];
static OPS_SHAM_SWINGR2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 8.0,
}];
static OPS_SHAM_SWINGR3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 14.0,
}];
static OPS_SHAM_SWINGR4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 7.0,
}];
static OPS_SHAM_SWINGR5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SHAM_SWINGR6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 6.0,
}];
static OPS_SHAM_SWINGR7: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 6.0,
    },
    MonsterOperation::Action { name: "sham_swingr7" },
];
static OPS_SHAM_SWINGR8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Charge,
    distance: 3.0,
}];
static OPS_SHAM_SWINGR9: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 1.0,
    },
    MonsterOperation::Ai {
        mode: MonsterAi::Charge,
        distance: 10.0,
    },
    MonsterOperation::Action { name: "sham_swingr9" },
];
static OPS_SHAM_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 10.0,
}];
static OPS_SHAM_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 9.0,
}];
static OPS_SHAM_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 7.0,
}];
static OPS_SHAM_WALK12: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 7.0,
    },
    MonsterOperation::Sound {
        path: "shambler/sidle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Greater,
        chance: Some(0.8),
    },
];
static OPS_SHAM_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 9.0,
}];
static OPS_SHAM_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 9.0,
}];
static OPS_SHAM_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_SHAM_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 6.0,
}];
static OPS_SHAM_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 12.0,
}];
static OPS_SHAM_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_SHAM_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_SHAM_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 13.0,
}];
static OPS_TBABY_DIE1: &[MonsterOperation] = &[MonsterOperation::Action { name: "tbaby_die1" }];
static OPS_TBABY_DIE2: &[MonsterOperation] = &[MonsterOperation::Action { name: "tbaby_die2" }];
static OPS_TBABY_FLY1: &[MonsterOperation] = &[];
static OPS_TBABY_FLY2: &[MonsterOperation] = &[];
static OPS_TBABY_FLY3: &[MonsterOperation] = &[];
static OPS_TBABY_FLY4: &[MonsterOperation] = &[MonsterOperation::Action { name: "tbaby_fly4" }];
static OPS_TBABY_HANG1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_TBABY_JUMP1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_JUMP2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_JUMP3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_JUMP4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_JUMP5: &[MonsterOperation] = &[MonsterOperation::Action { name: "tbaby_jump5" }];
static OPS_TBABY_JUMP6: &[MonsterOperation] = &[];
static OPS_TBABY_RUN1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN22: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN23: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN24: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN25: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_TBABY_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_TBABY_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_TBABY_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK19: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK20: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK21: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK22: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK23: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK24: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK25: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_TBABY_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_TBABY_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Turn,
    distance: 0.0,
}];
static OPS_WIZ_DEATH1: &[MonsterOperation] = &[MonsterOperation::Action { name: "wiz_death1" }];
static OPS_WIZ_DEATH2: &[MonsterOperation] = &[];
static OPS_WIZ_DEATH3: &[MonsterOperation] = &[MonsterOperation::Solid { solid: Q1Solid::None }];
static OPS_WIZ_DEATH4: &[MonsterOperation] = &[];
static OPS_WIZ_DEATH5: &[MonsterOperation] = &[];
static OPS_WIZ_DEATH6: &[MonsterOperation] = &[];
static OPS_WIZ_DEATH7: &[MonsterOperation] = &[];
static OPS_WIZ_DEATH8: &[MonsterOperation] = &[];
static OPS_WIZ_FAST1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "wiz_fast1" },
];
static OPS_WIZ_FAST10: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "wiz_fast10" },
];
static OPS_WIZ_FAST2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_FAST9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_WIZ_PAIN1: &[MonsterOperation] = &[];
static OPS_WIZ_PAIN2: &[MonsterOperation] = &[];
static OPS_WIZ_PAIN3: &[MonsterOperation] = &[];
static OPS_WIZ_PAIN4: &[MonsterOperation] = &[];
static OPS_WIZ_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 16.0,
    },
    MonsterOperation::Action { name: "wiz_run1" },
];
static OPS_WIZ_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 16.0,
}];
static OPS_WIZ_SIDE1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 8.0,
    },
    MonsterOperation::Action { name: "wiz_side1" },
];
static OPS_WIZ_SIDE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_SIDE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_SIDE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_SIDE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_SIDE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_SIDE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_SIDE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_WIZ_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_WIZ_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 8.0,
    },
    MonsterOperation::Action { name: "wiz_walk1" },
];
static OPS_WIZ_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WIZ_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WIZ_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WIZ_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WIZ_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WIZ_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_WIZ_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 8.0,
}];
static OPS_ZOMBIE_ATTA1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA13: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "zombie_atta13" },
];
static OPS_ZOMBIE_ATTA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTA9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB14: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "zombie_attb14" },
];
static OPS_ZOMBIE_ATTB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTB9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC12: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Face,
        distance: 0.0,
    },
    MonsterOperation::Action { name: "zombie_attc12" },
];
static OPS_ZOMBIE_ATTC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_ATTC9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_ZOMBIE_CRUC1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/idle_w2.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 3.0,
    comparison: SoundComparison::Less,
    chance: Some(0.1),
}];
static OPS_ZOMBIE_CRUC2: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_cruc2" }];
static OPS_ZOMBIE_CRUC3: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_cruc3" }];
static OPS_ZOMBIE_CRUC4: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_cruc4" }];
static OPS_ZOMBIE_CRUC5: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_cruc5" }];
static OPS_ZOMBIE_CRUC6: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_cruc6" }];
static OPS_ZOMBIE_PAINA1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_ZOMBIE_PAINA10: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINA11: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINA12: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 3.0,
}];
static OPS_ZOMBIE_PAINA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 3.0,
}];
static OPS_ZOMBIE_PAINA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINA7: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINA8: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINA9: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_ZOMBIE_PAINB10: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB11: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB12: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB13: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB14: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB15: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB16: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB17: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB18: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB19: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_ZOMBIE_PAINB20: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB21: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB22: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB23: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB24: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB25: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINB26: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB27: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB28: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 8.0,
}];
static OPS_ZOMBIE_PAINB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_ZOMBIE_PAINB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_ZOMBIE_PAINB6: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB7: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB8: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINB9: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_fall.wav",
    channel: Q1SoundChannel::Body,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_ZOMBIE_PAINC1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_ZOMBIE_PAINC10: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINC12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINC13: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC14: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC15: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC16: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC17: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC18: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC2: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 3.0,
}];
static OPS_ZOMBIE_PAINC4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINC5: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC6: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC7: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC8: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINC9: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_ZOMBIE_PAIND10: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND11: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND12: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND13: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND2: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND3: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND4: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND5: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND6: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND7: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND8: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAIND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINE1: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "zombie/z_pain.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Action { name: "zombie_paine1" },
];
static OPS_ZOMBIE_PAINE10: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "zombie/z_fall.wav",
        channel: Q1SoundChannel::Body,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Solid { solid: Q1Solid::None },
];
static OPS_ZOMBIE_PAINE11: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_paine11" }];
static OPS_ZOMBIE_PAINE12: &[MonsterOperation] = &[MonsterOperation::Action { name: "zombie_paine12" }];
static OPS_ZOMBIE_PAINE13: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE14: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE15: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE16: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE17: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE18: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE19: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 8.0,
}];
static OPS_ZOMBIE_PAINE20: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE21: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE22: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE23: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE24: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE25: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 5.0,
}];
static OPS_ZOMBIE_PAINE26: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 3.0,
}];
static OPS_ZOMBIE_PAINE27: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINE28: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINE29: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 5.0,
}];
static OPS_ZOMBIE_PAINE30: &[MonsterOperation] = &[];
static OPS_ZOMBIE_PAINE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 3.0,
}];
static OPS_ZOMBIE_PAINE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_ZOMBIE_PAINE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_ZOMBIE_PAINE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_ZOMBIE_RUN1: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 1.0,
    },
    MonsterOperation::Action { name: "zombie_run1" },
];
static OPS_ZOMBIE_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_ZOMBIE_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_ZOMBIE_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_ZOMBIE_RUN13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_ZOMBIE_RUN14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_ZOMBIE_RUN15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 6.0,
}];
static OPS_ZOMBIE_RUN16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 7.0,
}];
static OPS_ZOMBIE_RUN17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_ZOMBIE_RUN18: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 8.0,
    },
    MonsterOperation::Sound {
        path: "zombie/z_idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
    MonsterOperation::Sound {
        path: "zombie/z_idle1.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Greater,
        chance: Some(0.8),
    },
];
static OPS_ZOMBIE_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 1.0,
}];
static OPS_ZOMBIE_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_ZOMBIE_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 1.0,
}];
static OPS_ZOMBIE_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_ZOMBIE_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 3.0,
}];
static OPS_ZOMBIE_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_ZOMBIE_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_ZOMBIE_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_ZOMBIE_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ZOMBIE_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ZOMBIE_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ZOMBIE_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK19: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Walk,
        distance: 0.0,
    },
    MonsterOperation::Sound {
        path: "zombie/z_idle.wav",
        channel: Q1SoundChannel::Voice,
        attenuation: 2.0,
        comparison: SoundComparison::Less,
        chance: Some(0.2),
    },
];
static OPS_ZOMBIE_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ZOMBIE_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_ZOMBIE_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_ZOMBIE_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 1.0,
}];
static OPS_ZOMBIE_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_ZOMBIE_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];

/// Monster frames sorted by name (`monsterFrames`).
pub static MONSTER_FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "boss_death1",
        MonsterFrame {
            frame: 48,
            next: "boss_death2",
            operations: OPS_BOSS_DEATH1,
        },
    ),
    (
        "boss_death10",
        MonsterFrame {
            frame: 56,
            next: "boss_death10",
            operations: OPS_BOSS_DEATH10,
        },
    ),
    (
        "boss_death2",
        MonsterFrame {
            frame: 49,
            next: "boss_death3",
            operations: OPS_BOSS_DEATH2,
        },
    ),
    (
        "boss_death3",
        MonsterFrame {
            frame: 50,
            next: "boss_death4",
            operations: OPS_BOSS_DEATH3,
        },
    ),
    (
        "boss_death4",
        MonsterFrame {
            frame: 51,
            next: "boss_death5",
            operations: OPS_BOSS_DEATH4,
        },
    ),
    (
        "boss_death5",
        MonsterFrame {
            frame: 52,
            next: "boss_death6",
            operations: OPS_BOSS_DEATH5,
        },
    ),
    (
        "boss_death6",
        MonsterFrame {
            frame: 53,
            next: "boss_death7",
            operations: OPS_BOSS_DEATH6,
        },
    ),
    (
        "boss_death7",
        MonsterFrame {
            frame: 54,
            next: "boss_death8",
            operations: OPS_BOSS_DEATH7,
        },
    ),
    (
        "boss_death8",
        MonsterFrame {
            frame: 55,
            next: "boss_death9",
            operations: OPS_BOSS_DEATH8,
        },
    ),
    (
        "boss_death9",
        MonsterFrame {
            frame: 56,
            next: "boss_death10",
            operations: OPS_BOSS_DEATH9,
        },
    ),
    (
        "boss_idle1",
        MonsterFrame {
            frame: 17,
            next: "boss_idle2",
            operations: OPS_BOSS_IDLE1,
        },
    ),
    (
        "boss_idle10",
        MonsterFrame {
            frame: 26,
            next: "boss_idle11",
            operations: OPS_BOSS_IDLE10,
        },
    ),
    (
        "boss_idle11",
        MonsterFrame {
            frame: 27,
            next: "boss_idle12",
            operations: OPS_BOSS_IDLE11,
        },
    ),
    (
        "boss_idle12",
        MonsterFrame {
            frame: 28,
            next: "boss_idle13",
            operations: OPS_BOSS_IDLE12,
        },
    ),
    (
        "boss_idle13",
        MonsterFrame {
            frame: 29,
            next: "boss_idle14",
            operations: OPS_BOSS_IDLE13,
        },
    ),
    (
        "boss_idle14",
        MonsterFrame {
            frame: 30,
            next: "boss_idle15",
            operations: OPS_BOSS_IDLE14,
        },
    ),
    (
        "boss_idle15",
        MonsterFrame {
            frame: 31,
            next: "boss_idle16",
            operations: OPS_BOSS_IDLE15,
        },
    ),
    (
        "boss_idle16",
        MonsterFrame {
            frame: 32,
            next: "boss_idle17",
            operations: OPS_BOSS_IDLE16,
        },
    ),
    (
        "boss_idle17",
        MonsterFrame {
            frame: 33,
            next: "boss_idle18",
            operations: OPS_BOSS_IDLE17,
        },
    ),
    (
        "boss_idle18",
        MonsterFrame {
            frame: 34,
            next: "boss_idle19",
            operations: OPS_BOSS_IDLE18,
        },
    ),
    (
        "boss_idle19",
        MonsterFrame {
            frame: 35,
            next: "boss_idle20",
            operations: OPS_BOSS_IDLE19,
        },
    ),
    (
        "boss_idle2",
        MonsterFrame {
            frame: 18,
            next: "boss_idle3",
            operations: OPS_BOSS_IDLE2,
        },
    ),
    (
        "boss_idle20",
        MonsterFrame {
            frame: 36,
            next: "boss_idle21",
            operations: OPS_BOSS_IDLE20,
        },
    ),
    (
        "boss_idle21",
        MonsterFrame {
            frame: 37,
            next: "boss_idle22",
            operations: OPS_BOSS_IDLE21,
        },
    ),
    (
        "boss_idle22",
        MonsterFrame {
            frame: 38,
            next: "boss_idle23",
            operations: OPS_BOSS_IDLE22,
        },
    ),
    (
        "boss_idle23",
        MonsterFrame {
            frame: 39,
            next: "boss_idle24",
            operations: OPS_BOSS_IDLE23,
        },
    ),
    (
        "boss_idle24",
        MonsterFrame {
            frame: 40,
            next: "boss_idle25",
            operations: OPS_BOSS_IDLE24,
        },
    ),
    (
        "boss_idle25",
        MonsterFrame {
            frame: 41,
            next: "boss_idle26",
            operations: OPS_BOSS_IDLE25,
        },
    ),
    (
        "boss_idle26",
        MonsterFrame {
            frame: 42,
            next: "boss_idle27",
            operations: OPS_BOSS_IDLE26,
        },
    ),
    (
        "boss_idle27",
        MonsterFrame {
            frame: 43,
            next: "boss_idle28",
            operations: OPS_BOSS_IDLE27,
        },
    ),
    (
        "boss_idle28",
        MonsterFrame {
            frame: 44,
            next: "boss_idle29",
            operations: OPS_BOSS_IDLE28,
        },
    ),
    (
        "boss_idle29",
        MonsterFrame {
            frame: 45,
            next: "boss_idle30",
            operations: OPS_BOSS_IDLE29,
        },
    ),
    (
        "boss_idle3",
        MonsterFrame {
            frame: 19,
            next: "boss_idle4",
            operations: OPS_BOSS_IDLE3,
        },
    ),
    (
        "boss_idle30",
        MonsterFrame {
            frame: 46,
            next: "boss_idle31",
            operations: OPS_BOSS_IDLE30,
        },
    ),
    (
        "boss_idle31",
        MonsterFrame {
            frame: 47,
            next: "boss_idle1",
            operations: OPS_BOSS_IDLE31,
        },
    ),
    (
        "boss_idle4",
        MonsterFrame {
            frame: 20,
            next: "boss_idle5",
            operations: OPS_BOSS_IDLE4,
        },
    ),
    (
        "boss_idle5",
        MonsterFrame {
            frame: 21,
            next: "boss_idle6",
            operations: OPS_BOSS_IDLE5,
        },
    ),
    (
        "boss_idle6",
        MonsterFrame {
            frame: 22,
            next: "boss_idle7",
            operations: OPS_BOSS_IDLE6,
        },
    ),
    (
        "boss_idle7",
        MonsterFrame {
            frame: 23,
            next: "boss_idle8",
            operations: OPS_BOSS_IDLE7,
        },
    ),
    (
        "boss_idle8",
        MonsterFrame {
            frame: 24,
            next: "boss_idle9",
            operations: OPS_BOSS_IDLE8,
        },
    ),
    (
        "boss_idle9",
        MonsterFrame {
            frame: 25,
            next: "boss_idle10",
            operations: OPS_BOSS_IDLE9,
        },
    ),
    (
        "boss_missile1",
        MonsterFrame {
            frame: 57,
            next: "boss_missile2",
            operations: OPS_BOSS_MISSILE1,
        },
    ),
    (
        "boss_missile10",
        MonsterFrame {
            frame: 66,
            next: "boss_missile11",
            operations: OPS_BOSS_MISSILE10,
        },
    ),
    (
        "boss_missile11",
        MonsterFrame {
            frame: 67,
            next: "boss_missile12",
            operations: OPS_BOSS_MISSILE11,
        },
    ),
    (
        "boss_missile12",
        MonsterFrame {
            frame: 68,
            next: "boss_missile13",
            operations: OPS_BOSS_MISSILE12,
        },
    ),
    (
        "boss_missile13",
        MonsterFrame {
            frame: 69,
            next: "boss_missile14",
            operations: OPS_BOSS_MISSILE13,
        },
    ),
    (
        "boss_missile14",
        MonsterFrame {
            frame: 70,
            next: "boss_missile15",
            operations: OPS_BOSS_MISSILE14,
        },
    ),
    (
        "boss_missile15",
        MonsterFrame {
            frame: 71,
            next: "boss_missile16",
            operations: OPS_BOSS_MISSILE15,
        },
    ),
    (
        "boss_missile16",
        MonsterFrame {
            frame: 72,
            next: "boss_missile17",
            operations: OPS_BOSS_MISSILE16,
        },
    ),
    (
        "boss_missile17",
        MonsterFrame {
            frame: 73,
            next: "boss_missile18",
            operations: OPS_BOSS_MISSILE17,
        },
    ),
    (
        "boss_missile18",
        MonsterFrame {
            frame: 74,
            next: "boss_missile19",
            operations: OPS_BOSS_MISSILE18,
        },
    ),
    (
        "boss_missile19",
        MonsterFrame {
            frame: 75,
            next: "boss_missile20",
            operations: OPS_BOSS_MISSILE19,
        },
    ),
    (
        "boss_missile2",
        MonsterFrame {
            frame: 58,
            next: "boss_missile3",
            operations: OPS_BOSS_MISSILE2,
        },
    ),
    (
        "boss_missile20",
        MonsterFrame {
            frame: 76,
            next: "boss_missile21",
            operations: OPS_BOSS_MISSILE20,
        },
    ),
    (
        "boss_missile21",
        MonsterFrame {
            frame: 77,
            next: "boss_missile22",
            operations: OPS_BOSS_MISSILE21,
        },
    ),
    (
        "boss_missile22",
        MonsterFrame {
            frame: 78,
            next: "boss_missile23",
            operations: OPS_BOSS_MISSILE22,
        },
    ),
    (
        "boss_missile23",
        MonsterFrame {
            frame: 79,
            next: "boss_missile1",
            operations: OPS_BOSS_MISSILE23,
        },
    ),
    (
        "boss_missile3",
        MonsterFrame {
            frame: 59,
            next: "boss_missile4",
            operations: OPS_BOSS_MISSILE3,
        },
    ),
    (
        "boss_missile4",
        MonsterFrame {
            frame: 60,
            next: "boss_missile5",
            operations: OPS_BOSS_MISSILE4,
        },
    ),
    (
        "boss_missile5",
        MonsterFrame {
            frame: 61,
            next: "boss_missile6",
            operations: OPS_BOSS_MISSILE5,
        },
    ),
    (
        "boss_missile6",
        MonsterFrame {
            frame: 62,
            next: "boss_missile7",
            operations: OPS_BOSS_MISSILE6,
        },
    ),
    (
        "boss_missile7",
        MonsterFrame {
            frame: 63,
            next: "boss_missile8",
            operations: OPS_BOSS_MISSILE7,
        },
    ),
    (
        "boss_missile8",
        MonsterFrame {
            frame: 64,
            next: "boss_missile9",
            operations: OPS_BOSS_MISSILE8,
        },
    ),
    (
        "boss_missile9",
        MonsterFrame {
            frame: 65,
            next: "boss_missile10",
            operations: OPS_BOSS_MISSILE9,
        },
    ),
    (
        "boss_rise1",
        MonsterFrame {
            frame: 0,
            next: "boss_rise2",
            operations: OPS_BOSS_RISE1,
        },
    ),
    (
        "boss_rise10",
        MonsterFrame {
            frame: 9,
            next: "boss_rise11",
            operations: OPS_BOSS_RISE10,
        },
    ),
    (
        "boss_rise11",
        MonsterFrame {
            frame: 10,
            next: "boss_rise12",
            operations: OPS_BOSS_RISE11,
        },
    ),
    (
        "boss_rise12",
        MonsterFrame {
            frame: 11,
            next: "boss_rise13",
            operations: OPS_BOSS_RISE12,
        },
    ),
    (
        "boss_rise13",
        MonsterFrame {
            frame: 12,
            next: "boss_rise14",
            operations: OPS_BOSS_RISE13,
        },
    ),
    (
        "boss_rise14",
        MonsterFrame {
            frame: 13,
            next: "boss_rise15",
            operations: OPS_BOSS_RISE14,
        },
    ),
    (
        "boss_rise15",
        MonsterFrame {
            frame: 14,
            next: "boss_rise16",
            operations: OPS_BOSS_RISE15,
        },
    ),
    (
        "boss_rise16",
        MonsterFrame {
            frame: 15,
            next: "boss_rise17",
            operations: OPS_BOSS_RISE16,
        },
    ),
    (
        "boss_rise17",
        MonsterFrame {
            frame: 16,
            next: "boss_missile1",
            operations: OPS_BOSS_RISE17,
        },
    ),
    (
        "boss_rise2",
        MonsterFrame {
            frame: 1,
            next: "boss_rise3",
            operations: OPS_BOSS_RISE2,
        },
    ),
    (
        "boss_rise3",
        MonsterFrame {
            frame: 2,
            next: "boss_rise4",
            operations: OPS_BOSS_RISE3,
        },
    ),
    (
        "boss_rise4",
        MonsterFrame {
            frame: 3,
            next: "boss_rise5",
            operations: OPS_BOSS_RISE4,
        },
    ),
    (
        "boss_rise5",
        MonsterFrame {
            frame: 4,
            next: "boss_rise6",
            operations: OPS_BOSS_RISE5,
        },
    ),
    (
        "boss_rise6",
        MonsterFrame {
            frame: 5,
            next: "boss_rise7",
            operations: OPS_BOSS_RISE6,
        },
    ),
    (
        "boss_rise7",
        MonsterFrame {
            frame: 6,
            next: "boss_rise8",
            operations: OPS_BOSS_RISE7,
        },
    ),
    (
        "boss_rise8",
        MonsterFrame {
            frame: 7,
            next: "boss_rise9",
            operations: OPS_BOSS_RISE8,
        },
    ),
    (
        "boss_rise9",
        MonsterFrame {
            frame: 8,
            next: "boss_rise10",
            operations: OPS_BOSS_RISE9,
        },
    ),
    (
        "boss_shocka1",
        MonsterFrame {
            frame: 80,
            next: "boss_shocka2",
            operations: OPS_BOSS_SHOCKA1,
        },
    ),
    (
        "boss_shocka10",
        MonsterFrame {
            frame: 89,
            next: "boss_missile1",
            operations: OPS_BOSS_SHOCKA10,
        },
    ),
    (
        "boss_shocka2",
        MonsterFrame {
            frame: 81,
            next: "boss_shocka3",
            operations: OPS_BOSS_SHOCKA2,
        },
    ),
    (
        "boss_shocka3",
        MonsterFrame {
            frame: 82,
            next: "boss_shocka4",
            operations: OPS_BOSS_SHOCKA3,
        },
    ),
    (
        "boss_shocka4",
        MonsterFrame {
            frame: 83,
            next: "boss_shocka5",
            operations: OPS_BOSS_SHOCKA4,
        },
    ),
    (
        "boss_shocka5",
        MonsterFrame {
            frame: 84,
            next: "boss_shocka6",
            operations: OPS_BOSS_SHOCKA5,
        },
    ),
    (
        "boss_shocka6",
        MonsterFrame {
            frame: 85,
            next: "boss_shocka7",
            operations: OPS_BOSS_SHOCKA6,
        },
    ),
    (
        "boss_shocka7",
        MonsterFrame {
            frame: 86,
            next: "boss_shocka8",
            operations: OPS_BOSS_SHOCKA7,
        },
    ),
    (
        "boss_shocka8",
        MonsterFrame {
            frame: 87,
            next: "boss_shocka9",
            operations: OPS_BOSS_SHOCKA8,
        },
    ),
    (
        "boss_shocka9",
        MonsterFrame {
            frame: 88,
            next: "boss_shocka10",
            operations: OPS_BOSS_SHOCKA9,
        },
    ),
    (
        "boss_shockb1",
        MonsterFrame {
            frame: 90,
            next: "boss_shockb2",
            operations: OPS_BOSS_SHOCKB1,
        },
    ),
    (
        "boss_shockb10",
        MonsterFrame {
            frame: 93,
            next: "boss_missile1",
            operations: OPS_BOSS_SHOCKB10,
        },
    ),
    (
        "boss_shockb2",
        MonsterFrame {
            frame: 91,
            next: "boss_shockb3",
            operations: OPS_BOSS_SHOCKB2,
        },
    ),
    (
        "boss_shockb3",
        MonsterFrame {
            frame: 92,
            next: "boss_shockb4",
            operations: OPS_BOSS_SHOCKB3,
        },
    ),
    (
        "boss_shockb4",
        MonsterFrame {
            frame: 93,
            next: "boss_shockb5",
            operations: OPS_BOSS_SHOCKB4,
        },
    ),
    (
        "boss_shockb5",
        MonsterFrame {
            frame: 94,
            next: "boss_shockb6",
            operations: OPS_BOSS_SHOCKB5,
        },
    ),
    (
        "boss_shockb6",
        MonsterFrame {
            frame: 95,
            next: "boss_shockb7",
            operations: OPS_BOSS_SHOCKB6,
        },
    ),
    (
        "boss_shockb7",
        MonsterFrame {
            frame: 90,
            next: "boss_shockb8",
            operations: OPS_BOSS_SHOCKB7,
        },
    ),
    (
        "boss_shockb8",
        MonsterFrame {
            frame: 91,
            next: "boss_shockb9",
            operations: OPS_BOSS_SHOCKB8,
        },
    ),
    (
        "boss_shockb9",
        MonsterFrame {
            frame: 92,
            next: "boss_shockb10",
            operations: OPS_BOSS_SHOCKB9,
        },
    ),
    (
        "boss_shockc1",
        MonsterFrame {
            frame: 96,
            next: "boss_shockc2",
            operations: OPS_BOSS_SHOCKC1,
        },
    ),
    (
        "boss_shockc10",
        MonsterFrame {
            frame: 105,
            next: "boss_death1",
            operations: OPS_BOSS_SHOCKC10,
        },
    ),
    (
        "boss_shockc2",
        MonsterFrame {
            frame: 97,
            next: "boss_shockc3",
            operations: OPS_BOSS_SHOCKC2,
        },
    ),
    (
        "boss_shockc3",
        MonsterFrame {
            frame: 98,
            next: "boss_shockc4",
            operations: OPS_BOSS_SHOCKC3,
        },
    ),
    (
        "boss_shockc4",
        MonsterFrame {
            frame: 99,
            next: "boss_shockc5",
            operations: OPS_BOSS_SHOCKC4,
        },
    ),
    (
        "boss_shockc5",
        MonsterFrame {
            frame: 100,
            next: "boss_shockc6",
            operations: OPS_BOSS_SHOCKC5,
        },
    ),
    (
        "boss_shockc6",
        MonsterFrame {
            frame: 101,
            next: "boss_shockc7",
            operations: OPS_BOSS_SHOCKC6,
        },
    ),
    (
        "boss_shockc7",
        MonsterFrame {
            frame: 102,
            next: "boss_shockc8",
            operations: OPS_BOSS_SHOCKC7,
        },
    ),
    (
        "boss_shockc8",
        MonsterFrame {
            frame: 103,
            next: "boss_shockc9",
            operations: OPS_BOSS_SHOCKC8,
        },
    ),
    (
        "boss_shockc9",
        MonsterFrame {
            frame: 104,
            next: "boss_shockc10",
            operations: OPS_BOSS_SHOCKC9,
        },
    ),
    (
        "demon1_atta1",
        MonsterFrame {
            frame: 54,
            next: "demon1_atta2",
            operations: OPS_DEMON1_ATTA1,
        },
    ),
    (
        "demon1_atta10",
        MonsterFrame {
            frame: 63,
            next: "demon1_atta11",
            operations: OPS_DEMON1_ATTA10,
        },
    ),
    (
        "demon1_atta11",
        MonsterFrame {
            frame: 64,
            next: "demon1_atta12",
            operations: OPS_DEMON1_ATTA11,
        },
    ),
    (
        "demon1_atta12",
        MonsterFrame {
            frame: 65,
            next: "demon1_atta13",
            operations: OPS_DEMON1_ATTA12,
        },
    ),
    (
        "demon1_atta13",
        MonsterFrame {
            frame: 66,
            next: "demon1_atta14",
            operations: OPS_DEMON1_ATTA13,
        },
    ),
    (
        "demon1_atta14",
        MonsterFrame {
            frame: 67,
            next: "demon1_atta15",
            operations: OPS_DEMON1_ATTA14,
        },
    ),
    (
        "demon1_atta15",
        MonsterFrame {
            frame: 68,
            next: "demon1_run1",
            operations: OPS_DEMON1_ATTA15,
        },
    ),
    (
        "demon1_atta2",
        MonsterFrame {
            frame: 55,
            next: "demon1_atta3",
            operations: OPS_DEMON1_ATTA2,
        },
    ),
    (
        "demon1_atta3",
        MonsterFrame {
            frame: 56,
            next: "demon1_atta4",
            operations: OPS_DEMON1_ATTA3,
        },
    ),
    (
        "demon1_atta4",
        MonsterFrame {
            frame: 57,
            next: "demon1_atta5",
            operations: OPS_DEMON1_ATTA4,
        },
    ),
    (
        "demon1_atta5",
        MonsterFrame {
            frame: 58,
            next: "demon1_atta6",
            operations: OPS_DEMON1_ATTA5,
        },
    ),
    (
        "demon1_atta6",
        MonsterFrame {
            frame: 59,
            next: "demon1_atta7",
            operations: OPS_DEMON1_ATTA6,
        },
    ),
    (
        "demon1_atta7",
        MonsterFrame {
            frame: 60,
            next: "demon1_atta8",
            operations: OPS_DEMON1_ATTA7,
        },
    ),
    (
        "demon1_atta8",
        MonsterFrame {
            frame: 61,
            next: "demon1_atta9",
            operations: OPS_DEMON1_ATTA8,
        },
    ),
    (
        "demon1_atta9",
        MonsterFrame {
            frame: 62,
            next: "demon1_atta10",
            operations: OPS_DEMON1_ATTA9,
        },
    ),
    (
        "demon1_die1",
        MonsterFrame {
            frame: 45,
            next: "demon1_die2",
            operations: OPS_DEMON1_DIE1,
        },
    ),
    (
        "demon1_die2",
        MonsterFrame {
            frame: 46,
            next: "demon1_die3",
            operations: OPS_DEMON1_DIE2,
        },
    ),
    (
        "demon1_die3",
        MonsterFrame {
            frame: 47,
            next: "demon1_die4",
            operations: OPS_DEMON1_DIE3,
        },
    ),
    (
        "demon1_die4",
        MonsterFrame {
            frame: 48,
            next: "demon1_die5",
            operations: OPS_DEMON1_DIE4,
        },
    ),
    (
        "demon1_die5",
        MonsterFrame {
            frame: 49,
            next: "demon1_die6",
            operations: OPS_DEMON1_DIE5,
        },
    ),
    (
        "demon1_die6",
        MonsterFrame {
            frame: 50,
            next: "demon1_die7",
            operations: OPS_DEMON1_DIE6,
        },
    ),
    (
        "demon1_die7",
        MonsterFrame {
            frame: 51,
            next: "demon1_die8",
            operations: OPS_DEMON1_DIE7,
        },
    ),
    (
        "demon1_die8",
        MonsterFrame {
            frame: 52,
            next: "demon1_die9",
            operations: OPS_DEMON1_DIE8,
        },
    ),
    (
        "demon1_die9",
        MonsterFrame {
            frame: 53,
            next: "demon1_die9",
            operations: OPS_DEMON1_DIE9,
        },
    ),
    (
        "demon1_jump1",
        MonsterFrame {
            frame: 27,
            next: "demon1_jump2",
            operations: OPS_DEMON1_JUMP1,
        },
    ),
    (
        "demon1_jump10",
        MonsterFrame {
            frame: 36,
            next: "demon1_jump1",
            operations: OPS_DEMON1_JUMP10,
        },
    ),
    (
        "demon1_jump11",
        MonsterFrame {
            frame: 37,
            next: "demon1_jump12",
            operations: OPS_DEMON1_JUMP11,
        },
    ),
    (
        "demon1_jump12",
        MonsterFrame {
            frame: 38,
            next: "demon1_run1",
            operations: OPS_DEMON1_JUMP12,
        },
    ),
    (
        "demon1_jump2",
        MonsterFrame {
            frame: 28,
            next: "demon1_jump3",
            operations: OPS_DEMON1_JUMP2,
        },
    ),
    (
        "demon1_jump3",
        MonsterFrame {
            frame: 29,
            next: "demon1_jump4",
            operations: OPS_DEMON1_JUMP3,
        },
    ),
    (
        "demon1_jump4",
        MonsterFrame {
            frame: 30,
            next: "demon1_jump5",
            operations: OPS_DEMON1_JUMP4,
        },
    ),
    (
        "demon1_jump5",
        MonsterFrame {
            frame: 31,
            next: "demon1_jump6",
            operations: OPS_DEMON1_JUMP5,
        },
    ),
    (
        "demon1_jump6",
        MonsterFrame {
            frame: 32,
            next: "demon1_jump7",
            operations: OPS_DEMON1_JUMP6,
        },
    ),
    (
        "demon1_jump7",
        MonsterFrame {
            frame: 33,
            next: "demon1_jump8",
            operations: OPS_DEMON1_JUMP7,
        },
    ),
    (
        "demon1_jump8",
        MonsterFrame {
            frame: 34,
            next: "demon1_jump9",
            operations: OPS_DEMON1_JUMP8,
        },
    ),
    (
        "demon1_jump9",
        MonsterFrame {
            frame: 35,
            next: "demon1_jump10",
            operations: OPS_DEMON1_JUMP9,
        },
    ),
    (
        "demon1_pain1",
        MonsterFrame {
            frame: 39,
            next: "demon1_pain2",
            operations: OPS_DEMON1_PAIN1,
        },
    ),
    (
        "demon1_pain2",
        MonsterFrame {
            frame: 40,
            next: "demon1_pain3",
            operations: OPS_DEMON1_PAIN2,
        },
    ),
    (
        "demon1_pain3",
        MonsterFrame {
            frame: 41,
            next: "demon1_pain4",
            operations: OPS_DEMON1_PAIN3,
        },
    ),
    (
        "demon1_pain4",
        MonsterFrame {
            frame: 42,
            next: "demon1_pain5",
            operations: OPS_DEMON1_PAIN4,
        },
    ),
    (
        "demon1_pain5",
        MonsterFrame {
            frame: 43,
            next: "demon1_pain6",
            operations: OPS_DEMON1_PAIN5,
        },
    ),
    (
        "demon1_pain6",
        MonsterFrame {
            frame: 44,
            next: "demon1_run1",
            operations: OPS_DEMON1_PAIN6,
        },
    ),
    (
        "demon1_run1",
        MonsterFrame {
            frame: 21,
            next: "demon1_run2",
            operations: OPS_DEMON1_RUN1,
        },
    ),
    (
        "demon1_run2",
        MonsterFrame {
            frame: 22,
            next: "demon1_run3",
            operations: OPS_DEMON1_RUN2,
        },
    ),
    (
        "demon1_run3",
        MonsterFrame {
            frame: 23,
            next: "demon1_run4",
            operations: OPS_DEMON1_RUN3,
        },
    ),
    (
        "demon1_run4",
        MonsterFrame {
            frame: 24,
            next: "demon1_run5",
            operations: OPS_DEMON1_RUN4,
        },
    ),
    (
        "demon1_run5",
        MonsterFrame {
            frame: 25,
            next: "demon1_run6",
            operations: OPS_DEMON1_RUN5,
        },
    ),
    (
        "demon1_run6",
        MonsterFrame {
            frame: 26,
            next: "demon1_run1",
            operations: OPS_DEMON1_RUN6,
        },
    ),
    (
        "demon1_stand1",
        MonsterFrame {
            frame: 0,
            next: "demon1_stand2",
            operations: OPS_DEMON1_STAND1,
        },
    ),
    (
        "demon1_stand10",
        MonsterFrame {
            frame: 9,
            next: "demon1_stand11",
            operations: OPS_DEMON1_STAND10,
        },
    ),
    (
        "demon1_stand11",
        MonsterFrame {
            frame: 10,
            next: "demon1_stand12",
            operations: OPS_DEMON1_STAND11,
        },
    ),
    (
        "demon1_stand12",
        MonsterFrame {
            frame: 11,
            next: "demon1_stand13",
            operations: OPS_DEMON1_STAND12,
        },
    ),
    (
        "demon1_stand13",
        MonsterFrame {
            frame: 12,
            next: "demon1_stand1",
            operations: OPS_DEMON1_STAND13,
        },
    ),
    (
        "demon1_stand2",
        MonsterFrame {
            frame: 1,
            next: "demon1_stand3",
            operations: OPS_DEMON1_STAND2,
        },
    ),
    (
        "demon1_stand3",
        MonsterFrame {
            frame: 2,
            next: "demon1_stand4",
            operations: OPS_DEMON1_STAND3,
        },
    ),
    (
        "demon1_stand4",
        MonsterFrame {
            frame: 3,
            next: "demon1_stand5",
            operations: OPS_DEMON1_STAND4,
        },
    ),
    (
        "demon1_stand5",
        MonsterFrame {
            frame: 4,
            next: "demon1_stand6",
            operations: OPS_DEMON1_STAND5,
        },
    ),
    (
        "demon1_stand6",
        MonsterFrame {
            frame: 5,
            next: "demon1_stand7",
            operations: OPS_DEMON1_STAND6,
        },
    ),
    (
        "demon1_stand7",
        MonsterFrame {
            frame: 6,
            next: "demon1_stand8",
            operations: OPS_DEMON1_STAND7,
        },
    ),
    (
        "demon1_stand8",
        MonsterFrame {
            frame: 7,
            next: "demon1_stand9",
            operations: OPS_DEMON1_STAND8,
        },
    ),
    (
        "demon1_stand9",
        MonsterFrame {
            frame: 8,
            next: "demon1_stand10",
            operations: OPS_DEMON1_STAND9,
        },
    ),
    (
        "demon1_walk1",
        MonsterFrame {
            frame: 13,
            next: "demon1_walk2",
            operations: OPS_DEMON1_WALK1,
        },
    ),
    (
        "demon1_walk2",
        MonsterFrame {
            frame: 14,
            next: "demon1_walk3",
            operations: OPS_DEMON1_WALK2,
        },
    ),
    (
        "demon1_walk3",
        MonsterFrame {
            frame: 15,
            next: "demon1_walk4",
            operations: OPS_DEMON1_WALK3,
        },
    ),
    (
        "demon1_walk4",
        MonsterFrame {
            frame: 16,
            next: "demon1_walk5",
            operations: OPS_DEMON1_WALK4,
        },
    ),
    (
        "demon1_walk5",
        MonsterFrame {
            frame: 17,
            next: "demon1_walk6",
            operations: OPS_DEMON1_WALK5,
        },
    ),
    (
        "demon1_walk6",
        MonsterFrame {
            frame: 18,
            next: "demon1_walk7",
            operations: OPS_DEMON1_WALK6,
        },
    ),
    (
        "demon1_walk7",
        MonsterFrame {
            frame: 19,
            next: "demon1_walk8",
            operations: OPS_DEMON1_WALK7,
        },
    ),
    (
        "demon1_walk8",
        MonsterFrame {
            frame: 20,
            next: "demon1_walk1",
            operations: OPS_DEMON1_WALK8,
        },
    ),
    (
        "enf_atk1",
        MonsterFrame {
            frame: 31,
            next: "enf_atk2",
            operations: OPS_ENF_ATK1,
        },
    ),
    (
        "enf_atk10",
        MonsterFrame {
            frame: 36,
            next: "enf_atk11",
            operations: OPS_ENF_ATK10,
        },
    ),
    (
        "enf_atk11",
        MonsterFrame {
            frame: 37,
            next: "enf_atk12",
            operations: OPS_ENF_ATK11,
        },
    ),
    (
        "enf_atk12",
        MonsterFrame {
            frame: 38,
            next: "enf_atk13",
            operations: OPS_ENF_ATK12,
        },
    ),
    (
        "enf_atk13",
        MonsterFrame {
            frame: 39,
            next: "enf_atk14",
            operations: OPS_ENF_ATK13,
        },
    ),
    (
        "enf_atk14",
        MonsterFrame {
            frame: 40,
            next: "enf_run1",
            operations: OPS_ENF_ATK14,
        },
    ),
    (
        "enf_atk2",
        MonsterFrame {
            frame: 32,
            next: "enf_atk3",
            operations: OPS_ENF_ATK2,
        },
    ),
    (
        "enf_atk3",
        MonsterFrame {
            frame: 33,
            next: "enf_atk4",
            operations: OPS_ENF_ATK3,
        },
    ),
    (
        "enf_atk4",
        MonsterFrame {
            frame: 34,
            next: "enf_atk5",
            operations: OPS_ENF_ATK4,
        },
    ),
    (
        "enf_atk5",
        MonsterFrame {
            frame: 35,
            next: "enf_atk6",
            operations: OPS_ENF_ATK5,
        },
    ),
    (
        "enf_atk6",
        MonsterFrame {
            frame: 36,
            next: "enf_atk7",
            operations: OPS_ENF_ATK6,
        },
    ),
    (
        "enf_atk7",
        MonsterFrame {
            frame: 37,
            next: "enf_atk8",
            operations: OPS_ENF_ATK7,
        },
    ),
    (
        "enf_atk8",
        MonsterFrame {
            frame: 38,
            next: "enf_atk9",
            operations: OPS_ENF_ATK8,
        },
    ),
    (
        "enf_atk9",
        MonsterFrame {
            frame: 35,
            next: "enf_atk10",
            operations: OPS_ENF_ATK9,
        },
    ),
    (
        "enf_die1",
        MonsterFrame {
            frame: 41,
            next: "enf_die2",
            operations: OPS_ENF_DIE1,
        },
    ),
    (
        "enf_die10",
        MonsterFrame {
            frame: 50,
            next: "enf_die11",
            operations: OPS_ENF_DIE10,
        },
    ),
    (
        "enf_die11",
        MonsterFrame {
            frame: 51,
            next: "enf_die12",
            operations: OPS_ENF_DIE11,
        },
    ),
    (
        "enf_die12",
        MonsterFrame {
            frame: 52,
            next: "enf_die13",
            operations: OPS_ENF_DIE12,
        },
    ),
    (
        "enf_die13",
        MonsterFrame {
            frame: 53,
            next: "enf_die14",
            operations: OPS_ENF_DIE13,
        },
    ),
    (
        "enf_die14",
        MonsterFrame {
            frame: 54,
            next: "enf_die14",
            operations: OPS_ENF_DIE14,
        },
    ),
    (
        "enf_die2",
        MonsterFrame {
            frame: 42,
            next: "enf_die3",
            operations: OPS_ENF_DIE2,
        },
    ),
    (
        "enf_die3",
        MonsterFrame {
            frame: 43,
            next: "enf_die4",
            operations: OPS_ENF_DIE3,
        },
    ),
    (
        "enf_die4",
        MonsterFrame {
            frame: 44,
            next: "enf_die5",
            operations: OPS_ENF_DIE4,
        },
    ),
    (
        "enf_die5",
        MonsterFrame {
            frame: 45,
            next: "enf_die6",
            operations: OPS_ENF_DIE5,
        },
    ),
    (
        "enf_die6",
        MonsterFrame {
            frame: 46,
            next: "enf_die7",
            operations: OPS_ENF_DIE6,
        },
    ),
    (
        "enf_die7",
        MonsterFrame {
            frame: 47,
            next: "enf_die8",
            operations: OPS_ENF_DIE7,
        },
    ),
    (
        "enf_die8",
        MonsterFrame {
            frame: 48,
            next: "enf_die9",
            operations: OPS_ENF_DIE8,
        },
    ),
    (
        "enf_die9",
        MonsterFrame {
            frame: 49,
            next: "enf_die10",
            operations: OPS_ENF_DIE9,
        },
    ),
    (
        "enf_fdie1",
        MonsterFrame {
            frame: 55,
            next: "enf_fdie2",
            operations: OPS_ENF_FDIE1,
        },
    ),
    (
        "enf_fdie10",
        MonsterFrame {
            frame: 64,
            next: "enf_fdie11",
            operations: OPS_ENF_FDIE10,
        },
    ),
    (
        "enf_fdie11",
        MonsterFrame {
            frame: 65,
            next: "enf_fdie11",
            operations: OPS_ENF_FDIE11,
        },
    ),
    (
        "enf_fdie2",
        MonsterFrame {
            frame: 56,
            next: "enf_fdie3",
            operations: OPS_ENF_FDIE2,
        },
    ),
    (
        "enf_fdie3",
        MonsterFrame {
            frame: 57,
            next: "enf_fdie4",
            operations: OPS_ENF_FDIE3,
        },
    ),
    (
        "enf_fdie4",
        MonsterFrame {
            frame: 58,
            next: "enf_fdie5",
            operations: OPS_ENF_FDIE4,
        },
    ),
    (
        "enf_fdie5",
        MonsterFrame {
            frame: 59,
            next: "enf_fdie6",
            operations: OPS_ENF_FDIE5,
        },
    ),
    (
        "enf_fdie6",
        MonsterFrame {
            frame: 60,
            next: "enf_fdie7",
            operations: OPS_ENF_FDIE6,
        },
    ),
    (
        "enf_fdie7",
        MonsterFrame {
            frame: 61,
            next: "enf_fdie8",
            operations: OPS_ENF_FDIE7,
        },
    ),
    (
        "enf_fdie8",
        MonsterFrame {
            frame: 62,
            next: "enf_fdie9",
            operations: OPS_ENF_FDIE8,
        },
    ),
    (
        "enf_fdie9",
        MonsterFrame {
            frame: 63,
            next: "enf_fdie10",
            operations: OPS_ENF_FDIE9,
        },
    ),
    (
        "enf_paina1",
        MonsterFrame {
            frame: 66,
            next: "enf_paina2",
            operations: OPS_ENF_PAINA1,
        },
    ),
    (
        "enf_paina2",
        MonsterFrame {
            frame: 67,
            next: "enf_paina3",
            operations: OPS_ENF_PAINA2,
        },
    ),
    (
        "enf_paina3",
        MonsterFrame {
            frame: 68,
            next: "enf_paina4",
            operations: OPS_ENF_PAINA3,
        },
    ),
    (
        "enf_paina4",
        MonsterFrame {
            frame: 69,
            next: "enf_run1",
            operations: OPS_ENF_PAINA4,
        },
    ),
    (
        "enf_painb1",
        MonsterFrame {
            frame: 70,
            next: "enf_painb2",
            operations: OPS_ENF_PAINB1,
        },
    ),
    (
        "enf_painb2",
        MonsterFrame {
            frame: 71,
            next: "enf_painb3",
            operations: OPS_ENF_PAINB2,
        },
    ),
    (
        "enf_painb3",
        MonsterFrame {
            frame: 72,
            next: "enf_painb4",
            operations: OPS_ENF_PAINB3,
        },
    ),
    (
        "enf_painb4",
        MonsterFrame {
            frame: 73,
            next: "enf_painb5",
            operations: OPS_ENF_PAINB4,
        },
    ),
    (
        "enf_painb5",
        MonsterFrame {
            frame: 74,
            next: "enf_run1",
            operations: OPS_ENF_PAINB5,
        },
    ),
    (
        "enf_painc1",
        MonsterFrame {
            frame: 75,
            next: "enf_painc2",
            operations: OPS_ENF_PAINC1,
        },
    ),
    (
        "enf_painc2",
        MonsterFrame {
            frame: 76,
            next: "enf_painc3",
            operations: OPS_ENF_PAINC2,
        },
    ),
    (
        "enf_painc3",
        MonsterFrame {
            frame: 77,
            next: "enf_painc4",
            operations: OPS_ENF_PAINC3,
        },
    ),
    (
        "enf_painc4",
        MonsterFrame {
            frame: 78,
            next: "enf_painc5",
            operations: OPS_ENF_PAINC4,
        },
    ),
    (
        "enf_painc5",
        MonsterFrame {
            frame: 79,
            next: "enf_painc6",
            operations: OPS_ENF_PAINC5,
        },
    ),
    (
        "enf_painc6",
        MonsterFrame {
            frame: 80,
            next: "enf_painc7",
            operations: OPS_ENF_PAINC6,
        },
    ),
    (
        "enf_painc7",
        MonsterFrame {
            frame: 81,
            next: "enf_painc8",
            operations: OPS_ENF_PAINC7,
        },
    ),
    (
        "enf_painc8",
        MonsterFrame {
            frame: 82,
            next: "enf_run1",
            operations: OPS_ENF_PAINC8,
        },
    ),
    (
        "enf_paind1",
        MonsterFrame {
            frame: 83,
            next: "enf_paind2",
            operations: OPS_ENF_PAIND1,
        },
    ),
    (
        "enf_paind10",
        MonsterFrame {
            frame: 92,
            next: "enf_paind11",
            operations: OPS_ENF_PAIND10,
        },
    ),
    (
        "enf_paind11",
        MonsterFrame {
            frame: 93,
            next: "enf_paind12",
            operations: OPS_ENF_PAIND11,
        },
    ),
    (
        "enf_paind12",
        MonsterFrame {
            frame: 94,
            next: "enf_paind13",
            operations: OPS_ENF_PAIND12,
        },
    ),
    (
        "enf_paind13",
        MonsterFrame {
            frame: 95,
            next: "enf_paind14",
            operations: OPS_ENF_PAIND13,
        },
    ),
    (
        "enf_paind14",
        MonsterFrame {
            frame: 96,
            next: "enf_paind15",
            operations: OPS_ENF_PAIND14,
        },
    ),
    (
        "enf_paind15",
        MonsterFrame {
            frame: 97,
            next: "enf_paind16",
            operations: OPS_ENF_PAIND15,
        },
    ),
    (
        "enf_paind16",
        MonsterFrame {
            frame: 98,
            next: "enf_paind17",
            operations: OPS_ENF_PAIND16,
        },
    ),
    (
        "enf_paind17",
        MonsterFrame {
            frame: 99,
            next: "enf_paind18",
            operations: OPS_ENF_PAIND17,
        },
    ),
    (
        "enf_paind18",
        MonsterFrame {
            frame: 100,
            next: "enf_paind19",
            operations: OPS_ENF_PAIND18,
        },
    ),
    (
        "enf_paind19",
        MonsterFrame {
            frame: 101,
            next: "enf_run1",
            operations: OPS_ENF_PAIND19,
        },
    ),
    (
        "enf_paind2",
        MonsterFrame {
            frame: 84,
            next: "enf_paind3",
            operations: OPS_ENF_PAIND2,
        },
    ),
    (
        "enf_paind3",
        MonsterFrame {
            frame: 85,
            next: "enf_paind4",
            operations: OPS_ENF_PAIND3,
        },
    ),
    (
        "enf_paind4",
        MonsterFrame {
            frame: 86,
            next: "enf_paind5",
            operations: OPS_ENF_PAIND4,
        },
    ),
    (
        "enf_paind5",
        MonsterFrame {
            frame: 87,
            next: "enf_paind6",
            operations: OPS_ENF_PAIND5,
        },
    ),
    (
        "enf_paind6",
        MonsterFrame {
            frame: 88,
            next: "enf_paind7",
            operations: OPS_ENF_PAIND6,
        },
    ),
    (
        "enf_paind7",
        MonsterFrame {
            frame: 89,
            next: "enf_paind8",
            operations: OPS_ENF_PAIND7,
        },
    ),
    (
        "enf_paind8",
        MonsterFrame {
            frame: 90,
            next: "enf_paind9",
            operations: OPS_ENF_PAIND8,
        },
    ),
    (
        "enf_paind9",
        MonsterFrame {
            frame: 91,
            next: "enf_paind10",
            operations: OPS_ENF_PAIND9,
        },
    ),
    (
        "enf_run1",
        MonsterFrame {
            frame: 23,
            next: "enf_run2",
            operations: OPS_ENF_RUN1,
        },
    ),
    (
        "enf_run2",
        MonsterFrame {
            frame: 24,
            next: "enf_run3",
            operations: OPS_ENF_RUN2,
        },
    ),
    (
        "enf_run3",
        MonsterFrame {
            frame: 25,
            next: "enf_run4",
            operations: OPS_ENF_RUN3,
        },
    ),
    (
        "enf_run4",
        MonsterFrame {
            frame: 26,
            next: "enf_run5",
            operations: OPS_ENF_RUN4,
        },
    ),
    (
        "enf_run5",
        MonsterFrame {
            frame: 27,
            next: "enf_run6",
            operations: OPS_ENF_RUN5,
        },
    ),
    (
        "enf_run6",
        MonsterFrame {
            frame: 28,
            next: "enf_run7",
            operations: OPS_ENF_RUN6,
        },
    ),
    (
        "enf_run7",
        MonsterFrame {
            frame: 29,
            next: "enf_run8",
            operations: OPS_ENF_RUN7,
        },
    ),
    (
        "enf_run8",
        MonsterFrame {
            frame: 30,
            next: "enf_run1",
            operations: OPS_ENF_RUN8,
        },
    ),
    (
        "enf_stand1",
        MonsterFrame {
            frame: 0,
            next: "enf_stand2",
            operations: OPS_ENF_STAND1,
        },
    ),
    (
        "enf_stand2",
        MonsterFrame {
            frame: 1,
            next: "enf_stand3",
            operations: OPS_ENF_STAND2,
        },
    ),
    (
        "enf_stand3",
        MonsterFrame {
            frame: 2,
            next: "enf_stand4",
            operations: OPS_ENF_STAND3,
        },
    ),
    (
        "enf_stand4",
        MonsterFrame {
            frame: 3,
            next: "enf_stand5",
            operations: OPS_ENF_STAND4,
        },
    ),
    (
        "enf_stand5",
        MonsterFrame {
            frame: 4,
            next: "enf_stand6",
            operations: OPS_ENF_STAND5,
        },
    ),
    (
        "enf_stand6",
        MonsterFrame {
            frame: 5,
            next: "enf_stand7",
            operations: OPS_ENF_STAND6,
        },
    ),
    (
        "enf_stand7",
        MonsterFrame {
            frame: 6,
            next: "enf_stand1",
            operations: OPS_ENF_STAND7,
        },
    ),
    (
        "enf_walk1",
        MonsterFrame {
            frame: 7,
            next: "enf_walk2",
            operations: OPS_ENF_WALK1,
        },
    ),
    (
        "enf_walk10",
        MonsterFrame {
            frame: 16,
            next: "enf_walk11",
            operations: OPS_ENF_WALK10,
        },
    ),
    (
        "enf_walk11",
        MonsterFrame {
            frame: 17,
            next: "enf_walk12",
            operations: OPS_ENF_WALK11,
        },
    ),
    (
        "enf_walk12",
        MonsterFrame {
            frame: 18,
            next: "enf_walk13",
            operations: OPS_ENF_WALK12,
        },
    ),
    (
        "enf_walk13",
        MonsterFrame {
            frame: 19,
            next: "enf_walk14",
            operations: OPS_ENF_WALK13,
        },
    ),
    (
        "enf_walk14",
        MonsterFrame {
            frame: 20,
            next: "enf_walk15",
            operations: OPS_ENF_WALK14,
        },
    ),
    (
        "enf_walk15",
        MonsterFrame {
            frame: 21,
            next: "enf_walk16",
            operations: OPS_ENF_WALK15,
        },
    ),
    (
        "enf_walk16",
        MonsterFrame {
            frame: 22,
            next: "enf_walk1",
            operations: OPS_ENF_WALK16,
        },
    ),
    (
        "enf_walk2",
        MonsterFrame {
            frame: 8,
            next: "enf_walk3",
            operations: OPS_ENF_WALK2,
        },
    ),
    (
        "enf_walk3",
        MonsterFrame {
            frame: 9,
            next: "enf_walk4",
            operations: OPS_ENF_WALK3,
        },
    ),
    (
        "enf_walk4",
        MonsterFrame {
            frame: 10,
            next: "enf_walk5",
            operations: OPS_ENF_WALK4,
        },
    ),
    (
        "enf_walk5",
        MonsterFrame {
            frame: 11,
            next: "enf_walk6",
            operations: OPS_ENF_WALK5,
        },
    ),
    (
        "enf_walk6",
        MonsterFrame {
            frame: 12,
            next: "enf_walk7",
            operations: OPS_ENF_WALK6,
        },
    ),
    (
        "enf_walk7",
        MonsterFrame {
            frame: 13,
            next: "enf_walk8",
            operations: OPS_ENF_WALK7,
        },
    ),
    (
        "enf_walk8",
        MonsterFrame {
            frame: 14,
            next: "enf_walk9",
            operations: OPS_ENF_WALK8,
        },
    ),
    (
        "enf_walk9",
        MonsterFrame {
            frame: 15,
            next: "enf_walk10",
            operations: OPS_ENF_WALK9,
        },
    ),
    (
        "f_attack1",
        MonsterFrame {
            frame: 0,
            next: "f_attack2",
            operations: OPS_F_ATTACK1,
        },
    ),
    (
        "f_attack10",
        MonsterFrame {
            frame: 9,
            next: "f_attack11",
            operations: OPS_F_ATTACK10,
        },
    ),
    (
        "f_attack11",
        MonsterFrame {
            frame: 10,
            next: "f_attack12",
            operations: OPS_F_ATTACK11,
        },
    ),
    (
        "f_attack12",
        MonsterFrame {
            frame: 11,
            next: "f_attack13",
            operations: OPS_F_ATTACK12,
        },
    ),
    (
        "f_attack13",
        MonsterFrame {
            frame: 12,
            next: "f_attack14",
            operations: OPS_F_ATTACK13,
        },
    ),
    (
        "f_attack14",
        MonsterFrame {
            frame: 13,
            next: "f_attack15",
            operations: OPS_F_ATTACK14,
        },
    ),
    (
        "f_attack15",
        MonsterFrame {
            frame: 14,
            next: "f_attack16",
            operations: OPS_F_ATTACK15,
        },
    ),
    (
        "f_attack16",
        MonsterFrame {
            frame: 15,
            next: "f_attack17",
            operations: OPS_F_ATTACK16,
        },
    ),
    (
        "f_attack17",
        MonsterFrame {
            frame: 16,
            next: "f_attack18",
            operations: OPS_F_ATTACK17,
        },
    ),
    (
        "f_attack18",
        MonsterFrame {
            frame: 17,
            next: "f_run1",
            operations: OPS_F_ATTACK18,
        },
    ),
    (
        "f_attack2",
        MonsterFrame {
            frame: 1,
            next: "f_attack3",
            operations: OPS_F_ATTACK2,
        },
    ),
    (
        "f_attack3",
        MonsterFrame {
            frame: 2,
            next: "f_attack4",
            operations: OPS_F_ATTACK3,
        },
    ),
    (
        "f_attack4",
        MonsterFrame {
            frame: 3,
            next: "f_attack5",
            operations: OPS_F_ATTACK4,
        },
    ),
    (
        "f_attack5",
        MonsterFrame {
            frame: 4,
            next: "f_attack6",
            operations: OPS_F_ATTACK5,
        },
    ),
    (
        "f_attack6",
        MonsterFrame {
            frame: 5,
            next: "f_attack7",
            operations: OPS_F_ATTACK6,
        },
    ),
    (
        "f_attack7",
        MonsterFrame {
            frame: 6,
            next: "f_attack8",
            operations: OPS_F_ATTACK7,
        },
    ),
    (
        "f_attack8",
        MonsterFrame {
            frame: 7,
            next: "f_attack9",
            operations: OPS_F_ATTACK8,
        },
    ),
    (
        "f_attack9",
        MonsterFrame {
            frame: 8,
            next: "f_attack10",
            operations: OPS_F_ATTACK9,
        },
    ),
    (
        "f_death1",
        MonsterFrame {
            frame: 18,
            next: "f_death2",
            operations: OPS_F_DEATH1,
        },
    ),
    (
        "f_death10",
        MonsterFrame {
            frame: 27,
            next: "f_death11",
            operations: OPS_F_DEATH10,
        },
    ),
    (
        "f_death11",
        MonsterFrame {
            frame: 28,
            next: "f_death12",
            operations: OPS_F_DEATH11,
        },
    ),
    (
        "f_death12",
        MonsterFrame {
            frame: 29,
            next: "f_death13",
            operations: OPS_F_DEATH12,
        },
    ),
    (
        "f_death13",
        MonsterFrame {
            frame: 30,
            next: "f_death14",
            operations: OPS_F_DEATH13,
        },
    ),
    (
        "f_death14",
        MonsterFrame {
            frame: 31,
            next: "f_death15",
            operations: OPS_F_DEATH14,
        },
    ),
    (
        "f_death15",
        MonsterFrame {
            frame: 32,
            next: "f_death16",
            operations: OPS_F_DEATH15,
        },
    ),
    (
        "f_death16",
        MonsterFrame {
            frame: 33,
            next: "f_death17",
            operations: OPS_F_DEATH16,
        },
    ),
    (
        "f_death17",
        MonsterFrame {
            frame: 34,
            next: "f_death18",
            operations: OPS_F_DEATH17,
        },
    ),
    (
        "f_death18",
        MonsterFrame {
            frame: 35,
            next: "f_death19",
            operations: OPS_F_DEATH18,
        },
    ),
    (
        "f_death19",
        MonsterFrame {
            frame: 36,
            next: "f_death20",
            operations: OPS_F_DEATH19,
        },
    ),
    (
        "f_death2",
        MonsterFrame {
            frame: 19,
            next: "f_death3",
            operations: OPS_F_DEATH2,
        },
    ),
    (
        "f_death20",
        MonsterFrame {
            frame: 37,
            next: "f_death21",
            operations: OPS_F_DEATH20,
        },
    ),
    (
        "f_death21",
        MonsterFrame {
            frame: 38,
            next: "f_death21",
            operations: OPS_F_DEATH21,
        },
    ),
    (
        "f_death3",
        MonsterFrame {
            frame: 20,
            next: "f_death4",
            operations: OPS_F_DEATH3,
        },
    ),
    (
        "f_death4",
        MonsterFrame {
            frame: 21,
            next: "f_death5",
            operations: OPS_F_DEATH4,
        },
    ),
    (
        "f_death5",
        MonsterFrame {
            frame: 22,
            next: "f_death6",
            operations: OPS_F_DEATH5,
        },
    ),
    (
        "f_death6",
        MonsterFrame {
            frame: 23,
            next: "f_death7",
            operations: OPS_F_DEATH6,
        },
    ),
    (
        "f_death7",
        MonsterFrame {
            frame: 24,
            next: "f_death8",
            operations: OPS_F_DEATH7,
        },
    ),
    (
        "f_death8",
        MonsterFrame {
            frame: 25,
            next: "f_death9",
            operations: OPS_F_DEATH8,
        },
    ),
    (
        "f_death9",
        MonsterFrame {
            frame: 26,
            next: "f_death10",
            operations: OPS_F_DEATH9,
        },
    ),
    (
        "f_pain1",
        MonsterFrame {
            frame: 57,
            next: "f_pain2",
            operations: OPS_F_PAIN1,
        },
    ),
    (
        "f_pain2",
        MonsterFrame {
            frame: 58,
            next: "f_pain3",
            operations: OPS_F_PAIN2,
        },
    ),
    (
        "f_pain3",
        MonsterFrame {
            frame: 59,
            next: "f_pain4",
            operations: OPS_F_PAIN3,
        },
    ),
    (
        "f_pain4",
        MonsterFrame {
            frame: 60,
            next: "f_pain5",
            operations: OPS_F_PAIN4,
        },
    ),
    (
        "f_pain5",
        MonsterFrame {
            frame: 61,
            next: "f_pain6",
            operations: OPS_F_PAIN5,
        },
    ),
    (
        "f_pain6",
        MonsterFrame {
            frame: 62,
            next: "f_pain7",
            operations: OPS_F_PAIN6,
        },
    ),
    (
        "f_pain7",
        MonsterFrame {
            frame: 63,
            next: "f_pain8",
            operations: OPS_F_PAIN7,
        },
    ),
    (
        "f_pain8",
        MonsterFrame {
            frame: 64,
            next: "f_pain9",
            operations: OPS_F_PAIN8,
        },
    ),
    (
        "f_pain9",
        MonsterFrame {
            frame: 65,
            next: "f_run1",
            operations: OPS_F_PAIN9,
        },
    ),
    (
        "f_run1",
        MonsterFrame {
            frame: 39,
            next: "f_run2",
            operations: OPS_F_RUN1,
        },
    ),
    (
        "f_run2",
        MonsterFrame {
            frame: 41,
            next: "f_run3",
            operations: OPS_F_RUN2,
        },
    ),
    (
        "f_run3",
        MonsterFrame {
            frame: 43,
            next: "f_run4",
            operations: OPS_F_RUN3,
        },
    ),
    (
        "f_run4",
        MonsterFrame {
            frame: 45,
            next: "f_run5",
            operations: OPS_F_RUN4,
        },
    ),
    (
        "f_run5",
        MonsterFrame {
            frame: 47,
            next: "f_run6",
            operations: OPS_F_RUN5,
        },
    ),
    (
        "f_run6",
        MonsterFrame {
            frame: 49,
            next: "f_run7",
            operations: OPS_F_RUN6,
        },
    ),
    (
        "f_run7",
        MonsterFrame {
            frame: 51,
            next: "f_run8",
            operations: OPS_F_RUN7,
        },
    ),
    (
        "f_run8",
        MonsterFrame {
            frame: 53,
            next: "f_run9",
            operations: OPS_F_RUN8,
        },
    ),
    (
        "f_run9",
        MonsterFrame {
            frame: 55,
            next: "f_run1",
            operations: OPS_F_RUN9,
        },
    ),
    (
        "f_stand1",
        MonsterFrame {
            frame: 39,
            next: "f_stand2",
            operations: OPS_F_STAND1,
        },
    ),
    (
        "f_stand10",
        MonsterFrame {
            frame: 48,
            next: "f_stand11",
            operations: OPS_F_STAND10,
        },
    ),
    (
        "f_stand11",
        MonsterFrame {
            frame: 49,
            next: "f_stand12",
            operations: OPS_F_STAND11,
        },
    ),
    (
        "f_stand12",
        MonsterFrame {
            frame: 50,
            next: "f_stand13",
            operations: OPS_F_STAND12,
        },
    ),
    (
        "f_stand13",
        MonsterFrame {
            frame: 51,
            next: "f_stand14",
            operations: OPS_F_STAND13,
        },
    ),
    (
        "f_stand14",
        MonsterFrame {
            frame: 52,
            next: "f_stand15",
            operations: OPS_F_STAND14,
        },
    ),
    (
        "f_stand15",
        MonsterFrame {
            frame: 53,
            next: "f_stand16",
            operations: OPS_F_STAND15,
        },
    ),
    (
        "f_stand16",
        MonsterFrame {
            frame: 54,
            next: "f_stand17",
            operations: OPS_F_STAND16,
        },
    ),
    (
        "f_stand17",
        MonsterFrame {
            frame: 55,
            next: "f_stand18",
            operations: OPS_F_STAND17,
        },
    ),
    (
        "f_stand18",
        MonsterFrame {
            frame: 56,
            next: "f_stand1",
            operations: OPS_F_STAND18,
        },
    ),
    (
        "f_stand2",
        MonsterFrame {
            frame: 40,
            next: "f_stand3",
            operations: OPS_F_STAND2,
        },
    ),
    (
        "f_stand3",
        MonsterFrame {
            frame: 41,
            next: "f_stand4",
            operations: OPS_F_STAND3,
        },
    ),
    (
        "f_stand4",
        MonsterFrame {
            frame: 42,
            next: "f_stand5",
            operations: OPS_F_STAND4,
        },
    ),
    (
        "f_stand5",
        MonsterFrame {
            frame: 43,
            next: "f_stand6",
            operations: OPS_F_STAND5,
        },
    ),
    (
        "f_stand6",
        MonsterFrame {
            frame: 44,
            next: "f_stand7",
            operations: OPS_F_STAND6,
        },
    ),
    (
        "f_stand7",
        MonsterFrame {
            frame: 45,
            next: "f_stand8",
            operations: OPS_F_STAND7,
        },
    ),
    (
        "f_stand8",
        MonsterFrame {
            frame: 46,
            next: "f_stand9",
            operations: OPS_F_STAND8,
        },
    ),
    (
        "f_stand9",
        MonsterFrame {
            frame: 47,
            next: "f_stand10",
            operations: OPS_F_STAND9,
        },
    ),
    (
        "f_walk1",
        MonsterFrame {
            frame: 39,
            next: "f_walk2",
            operations: OPS_F_WALK1,
        },
    ),
    (
        "f_walk10",
        MonsterFrame {
            frame: 48,
            next: "f_walk11",
            operations: OPS_F_WALK10,
        },
    ),
    (
        "f_walk11",
        MonsterFrame {
            frame: 49,
            next: "f_walk12",
            operations: OPS_F_WALK11,
        },
    ),
    (
        "f_walk12",
        MonsterFrame {
            frame: 50,
            next: "f_walk13",
            operations: OPS_F_WALK12,
        },
    ),
    (
        "f_walk13",
        MonsterFrame {
            frame: 51,
            next: "f_walk14",
            operations: OPS_F_WALK13,
        },
    ),
    (
        "f_walk14",
        MonsterFrame {
            frame: 52,
            next: "f_walk15",
            operations: OPS_F_WALK14,
        },
    ),
    (
        "f_walk15",
        MonsterFrame {
            frame: 53,
            next: "f_walk16",
            operations: OPS_F_WALK15,
        },
    ),
    (
        "f_walk16",
        MonsterFrame {
            frame: 54,
            next: "f_walk17",
            operations: OPS_F_WALK16,
        },
    ),
    (
        "f_walk17",
        MonsterFrame {
            frame: 55,
            next: "f_walk18",
            operations: OPS_F_WALK17,
        },
    ),
    (
        "f_walk18",
        MonsterFrame {
            frame: 56,
            next: "f_walk1",
            operations: OPS_F_WALK18,
        },
    ),
    (
        "f_walk2",
        MonsterFrame {
            frame: 40,
            next: "f_walk3",
            operations: OPS_F_WALK2,
        },
    ),
    (
        "f_walk3",
        MonsterFrame {
            frame: 41,
            next: "f_walk4",
            operations: OPS_F_WALK3,
        },
    ),
    (
        "f_walk4",
        MonsterFrame {
            frame: 42,
            next: "f_walk5",
            operations: OPS_F_WALK4,
        },
    ),
    (
        "f_walk5",
        MonsterFrame {
            frame: 43,
            next: "f_walk6",
            operations: OPS_F_WALK5,
        },
    ),
    (
        "f_walk6",
        MonsterFrame {
            frame: 44,
            next: "f_walk7",
            operations: OPS_F_WALK6,
        },
    ),
    (
        "f_walk7",
        MonsterFrame {
            frame: 45,
            next: "f_walk8",
            operations: OPS_F_WALK7,
        },
    ),
    (
        "f_walk8",
        MonsterFrame {
            frame: 46,
            next: "f_walk9",
            operations: OPS_F_WALK8,
        },
    ),
    (
        "f_walk9",
        MonsterFrame {
            frame: 47,
            next: "f_walk10",
            operations: OPS_F_WALK9,
        },
    ),
    (
        "hknight_char_a1",
        MonsterFrame {
            frame: 63,
            next: "hknight_char_a2",
            operations: OPS_HKNIGHT_CHAR_A1,
        },
    ),
    (
        "hknight_char_a10",
        MonsterFrame {
            frame: 72,
            next: "hknight_char_a11",
            operations: OPS_HKNIGHT_CHAR_A10,
        },
    ),
    (
        "hknight_char_a11",
        MonsterFrame {
            frame: 73,
            next: "hknight_char_a12",
            operations: OPS_HKNIGHT_CHAR_A11,
        },
    ),
    (
        "hknight_char_a12",
        MonsterFrame {
            frame: 74,
            next: "hknight_char_a13",
            operations: OPS_HKNIGHT_CHAR_A12,
        },
    ),
    (
        "hknight_char_a13",
        MonsterFrame {
            frame: 75,
            next: "hknight_char_a14",
            operations: OPS_HKNIGHT_CHAR_A13,
        },
    ),
    (
        "hknight_char_a14",
        MonsterFrame {
            frame: 76,
            next: "hknight_char_a15",
            operations: OPS_HKNIGHT_CHAR_A14,
        },
    ),
    (
        "hknight_char_a15",
        MonsterFrame {
            frame: 77,
            next: "hknight_char_a16",
            operations: OPS_HKNIGHT_CHAR_A15,
        },
    ),
    (
        "hknight_char_a16",
        MonsterFrame {
            frame: 78,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_CHAR_A16,
        },
    ),
    (
        "hknight_char_a2",
        MonsterFrame {
            frame: 64,
            next: "hknight_char_a3",
            operations: OPS_HKNIGHT_CHAR_A2,
        },
    ),
    (
        "hknight_char_a3",
        MonsterFrame {
            frame: 65,
            next: "hknight_char_a4",
            operations: OPS_HKNIGHT_CHAR_A3,
        },
    ),
    (
        "hknight_char_a4",
        MonsterFrame {
            frame: 66,
            next: "hknight_char_a5",
            operations: OPS_HKNIGHT_CHAR_A4,
        },
    ),
    (
        "hknight_char_a5",
        MonsterFrame {
            frame: 67,
            next: "hknight_char_a6",
            operations: OPS_HKNIGHT_CHAR_A5,
        },
    ),
    (
        "hknight_char_a6",
        MonsterFrame {
            frame: 68,
            next: "hknight_char_a7",
            operations: OPS_HKNIGHT_CHAR_A6,
        },
    ),
    (
        "hknight_char_a7",
        MonsterFrame {
            frame: 69,
            next: "hknight_char_a8",
            operations: OPS_HKNIGHT_CHAR_A7,
        },
    ),
    (
        "hknight_char_a8",
        MonsterFrame {
            frame: 70,
            next: "hknight_char_a9",
            operations: OPS_HKNIGHT_CHAR_A8,
        },
    ),
    (
        "hknight_char_a9",
        MonsterFrame {
            frame: 71,
            next: "hknight_char_a10",
            operations: OPS_HKNIGHT_CHAR_A9,
        },
    ),
    (
        "hknight_char_b1",
        MonsterFrame {
            frame: 106,
            next: "hknight_char_b2",
            operations: OPS_HKNIGHT_CHAR_B1,
        },
    ),
    (
        "hknight_char_b2",
        MonsterFrame {
            frame: 107,
            next: "hknight_char_b3",
            operations: OPS_HKNIGHT_CHAR_B2,
        },
    ),
    (
        "hknight_char_b3",
        MonsterFrame {
            frame: 108,
            next: "hknight_char_b4",
            operations: OPS_HKNIGHT_CHAR_B3,
        },
    ),
    (
        "hknight_char_b4",
        MonsterFrame {
            frame: 109,
            next: "hknight_char_b5",
            operations: OPS_HKNIGHT_CHAR_B4,
        },
    ),
    (
        "hknight_char_b5",
        MonsterFrame {
            frame: 110,
            next: "hknight_char_b6",
            operations: OPS_HKNIGHT_CHAR_B5,
        },
    ),
    (
        "hknight_char_b6",
        MonsterFrame {
            frame: 111,
            next: "hknight_char_b1",
            operations: OPS_HKNIGHT_CHAR_B6,
        },
    ),
    (
        "hknight_die1",
        MonsterFrame {
            frame: 42,
            next: "hknight_die2",
            operations: OPS_HKNIGHT_DIE1,
        },
    ),
    (
        "hknight_die10",
        MonsterFrame {
            frame: 51,
            next: "hknight_die11",
            operations: OPS_HKNIGHT_DIE10,
        },
    ),
    (
        "hknight_die11",
        MonsterFrame {
            frame: 52,
            next: "hknight_die12",
            operations: OPS_HKNIGHT_DIE11,
        },
    ),
    (
        "hknight_die12",
        MonsterFrame {
            frame: 53,
            next: "hknight_die12",
            operations: OPS_HKNIGHT_DIE12,
        },
    ),
    (
        "hknight_die2",
        MonsterFrame {
            frame: 43,
            next: "hknight_die3",
            operations: OPS_HKNIGHT_DIE2,
        },
    ),
    (
        "hknight_die3",
        MonsterFrame {
            frame: 44,
            next: "hknight_die4",
            operations: OPS_HKNIGHT_DIE3,
        },
    ),
    (
        "hknight_die4",
        MonsterFrame {
            frame: 45,
            next: "hknight_die5",
            operations: OPS_HKNIGHT_DIE4,
        },
    ),
    (
        "hknight_die5",
        MonsterFrame {
            frame: 46,
            next: "hknight_die6",
            operations: OPS_HKNIGHT_DIE5,
        },
    ),
    (
        "hknight_die6",
        MonsterFrame {
            frame: 47,
            next: "hknight_die7",
            operations: OPS_HKNIGHT_DIE6,
        },
    ),
    (
        "hknight_die7",
        MonsterFrame {
            frame: 48,
            next: "hknight_die8",
            operations: OPS_HKNIGHT_DIE7,
        },
    ),
    (
        "hknight_die8",
        MonsterFrame {
            frame: 49,
            next: "hknight_die9",
            operations: OPS_HKNIGHT_DIE8,
        },
    ),
    (
        "hknight_die9",
        MonsterFrame {
            frame: 50,
            next: "hknight_die10",
            operations: OPS_HKNIGHT_DIE9,
        },
    ),
    (
        "hknight_dieb1",
        MonsterFrame {
            frame: 54,
            next: "hknight_dieb2",
            operations: OPS_HKNIGHT_DIEB1,
        },
    ),
    (
        "hknight_dieb2",
        MonsterFrame {
            frame: 55,
            next: "hknight_dieb3",
            operations: OPS_HKNIGHT_DIEB2,
        },
    ),
    (
        "hknight_dieb3",
        MonsterFrame {
            frame: 56,
            next: "hknight_dieb4",
            operations: OPS_HKNIGHT_DIEB3,
        },
    ),
    (
        "hknight_dieb4",
        MonsterFrame {
            frame: 57,
            next: "hknight_dieb5",
            operations: OPS_HKNIGHT_DIEB4,
        },
    ),
    (
        "hknight_dieb5",
        MonsterFrame {
            frame: 58,
            next: "hknight_dieb6",
            operations: OPS_HKNIGHT_DIEB5,
        },
    ),
    (
        "hknight_dieb6",
        MonsterFrame {
            frame: 59,
            next: "hknight_dieb7",
            operations: OPS_HKNIGHT_DIEB6,
        },
    ),
    (
        "hknight_dieb7",
        MonsterFrame {
            frame: 60,
            next: "hknight_dieb8",
            operations: OPS_HKNIGHT_DIEB7,
        },
    ),
    (
        "hknight_dieb8",
        MonsterFrame {
            frame: 61,
            next: "hknight_dieb9",
            operations: OPS_HKNIGHT_DIEB8,
        },
    ),
    (
        "hknight_dieb9",
        MonsterFrame {
            frame: 62,
            next: "hknight_dieb9",
            operations: OPS_HKNIGHT_DIEB9,
        },
    ),
    (
        "hknight_magica1",
        MonsterFrame {
            frame: 79,
            next: "hknight_magica2",
            operations: OPS_HKNIGHT_MAGICA1,
        },
    ),
    (
        "hknight_magica10",
        MonsterFrame {
            frame: 88,
            next: "hknight_magica11",
            operations: OPS_HKNIGHT_MAGICA10,
        },
    ),
    (
        "hknight_magica11",
        MonsterFrame {
            frame: 89,
            next: "hknight_magica12",
            operations: OPS_HKNIGHT_MAGICA11,
        },
    ),
    (
        "hknight_magica12",
        MonsterFrame {
            frame: 90,
            next: "hknight_magica13",
            operations: OPS_HKNIGHT_MAGICA12,
        },
    ),
    (
        "hknight_magica13",
        MonsterFrame {
            frame: 91,
            next: "hknight_magica14",
            operations: OPS_HKNIGHT_MAGICA13,
        },
    ),
    (
        "hknight_magica14",
        MonsterFrame {
            frame: 92,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_MAGICA14,
        },
    ),
    (
        "hknight_magica2",
        MonsterFrame {
            frame: 80,
            next: "hknight_magica3",
            operations: OPS_HKNIGHT_MAGICA2,
        },
    ),
    (
        "hknight_magica3",
        MonsterFrame {
            frame: 81,
            next: "hknight_magica4",
            operations: OPS_HKNIGHT_MAGICA3,
        },
    ),
    (
        "hknight_magica4",
        MonsterFrame {
            frame: 82,
            next: "hknight_magica5",
            operations: OPS_HKNIGHT_MAGICA4,
        },
    ),
    (
        "hknight_magica5",
        MonsterFrame {
            frame: 83,
            next: "hknight_magica6",
            operations: OPS_HKNIGHT_MAGICA5,
        },
    ),
    (
        "hknight_magica6",
        MonsterFrame {
            frame: 84,
            next: "hknight_magica7",
            operations: OPS_HKNIGHT_MAGICA6,
        },
    ),
    (
        "hknight_magica7",
        MonsterFrame {
            frame: 85,
            next: "hknight_magica8",
            operations: OPS_HKNIGHT_MAGICA7,
        },
    ),
    (
        "hknight_magica8",
        MonsterFrame {
            frame: 86,
            next: "hknight_magica9",
            operations: OPS_HKNIGHT_MAGICA8,
        },
    ),
    (
        "hknight_magica9",
        MonsterFrame {
            frame: 87,
            next: "hknight_magica10",
            operations: OPS_HKNIGHT_MAGICA9,
        },
    ),
    (
        "hknight_magicb1",
        MonsterFrame {
            frame: 93,
            next: "hknight_magicb2",
            operations: OPS_HKNIGHT_MAGICB1,
        },
    ),
    (
        "hknight_magicb10",
        MonsterFrame {
            frame: 102,
            next: "hknight_magicb11",
            operations: OPS_HKNIGHT_MAGICB10,
        },
    ),
    (
        "hknight_magicb11",
        MonsterFrame {
            frame: 103,
            next: "hknight_magicb12",
            operations: OPS_HKNIGHT_MAGICB11,
        },
    ),
    (
        "hknight_magicb12",
        MonsterFrame {
            frame: 104,
            next: "hknight_magicb13",
            operations: OPS_HKNIGHT_MAGICB12,
        },
    ),
    (
        "hknight_magicb13",
        MonsterFrame {
            frame: 105,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_MAGICB13,
        },
    ),
    (
        "hknight_magicb2",
        MonsterFrame {
            frame: 94,
            next: "hknight_magicb3",
            operations: OPS_HKNIGHT_MAGICB2,
        },
    ),
    (
        "hknight_magicb3",
        MonsterFrame {
            frame: 95,
            next: "hknight_magicb4",
            operations: OPS_HKNIGHT_MAGICB3,
        },
    ),
    (
        "hknight_magicb4",
        MonsterFrame {
            frame: 96,
            next: "hknight_magicb5",
            operations: OPS_HKNIGHT_MAGICB4,
        },
    ),
    (
        "hknight_magicb5",
        MonsterFrame {
            frame: 97,
            next: "hknight_magicb6",
            operations: OPS_HKNIGHT_MAGICB5,
        },
    ),
    (
        "hknight_magicb6",
        MonsterFrame {
            frame: 98,
            next: "hknight_magicb7",
            operations: OPS_HKNIGHT_MAGICB6,
        },
    ),
    (
        "hknight_magicb7",
        MonsterFrame {
            frame: 99,
            next: "hknight_magicb8",
            operations: OPS_HKNIGHT_MAGICB7,
        },
    ),
    (
        "hknight_magicb8",
        MonsterFrame {
            frame: 100,
            next: "hknight_magicb9",
            operations: OPS_HKNIGHT_MAGICB8,
        },
    ),
    (
        "hknight_magicb9",
        MonsterFrame {
            frame: 101,
            next: "hknight_magicb10",
            operations: OPS_HKNIGHT_MAGICB9,
        },
    ),
    (
        "hknight_magicc1",
        MonsterFrame {
            frame: 155,
            next: "hknight_magicc2",
            operations: OPS_HKNIGHT_MAGICC1,
        },
    ),
    (
        "hknight_magicc10",
        MonsterFrame {
            frame: 164,
            next: "hknight_magicc11",
            operations: OPS_HKNIGHT_MAGICC10,
        },
    ),
    (
        "hknight_magicc11",
        MonsterFrame {
            frame: 165,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_MAGICC11,
        },
    ),
    (
        "hknight_magicc2",
        MonsterFrame {
            frame: 156,
            next: "hknight_magicc3",
            operations: OPS_HKNIGHT_MAGICC2,
        },
    ),
    (
        "hknight_magicc3",
        MonsterFrame {
            frame: 157,
            next: "hknight_magicc4",
            operations: OPS_HKNIGHT_MAGICC3,
        },
    ),
    (
        "hknight_magicc4",
        MonsterFrame {
            frame: 158,
            next: "hknight_magicc5",
            operations: OPS_HKNIGHT_MAGICC4,
        },
    ),
    (
        "hknight_magicc5",
        MonsterFrame {
            frame: 159,
            next: "hknight_magicc6",
            operations: OPS_HKNIGHT_MAGICC5,
        },
    ),
    (
        "hknight_magicc6",
        MonsterFrame {
            frame: 160,
            next: "hknight_magicc7",
            operations: OPS_HKNIGHT_MAGICC6,
        },
    ),
    (
        "hknight_magicc7",
        MonsterFrame {
            frame: 161,
            next: "hknight_magicc8",
            operations: OPS_HKNIGHT_MAGICC7,
        },
    ),
    (
        "hknight_magicc8",
        MonsterFrame {
            frame: 162,
            next: "hknight_magicc9",
            operations: OPS_HKNIGHT_MAGICC8,
        },
    ),
    (
        "hknight_magicc9",
        MonsterFrame {
            frame: 163,
            next: "hknight_magicc10",
            operations: OPS_HKNIGHT_MAGICC9,
        },
    ),
    (
        "hknight_pain1",
        MonsterFrame {
            frame: 37,
            next: "hknight_pain2",
            operations: OPS_HKNIGHT_PAIN1,
        },
    ),
    (
        "hknight_pain2",
        MonsterFrame {
            frame: 38,
            next: "hknight_pain3",
            operations: OPS_HKNIGHT_PAIN2,
        },
    ),
    (
        "hknight_pain3",
        MonsterFrame {
            frame: 39,
            next: "hknight_pain4",
            operations: OPS_HKNIGHT_PAIN3,
        },
    ),
    (
        "hknight_pain4",
        MonsterFrame {
            frame: 40,
            next: "hknight_pain5",
            operations: OPS_HKNIGHT_PAIN4,
        },
    ),
    (
        "hknight_pain5",
        MonsterFrame {
            frame: 41,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_PAIN5,
        },
    ),
    (
        "hknight_run1",
        MonsterFrame {
            frame: 29,
            next: "hknight_run2",
            operations: OPS_HKNIGHT_RUN1,
        },
    ),
    (
        "hknight_run2",
        MonsterFrame {
            frame: 30,
            next: "hknight_run3",
            operations: OPS_HKNIGHT_RUN2,
        },
    ),
    (
        "hknight_run3",
        MonsterFrame {
            frame: 31,
            next: "hknight_run4",
            operations: OPS_HKNIGHT_RUN3,
        },
    ),
    (
        "hknight_run4",
        MonsterFrame {
            frame: 32,
            next: "hknight_run5",
            operations: OPS_HKNIGHT_RUN4,
        },
    ),
    (
        "hknight_run5",
        MonsterFrame {
            frame: 33,
            next: "hknight_run6",
            operations: OPS_HKNIGHT_RUN5,
        },
    ),
    (
        "hknight_run6",
        MonsterFrame {
            frame: 34,
            next: "hknight_run7",
            operations: OPS_HKNIGHT_RUN6,
        },
    ),
    (
        "hknight_run7",
        MonsterFrame {
            frame: 35,
            next: "hknight_run8",
            operations: OPS_HKNIGHT_RUN7,
        },
    ),
    (
        "hknight_run8",
        MonsterFrame {
            frame: 36,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_RUN8,
        },
    ),
    (
        "hknight_slice1",
        MonsterFrame {
            frame: 112,
            next: "hknight_slice2",
            operations: OPS_HKNIGHT_SLICE1,
        },
    ),
    (
        "hknight_slice10",
        MonsterFrame {
            frame: 121,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_SLICE10,
        },
    ),
    (
        "hknight_slice2",
        MonsterFrame {
            frame: 113,
            next: "hknight_slice3",
            operations: OPS_HKNIGHT_SLICE2,
        },
    ),
    (
        "hknight_slice3",
        MonsterFrame {
            frame: 114,
            next: "hknight_slice4",
            operations: OPS_HKNIGHT_SLICE3,
        },
    ),
    (
        "hknight_slice4",
        MonsterFrame {
            frame: 115,
            next: "hknight_slice5",
            operations: OPS_HKNIGHT_SLICE4,
        },
    ),
    (
        "hknight_slice5",
        MonsterFrame {
            frame: 116,
            next: "hknight_slice6",
            operations: OPS_HKNIGHT_SLICE5,
        },
    ),
    (
        "hknight_slice6",
        MonsterFrame {
            frame: 117,
            next: "hknight_slice7",
            operations: OPS_HKNIGHT_SLICE6,
        },
    ),
    (
        "hknight_slice7",
        MonsterFrame {
            frame: 118,
            next: "hknight_slice8",
            operations: OPS_HKNIGHT_SLICE7,
        },
    ),
    (
        "hknight_slice8",
        MonsterFrame {
            frame: 119,
            next: "hknight_slice9",
            operations: OPS_HKNIGHT_SLICE8,
        },
    ),
    (
        "hknight_slice9",
        MonsterFrame {
            frame: 120,
            next: "hknight_slice10",
            operations: OPS_HKNIGHT_SLICE9,
        },
    ),
    (
        "hknight_smash1",
        MonsterFrame {
            frame: 122,
            next: "hknight_smash2",
            operations: OPS_HKNIGHT_SMASH1,
        },
    ),
    (
        "hknight_smash10",
        MonsterFrame {
            frame: 131,
            next: "hknight_smash11",
            operations: OPS_HKNIGHT_SMASH10,
        },
    ),
    (
        "hknight_smash11",
        MonsterFrame {
            frame: 132,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_SMASH11,
        },
    ),
    (
        "hknight_smash2",
        MonsterFrame {
            frame: 123,
            next: "hknight_smash3",
            operations: OPS_HKNIGHT_SMASH2,
        },
    ),
    (
        "hknight_smash3",
        MonsterFrame {
            frame: 124,
            next: "hknight_smash4",
            operations: OPS_HKNIGHT_SMASH3,
        },
    ),
    (
        "hknight_smash4",
        MonsterFrame {
            frame: 125,
            next: "hknight_smash5",
            operations: OPS_HKNIGHT_SMASH4,
        },
    ),
    (
        "hknight_smash5",
        MonsterFrame {
            frame: 126,
            next: "hknight_smash6",
            operations: OPS_HKNIGHT_SMASH5,
        },
    ),
    (
        "hknight_smash6",
        MonsterFrame {
            frame: 127,
            next: "hknight_smash7",
            operations: OPS_HKNIGHT_SMASH6,
        },
    ),
    (
        "hknight_smash7",
        MonsterFrame {
            frame: 128,
            next: "hknight_smash8",
            operations: OPS_HKNIGHT_SMASH7,
        },
    ),
    (
        "hknight_smash8",
        MonsterFrame {
            frame: 129,
            next: "hknight_smash9",
            operations: OPS_HKNIGHT_SMASH8,
        },
    ),
    (
        "hknight_smash9",
        MonsterFrame {
            frame: 130,
            next: "hknight_smash10",
            operations: OPS_HKNIGHT_SMASH9,
        },
    ),
    (
        "hknight_stand1",
        MonsterFrame {
            frame: 0,
            next: "hknight_stand2",
            operations: OPS_HKNIGHT_STAND1,
        },
    ),
    (
        "hknight_stand2",
        MonsterFrame {
            frame: 1,
            next: "hknight_stand3",
            operations: OPS_HKNIGHT_STAND2,
        },
    ),
    (
        "hknight_stand3",
        MonsterFrame {
            frame: 2,
            next: "hknight_stand4",
            operations: OPS_HKNIGHT_STAND3,
        },
    ),
    (
        "hknight_stand4",
        MonsterFrame {
            frame: 3,
            next: "hknight_stand5",
            operations: OPS_HKNIGHT_STAND4,
        },
    ),
    (
        "hknight_stand5",
        MonsterFrame {
            frame: 4,
            next: "hknight_stand6",
            operations: OPS_HKNIGHT_STAND5,
        },
    ),
    (
        "hknight_stand6",
        MonsterFrame {
            frame: 5,
            next: "hknight_stand7",
            operations: OPS_HKNIGHT_STAND6,
        },
    ),
    (
        "hknight_stand7",
        MonsterFrame {
            frame: 6,
            next: "hknight_stand8",
            operations: OPS_HKNIGHT_STAND7,
        },
    ),
    (
        "hknight_stand8",
        MonsterFrame {
            frame: 7,
            next: "hknight_stand9",
            operations: OPS_HKNIGHT_STAND8,
        },
    ),
    (
        "hknight_stand9",
        MonsterFrame {
            frame: 8,
            next: "hknight_stand1",
            operations: OPS_HKNIGHT_STAND9,
        },
    ),
    (
        "hknight_walk1",
        MonsterFrame {
            frame: 9,
            next: "hknight_walk2",
            operations: OPS_HKNIGHT_WALK1,
        },
    ),
    (
        "hknight_walk10",
        MonsterFrame {
            frame: 18,
            next: "hknight_walk11",
            operations: OPS_HKNIGHT_WALK10,
        },
    ),
    (
        "hknight_walk11",
        MonsterFrame {
            frame: 19,
            next: "hknight_walk12",
            operations: OPS_HKNIGHT_WALK11,
        },
    ),
    (
        "hknight_walk12",
        MonsterFrame {
            frame: 20,
            next: "hknight_walk13",
            operations: OPS_HKNIGHT_WALK12,
        },
    ),
    (
        "hknight_walk13",
        MonsterFrame {
            frame: 21,
            next: "hknight_walk14",
            operations: OPS_HKNIGHT_WALK13,
        },
    ),
    (
        "hknight_walk14",
        MonsterFrame {
            frame: 22,
            next: "hknight_walk15",
            operations: OPS_HKNIGHT_WALK14,
        },
    ),
    (
        "hknight_walk15",
        MonsterFrame {
            frame: 23,
            next: "hknight_walk16",
            operations: OPS_HKNIGHT_WALK15,
        },
    ),
    (
        "hknight_walk16",
        MonsterFrame {
            frame: 24,
            next: "hknight_walk17",
            operations: OPS_HKNIGHT_WALK16,
        },
    ),
    (
        "hknight_walk17",
        MonsterFrame {
            frame: 25,
            next: "hknight_walk18",
            operations: OPS_HKNIGHT_WALK17,
        },
    ),
    (
        "hknight_walk18",
        MonsterFrame {
            frame: 26,
            next: "hknight_walk19",
            operations: OPS_HKNIGHT_WALK18,
        },
    ),
    (
        "hknight_walk19",
        MonsterFrame {
            frame: 27,
            next: "hknight_walk20",
            operations: OPS_HKNIGHT_WALK19,
        },
    ),
    (
        "hknight_walk2",
        MonsterFrame {
            frame: 10,
            next: "hknight_walk3",
            operations: OPS_HKNIGHT_WALK2,
        },
    ),
    (
        "hknight_walk20",
        MonsterFrame {
            frame: 28,
            next: "hknight_walk1",
            operations: OPS_HKNIGHT_WALK20,
        },
    ),
    (
        "hknight_walk3",
        MonsterFrame {
            frame: 11,
            next: "hknight_walk4",
            operations: OPS_HKNIGHT_WALK3,
        },
    ),
    (
        "hknight_walk4",
        MonsterFrame {
            frame: 12,
            next: "hknight_walk5",
            operations: OPS_HKNIGHT_WALK4,
        },
    ),
    (
        "hknight_walk5",
        MonsterFrame {
            frame: 13,
            next: "hknight_walk6",
            operations: OPS_HKNIGHT_WALK5,
        },
    ),
    (
        "hknight_walk6",
        MonsterFrame {
            frame: 14,
            next: "hknight_walk7",
            operations: OPS_HKNIGHT_WALK6,
        },
    ),
    (
        "hknight_walk7",
        MonsterFrame {
            frame: 15,
            next: "hknight_walk8",
            operations: OPS_HKNIGHT_WALK7,
        },
    ),
    (
        "hknight_walk8",
        MonsterFrame {
            frame: 16,
            next: "hknight_walk9",
            operations: OPS_HKNIGHT_WALK8,
        },
    ),
    (
        "hknight_walk9",
        MonsterFrame {
            frame: 17,
            next: "hknight_walk10",
            operations: OPS_HKNIGHT_WALK9,
        },
    ),
    (
        "hknight_watk1",
        MonsterFrame {
            frame: 133,
            next: "hknight_watk2",
            operations: OPS_HKNIGHT_WATK1,
        },
    ),
    (
        "hknight_watk10",
        MonsterFrame {
            frame: 142,
            next: "hknight_watk11",
            operations: OPS_HKNIGHT_WATK10,
        },
    ),
    (
        "hknight_watk11",
        MonsterFrame {
            frame: 143,
            next: "hknight_watk12",
            operations: OPS_HKNIGHT_WATK11,
        },
    ),
    (
        "hknight_watk12",
        MonsterFrame {
            frame: 144,
            next: "hknight_watk13",
            operations: OPS_HKNIGHT_WATK12,
        },
    ),
    (
        "hknight_watk13",
        MonsterFrame {
            frame: 145,
            next: "hknight_watk14",
            operations: OPS_HKNIGHT_WATK13,
        },
    ),
    (
        "hknight_watk14",
        MonsterFrame {
            frame: 146,
            next: "hknight_watk15",
            operations: OPS_HKNIGHT_WATK14,
        },
    ),
    (
        "hknight_watk15",
        MonsterFrame {
            frame: 147,
            next: "hknight_watk16",
            operations: OPS_HKNIGHT_WATK15,
        },
    ),
    (
        "hknight_watk16",
        MonsterFrame {
            frame: 148,
            next: "hknight_watk17",
            operations: OPS_HKNIGHT_WATK16,
        },
    ),
    (
        "hknight_watk17",
        MonsterFrame {
            frame: 149,
            next: "hknight_watk18",
            operations: OPS_HKNIGHT_WATK17,
        },
    ),
    (
        "hknight_watk18",
        MonsterFrame {
            frame: 150,
            next: "hknight_watk19",
            operations: OPS_HKNIGHT_WATK18,
        },
    ),
    (
        "hknight_watk19",
        MonsterFrame {
            frame: 151,
            next: "hknight_watk20",
            operations: OPS_HKNIGHT_WATK19,
        },
    ),
    (
        "hknight_watk2",
        MonsterFrame {
            frame: 134,
            next: "hknight_watk3",
            operations: OPS_HKNIGHT_WATK2,
        },
    ),
    (
        "hknight_watk20",
        MonsterFrame {
            frame: 152,
            next: "hknight_watk21",
            operations: OPS_HKNIGHT_WATK20,
        },
    ),
    (
        "hknight_watk21",
        MonsterFrame {
            frame: 153,
            next: "hknight_watk22",
            operations: OPS_HKNIGHT_WATK21,
        },
    ),
    (
        "hknight_watk22",
        MonsterFrame {
            frame: 154,
            next: "hknight_run1",
            operations: OPS_HKNIGHT_WATK22,
        },
    ),
    (
        "hknight_watk3",
        MonsterFrame {
            frame: 135,
            next: "hknight_watk4",
            operations: OPS_HKNIGHT_WATK3,
        },
    ),
    (
        "hknight_watk4",
        MonsterFrame {
            frame: 136,
            next: "hknight_watk5",
            operations: OPS_HKNIGHT_WATK4,
        },
    ),
    (
        "hknight_watk5",
        MonsterFrame {
            frame: 137,
            next: "hknight_watk6",
            operations: OPS_HKNIGHT_WATK5,
        },
    ),
    (
        "hknight_watk6",
        MonsterFrame {
            frame: 138,
            next: "hknight_watk7",
            operations: OPS_HKNIGHT_WATK6,
        },
    ),
    (
        "hknight_watk7",
        MonsterFrame {
            frame: 139,
            next: "hknight_watk8",
            operations: OPS_HKNIGHT_WATK7,
        },
    ),
    (
        "hknight_watk8",
        MonsterFrame {
            frame: 140,
            next: "hknight_watk9",
            operations: OPS_HKNIGHT_WATK8,
        },
    ),
    (
        "hknight_watk9",
        MonsterFrame {
            frame: 141,
            next: "hknight_watk10",
            operations: OPS_HKNIGHT_WATK9,
        },
    ),
    (
        "knight_atk1",
        MonsterFrame {
            frame: 42,
            next: "knight_atk2",
            operations: OPS_KNIGHT_ATK1,
        },
    ),
    (
        "knight_atk10",
        MonsterFrame {
            frame: 51,
            next: "knight_run1",
            operations: OPS_KNIGHT_ATK10,
        },
    ),
    (
        "knight_atk2",
        MonsterFrame {
            frame: 43,
            next: "knight_atk3",
            operations: OPS_KNIGHT_ATK2,
        },
    ),
    (
        "knight_atk3",
        MonsterFrame {
            frame: 44,
            next: "knight_atk4",
            operations: OPS_KNIGHT_ATK3,
        },
    ),
    (
        "knight_atk4",
        MonsterFrame {
            frame: 45,
            next: "knight_atk5",
            operations: OPS_KNIGHT_ATK4,
        },
    ),
    (
        "knight_atk5",
        MonsterFrame {
            frame: 46,
            next: "knight_atk6",
            operations: OPS_KNIGHT_ATK5,
        },
    ),
    (
        "knight_atk6",
        MonsterFrame {
            frame: 47,
            next: "knight_atk7",
            operations: OPS_KNIGHT_ATK6,
        },
    ),
    (
        "knight_atk7",
        MonsterFrame {
            frame: 48,
            next: "knight_atk8",
            operations: OPS_KNIGHT_ATK7,
        },
    ),
    (
        "knight_atk8",
        MonsterFrame {
            frame: 49,
            next: "knight_atk9",
            operations: OPS_KNIGHT_ATK8,
        },
    ),
    (
        "knight_atk9",
        MonsterFrame {
            frame: 50,
            next: "knight_atk10",
            operations: OPS_KNIGHT_ATK9,
        },
    ),
    (
        "knight_bow1",
        MonsterFrame {
            frame: 67,
            next: "knight_bow2",
            operations: OPS_KNIGHT_BOW1,
        },
    ),
    (
        "knight_bow10",
        MonsterFrame {
            frame: 53,
            next: "knight_walk1",
            operations: OPS_KNIGHT_BOW10,
        },
    ),
    (
        "knight_bow2",
        MonsterFrame {
            frame: 68,
            next: "knight_bow3",
            operations: OPS_KNIGHT_BOW2,
        },
    ),
    (
        "knight_bow3",
        MonsterFrame {
            frame: 69,
            next: "knight_bow4",
            operations: OPS_KNIGHT_BOW3,
        },
    ),
    (
        "knight_bow4",
        MonsterFrame {
            frame: 70,
            next: "knight_bow5",
            operations: OPS_KNIGHT_BOW4,
        },
    ),
    (
        "knight_bow5",
        MonsterFrame {
            frame: 71,
            next: "knight_bow5",
            operations: OPS_KNIGHT_BOW5,
        },
    ),
    (
        "knight_bow6",
        MonsterFrame {
            frame: 70,
            next: "knight_bow7",
            operations: OPS_KNIGHT_BOW6,
        },
    ),
    (
        "knight_bow7",
        MonsterFrame {
            frame: 69,
            next: "knight_bow8",
            operations: OPS_KNIGHT_BOW7,
        },
    ),
    (
        "knight_bow8",
        MonsterFrame {
            frame: 68,
            next: "knight_bow9",
            operations: OPS_KNIGHT_BOW8,
        },
    ),
    (
        "knight_bow9",
        MonsterFrame {
            frame: 67,
            next: "knight_bow10",
            operations: OPS_KNIGHT_BOW9,
        },
    ),
    (
        "knight_die1",
        MonsterFrame {
            frame: 76,
            next: "knight_die2",
            operations: OPS_KNIGHT_DIE1,
        },
    ),
    (
        "knight_die10",
        MonsterFrame {
            frame: 85,
            next: "knight_die10",
            operations: OPS_KNIGHT_DIE10,
        },
    ),
    (
        "knight_die2",
        MonsterFrame {
            frame: 77,
            next: "knight_die3",
            operations: OPS_KNIGHT_DIE2,
        },
    ),
    (
        "knight_die3",
        MonsterFrame {
            frame: 78,
            next: "knight_die4",
            operations: OPS_KNIGHT_DIE3,
        },
    ),
    (
        "knight_die4",
        MonsterFrame {
            frame: 79,
            next: "knight_die5",
            operations: OPS_KNIGHT_DIE4,
        },
    ),
    (
        "knight_die5",
        MonsterFrame {
            frame: 80,
            next: "knight_die6",
            operations: OPS_KNIGHT_DIE5,
        },
    ),
    (
        "knight_die6",
        MonsterFrame {
            frame: 81,
            next: "knight_die7",
            operations: OPS_KNIGHT_DIE6,
        },
    ),
    (
        "knight_die7",
        MonsterFrame {
            frame: 82,
            next: "knight_die8",
            operations: OPS_KNIGHT_DIE7,
        },
    ),
    (
        "knight_die8",
        MonsterFrame {
            frame: 83,
            next: "knight_die9",
            operations: OPS_KNIGHT_DIE8,
        },
    ),
    (
        "knight_die9",
        MonsterFrame {
            frame: 84,
            next: "knight_die10",
            operations: OPS_KNIGHT_DIE9,
        },
    ),
    (
        "knight_dieb1",
        MonsterFrame {
            frame: 86,
            next: "knight_dieb2",
            operations: OPS_KNIGHT_DIEB1,
        },
    ),
    (
        "knight_dieb10",
        MonsterFrame {
            frame: 95,
            next: "knight_dieb11",
            operations: OPS_KNIGHT_DIEB10,
        },
    ),
    (
        "knight_dieb11",
        MonsterFrame {
            frame: 96,
            next: "knight_dieb11",
            operations: OPS_KNIGHT_DIEB11,
        },
    ),
    (
        "knight_dieb2",
        MonsterFrame {
            frame: 87,
            next: "knight_dieb3",
            operations: OPS_KNIGHT_DIEB2,
        },
    ),
    (
        "knight_dieb3",
        MonsterFrame {
            frame: 88,
            next: "knight_dieb4",
            operations: OPS_KNIGHT_DIEB3,
        },
    ),
    (
        "knight_dieb4",
        MonsterFrame {
            frame: 89,
            next: "knight_dieb5",
            operations: OPS_KNIGHT_DIEB4,
        },
    ),
    (
        "knight_dieb5",
        MonsterFrame {
            frame: 90,
            next: "knight_dieb6",
            operations: OPS_KNIGHT_DIEB5,
        },
    ),
    (
        "knight_dieb6",
        MonsterFrame {
            frame: 91,
            next: "knight_dieb7",
            operations: OPS_KNIGHT_DIEB6,
        },
    ),
    (
        "knight_dieb7",
        MonsterFrame {
            frame: 92,
            next: "knight_dieb8",
            operations: OPS_KNIGHT_DIEB7,
        },
    ),
    (
        "knight_dieb8",
        MonsterFrame {
            frame: 93,
            next: "knight_dieb9",
            operations: OPS_KNIGHT_DIEB8,
        },
    ),
    (
        "knight_dieb9",
        MonsterFrame {
            frame: 94,
            next: "knight_dieb10",
            operations: OPS_KNIGHT_DIEB9,
        },
    ),
    (
        "knight_pain1",
        MonsterFrame {
            frame: 28,
            next: "knight_pain2",
            operations: OPS_KNIGHT_PAIN1,
        },
    ),
    (
        "knight_pain2",
        MonsterFrame {
            frame: 29,
            next: "knight_pain3",
            operations: OPS_KNIGHT_PAIN2,
        },
    ),
    (
        "knight_pain3",
        MonsterFrame {
            frame: 30,
            next: "knight_run1",
            operations: OPS_KNIGHT_PAIN3,
        },
    ),
    (
        "knight_painb1",
        MonsterFrame {
            frame: 31,
            next: "knight_painb2",
            operations: OPS_KNIGHT_PAINB1,
        },
    ),
    (
        "knight_painb10",
        MonsterFrame {
            frame: 40,
            next: "knight_painb11",
            operations: OPS_KNIGHT_PAINB10,
        },
    ),
    (
        "knight_painb11",
        MonsterFrame {
            frame: 41,
            next: "knight_run1",
            operations: OPS_KNIGHT_PAINB11,
        },
    ),
    (
        "knight_painb2",
        MonsterFrame {
            frame: 32,
            next: "knight_painb3",
            operations: OPS_KNIGHT_PAINB2,
        },
    ),
    (
        "knight_painb3",
        MonsterFrame {
            frame: 33,
            next: "knight_painb4",
            operations: OPS_KNIGHT_PAINB3,
        },
    ),
    (
        "knight_painb4",
        MonsterFrame {
            frame: 34,
            next: "knight_painb5",
            operations: OPS_KNIGHT_PAINB4,
        },
    ),
    (
        "knight_painb5",
        MonsterFrame {
            frame: 35,
            next: "knight_painb6",
            operations: OPS_KNIGHT_PAINB5,
        },
    ),
    (
        "knight_painb6",
        MonsterFrame {
            frame: 36,
            next: "knight_painb7",
            operations: OPS_KNIGHT_PAINB6,
        },
    ),
    (
        "knight_painb7",
        MonsterFrame {
            frame: 37,
            next: "knight_painb8",
            operations: OPS_KNIGHT_PAINB7,
        },
    ),
    (
        "knight_painb8",
        MonsterFrame {
            frame: 38,
            next: "knight_painb9",
            operations: OPS_KNIGHT_PAINB8,
        },
    ),
    (
        "knight_painb9",
        MonsterFrame {
            frame: 39,
            next: "knight_painb10",
            operations: OPS_KNIGHT_PAINB9,
        },
    ),
    (
        "knight_run1",
        MonsterFrame {
            frame: 9,
            next: "knight_run2",
            operations: OPS_KNIGHT_RUN1,
        },
    ),
    (
        "knight_run2",
        MonsterFrame {
            frame: 10,
            next: "knight_run3",
            operations: OPS_KNIGHT_RUN2,
        },
    ),
    (
        "knight_run3",
        MonsterFrame {
            frame: 11,
            next: "knight_run4",
            operations: OPS_KNIGHT_RUN3,
        },
    ),
    (
        "knight_run4",
        MonsterFrame {
            frame: 12,
            next: "knight_run5",
            operations: OPS_KNIGHT_RUN4,
        },
    ),
    (
        "knight_run5",
        MonsterFrame {
            frame: 13,
            next: "knight_run6",
            operations: OPS_KNIGHT_RUN5,
        },
    ),
    (
        "knight_run6",
        MonsterFrame {
            frame: 14,
            next: "knight_run7",
            operations: OPS_KNIGHT_RUN6,
        },
    ),
    (
        "knight_run7",
        MonsterFrame {
            frame: 15,
            next: "knight_run8",
            operations: OPS_KNIGHT_RUN7,
        },
    ),
    (
        "knight_run8",
        MonsterFrame {
            frame: 16,
            next: "knight_run1",
            operations: OPS_KNIGHT_RUN8,
        },
    ),
    (
        "knight_runatk1",
        MonsterFrame {
            frame: 17,
            next: "knight_runatk2",
            operations: OPS_KNIGHT_RUNATK1,
        },
    ),
    (
        "knight_runatk10",
        MonsterFrame {
            frame: 26,
            next: "knight_runatk11",
            operations: OPS_KNIGHT_RUNATK10,
        },
    ),
    (
        "knight_runatk11",
        MonsterFrame {
            frame: 27,
            next: "knight_run1",
            operations: OPS_KNIGHT_RUNATK11,
        },
    ),
    (
        "knight_runatk2",
        MonsterFrame {
            frame: 18,
            next: "knight_runatk3",
            operations: OPS_KNIGHT_RUNATK2,
        },
    ),
    (
        "knight_runatk3",
        MonsterFrame {
            frame: 19,
            next: "knight_runatk4",
            operations: OPS_KNIGHT_RUNATK3,
        },
    ),
    (
        "knight_runatk4",
        MonsterFrame {
            frame: 20,
            next: "knight_runatk5",
            operations: OPS_KNIGHT_RUNATK4,
        },
    ),
    (
        "knight_runatk5",
        MonsterFrame {
            frame: 21,
            next: "knight_runatk6",
            operations: OPS_KNIGHT_RUNATK5,
        },
    ),
    (
        "knight_runatk6",
        MonsterFrame {
            frame: 22,
            next: "knight_runatk7",
            operations: OPS_KNIGHT_RUNATK6,
        },
    ),
    (
        "knight_runatk7",
        MonsterFrame {
            frame: 23,
            next: "knight_runatk8",
            operations: OPS_KNIGHT_RUNATK7,
        },
    ),
    (
        "knight_runatk8",
        MonsterFrame {
            frame: 24,
            next: "knight_runatk9",
            operations: OPS_KNIGHT_RUNATK8,
        },
    ),
    (
        "knight_runatk9",
        MonsterFrame {
            frame: 25,
            next: "knight_runatk10",
            operations: OPS_KNIGHT_RUNATK9,
        },
    ),
    (
        "knight_stand1",
        MonsterFrame {
            frame: 0,
            next: "knight_stand2",
            operations: OPS_KNIGHT_STAND1,
        },
    ),
    (
        "knight_stand2",
        MonsterFrame {
            frame: 1,
            next: "knight_stand3",
            operations: OPS_KNIGHT_STAND2,
        },
    ),
    (
        "knight_stand3",
        MonsterFrame {
            frame: 2,
            next: "knight_stand4",
            operations: OPS_KNIGHT_STAND3,
        },
    ),
    (
        "knight_stand4",
        MonsterFrame {
            frame: 3,
            next: "knight_stand5",
            operations: OPS_KNIGHT_STAND4,
        },
    ),
    (
        "knight_stand5",
        MonsterFrame {
            frame: 4,
            next: "knight_stand6",
            operations: OPS_KNIGHT_STAND5,
        },
    ),
    (
        "knight_stand6",
        MonsterFrame {
            frame: 5,
            next: "knight_stand7",
            operations: OPS_KNIGHT_STAND6,
        },
    ),
    (
        "knight_stand7",
        MonsterFrame {
            frame: 6,
            next: "knight_stand8",
            operations: OPS_KNIGHT_STAND7,
        },
    ),
    (
        "knight_stand8",
        MonsterFrame {
            frame: 7,
            next: "knight_stand9",
            operations: OPS_KNIGHT_STAND8,
        },
    ),
    (
        "knight_stand9",
        MonsterFrame {
            frame: 8,
            next: "knight_stand1",
            operations: OPS_KNIGHT_STAND9,
        },
    ),
    (
        "knight_walk1",
        MonsterFrame {
            frame: 53,
            next: "knight_walk2",
            operations: OPS_KNIGHT_WALK1,
        },
    ),
    (
        "knight_walk10",
        MonsterFrame {
            frame: 62,
            next: "knight_walk11",
            operations: OPS_KNIGHT_WALK10,
        },
    ),
    (
        "knight_walk11",
        MonsterFrame {
            frame: 63,
            next: "knight_walk12",
            operations: OPS_KNIGHT_WALK11,
        },
    ),
    (
        "knight_walk12",
        MonsterFrame {
            frame: 64,
            next: "knight_walk13",
            operations: OPS_KNIGHT_WALK12,
        },
    ),
    (
        "knight_walk13",
        MonsterFrame {
            frame: 65,
            next: "knight_walk14",
            operations: OPS_KNIGHT_WALK13,
        },
    ),
    (
        "knight_walk14",
        MonsterFrame {
            frame: 66,
            next: "knight_walk1",
            operations: OPS_KNIGHT_WALK14,
        },
    ),
    (
        "knight_walk2",
        MonsterFrame {
            frame: 54,
            next: "knight_walk3",
            operations: OPS_KNIGHT_WALK2,
        },
    ),
    (
        "knight_walk3",
        MonsterFrame {
            frame: 55,
            next: "knight_walk4",
            operations: OPS_KNIGHT_WALK3,
        },
    ),
    (
        "knight_walk4",
        MonsterFrame {
            frame: 56,
            next: "knight_walk5",
            operations: OPS_KNIGHT_WALK4,
        },
    ),
    (
        "knight_walk5",
        MonsterFrame {
            frame: 57,
            next: "knight_walk6",
            operations: OPS_KNIGHT_WALK5,
        },
    ),
    (
        "knight_walk6",
        MonsterFrame {
            frame: 58,
            next: "knight_walk7",
            operations: OPS_KNIGHT_WALK6,
        },
    ),
    (
        "knight_walk7",
        MonsterFrame {
            frame: 59,
            next: "knight_walk8",
            operations: OPS_KNIGHT_WALK7,
        },
    ),
    (
        "knight_walk8",
        MonsterFrame {
            frame: 60,
            next: "knight_walk9",
            operations: OPS_KNIGHT_WALK8,
        },
    ),
    (
        "knight_walk9",
        MonsterFrame {
            frame: 61,
            next: "knight_walk10",
            operations: OPS_KNIGHT_WALK9,
        },
    ),
    (
        "ogre_bdie1",
        MonsterFrame {
            frame: 126,
            next: "ogre_bdie2",
            operations: OPS_OGRE_BDIE1,
        },
    ),
    (
        "ogre_bdie10",
        MonsterFrame {
            frame: 135,
            next: "ogre_bdie10",
            operations: OPS_OGRE_BDIE10,
        },
    ),
    (
        "ogre_bdie2",
        MonsterFrame {
            frame: 127,
            next: "ogre_bdie3",
            operations: OPS_OGRE_BDIE2,
        },
    ),
    (
        "ogre_bdie3",
        MonsterFrame {
            frame: 128,
            next: "ogre_bdie4",
            operations: OPS_OGRE_BDIE3,
        },
    ),
    (
        "ogre_bdie4",
        MonsterFrame {
            frame: 129,
            next: "ogre_bdie5",
            operations: OPS_OGRE_BDIE4,
        },
    ),
    (
        "ogre_bdie5",
        MonsterFrame {
            frame: 130,
            next: "ogre_bdie6",
            operations: OPS_OGRE_BDIE5,
        },
    ),
    (
        "ogre_bdie6",
        MonsterFrame {
            frame: 131,
            next: "ogre_bdie7",
            operations: OPS_OGRE_BDIE6,
        },
    ),
    (
        "ogre_bdie7",
        MonsterFrame {
            frame: 132,
            next: "ogre_bdie8",
            operations: OPS_OGRE_BDIE7,
        },
    ),
    (
        "ogre_bdie8",
        MonsterFrame {
            frame: 133,
            next: "ogre_bdie9",
            operations: OPS_OGRE_BDIE8,
        },
    ),
    (
        "ogre_bdie9",
        MonsterFrame {
            frame: 134,
            next: "ogre_bdie10",
            operations: OPS_OGRE_BDIE9,
        },
    ),
    (
        "ogre_die1",
        MonsterFrame {
            frame: 112,
            next: "ogre_die2",
            operations: OPS_OGRE_DIE1,
        },
    ),
    (
        "ogre_die10",
        MonsterFrame {
            frame: 121,
            next: "ogre_die11",
            operations: OPS_OGRE_DIE10,
        },
    ),
    (
        "ogre_die11",
        MonsterFrame {
            frame: 122,
            next: "ogre_die12",
            operations: OPS_OGRE_DIE11,
        },
    ),
    (
        "ogre_die12",
        MonsterFrame {
            frame: 123,
            next: "ogre_die13",
            operations: OPS_OGRE_DIE12,
        },
    ),
    (
        "ogre_die13",
        MonsterFrame {
            frame: 124,
            next: "ogre_die14",
            operations: OPS_OGRE_DIE13,
        },
    ),
    (
        "ogre_die14",
        MonsterFrame {
            frame: 125,
            next: "ogre_die14",
            operations: OPS_OGRE_DIE14,
        },
    ),
    (
        "ogre_die2",
        MonsterFrame {
            frame: 113,
            next: "ogre_die3",
            operations: OPS_OGRE_DIE2,
        },
    ),
    (
        "ogre_die3",
        MonsterFrame {
            frame: 114,
            next: "ogre_die4",
            operations: OPS_OGRE_DIE3,
        },
    ),
    (
        "ogre_die4",
        MonsterFrame {
            frame: 115,
            next: "ogre_die5",
            operations: OPS_OGRE_DIE4,
        },
    ),
    (
        "ogre_die5",
        MonsterFrame {
            frame: 116,
            next: "ogre_die6",
            operations: OPS_OGRE_DIE5,
        },
    ),
    (
        "ogre_die6",
        MonsterFrame {
            frame: 117,
            next: "ogre_die7",
            operations: OPS_OGRE_DIE6,
        },
    ),
    (
        "ogre_die7",
        MonsterFrame {
            frame: 118,
            next: "ogre_die8",
            operations: OPS_OGRE_DIE7,
        },
    ),
    (
        "ogre_die8",
        MonsterFrame {
            frame: 119,
            next: "ogre_die9",
            operations: OPS_OGRE_DIE8,
        },
    ),
    (
        "ogre_die9",
        MonsterFrame {
            frame: 120,
            next: "ogre_die10",
            operations: OPS_OGRE_DIE9,
        },
    ),
    (
        "ogre_nail1",
        MonsterFrame {
            frame: 61,
            next: "ogre_nail2",
            operations: OPS_OGRE_NAIL1,
        },
    ),
    (
        "ogre_nail2",
        MonsterFrame {
            frame: 62,
            next: "ogre_nail3",
            operations: OPS_OGRE_NAIL2,
        },
    ),
    (
        "ogre_nail3",
        MonsterFrame {
            frame: 62,
            next: "ogre_nail4",
            operations: OPS_OGRE_NAIL3,
        },
    ),
    (
        "ogre_nail4",
        MonsterFrame {
            frame: 63,
            next: "ogre_nail5",
            operations: OPS_OGRE_NAIL4,
        },
    ),
    (
        "ogre_nail5",
        MonsterFrame {
            frame: 64,
            next: "ogre_nail6",
            operations: OPS_OGRE_NAIL5,
        },
    ),
    (
        "ogre_nail6",
        MonsterFrame {
            frame: 65,
            next: "ogre_nail7",
            operations: OPS_OGRE_NAIL6,
        },
    ),
    (
        "ogre_nail7",
        MonsterFrame {
            frame: 66,
            next: "ogre_run1",
            operations: OPS_OGRE_NAIL7,
        },
    ),
    (
        "ogre_pain1",
        MonsterFrame {
            frame: 67,
            next: "ogre_pain2",
            operations: OPS_OGRE_PAIN1,
        },
    ),
    (
        "ogre_pain2",
        MonsterFrame {
            frame: 68,
            next: "ogre_pain3",
            operations: OPS_OGRE_PAIN2,
        },
    ),
    (
        "ogre_pain3",
        MonsterFrame {
            frame: 69,
            next: "ogre_pain4",
            operations: OPS_OGRE_PAIN3,
        },
    ),
    (
        "ogre_pain4",
        MonsterFrame {
            frame: 70,
            next: "ogre_pain5",
            operations: OPS_OGRE_PAIN4,
        },
    ),
    (
        "ogre_pain5",
        MonsterFrame {
            frame: 71,
            next: "ogre_run1",
            operations: OPS_OGRE_PAIN5,
        },
    ),
    (
        "ogre_painb1",
        MonsterFrame {
            frame: 72,
            next: "ogre_painb2",
            operations: OPS_OGRE_PAINB1,
        },
    ),
    (
        "ogre_painb2",
        MonsterFrame {
            frame: 73,
            next: "ogre_painb3",
            operations: OPS_OGRE_PAINB2,
        },
    ),
    (
        "ogre_painb3",
        MonsterFrame {
            frame: 74,
            next: "ogre_run1",
            operations: OPS_OGRE_PAINB3,
        },
    ),
    (
        "ogre_painc1",
        MonsterFrame {
            frame: 75,
            next: "ogre_painc2",
            operations: OPS_OGRE_PAINC1,
        },
    ),
    (
        "ogre_painc2",
        MonsterFrame {
            frame: 76,
            next: "ogre_painc3",
            operations: OPS_OGRE_PAINC2,
        },
    ),
    (
        "ogre_painc3",
        MonsterFrame {
            frame: 77,
            next: "ogre_painc4",
            operations: OPS_OGRE_PAINC3,
        },
    ),
    (
        "ogre_painc4",
        MonsterFrame {
            frame: 78,
            next: "ogre_painc5",
            operations: OPS_OGRE_PAINC4,
        },
    ),
    (
        "ogre_painc5",
        MonsterFrame {
            frame: 79,
            next: "ogre_painc6",
            operations: OPS_OGRE_PAINC5,
        },
    ),
    (
        "ogre_painc6",
        MonsterFrame {
            frame: 80,
            next: "ogre_run1",
            operations: OPS_OGRE_PAINC6,
        },
    ),
    (
        "ogre_paind1",
        MonsterFrame {
            frame: 81,
            next: "ogre_paind2",
            operations: OPS_OGRE_PAIND1,
        },
    ),
    (
        "ogre_paind10",
        MonsterFrame {
            frame: 90,
            next: "ogre_paind11",
            operations: OPS_OGRE_PAIND10,
        },
    ),
    (
        "ogre_paind11",
        MonsterFrame {
            frame: 91,
            next: "ogre_paind12",
            operations: OPS_OGRE_PAIND11,
        },
    ),
    (
        "ogre_paind12",
        MonsterFrame {
            frame: 92,
            next: "ogre_paind13",
            operations: OPS_OGRE_PAIND12,
        },
    ),
    (
        "ogre_paind13",
        MonsterFrame {
            frame: 93,
            next: "ogre_paind14",
            operations: OPS_OGRE_PAIND13,
        },
    ),
    (
        "ogre_paind14",
        MonsterFrame {
            frame: 94,
            next: "ogre_paind15",
            operations: OPS_OGRE_PAIND14,
        },
    ),
    (
        "ogre_paind15",
        MonsterFrame {
            frame: 95,
            next: "ogre_paind16",
            operations: OPS_OGRE_PAIND15,
        },
    ),
    (
        "ogre_paind16",
        MonsterFrame {
            frame: 96,
            next: "ogre_run1",
            operations: OPS_OGRE_PAIND16,
        },
    ),
    (
        "ogre_paind2",
        MonsterFrame {
            frame: 82,
            next: "ogre_paind3",
            operations: OPS_OGRE_PAIND2,
        },
    ),
    (
        "ogre_paind3",
        MonsterFrame {
            frame: 83,
            next: "ogre_paind4",
            operations: OPS_OGRE_PAIND3,
        },
    ),
    (
        "ogre_paind4",
        MonsterFrame {
            frame: 84,
            next: "ogre_paind5",
            operations: OPS_OGRE_PAIND4,
        },
    ),
    (
        "ogre_paind5",
        MonsterFrame {
            frame: 85,
            next: "ogre_paind6",
            operations: OPS_OGRE_PAIND5,
        },
    ),
    (
        "ogre_paind6",
        MonsterFrame {
            frame: 86,
            next: "ogre_paind7",
            operations: OPS_OGRE_PAIND6,
        },
    ),
    (
        "ogre_paind7",
        MonsterFrame {
            frame: 87,
            next: "ogre_paind8",
            operations: OPS_OGRE_PAIND7,
        },
    ),
    (
        "ogre_paind8",
        MonsterFrame {
            frame: 88,
            next: "ogre_paind9",
            operations: OPS_OGRE_PAIND8,
        },
    ),
    (
        "ogre_paind9",
        MonsterFrame {
            frame: 89,
            next: "ogre_paind10",
            operations: OPS_OGRE_PAIND9,
        },
    ),
    (
        "ogre_paine1",
        MonsterFrame {
            frame: 97,
            next: "ogre_paine2",
            operations: OPS_OGRE_PAINE1,
        },
    ),
    (
        "ogre_paine10",
        MonsterFrame {
            frame: 106,
            next: "ogre_paine11",
            operations: OPS_OGRE_PAINE10,
        },
    ),
    (
        "ogre_paine11",
        MonsterFrame {
            frame: 107,
            next: "ogre_paine12",
            operations: OPS_OGRE_PAINE11,
        },
    ),
    (
        "ogre_paine12",
        MonsterFrame {
            frame: 108,
            next: "ogre_paine13",
            operations: OPS_OGRE_PAINE12,
        },
    ),
    (
        "ogre_paine13",
        MonsterFrame {
            frame: 109,
            next: "ogre_paine14",
            operations: OPS_OGRE_PAINE13,
        },
    ),
    (
        "ogre_paine14",
        MonsterFrame {
            frame: 110,
            next: "ogre_paine15",
            operations: OPS_OGRE_PAINE14,
        },
    ),
    (
        "ogre_paine15",
        MonsterFrame {
            frame: 111,
            next: "ogre_run1",
            operations: OPS_OGRE_PAINE15,
        },
    ),
    (
        "ogre_paine2",
        MonsterFrame {
            frame: 98,
            next: "ogre_paine3",
            operations: OPS_OGRE_PAINE2,
        },
    ),
    (
        "ogre_paine3",
        MonsterFrame {
            frame: 99,
            next: "ogre_paine4",
            operations: OPS_OGRE_PAINE3,
        },
    ),
    (
        "ogre_paine4",
        MonsterFrame {
            frame: 100,
            next: "ogre_paine5",
            operations: OPS_OGRE_PAINE4,
        },
    ),
    (
        "ogre_paine5",
        MonsterFrame {
            frame: 101,
            next: "ogre_paine6",
            operations: OPS_OGRE_PAINE5,
        },
    ),
    (
        "ogre_paine6",
        MonsterFrame {
            frame: 102,
            next: "ogre_paine7",
            operations: OPS_OGRE_PAINE6,
        },
    ),
    (
        "ogre_paine7",
        MonsterFrame {
            frame: 103,
            next: "ogre_paine8",
            operations: OPS_OGRE_PAINE7,
        },
    ),
    (
        "ogre_paine8",
        MonsterFrame {
            frame: 104,
            next: "ogre_paine9",
            operations: OPS_OGRE_PAINE8,
        },
    ),
    (
        "ogre_paine9",
        MonsterFrame {
            frame: 105,
            next: "ogre_paine10",
            operations: OPS_OGRE_PAINE9,
        },
    ),
    (
        "ogre_run1",
        MonsterFrame {
            frame: 25,
            next: "ogre_run2",
            operations: OPS_OGRE_RUN1,
        },
    ),
    (
        "ogre_run2",
        MonsterFrame {
            frame: 26,
            next: "ogre_run3",
            operations: OPS_OGRE_RUN2,
        },
    ),
    (
        "ogre_run3",
        MonsterFrame {
            frame: 27,
            next: "ogre_run4",
            operations: OPS_OGRE_RUN3,
        },
    ),
    (
        "ogre_run4",
        MonsterFrame {
            frame: 28,
            next: "ogre_run5",
            operations: OPS_OGRE_RUN4,
        },
    ),
    (
        "ogre_run5",
        MonsterFrame {
            frame: 29,
            next: "ogre_run6",
            operations: OPS_OGRE_RUN5,
        },
    ),
    (
        "ogre_run6",
        MonsterFrame {
            frame: 30,
            next: "ogre_run7",
            operations: OPS_OGRE_RUN6,
        },
    ),
    (
        "ogre_run7",
        MonsterFrame {
            frame: 31,
            next: "ogre_run8",
            operations: OPS_OGRE_RUN7,
        },
    ),
    (
        "ogre_run8",
        MonsterFrame {
            frame: 32,
            next: "ogre_run1",
            operations: OPS_OGRE_RUN8,
        },
    ),
    (
        "ogre_smash1",
        MonsterFrame {
            frame: 47,
            next: "ogre_smash2",
            operations: OPS_OGRE_SMASH1,
        },
    ),
    (
        "ogre_smash10",
        MonsterFrame {
            frame: 56,
            next: "ogre_smash11",
            operations: OPS_OGRE_SMASH10,
        },
    ),
    (
        "ogre_smash11",
        MonsterFrame {
            frame: 57,
            next: "ogre_smash12",
            operations: OPS_OGRE_SMASH11,
        },
    ),
    (
        "ogre_smash12",
        MonsterFrame {
            frame: 58,
            next: "ogre_smash13",
            operations: OPS_OGRE_SMASH12,
        },
    ),
    (
        "ogre_smash13",
        MonsterFrame {
            frame: 59,
            next: "ogre_smash14",
            operations: OPS_OGRE_SMASH13,
        },
    ),
    (
        "ogre_smash14",
        MonsterFrame {
            frame: 60,
            next: "ogre_run1",
            operations: OPS_OGRE_SMASH14,
        },
    ),
    (
        "ogre_smash2",
        MonsterFrame {
            frame: 48,
            next: "ogre_smash3",
            operations: OPS_OGRE_SMASH2,
        },
    ),
    (
        "ogre_smash3",
        MonsterFrame {
            frame: 49,
            next: "ogre_smash4",
            operations: OPS_OGRE_SMASH3,
        },
    ),
    (
        "ogre_smash4",
        MonsterFrame {
            frame: 50,
            next: "ogre_smash5",
            operations: OPS_OGRE_SMASH4,
        },
    ),
    (
        "ogre_smash5",
        MonsterFrame {
            frame: 51,
            next: "ogre_smash6",
            operations: OPS_OGRE_SMASH5,
        },
    ),
    (
        "ogre_smash6",
        MonsterFrame {
            frame: 52,
            next: "ogre_smash7",
            operations: OPS_OGRE_SMASH6,
        },
    ),
    (
        "ogre_smash7",
        MonsterFrame {
            frame: 53,
            next: "ogre_smash8",
            operations: OPS_OGRE_SMASH7,
        },
    ),
    (
        "ogre_smash8",
        MonsterFrame {
            frame: 54,
            next: "ogre_smash9",
            operations: OPS_OGRE_SMASH8,
        },
    ),
    (
        "ogre_smash9",
        MonsterFrame {
            frame: 55,
            next: "ogre_smash10",
            operations: OPS_OGRE_SMASH9,
        },
    ),
    (
        "ogre_stand1",
        MonsterFrame {
            frame: 0,
            next: "ogre_stand2",
            operations: OPS_OGRE_STAND1,
        },
    ),
    (
        "ogre_stand2",
        MonsterFrame {
            frame: 1,
            next: "ogre_stand3",
            operations: OPS_OGRE_STAND2,
        },
    ),
    (
        "ogre_stand3",
        MonsterFrame {
            frame: 2,
            next: "ogre_stand4",
            operations: OPS_OGRE_STAND3,
        },
    ),
    (
        "ogre_stand4",
        MonsterFrame {
            frame: 3,
            next: "ogre_stand5",
            operations: OPS_OGRE_STAND4,
        },
    ),
    (
        "ogre_stand5",
        MonsterFrame {
            frame: 4,
            next: "ogre_stand6",
            operations: OPS_OGRE_STAND5,
        },
    ),
    (
        "ogre_stand6",
        MonsterFrame {
            frame: 5,
            next: "ogre_stand7",
            operations: OPS_OGRE_STAND6,
        },
    ),
    (
        "ogre_stand7",
        MonsterFrame {
            frame: 6,
            next: "ogre_stand8",
            operations: OPS_OGRE_STAND7,
        },
    ),
    (
        "ogre_stand8",
        MonsterFrame {
            frame: 7,
            next: "ogre_stand9",
            operations: OPS_OGRE_STAND8,
        },
    ),
    (
        "ogre_stand9",
        MonsterFrame {
            frame: 8,
            next: "ogre_stand1",
            operations: OPS_OGRE_STAND9,
        },
    ),
    (
        "ogre_swing1",
        MonsterFrame {
            frame: 33,
            next: "ogre_swing2",
            operations: OPS_OGRE_SWING1,
        },
    ),
    (
        "ogre_swing10",
        MonsterFrame {
            frame: 42,
            next: "ogre_swing11",
            operations: OPS_OGRE_SWING10,
        },
    ),
    (
        "ogre_swing11",
        MonsterFrame {
            frame: 43,
            next: "ogre_swing12",
            operations: OPS_OGRE_SWING11,
        },
    ),
    (
        "ogre_swing12",
        MonsterFrame {
            frame: 44,
            next: "ogre_swing13",
            operations: OPS_OGRE_SWING12,
        },
    ),
    (
        "ogre_swing13",
        MonsterFrame {
            frame: 45,
            next: "ogre_swing14",
            operations: OPS_OGRE_SWING13,
        },
    ),
    (
        "ogre_swing14",
        MonsterFrame {
            frame: 46,
            next: "ogre_run1",
            operations: OPS_OGRE_SWING14,
        },
    ),
    (
        "ogre_swing2",
        MonsterFrame {
            frame: 34,
            next: "ogre_swing3",
            operations: OPS_OGRE_SWING2,
        },
    ),
    (
        "ogre_swing3",
        MonsterFrame {
            frame: 35,
            next: "ogre_swing4",
            operations: OPS_OGRE_SWING3,
        },
    ),
    (
        "ogre_swing4",
        MonsterFrame {
            frame: 36,
            next: "ogre_swing5",
            operations: OPS_OGRE_SWING4,
        },
    ),
    (
        "ogre_swing5",
        MonsterFrame {
            frame: 37,
            next: "ogre_swing6",
            operations: OPS_OGRE_SWING5,
        },
    ),
    (
        "ogre_swing6",
        MonsterFrame {
            frame: 38,
            next: "ogre_swing7",
            operations: OPS_OGRE_SWING6,
        },
    ),
    (
        "ogre_swing7",
        MonsterFrame {
            frame: 39,
            next: "ogre_swing8",
            operations: OPS_OGRE_SWING7,
        },
    ),
    (
        "ogre_swing8",
        MonsterFrame {
            frame: 40,
            next: "ogre_swing9",
            operations: OPS_OGRE_SWING8,
        },
    ),
    (
        "ogre_swing9",
        MonsterFrame {
            frame: 41,
            next: "ogre_swing10",
            operations: OPS_OGRE_SWING9,
        },
    ),
    (
        "ogre_walk1",
        MonsterFrame {
            frame: 9,
            next: "ogre_walk2",
            operations: OPS_OGRE_WALK1,
        },
    ),
    (
        "ogre_walk10",
        MonsterFrame {
            frame: 18,
            next: "ogre_walk11",
            operations: OPS_OGRE_WALK10,
        },
    ),
    (
        "ogre_walk11",
        MonsterFrame {
            frame: 19,
            next: "ogre_walk12",
            operations: OPS_OGRE_WALK11,
        },
    ),
    (
        "ogre_walk12",
        MonsterFrame {
            frame: 20,
            next: "ogre_walk13",
            operations: OPS_OGRE_WALK12,
        },
    ),
    (
        "ogre_walk13",
        MonsterFrame {
            frame: 21,
            next: "ogre_walk14",
            operations: OPS_OGRE_WALK13,
        },
    ),
    (
        "ogre_walk14",
        MonsterFrame {
            frame: 22,
            next: "ogre_walk15",
            operations: OPS_OGRE_WALK14,
        },
    ),
    (
        "ogre_walk15",
        MonsterFrame {
            frame: 23,
            next: "ogre_walk16",
            operations: OPS_OGRE_WALK15,
        },
    ),
    (
        "ogre_walk16",
        MonsterFrame {
            frame: 24,
            next: "ogre_walk1",
            operations: OPS_OGRE_WALK16,
        },
    ),
    (
        "ogre_walk2",
        MonsterFrame {
            frame: 10,
            next: "ogre_walk3",
            operations: OPS_OGRE_WALK2,
        },
    ),
    (
        "ogre_walk3",
        MonsterFrame {
            frame: 11,
            next: "ogre_walk4",
            operations: OPS_OGRE_WALK3,
        },
    ),
    (
        "ogre_walk4",
        MonsterFrame {
            frame: 12,
            next: "ogre_walk5",
            operations: OPS_OGRE_WALK4,
        },
    ),
    (
        "ogre_walk5",
        MonsterFrame {
            frame: 13,
            next: "ogre_walk6",
            operations: OPS_OGRE_WALK5,
        },
    ),
    (
        "ogre_walk6",
        MonsterFrame {
            frame: 14,
            next: "ogre_walk7",
            operations: OPS_OGRE_WALK6,
        },
    ),
    (
        "ogre_walk7",
        MonsterFrame {
            frame: 15,
            next: "ogre_walk8",
            operations: OPS_OGRE_WALK7,
        },
    ),
    (
        "ogre_walk8",
        MonsterFrame {
            frame: 16,
            next: "ogre_walk9",
            operations: OPS_OGRE_WALK8,
        },
    ),
    (
        "ogre_walk9",
        MonsterFrame {
            frame: 17,
            next: "ogre_walk10",
            operations: OPS_OGRE_WALK9,
        },
    ),
    (
        "old_idle1",
        MonsterFrame {
            frame: 0,
            next: "old_idle2",
            operations: OPS_OLD_IDLE1,
        },
    ),
    (
        "old_idle10",
        MonsterFrame {
            frame: 9,
            next: "old_idle11",
            operations: OPS_OLD_IDLE10,
        },
    ),
    (
        "old_idle11",
        MonsterFrame {
            frame: 10,
            next: "old_idle12",
            operations: OPS_OLD_IDLE11,
        },
    ),
    (
        "old_idle12",
        MonsterFrame {
            frame: 11,
            next: "old_idle13",
            operations: OPS_OLD_IDLE12,
        },
    ),
    (
        "old_idle13",
        MonsterFrame {
            frame: 12,
            next: "old_idle14",
            operations: OPS_OLD_IDLE13,
        },
    ),
    (
        "old_idle14",
        MonsterFrame {
            frame: 13,
            next: "old_idle15",
            operations: OPS_OLD_IDLE14,
        },
    ),
    (
        "old_idle15",
        MonsterFrame {
            frame: 14,
            next: "old_idle16",
            operations: OPS_OLD_IDLE15,
        },
    ),
    (
        "old_idle16",
        MonsterFrame {
            frame: 15,
            next: "old_idle17",
            operations: OPS_OLD_IDLE16,
        },
    ),
    (
        "old_idle17",
        MonsterFrame {
            frame: 16,
            next: "old_idle18",
            operations: OPS_OLD_IDLE17,
        },
    ),
    (
        "old_idle18",
        MonsterFrame {
            frame: 17,
            next: "old_idle19",
            operations: OPS_OLD_IDLE18,
        },
    ),
    (
        "old_idle19",
        MonsterFrame {
            frame: 18,
            next: "old_idle20",
            operations: OPS_OLD_IDLE19,
        },
    ),
    (
        "old_idle2",
        MonsterFrame {
            frame: 1,
            next: "old_idle3",
            operations: OPS_OLD_IDLE2,
        },
    ),
    (
        "old_idle20",
        MonsterFrame {
            frame: 19,
            next: "old_idle21",
            operations: OPS_OLD_IDLE20,
        },
    ),
    (
        "old_idle21",
        MonsterFrame {
            frame: 20,
            next: "old_idle22",
            operations: OPS_OLD_IDLE21,
        },
    ),
    (
        "old_idle22",
        MonsterFrame {
            frame: 21,
            next: "old_idle23",
            operations: OPS_OLD_IDLE22,
        },
    ),
    (
        "old_idle23",
        MonsterFrame {
            frame: 22,
            next: "old_idle24",
            operations: OPS_OLD_IDLE23,
        },
    ),
    (
        "old_idle24",
        MonsterFrame {
            frame: 23,
            next: "old_idle25",
            operations: OPS_OLD_IDLE24,
        },
    ),
    (
        "old_idle25",
        MonsterFrame {
            frame: 24,
            next: "old_idle26",
            operations: OPS_OLD_IDLE25,
        },
    ),
    (
        "old_idle26",
        MonsterFrame {
            frame: 25,
            next: "old_idle27",
            operations: OPS_OLD_IDLE26,
        },
    ),
    (
        "old_idle27",
        MonsterFrame {
            frame: 26,
            next: "old_idle28",
            operations: OPS_OLD_IDLE27,
        },
    ),
    (
        "old_idle28",
        MonsterFrame {
            frame: 27,
            next: "old_idle29",
            operations: OPS_OLD_IDLE28,
        },
    ),
    (
        "old_idle29",
        MonsterFrame {
            frame: 28,
            next: "old_idle30",
            operations: OPS_OLD_IDLE29,
        },
    ),
    (
        "old_idle3",
        MonsterFrame {
            frame: 2,
            next: "old_idle4",
            operations: OPS_OLD_IDLE3,
        },
    ),
    (
        "old_idle30",
        MonsterFrame {
            frame: 29,
            next: "old_idle31",
            operations: OPS_OLD_IDLE30,
        },
    ),
    (
        "old_idle31",
        MonsterFrame {
            frame: 30,
            next: "old_idle32",
            operations: OPS_OLD_IDLE31,
        },
    ),
    (
        "old_idle32",
        MonsterFrame {
            frame: 31,
            next: "old_idle33",
            operations: OPS_OLD_IDLE32,
        },
    ),
    (
        "old_idle33",
        MonsterFrame {
            frame: 32,
            next: "old_idle34",
            operations: OPS_OLD_IDLE33,
        },
    ),
    (
        "old_idle34",
        MonsterFrame {
            frame: 33,
            next: "old_idle35",
            operations: OPS_OLD_IDLE34,
        },
    ),
    (
        "old_idle35",
        MonsterFrame {
            frame: 34,
            next: "old_idle36",
            operations: OPS_OLD_IDLE35,
        },
    ),
    (
        "old_idle36",
        MonsterFrame {
            frame: 35,
            next: "old_idle37",
            operations: OPS_OLD_IDLE36,
        },
    ),
    (
        "old_idle37",
        MonsterFrame {
            frame: 36,
            next: "old_idle38",
            operations: OPS_OLD_IDLE37,
        },
    ),
    (
        "old_idle38",
        MonsterFrame {
            frame: 37,
            next: "old_idle39",
            operations: OPS_OLD_IDLE38,
        },
    ),
    (
        "old_idle39",
        MonsterFrame {
            frame: 38,
            next: "old_idle40",
            operations: OPS_OLD_IDLE39,
        },
    ),
    (
        "old_idle4",
        MonsterFrame {
            frame: 3,
            next: "old_idle5",
            operations: OPS_OLD_IDLE4,
        },
    ),
    (
        "old_idle40",
        MonsterFrame {
            frame: 39,
            next: "old_idle41",
            operations: OPS_OLD_IDLE40,
        },
    ),
    (
        "old_idle41",
        MonsterFrame {
            frame: 40,
            next: "old_idle42",
            operations: OPS_OLD_IDLE41,
        },
    ),
    (
        "old_idle42",
        MonsterFrame {
            frame: 41,
            next: "old_idle43",
            operations: OPS_OLD_IDLE42,
        },
    ),
    (
        "old_idle43",
        MonsterFrame {
            frame: 42,
            next: "old_idle44",
            operations: OPS_OLD_IDLE43,
        },
    ),
    (
        "old_idle44",
        MonsterFrame {
            frame: 43,
            next: "old_idle45",
            operations: OPS_OLD_IDLE44,
        },
    ),
    (
        "old_idle45",
        MonsterFrame {
            frame: 44,
            next: "old_idle46",
            operations: OPS_OLD_IDLE45,
        },
    ),
    (
        "old_idle46",
        MonsterFrame {
            frame: 45,
            next: "old_idle1",
            operations: OPS_OLD_IDLE46,
        },
    ),
    (
        "old_idle5",
        MonsterFrame {
            frame: 4,
            next: "old_idle6",
            operations: OPS_OLD_IDLE5,
        },
    ),
    (
        "old_idle6",
        MonsterFrame {
            frame: 5,
            next: "old_idle7",
            operations: OPS_OLD_IDLE6,
        },
    ),
    (
        "old_idle7",
        MonsterFrame {
            frame: 6,
            next: "old_idle8",
            operations: OPS_OLD_IDLE7,
        },
    ),
    (
        "old_idle8",
        MonsterFrame {
            frame: 7,
            next: "old_idle9",
            operations: OPS_OLD_IDLE8,
        },
    ),
    (
        "old_idle9",
        MonsterFrame {
            frame: 8,
            next: "old_idle10",
            operations: OPS_OLD_IDLE9,
        },
    ),
    (
        "old_thrash1",
        MonsterFrame {
            frame: 46,
            next: "old_thrash2",
            operations: OPS_OLD_THRASH1,
        },
    ),
    (
        "old_thrash10",
        MonsterFrame {
            frame: 55,
            next: "old_thrash11",
            operations: OPS_OLD_THRASH10,
        },
    ),
    (
        "old_thrash11",
        MonsterFrame {
            frame: 56,
            next: "old_thrash12",
            operations: OPS_OLD_THRASH11,
        },
    ),
    (
        "old_thrash12",
        MonsterFrame {
            frame: 57,
            next: "old_thrash13",
            operations: OPS_OLD_THRASH12,
        },
    ),
    (
        "old_thrash13",
        MonsterFrame {
            frame: 58,
            next: "old_thrash14",
            operations: OPS_OLD_THRASH13,
        },
    ),
    (
        "old_thrash14",
        MonsterFrame {
            frame: 59,
            next: "old_thrash15",
            operations: OPS_OLD_THRASH14,
        },
    ),
    (
        "old_thrash15",
        MonsterFrame {
            frame: 60,
            next: "old_thrash16",
            operations: OPS_OLD_THRASH15,
        },
    ),
    (
        "old_thrash16",
        MonsterFrame {
            frame: 61,
            next: "old_thrash17",
            operations: OPS_OLD_THRASH16,
        },
    ),
    (
        "old_thrash17",
        MonsterFrame {
            frame: 62,
            next: "old_thrash18",
            operations: OPS_OLD_THRASH17,
        },
    ),
    (
        "old_thrash18",
        MonsterFrame {
            frame: 63,
            next: "old_thrash19",
            operations: OPS_OLD_THRASH18,
        },
    ),
    (
        "old_thrash19",
        MonsterFrame {
            frame: 64,
            next: "old_thrash20",
            operations: OPS_OLD_THRASH19,
        },
    ),
    (
        "old_thrash2",
        MonsterFrame {
            frame: 47,
            next: "old_thrash3",
            operations: OPS_OLD_THRASH2,
        },
    ),
    (
        "old_thrash20",
        MonsterFrame {
            frame: 65,
            next: "old_thrash20",
            operations: OPS_OLD_THRASH20,
        },
    ),
    (
        "old_thrash3",
        MonsterFrame {
            frame: 48,
            next: "old_thrash4",
            operations: OPS_OLD_THRASH3,
        },
    ),
    (
        "old_thrash4",
        MonsterFrame {
            frame: 49,
            next: "old_thrash5",
            operations: OPS_OLD_THRASH4,
        },
    ),
    (
        "old_thrash5",
        MonsterFrame {
            frame: 50,
            next: "old_thrash6",
            operations: OPS_OLD_THRASH5,
        },
    ),
    (
        "old_thrash6",
        MonsterFrame {
            frame: 51,
            next: "old_thrash7",
            operations: OPS_OLD_THRASH6,
        },
    ),
    (
        "old_thrash7",
        MonsterFrame {
            frame: 52,
            next: "old_thrash8",
            operations: OPS_OLD_THRASH7,
        },
    ),
    (
        "old_thrash8",
        MonsterFrame {
            frame: 53,
            next: "old_thrash9",
            operations: OPS_OLD_THRASH8,
        },
    ),
    (
        "old_thrash9",
        MonsterFrame {
            frame: 54,
            next: "old_thrash10",
            operations: OPS_OLD_THRASH9,
        },
    ),
    (
        "shal_attack1",
        MonsterFrame {
            frame: 0,
            next: "shal_attack2",
            operations: OPS_SHAL_ATTACK1,
        },
    ),
    (
        "shal_attack10",
        MonsterFrame {
            frame: 9,
            next: "shal_attack11",
            operations: OPS_SHAL_ATTACK10,
        },
    ),
    (
        "shal_attack11",
        MonsterFrame {
            frame: 10,
            next: "shal_run1",
            operations: OPS_SHAL_ATTACK11,
        },
    ),
    (
        "shal_attack2",
        MonsterFrame {
            frame: 1,
            next: "shal_attack3",
            operations: OPS_SHAL_ATTACK2,
        },
    ),
    (
        "shal_attack3",
        MonsterFrame {
            frame: 2,
            next: "shal_attack4",
            operations: OPS_SHAL_ATTACK3,
        },
    ),
    (
        "shal_attack4",
        MonsterFrame {
            frame: 3,
            next: "shal_attack5",
            operations: OPS_SHAL_ATTACK4,
        },
    ),
    (
        "shal_attack5",
        MonsterFrame {
            frame: 4,
            next: "shal_attack6",
            operations: OPS_SHAL_ATTACK5,
        },
    ),
    (
        "shal_attack6",
        MonsterFrame {
            frame: 5,
            next: "shal_attack7",
            operations: OPS_SHAL_ATTACK6,
        },
    ),
    (
        "shal_attack7",
        MonsterFrame {
            frame: 6,
            next: "shal_attack8",
            operations: OPS_SHAL_ATTACK7,
        },
    ),
    (
        "shal_attack8",
        MonsterFrame {
            frame: 7,
            next: "shal_attack9",
            operations: OPS_SHAL_ATTACK8,
        },
    ),
    (
        "shal_attack9",
        MonsterFrame {
            frame: 8,
            next: "shal_attack10",
            operations: OPS_SHAL_ATTACK9,
        },
    ),
    (
        "shal_death1",
        MonsterFrame {
            frame: 16,
            next: "shal_death2",
            operations: OPS_SHAL_DEATH1,
        },
    ),
    (
        "shal_death2",
        MonsterFrame {
            frame: 17,
            next: "shal_death3",
            operations: OPS_SHAL_DEATH2,
        },
    ),
    (
        "shal_death3",
        MonsterFrame {
            frame: 18,
            next: "shal_death4",
            operations: OPS_SHAL_DEATH3,
        },
    ),
    (
        "shal_death4",
        MonsterFrame {
            frame: 19,
            next: "shal_death5",
            operations: OPS_SHAL_DEATH4,
        },
    ),
    (
        "shal_death5",
        MonsterFrame {
            frame: 20,
            next: "shal_death6",
            operations: OPS_SHAL_DEATH5,
        },
    ),
    (
        "shal_death6",
        MonsterFrame {
            frame: 21,
            next: "shal_death7",
            operations: OPS_SHAL_DEATH6,
        },
    ),
    (
        "shal_death7",
        MonsterFrame {
            frame: 22,
            next: "shal_death7",
            operations: OPS_SHAL_DEATH7,
        },
    ),
    (
        "shal_pain1",
        MonsterFrame {
            frame: 11,
            next: "shal_pain2",
            operations: OPS_SHAL_PAIN1,
        },
    ),
    (
        "shal_pain2",
        MonsterFrame {
            frame: 12,
            next: "shal_pain3",
            operations: OPS_SHAL_PAIN2,
        },
    ),
    (
        "shal_pain3",
        MonsterFrame {
            frame: 13,
            next: "shal_pain4",
            operations: OPS_SHAL_PAIN3,
        },
    ),
    (
        "shal_pain4",
        MonsterFrame {
            frame: 14,
            next: "shal_pain5",
            operations: OPS_SHAL_PAIN4,
        },
    ),
    (
        "shal_pain5",
        MonsterFrame {
            frame: 15,
            next: "shal_run1",
            operations: OPS_SHAL_PAIN5,
        },
    ),
    (
        "shal_run1",
        MonsterFrame {
            frame: 24,
            next: "shal_run2",
            operations: OPS_SHAL_RUN1,
        },
    ),
    (
        "shal_run10",
        MonsterFrame {
            frame: 33,
            next: "shal_run11",
            operations: OPS_SHAL_RUN10,
        },
    ),
    (
        "shal_run11",
        MonsterFrame {
            frame: 34,
            next: "shal_run12",
            operations: OPS_SHAL_RUN11,
        },
    ),
    (
        "shal_run12",
        MonsterFrame {
            frame: 23,
            next: "shal_run1",
            operations: OPS_SHAL_RUN12,
        },
    ),
    (
        "shal_run2",
        MonsterFrame {
            frame: 25,
            next: "shal_run3",
            operations: OPS_SHAL_RUN2,
        },
    ),
    (
        "shal_run3",
        MonsterFrame {
            frame: 26,
            next: "shal_run4",
            operations: OPS_SHAL_RUN3,
        },
    ),
    (
        "shal_run4",
        MonsterFrame {
            frame: 27,
            next: "shal_run5",
            operations: OPS_SHAL_RUN4,
        },
    ),
    (
        "shal_run5",
        MonsterFrame {
            frame: 28,
            next: "shal_run6",
            operations: OPS_SHAL_RUN5,
        },
    ),
    (
        "shal_run6",
        MonsterFrame {
            frame: 29,
            next: "shal_run7",
            operations: OPS_SHAL_RUN6,
        },
    ),
    (
        "shal_run7",
        MonsterFrame {
            frame: 30,
            next: "shal_run8",
            operations: OPS_SHAL_RUN7,
        },
    ),
    (
        "shal_run8",
        MonsterFrame {
            frame: 31,
            next: "shal_run9",
            operations: OPS_SHAL_RUN8,
        },
    ),
    (
        "shal_run9",
        MonsterFrame {
            frame: 32,
            next: "shal_run10",
            operations: OPS_SHAL_RUN9,
        },
    ),
    (
        "shal_stand",
        MonsterFrame {
            frame: 23,
            next: "shal_stand",
            operations: OPS_SHAL_STAND,
        },
    ),
    (
        "shal_walk1",
        MonsterFrame {
            frame: 24,
            next: "shal_walk2",
            operations: OPS_SHAL_WALK1,
        },
    ),
    (
        "shal_walk10",
        MonsterFrame {
            frame: 33,
            next: "shal_walk11",
            operations: OPS_SHAL_WALK10,
        },
    ),
    (
        "shal_walk11",
        MonsterFrame {
            frame: 34,
            next: "shal_walk12",
            operations: OPS_SHAL_WALK11,
        },
    ),
    (
        "shal_walk12",
        MonsterFrame {
            frame: 23,
            next: "shal_walk1",
            operations: OPS_SHAL_WALK12,
        },
    ),
    (
        "shal_walk2",
        MonsterFrame {
            frame: 25,
            next: "shal_walk3",
            operations: OPS_SHAL_WALK2,
        },
    ),
    (
        "shal_walk3",
        MonsterFrame {
            frame: 26,
            next: "shal_walk4",
            operations: OPS_SHAL_WALK3,
        },
    ),
    (
        "shal_walk4",
        MonsterFrame {
            frame: 27,
            next: "shal_walk5",
            operations: OPS_SHAL_WALK4,
        },
    ),
    (
        "shal_walk5",
        MonsterFrame {
            frame: 28,
            next: "shal_walk6",
            operations: OPS_SHAL_WALK5,
        },
    ),
    (
        "shal_walk6",
        MonsterFrame {
            frame: 29,
            next: "shal_walk7",
            operations: OPS_SHAL_WALK6,
        },
    ),
    (
        "shal_walk7",
        MonsterFrame {
            frame: 30,
            next: "shal_walk8",
            operations: OPS_SHAL_WALK7,
        },
    ),
    (
        "shal_walk8",
        MonsterFrame {
            frame: 31,
            next: "shal_walk9",
            operations: OPS_SHAL_WALK8,
        },
    ),
    (
        "shal_walk9",
        MonsterFrame {
            frame: 32,
            next: "shal_walk10",
            operations: OPS_SHAL_WALK9,
        },
    ),
    (
        "sham_death1",
        MonsterFrame {
            frame: 83,
            next: "sham_death2",
            operations: OPS_SHAM_DEATH1,
        },
    ),
    (
        "sham_death10",
        MonsterFrame {
            frame: 92,
            next: "sham_death11",
            operations: OPS_SHAM_DEATH10,
        },
    ),
    (
        "sham_death11",
        MonsterFrame {
            frame: 93,
            next: "sham_death11",
            operations: OPS_SHAM_DEATH11,
        },
    ),
    (
        "sham_death2",
        MonsterFrame {
            frame: 84,
            next: "sham_death3",
            operations: OPS_SHAM_DEATH2,
        },
    ),
    (
        "sham_death3",
        MonsterFrame {
            frame: 85,
            next: "sham_death4",
            operations: OPS_SHAM_DEATH3,
        },
    ),
    (
        "sham_death4",
        MonsterFrame {
            frame: 86,
            next: "sham_death5",
            operations: OPS_SHAM_DEATH4,
        },
    ),
    (
        "sham_death5",
        MonsterFrame {
            frame: 87,
            next: "sham_death6",
            operations: OPS_SHAM_DEATH5,
        },
    ),
    (
        "sham_death6",
        MonsterFrame {
            frame: 88,
            next: "sham_death7",
            operations: OPS_SHAM_DEATH6,
        },
    ),
    (
        "sham_death7",
        MonsterFrame {
            frame: 89,
            next: "sham_death8",
            operations: OPS_SHAM_DEATH7,
        },
    ),
    (
        "sham_death8",
        MonsterFrame {
            frame: 90,
            next: "sham_death9",
            operations: OPS_SHAM_DEATH8,
        },
    ),
    (
        "sham_death9",
        MonsterFrame {
            frame: 91,
            next: "sham_death10",
            operations: OPS_SHAM_DEATH9,
        },
    ),
    (
        "sham_magic1",
        MonsterFrame {
            frame: 65,
            next: "sham_magic2",
            operations: OPS_SHAM_MAGIC1,
        },
    ),
    (
        "sham_magic10",
        MonsterFrame {
            frame: 74,
            next: "sham_magic11",
            operations: OPS_SHAM_MAGIC10,
        },
    ),
    (
        "sham_magic11",
        MonsterFrame {
            frame: 75,
            next: "sham_magic12",
            operations: OPS_SHAM_MAGIC11,
        },
    ),
    (
        "sham_magic12",
        MonsterFrame {
            frame: 76,
            next: "sham_run1",
            operations: OPS_SHAM_MAGIC12,
        },
    ),
    (
        "sham_magic2",
        MonsterFrame {
            frame: 66,
            next: "sham_magic3",
            operations: OPS_SHAM_MAGIC2,
        },
    ),
    (
        "sham_magic3",
        MonsterFrame {
            frame: 67,
            next: "sham_magic4",
            operations: OPS_SHAM_MAGIC3,
        },
    ),
    (
        "sham_magic4",
        MonsterFrame {
            frame: 68,
            next: "sham_magic5",
            operations: OPS_SHAM_MAGIC4,
        },
    ),
    (
        "sham_magic5",
        MonsterFrame {
            frame: 69,
            next: "sham_magic6",
            operations: OPS_SHAM_MAGIC5,
        },
    ),
    (
        "sham_magic6",
        MonsterFrame {
            frame: 70,
            next: "sham_magic9",
            operations: OPS_SHAM_MAGIC6,
        },
    ),
    (
        "sham_magic9",
        MonsterFrame {
            frame: 73,
            next: "sham_magic10",
            operations: OPS_SHAM_MAGIC9,
        },
    ),
    (
        "sham_pain1",
        MonsterFrame {
            frame: 77,
            next: "sham_pain2",
            operations: OPS_SHAM_PAIN1,
        },
    ),
    (
        "sham_pain2",
        MonsterFrame {
            frame: 78,
            next: "sham_pain3",
            operations: OPS_SHAM_PAIN2,
        },
    ),
    (
        "sham_pain3",
        MonsterFrame {
            frame: 79,
            next: "sham_pain4",
            operations: OPS_SHAM_PAIN3,
        },
    ),
    (
        "sham_pain4",
        MonsterFrame {
            frame: 80,
            next: "sham_pain5",
            operations: OPS_SHAM_PAIN4,
        },
    ),
    (
        "sham_pain5",
        MonsterFrame {
            frame: 81,
            next: "sham_pain6",
            operations: OPS_SHAM_PAIN5,
        },
    ),
    (
        "sham_pain6",
        MonsterFrame {
            frame: 82,
            next: "sham_run1",
            operations: OPS_SHAM_PAIN6,
        },
    ),
    (
        "sham_run1",
        MonsterFrame {
            frame: 29,
            next: "sham_run2",
            operations: OPS_SHAM_RUN1,
        },
    ),
    (
        "sham_run2",
        MonsterFrame {
            frame: 30,
            next: "sham_run3",
            operations: OPS_SHAM_RUN2,
        },
    ),
    (
        "sham_run3",
        MonsterFrame {
            frame: 31,
            next: "sham_run4",
            operations: OPS_SHAM_RUN3,
        },
    ),
    (
        "sham_run4",
        MonsterFrame {
            frame: 32,
            next: "sham_run5",
            operations: OPS_SHAM_RUN4,
        },
    ),
    (
        "sham_run5",
        MonsterFrame {
            frame: 33,
            next: "sham_run6",
            operations: OPS_SHAM_RUN5,
        },
    ),
    (
        "sham_run6",
        MonsterFrame {
            frame: 34,
            next: "sham_run1",
            operations: OPS_SHAM_RUN6,
        },
    ),
    (
        "sham_smash1",
        MonsterFrame {
            frame: 35,
            next: "sham_smash2",
            operations: OPS_SHAM_SMASH1,
        },
    ),
    (
        "sham_smash10",
        MonsterFrame {
            frame: 44,
            next: "sham_smash11",
            operations: OPS_SHAM_SMASH10,
        },
    ),
    (
        "sham_smash11",
        MonsterFrame {
            frame: 45,
            next: "sham_smash12",
            operations: OPS_SHAM_SMASH11,
        },
    ),
    (
        "sham_smash12",
        MonsterFrame {
            frame: 46,
            next: "sham_run1",
            operations: OPS_SHAM_SMASH12,
        },
    ),
    (
        "sham_smash2",
        MonsterFrame {
            frame: 36,
            next: "sham_smash3",
            operations: OPS_SHAM_SMASH2,
        },
    ),
    (
        "sham_smash3",
        MonsterFrame {
            frame: 37,
            next: "sham_smash4",
            operations: OPS_SHAM_SMASH3,
        },
    ),
    (
        "sham_smash4",
        MonsterFrame {
            frame: 38,
            next: "sham_smash5",
            operations: OPS_SHAM_SMASH4,
        },
    ),
    (
        "sham_smash5",
        MonsterFrame {
            frame: 39,
            next: "sham_smash6",
            operations: OPS_SHAM_SMASH5,
        },
    ),
    (
        "sham_smash6",
        MonsterFrame {
            frame: 40,
            next: "sham_smash7",
            operations: OPS_SHAM_SMASH6,
        },
    ),
    (
        "sham_smash7",
        MonsterFrame {
            frame: 41,
            next: "sham_smash8",
            operations: OPS_SHAM_SMASH7,
        },
    ),
    (
        "sham_smash8",
        MonsterFrame {
            frame: 42,
            next: "sham_smash9",
            operations: OPS_SHAM_SMASH8,
        },
    ),
    (
        "sham_smash9",
        MonsterFrame {
            frame: 43,
            next: "sham_smash10",
            operations: OPS_SHAM_SMASH9,
        },
    ),
    (
        "sham_stand1",
        MonsterFrame {
            frame: 0,
            next: "sham_stand2",
            operations: OPS_SHAM_STAND1,
        },
    ),
    (
        "sham_stand10",
        MonsterFrame {
            frame: 9,
            next: "sham_stand11",
            operations: OPS_SHAM_STAND10,
        },
    ),
    (
        "sham_stand11",
        MonsterFrame {
            frame: 10,
            next: "sham_stand12",
            operations: OPS_SHAM_STAND11,
        },
    ),
    (
        "sham_stand12",
        MonsterFrame {
            frame: 11,
            next: "sham_stand13",
            operations: OPS_SHAM_STAND12,
        },
    ),
    (
        "sham_stand13",
        MonsterFrame {
            frame: 12,
            next: "sham_stand14",
            operations: OPS_SHAM_STAND13,
        },
    ),
    (
        "sham_stand14",
        MonsterFrame {
            frame: 13,
            next: "sham_stand15",
            operations: OPS_SHAM_STAND14,
        },
    ),
    (
        "sham_stand15",
        MonsterFrame {
            frame: 14,
            next: "sham_stand16",
            operations: OPS_SHAM_STAND15,
        },
    ),
    (
        "sham_stand16",
        MonsterFrame {
            frame: 15,
            next: "sham_stand17",
            operations: OPS_SHAM_STAND16,
        },
    ),
    (
        "sham_stand17",
        MonsterFrame {
            frame: 16,
            next: "sham_stand1",
            operations: OPS_SHAM_STAND17,
        },
    ),
    (
        "sham_stand2",
        MonsterFrame {
            frame: 1,
            next: "sham_stand3",
            operations: OPS_SHAM_STAND2,
        },
    ),
    (
        "sham_stand3",
        MonsterFrame {
            frame: 2,
            next: "sham_stand4",
            operations: OPS_SHAM_STAND3,
        },
    ),
    (
        "sham_stand4",
        MonsterFrame {
            frame: 3,
            next: "sham_stand5",
            operations: OPS_SHAM_STAND4,
        },
    ),
    (
        "sham_stand5",
        MonsterFrame {
            frame: 4,
            next: "sham_stand6",
            operations: OPS_SHAM_STAND5,
        },
    ),
    (
        "sham_stand6",
        MonsterFrame {
            frame: 5,
            next: "sham_stand7",
            operations: OPS_SHAM_STAND6,
        },
    ),
    (
        "sham_stand7",
        MonsterFrame {
            frame: 6,
            next: "sham_stand8",
            operations: OPS_SHAM_STAND7,
        },
    ),
    (
        "sham_stand8",
        MonsterFrame {
            frame: 7,
            next: "sham_stand9",
            operations: OPS_SHAM_STAND8,
        },
    ),
    (
        "sham_stand9",
        MonsterFrame {
            frame: 8,
            next: "sham_stand10",
            operations: OPS_SHAM_STAND9,
        },
    ),
    (
        "sham_swingl1",
        MonsterFrame {
            frame: 56,
            next: "sham_swingl2",
            operations: OPS_SHAM_SWINGL1,
        },
    ),
    (
        "sham_swingl2",
        MonsterFrame {
            frame: 57,
            next: "sham_swingl3",
            operations: OPS_SHAM_SWINGL2,
        },
    ),
    (
        "sham_swingl3",
        MonsterFrame {
            frame: 58,
            next: "sham_swingl4",
            operations: OPS_SHAM_SWINGL3,
        },
    ),
    (
        "sham_swingl4",
        MonsterFrame {
            frame: 59,
            next: "sham_swingl5",
            operations: OPS_SHAM_SWINGL4,
        },
    ),
    (
        "sham_swingl5",
        MonsterFrame {
            frame: 60,
            next: "sham_swingl6",
            operations: OPS_SHAM_SWINGL5,
        },
    ),
    (
        "sham_swingl6",
        MonsterFrame {
            frame: 61,
            next: "sham_swingl7",
            operations: OPS_SHAM_SWINGL6,
        },
    ),
    (
        "sham_swingl7",
        MonsterFrame {
            frame: 62,
            next: "sham_swingl8",
            operations: OPS_SHAM_SWINGL7,
        },
    ),
    (
        "sham_swingl8",
        MonsterFrame {
            frame: 63,
            next: "sham_swingl9",
            operations: OPS_SHAM_SWINGL8,
        },
    ),
    (
        "sham_swingl9",
        MonsterFrame {
            frame: 64,
            next: "sham_run1",
            operations: OPS_SHAM_SWINGL9,
        },
    ),
    (
        "sham_swingr1",
        MonsterFrame {
            frame: 47,
            next: "sham_swingr2",
            operations: OPS_SHAM_SWINGR1,
        },
    ),
    (
        "sham_swingr2",
        MonsterFrame {
            frame: 48,
            next: "sham_swingr3",
            operations: OPS_SHAM_SWINGR2,
        },
    ),
    (
        "sham_swingr3",
        MonsterFrame {
            frame: 49,
            next: "sham_swingr4",
            operations: OPS_SHAM_SWINGR3,
        },
    ),
    (
        "sham_swingr4",
        MonsterFrame {
            frame: 50,
            next: "sham_swingr5",
            operations: OPS_SHAM_SWINGR4,
        },
    ),
    (
        "sham_swingr5",
        MonsterFrame {
            frame: 51,
            next: "sham_swingr6",
            operations: OPS_SHAM_SWINGR5,
        },
    ),
    (
        "sham_swingr6",
        MonsterFrame {
            frame: 52,
            next: "sham_swingr7",
            operations: OPS_SHAM_SWINGR6,
        },
    ),
    (
        "sham_swingr7",
        MonsterFrame {
            frame: 53,
            next: "sham_swingr8",
            operations: OPS_SHAM_SWINGR7,
        },
    ),
    (
        "sham_swingr8",
        MonsterFrame {
            frame: 54,
            next: "sham_swingr9",
            operations: OPS_SHAM_SWINGR8,
        },
    ),
    (
        "sham_swingr9",
        MonsterFrame {
            frame: 55,
            next: "sham_run1",
            operations: OPS_SHAM_SWINGR9,
        },
    ),
    (
        "sham_walk1",
        MonsterFrame {
            frame: 17,
            next: "sham_walk2",
            operations: OPS_SHAM_WALK1,
        },
    ),
    (
        "sham_walk10",
        MonsterFrame {
            frame: 26,
            next: "sham_walk11",
            operations: OPS_SHAM_WALK10,
        },
    ),
    (
        "sham_walk11",
        MonsterFrame {
            frame: 27,
            next: "sham_walk12",
            operations: OPS_SHAM_WALK11,
        },
    ),
    (
        "sham_walk12",
        MonsterFrame {
            frame: 28,
            next: "sham_walk1",
            operations: OPS_SHAM_WALK12,
        },
    ),
    (
        "sham_walk2",
        MonsterFrame {
            frame: 18,
            next: "sham_walk3",
            operations: OPS_SHAM_WALK2,
        },
    ),
    (
        "sham_walk3",
        MonsterFrame {
            frame: 19,
            next: "sham_walk4",
            operations: OPS_SHAM_WALK3,
        },
    ),
    (
        "sham_walk4",
        MonsterFrame {
            frame: 20,
            next: "sham_walk5",
            operations: OPS_SHAM_WALK4,
        },
    ),
    (
        "sham_walk5",
        MonsterFrame {
            frame: 21,
            next: "sham_walk6",
            operations: OPS_SHAM_WALK5,
        },
    ),
    (
        "sham_walk6",
        MonsterFrame {
            frame: 22,
            next: "sham_walk7",
            operations: OPS_SHAM_WALK6,
        },
    ),
    (
        "sham_walk7",
        MonsterFrame {
            frame: 23,
            next: "sham_walk8",
            operations: OPS_SHAM_WALK7,
        },
    ),
    (
        "sham_walk8",
        MonsterFrame {
            frame: 24,
            next: "sham_walk9",
            operations: OPS_SHAM_WALK8,
        },
    ),
    (
        "sham_walk9",
        MonsterFrame {
            frame: 25,
            next: "sham_walk10",
            operations: OPS_SHAM_WALK9,
        },
    ),
    (
        "tbaby_die1",
        MonsterFrame {
            frame: 60,
            next: "tbaby_die2",
            operations: OPS_TBABY_DIE1,
        },
    ),
    (
        "tbaby_die2",
        MonsterFrame {
            frame: 60,
            next: "tbaby_run1",
            operations: OPS_TBABY_DIE2,
        },
    ),
    (
        "tbaby_fly1",
        MonsterFrame {
            frame: 56,
            next: "tbaby_fly2",
            operations: OPS_TBABY_FLY1,
        },
    ),
    (
        "tbaby_fly2",
        MonsterFrame {
            frame: 57,
            next: "tbaby_fly3",
            operations: OPS_TBABY_FLY2,
        },
    ),
    (
        "tbaby_fly3",
        MonsterFrame {
            frame: 58,
            next: "tbaby_fly4",
            operations: OPS_TBABY_FLY3,
        },
    ),
    (
        "tbaby_fly4",
        MonsterFrame {
            frame: 59,
            next: "tbaby_fly1",
            operations: OPS_TBABY_FLY4,
        },
    ),
    (
        "tbaby_hang1",
        MonsterFrame {
            frame: 0,
            next: "tbaby_hang1",
            operations: OPS_TBABY_HANG1,
        },
    ),
    (
        "tbaby_jump1",
        MonsterFrame {
            frame: 50,
            next: "tbaby_jump2",
            operations: OPS_TBABY_JUMP1,
        },
    ),
    (
        "tbaby_jump2",
        MonsterFrame {
            frame: 51,
            next: "tbaby_jump3",
            operations: OPS_TBABY_JUMP2,
        },
    ),
    (
        "tbaby_jump3",
        MonsterFrame {
            frame: 52,
            next: "tbaby_jump4",
            operations: OPS_TBABY_JUMP3,
        },
    ),
    (
        "tbaby_jump4",
        MonsterFrame {
            frame: 53,
            next: "tbaby_jump5",
            operations: OPS_TBABY_JUMP4,
        },
    ),
    (
        "tbaby_jump5",
        MonsterFrame {
            frame: 54,
            next: "tbaby_jump6",
            operations: OPS_TBABY_JUMP5,
        },
    ),
    (
        "tbaby_jump6",
        MonsterFrame {
            frame: 55,
            next: "tbaby_fly1",
            operations: OPS_TBABY_JUMP6,
        },
    ),
    (
        "tbaby_run1",
        MonsterFrame {
            frame: 25,
            next: "tbaby_run2",
            operations: OPS_TBABY_RUN1,
        },
    ),
    (
        "tbaby_run10",
        MonsterFrame {
            frame: 34,
            next: "tbaby_run11",
            operations: OPS_TBABY_RUN10,
        },
    ),
    (
        "tbaby_run11",
        MonsterFrame {
            frame: 35,
            next: "tbaby_run12",
            operations: OPS_TBABY_RUN11,
        },
    ),
    (
        "tbaby_run12",
        MonsterFrame {
            frame: 36,
            next: "tbaby_run13",
            operations: OPS_TBABY_RUN12,
        },
    ),
    (
        "tbaby_run13",
        MonsterFrame {
            frame: 37,
            next: "tbaby_run14",
            operations: OPS_TBABY_RUN13,
        },
    ),
    (
        "tbaby_run14",
        MonsterFrame {
            frame: 38,
            next: "tbaby_run15",
            operations: OPS_TBABY_RUN14,
        },
    ),
    (
        "tbaby_run15",
        MonsterFrame {
            frame: 39,
            next: "tbaby_run16",
            operations: OPS_TBABY_RUN15,
        },
    ),
    (
        "tbaby_run16",
        MonsterFrame {
            frame: 40,
            next: "tbaby_run17",
            operations: OPS_TBABY_RUN16,
        },
    ),
    (
        "tbaby_run17",
        MonsterFrame {
            frame: 41,
            next: "tbaby_run18",
            operations: OPS_TBABY_RUN17,
        },
    ),
    (
        "tbaby_run18",
        MonsterFrame {
            frame: 42,
            next: "tbaby_run19",
            operations: OPS_TBABY_RUN18,
        },
    ),
    (
        "tbaby_run19",
        MonsterFrame {
            frame: 43,
            next: "tbaby_run20",
            operations: OPS_TBABY_RUN19,
        },
    ),
    (
        "tbaby_run2",
        MonsterFrame {
            frame: 26,
            next: "tbaby_run3",
            operations: OPS_TBABY_RUN2,
        },
    ),
    (
        "tbaby_run20",
        MonsterFrame {
            frame: 44,
            next: "tbaby_run21",
            operations: OPS_TBABY_RUN20,
        },
    ),
    (
        "tbaby_run21",
        MonsterFrame {
            frame: 45,
            next: "tbaby_run22",
            operations: OPS_TBABY_RUN21,
        },
    ),
    (
        "tbaby_run22",
        MonsterFrame {
            frame: 46,
            next: "tbaby_run23",
            operations: OPS_TBABY_RUN22,
        },
    ),
    (
        "tbaby_run23",
        MonsterFrame {
            frame: 47,
            next: "tbaby_run24",
            operations: OPS_TBABY_RUN23,
        },
    ),
    (
        "tbaby_run24",
        MonsterFrame {
            frame: 48,
            next: "tbaby_run25",
            operations: OPS_TBABY_RUN24,
        },
    ),
    (
        "tbaby_run25",
        MonsterFrame {
            frame: 49,
            next: "tbaby_run1",
            operations: OPS_TBABY_RUN25,
        },
    ),
    (
        "tbaby_run3",
        MonsterFrame {
            frame: 27,
            next: "tbaby_run4",
            operations: OPS_TBABY_RUN3,
        },
    ),
    (
        "tbaby_run4",
        MonsterFrame {
            frame: 28,
            next: "tbaby_run5",
            operations: OPS_TBABY_RUN4,
        },
    ),
    (
        "tbaby_run5",
        MonsterFrame {
            frame: 29,
            next: "tbaby_run6",
            operations: OPS_TBABY_RUN5,
        },
    ),
    (
        "tbaby_run6",
        MonsterFrame {
            frame: 30,
            next: "tbaby_run7",
            operations: OPS_TBABY_RUN6,
        },
    ),
    (
        "tbaby_run7",
        MonsterFrame {
            frame: 31,
            next: "tbaby_run8",
            operations: OPS_TBABY_RUN7,
        },
    ),
    (
        "tbaby_run8",
        MonsterFrame {
            frame: 32,
            next: "tbaby_run9",
            operations: OPS_TBABY_RUN8,
        },
    ),
    (
        "tbaby_run9",
        MonsterFrame {
            frame: 33,
            next: "tbaby_run10",
            operations: OPS_TBABY_RUN9,
        },
    ),
    (
        "tbaby_stand1",
        MonsterFrame {
            frame: 0,
            next: "tbaby_stand1",
            operations: OPS_TBABY_STAND1,
        },
    ),
    (
        "tbaby_walk1",
        MonsterFrame {
            frame: 0,
            next: "tbaby_walk2",
            operations: OPS_TBABY_WALK1,
        },
    ),
    (
        "tbaby_walk10",
        MonsterFrame {
            frame: 9,
            next: "tbaby_walk11",
            operations: OPS_TBABY_WALK10,
        },
    ),
    (
        "tbaby_walk11",
        MonsterFrame {
            frame: 10,
            next: "tbaby_walk12",
            operations: OPS_TBABY_WALK11,
        },
    ),
    (
        "tbaby_walk12",
        MonsterFrame {
            frame: 11,
            next: "tbaby_walk13",
            operations: OPS_TBABY_WALK12,
        },
    ),
    (
        "tbaby_walk13",
        MonsterFrame {
            frame: 12,
            next: "tbaby_walk14",
            operations: OPS_TBABY_WALK13,
        },
    ),
    (
        "tbaby_walk14",
        MonsterFrame {
            frame: 13,
            next: "tbaby_walk15",
            operations: OPS_TBABY_WALK14,
        },
    ),
    (
        "tbaby_walk15",
        MonsterFrame {
            frame: 14,
            next: "tbaby_walk16",
            operations: OPS_TBABY_WALK15,
        },
    ),
    (
        "tbaby_walk16",
        MonsterFrame {
            frame: 15,
            next: "tbaby_walk17",
            operations: OPS_TBABY_WALK16,
        },
    ),
    (
        "tbaby_walk17",
        MonsterFrame {
            frame: 16,
            next: "tbaby_walk18",
            operations: OPS_TBABY_WALK17,
        },
    ),
    (
        "tbaby_walk18",
        MonsterFrame {
            frame: 17,
            next: "tbaby_walk19",
            operations: OPS_TBABY_WALK18,
        },
    ),
    (
        "tbaby_walk19",
        MonsterFrame {
            frame: 18,
            next: "tbaby_walk20",
            operations: OPS_TBABY_WALK19,
        },
    ),
    (
        "tbaby_walk2",
        MonsterFrame {
            frame: 1,
            next: "tbaby_walk3",
            operations: OPS_TBABY_WALK2,
        },
    ),
    (
        "tbaby_walk20",
        MonsterFrame {
            frame: 19,
            next: "tbaby_walk21",
            operations: OPS_TBABY_WALK20,
        },
    ),
    (
        "tbaby_walk21",
        MonsterFrame {
            frame: 20,
            next: "tbaby_walk22",
            operations: OPS_TBABY_WALK21,
        },
    ),
    (
        "tbaby_walk22",
        MonsterFrame {
            frame: 21,
            next: "tbaby_walk23",
            operations: OPS_TBABY_WALK22,
        },
    ),
    (
        "tbaby_walk23",
        MonsterFrame {
            frame: 22,
            next: "tbaby_walk24",
            operations: OPS_TBABY_WALK23,
        },
    ),
    (
        "tbaby_walk24",
        MonsterFrame {
            frame: 23,
            next: "tbaby_walk25",
            operations: OPS_TBABY_WALK24,
        },
    ),
    (
        "tbaby_walk25",
        MonsterFrame {
            frame: 24,
            next: "tbaby_walk1",
            operations: OPS_TBABY_WALK25,
        },
    ),
    (
        "tbaby_walk3",
        MonsterFrame {
            frame: 2,
            next: "tbaby_walk4",
            operations: OPS_TBABY_WALK3,
        },
    ),
    (
        "tbaby_walk4",
        MonsterFrame {
            frame: 3,
            next: "tbaby_walk5",
            operations: OPS_TBABY_WALK4,
        },
    ),
    (
        "tbaby_walk5",
        MonsterFrame {
            frame: 4,
            next: "tbaby_walk6",
            operations: OPS_TBABY_WALK5,
        },
    ),
    (
        "tbaby_walk6",
        MonsterFrame {
            frame: 5,
            next: "tbaby_walk7",
            operations: OPS_TBABY_WALK6,
        },
    ),
    (
        "tbaby_walk7",
        MonsterFrame {
            frame: 6,
            next: "tbaby_walk8",
            operations: OPS_TBABY_WALK7,
        },
    ),
    (
        "tbaby_walk8",
        MonsterFrame {
            frame: 7,
            next: "tbaby_walk9",
            operations: OPS_TBABY_WALK8,
        },
    ),
    (
        "tbaby_walk9",
        MonsterFrame {
            frame: 8,
            next: "tbaby_walk10",
            operations: OPS_TBABY_WALK9,
        },
    ),
    (
        "wiz_death1",
        MonsterFrame {
            frame: 46,
            next: "wiz_death2",
            operations: OPS_WIZ_DEATH1,
        },
    ),
    (
        "wiz_death2",
        MonsterFrame {
            frame: 47,
            next: "wiz_death3",
            operations: OPS_WIZ_DEATH2,
        },
    ),
    (
        "wiz_death3",
        MonsterFrame {
            frame: 48,
            next: "wiz_death4",
            operations: OPS_WIZ_DEATH3,
        },
    ),
    (
        "wiz_death4",
        MonsterFrame {
            frame: 49,
            next: "wiz_death5",
            operations: OPS_WIZ_DEATH4,
        },
    ),
    (
        "wiz_death5",
        MonsterFrame {
            frame: 50,
            next: "wiz_death6",
            operations: OPS_WIZ_DEATH5,
        },
    ),
    (
        "wiz_death6",
        MonsterFrame {
            frame: 51,
            next: "wiz_death7",
            operations: OPS_WIZ_DEATH6,
        },
    ),
    (
        "wiz_death7",
        MonsterFrame {
            frame: 52,
            next: "wiz_death8",
            operations: OPS_WIZ_DEATH7,
        },
    ),
    (
        "wiz_death8",
        MonsterFrame {
            frame: 53,
            next: "wiz_death8",
            operations: OPS_WIZ_DEATH8,
        },
    ),
    (
        "wiz_fast1",
        MonsterFrame {
            frame: 29,
            next: "wiz_fast2",
            operations: OPS_WIZ_FAST1,
        },
    ),
    (
        "wiz_fast10",
        MonsterFrame {
            frame: 30,
            next: "wiz_run1",
            operations: OPS_WIZ_FAST10,
        },
    ),
    (
        "wiz_fast2",
        MonsterFrame {
            frame: 30,
            next: "wiz_fast3",
            operations: OPS_WIZ_FAST2,
        },
    ),
    (
        "wiz_fast3",
        MonsterFrame {
            frame: 31,
            next: "wiz_fast4",
            operations: OPS_WIZ_FAST3,
        },
    ),
    (
        "wiz_fast4",
        MonsterFrame {
            frame: 32,
            next: "wiz_fast5",
            operations: OPS_WIZ_FAST4,
        },
    ),
    (
        "wiz_fast5",
        MonsterFrame {
            frame: 33,
            next: "wiz_fast6",
            operations: OPS_WIZ_FAST5,
        },
    ),
    (
        "wiz_fast6",
        MonsterFrame {
            frame: 34,
            next: "wiz_fast7",
            operations: OPS_WIZ_FAST6,
        },
    ),
    (
        "wiz_fast7",
        MonsterFrame {
            frame: 33,
            next: "wiz_fast8",
            operations: OPS_WIZ_FAST7,
        },
    ),
    (
        "wiz_fast8",
        MonsterFrame {
            frame: 32,
            next: "wiz_fast9",
            operations: OPS_WIZ_FAST8,
        },
    ),
    (
        "wiz_fast9",
        MonsterFrame {
            frame: 31,
            next: "wiz_fast10",
            operations: OPS_WIZ_FAST9,
        },
    ),
    (
        "wiz_pain1",
        MonsterFrame {
            frame: 42,
            next: "wiz_pain2",
            operations: OPS_WIZ_PAIN1,
        },
    ),
    (
        "wiz_pain2",
        MonsterFrame {
            frame: 43,
            next: "wiz_pain3",
            operations: OPS_WIZ_PAIN2,
        },
    ),
    (
        "wiz_pain3",
        MonsterFrame {
            frame: 44,
            next: "wiz_pain4",
            operations: OPS_WIZ_PAIN3,
        },
    ),
    (
        "wiz_pain4",
        MonsterFrame {
            frame: 45,
            next: "wiz_run1",
            operations: OPS_WIZ_PAIN4,
        },
    ),
    (
        "wiz_run1",
        MonsterFrame {
            frame: 15,
            next: "wiz_run2",
            operations: OPS_WIZ_RUN1,
        },
    ),
    (
        "wiz_run10",
        MonsterFrame {
            frame: 24,
            next: "wiz_run11",
            operations: OPS_WIZ_RUN10,
        },
    ),
    (
        "wiz_run11",
        MonsterFrame {
            frame: 25,
            next: "wiz_run12",
            operations: OPS_WIZ_RUN11,
        },
    ),
    (
        "wiz_run12",
        MonsterFrame {
            frame: 26,
            next: "wiz_run13",
            operations: OPS_WIZ_RUN12,
        },
    ),
    (
        "wiz_run13",
        MonsterFrame {
            frame: 27,
            next: "wiz_run14",
            operations: OPS_WIZ_RUN13,
        },
    ),
    (
        "wiz_run14",
        MonsterFrame {
            frame: 28,
            next: "wiz_run1",
            operations: OPS_WIZ_RUN14,
        },
    ),
    (
        "wiz_run2",
        MonsterFrame {
            frame: 16,
            next: "wiz_run3",
            operations: OPS_WIZ_RUN2,
        },
    ),
    (
        "wiz_run3",
        MonsterFrame {
            frame: 17,
            next: "wiz_run4",
            operations: OPS_WIZ_RUN3,
        },
    ),
    (
        "wiz_run4",
        MonsterFrame {
            frame: 18,
            next: "wiz_run5",
            operations: OPS_WIZ_RUN4,
        },
    ),
    (
        "wiz_run5",
        MonsterFrame {
            frame: 19,
            next: "wiz_run6",
            operations: OPS_WIZ_RUN5,
        },
    ),
    (
        "wiz_run6",
        MonsterFrame {
            frame: 20,
            next: "wiz_run7",
            operations: OPS_WIZ_RUN6,
        },
    ),
    (
        "wiz_run7",
        MonsterFrame {
            frame: 21,
            next: "wiz_run8",
            operations: OPS_WIZ_RUN7,
        },
    ),
    (
        "wiz_run8",
        MonsterFrame {
            frame: 22,
            next: "wiz_run9",
            operations: OPS_WIZ_RUN8,
        },
    ),
    (
        "wiz_run9",
        MonsterFrame {
            frame: 23,
            next: "wiz_run10",
            operations: OPS_WIZ_RUN9,
        },
    ),
    (
        "wiz_side1",
        MonsterFrame {
            frame: 0,
            next: "wiz_side2",
            operations: OPS_WIZ_SIDE1,
        },
    ),
    (
        "wiz_side2",
        MonsterFrame {
            frame: 1,
            next: "wiz_side3",
            operations: OPS_WIZ_SIDE2,
        },
    ),
    (
        "wiz_side3",
        MonsterFrame {
            frame: 2,
            next: "wiz_side4",
            operations: OPS_WIZ_SIDE3,
        },
    ),
    (
        "wiz_side4",
        MonsterFrame {
            frame: 3,
            next: "wiz_side5",
            operations: OPS_WIZ_SIDE4,
        },
    ),
    (
        "wiz_side5",
        MonsterFrame {
            frame: 4,
            next: "wiz_side6",
            operations: OPS_WIZ_SIDE5,
        },
    ),
    (
        "wiz_side6",
        MonsterFrame {
            frame: 5,
            next: "wiz_side7",
            operations: OPS_WIZ_SIDE6,
        },
    ),
    (
        "wiz_side7",
        MonsterFrame {
            frame: 6,
            next: "wiz_side8",
            operations: OPS_WIZ_SIDE7,
        },
    ),
    (
        "wiz_side8",
        MonsterFrame {
            frame: 7,
            next: "wiz_side1",
            operations: OPS_WIZ_SIDE8,
        },
    ),
    (
        "wiz_stand1",
        MonsterFrame {
            frame: 0,
            next: "wiz_stand2",
            operations: OPS_WIZ_STAND1,
        },
    ),
    (
        "wiz_stand2",
        MonsterFrame {
            frame: 1,
            next: "wiz_stand3",
            operations: OPS_WIZ_STAND2,
        },
    ),
    (
        "wiz_stand3",
        MonsterFrame {
            frame: 2,
            next: "wiz_stand4",
            operations: OPS_WIZ_STAND3,
        },
    ),
    (
        "wiz_stand4",
        MonsterFrame {
            frame: 3,
            next: "wiz_stand5",
            operations: OPS_WIZ_STAND4,
        },
    ),
    (
        "wiz_stand5",
        MonsterFrame {
            frame: 4,
            next: "wiz_stand6",
            operations: OPS_WIZ_STAND5,
        },
    ),
    (
        "wiz_stand6",
        MonsterFrame {
            frame: 5,
            next: "wiz_stand7",
            operations: OPS_WIZ_STAND6,
        },
    ),
    (
        "wiz_stand7",
        MonsterFrame {
            frame: 6,
            next: "wiz_stand8",
            operations: OPS_WIZ_STAND7,
        },
    ),
    (
        "wiz_stand8",
        MonsterFrame {
            frame: 7,
            next: "wiz_stand1",
            operations: OPS_WIZ_STAND8,
        },
    ),
    (
        "wiz_walk1",
        MonsterFrame {
            frame: 0,
            next: "wiz_walk2",
            operations: OPS_WIZ_WALK1,
        },
    ),
    (
        "wiz_walk2",
        MonsterFrame {
            frame: 1,
            next: "wiz_walk3",
            operations: OPS_WIZ_WALK2,
        },
    ),
    (
        "wiz_walk3",
        MonsterFrame {
            frame: 2,
            next: "wiz_walk4",
            operations: OPS_WIZ_WALK3,
        },
    ),
    (
        "wiz_walk4",
        MonsterFrame {
            frame: 3,
            next: "wiz_walk5",
            operations: OPS_WIZ_WALK4,
        },
    ),
    (
        "wiz_walk5",
        MonsterFrame {
            frame: 4,
            next: "wiz_walk6",
            operations: OPS_WIZ_WALK5,
        },
    ),
    (
        "wiz_walk6",
        MonsterFrame {
            frame: 5,
            next: "wiz_walk7",
            operations: OPS_WIZ_WALK6,
        },
    ),
    (
        "wiz_walk7",
        MonsterFrame {
            frame: 6,
            next: "wiz_walk8",
            operations: OPS_WIZ_WALK7,
        },
    ),
    (
        "wiz_walk8",
        MonsterFrame {
            frame: 7,
            next: "wiz_walk1",
            operations: OPS_WIZ_WALK8,
        },
    ),
    (
        "zombie_atta1",
        MonsterFrame {
            frame: 52,
            next: "zombie_atta2",
            operations: OPS_ZOMBIE_ATTA1,
        },
    ),
    (
        "zombie_atta10",
        MonsterFrame {
            frame: 61,
            next: "zombie_atta11",
            operations: OPS_ZOMBIE_ATTA10,
        },
    ),
    (
        "zombie_atta11",
        MonsterFrame {
            frame: 62,
            next: "zombie_atta12",
            operations: OPS_ZOMBIE_ATTA11,
        },
    ),
    (
        "zombie_atta12",
        MonsterFrame {
            frame: 63,
            next: "zombie_atta13",
            operations: OPS_ZOMBIE_ATTA12,
        },
    ),
    (
        "zombie_atta13",
        MonsterFrame {
            frame: 64,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_ATTA13,
        },
    ),
    (
        "zombie_atta2",
        MonsterFrame {
            frame: 53,
            next: "zombie_atta3",
            operations: OPS_ZOMBIE_ATTA2,
        },
    ),
    (
        "zombie_atta3",
        MonsterFrame {
            frame: 54,
            next: "zombie_atta4",
            operations: OPS_ZOMBIE_ATTA3,
        },
    ),
    (
        "zombie_atta4",
        MonsterFrame {
            frame: 55,
            next: "zombie_atta5",
            operations: OPS_ZOMBIE_ATTA4,
        },
    ),
    (
        "zombie_atta5",
        MonsterFrame {
            frame: 56,
            next: "zombie_atta6",
            operations: OPS_ZOMBIE_ATTA5,
        },
    ),
    (
        "zombie_atta6",
        MonsterFrame {
            frame: 57,
            next: "zombie_atta7",
            operations: OPS_ZOMBIE_ATTA6,
        },
    ),
    (
        "zombie_atta7",
        MonsterFrame {
            frame: 58,
            next: "zombie_atta8",
            operations: OPS_ZOMBIE_ATTA7,
        },
    ),
    (
        "zombie_atta8",
        MonsterFrame {
            frame: 59,
            next: "zombie_atta9",
            operations: OPS_ZOMBIE_ATTA8,
        },
    ),
    (
        "zombie_atta9",
        MonsterFrame {
            frame: 60,
            next: "zombie_atta10",
            operations: OPS_ZOMBIE_ATTA9,
        },
    ),
    (
        "zombie_attb1",
        MonsterFrame {
            frame: 65,
            next: "zombie_attb2",
            operations: OPS_ZOMBIE_ATTB1,
        },
    ),
    (
        "zombie_attb10",
        MonsterFrame {
            frame: 74,
            next: "zombie_attb11",
            operations: OPS_ZOMBIE_ATTB10,
        },
    ),
    (
        "zombie_attb11",
        MonsterFrame {
            frame: 75,
            next: "zombie_attb12",
            operations: OPS_ZOMBIE_ATTB11,
        },
    ),
    (
        "zombie_attb12",
        MonsterFrame {
            frame: 76,
            next: "zombie_attb13",
            operations: OPS_ZOMBIE_ATTB12,
        },
    ),
    (
        "zombie_attb13",
        MonsterFrame {
            frame: 77,
            next: "zombie_attb14",
            operations: OPS_ZOMBIE_ATTB13,
        },
    ),
    (
        "zombie_attb14",
        MonsterFrame {
            frame: 77,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_ATTB14,
        },
    ),
    (
        "zombie_attb2",
        MonsterFrame {
            frame: 66,
            next: "zombie_attb3",
            operations: OPS_ZOMBIE_ATTB2,
        },
    ),
    (
        "zombie_attb3",
        MonsterFrame {
            frame: 67,
            next: "zombie_attb4",
            operations: OPS_ZOMBIE_ATTB3,
        },
    ),
    (
        "zombie_attb4",
        MonsterFrame {
            frame: 68,
            next: "zombie_attb5",
            operations: OPS_ZOMBIE_ATTB4,
        },
    ),
    (
        "zombie_attb5",
        MonsterFrame {
            frame: 69,
            next: "zombie_attb6",
            operations: OPS_ZOMBIE_ATTB5,
        },
    ),
    (
        "zombie_attb6",
        MonsterFrame {
            frame: 70,
            next: "zombie_attb7",
            operations: OPS_ZOMBIE_ATTB6,
        },
    ),
    (
        "zombie_attb7",
        MonsterFrame {
            frame: 71,
            next: "zombie_attb8",
            operations: OPS_ZOMBIE_ATTB7,
        },
    ),
    (
        "zombie_attb8",
        MonsterFrame {
            frame: 72,
            next: "zombie_attb9",
            operations: OPS_ZOMBIE_ATTB8,
        },
    ),
    (
        "zombie_attb9",
        MonsterFrame {
            frame: 73,
            next: "zombie_attb10",
            operations: OPS_ZOMBIE_ATTB9,
        },
    ),
    (
        "zombie_attc1",
        MonsterFrame {
            frame: 79,
            next: "zombie_attc2",
            operations: OPS_ZOMBIE_ATTC1,
        },
    ),
    (
        "zombie_attc10",
        MonsterFrame {
            frame: 88,
            next: "zombie_attc11",
            operations: OPS_ZOMBIE_ATTC10,
        },
    ),
    (
        "zombie_attc11",
        MonsterFrame {
            frame: 89,
            next: "zombie_attc12",
            operations: OPS_ZOMBIE_ATTC11,
        },
    ),
    (
        "zombie_attc12",
        MonsterFrame {
            frame: 90,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_ATTC12,
        },
    ),
    (
        "zombie_attc2",
        MonsterFrame {
            frame: 80,
            next: "zombie_attc3",
            operations: OPS_ZOMBIE_ATTC2,
        },
    ),
    (
        "zombie_attc3",
        MonsterFrame {
            frame: 81,
            next: "zombie_attc4",
            operations: OPS_ZOMBIE_ATTC3,
        },
    ),
    (
        "zombie_attc4",
        MonsterFrame {
            frame: 82,
            next: "zombie_attc5",
            operations: OPS_ZOMBIE_ATTC4,
        },
    ),
    (
        "zombie_attc5",
        MonsterFrame {
            frame: 83,
            next: "zombie_attc6",
            operations: OPS_ZOMBIE_ATTC5,
        },
    ),
    (
        "zombie_attc6",
        MonsterFrame {
            frame: 84,
            next: "zombie_attc7",
            operations: OPS_ZOMBIE_ATTC6,
        },
    ),
    (
        "zombie_attc7",
        MonsterFrame {
            frame: 85,
            next: "zombie_attc8",
            operations: OPS_ZOMBIE_ATTC7,
        },
    ),
    (
        "zombie_attc8",
        MonsterFrame {
            frame: 86,
            next: "zombie_attc9",
            operations: OPS_ZOMBIE_ATTC8,
        },
    ),
    (
        "zombie_attc9",
        MonsterFrame {
            frame: 87,
            next: "zombie_attc10",
            operations: OPS_ZOMBIE_ATTC9,
        },
    ),
    (
        "zombie_cruc1",
        MonsterFrame {
            frame: 192,
            next: "zombie_cruc2",
            operations: OPS_ZOMBIE_CRUC1,
        },
    ),
    (
        "zombie_cruc2",
        MonsterFrame {
            frame: 193,
            next: "zombie_cruc3",
            operations: OPS_ZOMBIE_CRUC2,
        },
    ),
    (
        "zombie_cruc3",
        MonsterFrame {
            frame: 194,
            next: "zombie_cruc4",
            operations: OPS_ZOMBIE_CRUC3,
        },
    ),
    (
        "zombie_cruc4",
        MonsterFrame {
            frame: 195,
            next: "zombie_cruc5",
            operations: OPS_ZOMBIE_CRUC4,
        },
    ),
    (
        "zombie_cruc5",
        MonsterFrame {
            frame: 196,
            next: "zombie_cruc6",
            operations: OPS_ZOMBIE_CRUC5,
        },
    ),
    (
        "zombie_cruc6",
        MonsterFrame {
            frame: 197,
            next: "zombie_cruc1",
            operations: OPS_ZOMBIE_CRUC6,
        },
    ),
    (
        "zombie_paina1",
        MonsterFrame {
            frame: 91,
            next: "zombie_paina2",
            operations: OPS_ZOMBIE_PAINA1,
        },
    ),
    (
        "zombie_paina10",
        MonsterFrame {
            frame: 100,
            next: "zombie_paina11",
            operations: OPS_ZOMBIE_PAINA10,
        },
    ),
    (
        "zombie_paina11",
        MonsterFrame {
            frame: 101,
            next: "zombie_paina12",
            operations: OPS_ZOMBIE_PAINA11,
        },
    ),
    (
        "zombie_paina12",
        MonsterFrame {
            frame: 102,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_PAINA12,
        },
    ),
    (
        "zombie_paina2",
        MonsterFrame {
            frame: 92,
            next: "zombie_paina3",
            operations: OPS_ZOMBIE_PAINA2,
        },
    ),
    (
        "zombie_paina3",
        MonsterFrame {
            frame: 93,
            next: "zombie_paina4",
            operations: OPS_ZOMBIE_PAINA3,
        },
    ),
    (
        "zombie_paina4",
        MonsterFrame {
            frame: 94,
            next: "zombie_paina5",
            operations: OPS_ZOMBIE_PAINA4,
        },
    ),
    (
        "zombie_paina5",
        MonsterFrame {
            frame: 95,
            next: "zombie_paina6",
            operations: OPS_ZOMBIE_PAINA5,
        },
    ),
    (
        "zombie_paina6",
        MonsterFrame {
            frame: 96,
            next: "zombie_paina7",
            operations: OPS_ZOMBIE_PAINA6,
        },
    ),
    (
        "zombie_paina7",
        MonsterFrame {
            frame: 97,
            next: "zombie_paina8",
            operations: OPS_ZOMBIE_PAINA7,
        },
    ),
    (
        "zombie_paina8",
        MonsterFrame {
            frame: 98,
            next: "zombie_paina9",
            operations: OPS_ZOMBIE_PAINA8,
        },
    ),
    (
        "zombie_paina9",
        MonsterFrame {
            frame: 99,
            next: "zombie_paina10",
            operations: OPS_ZOMBIE_PAINA9,
        },
    ),
    (
        "zombie_painb1",
        MonsterFrame {
            frame: 103,
            next: "zombie_painb2",
            operations: OPS_ZOMBIE_PAINB1,
        },
    ),
    (
        "zombie_painb10",
        MonsterFrame {
            frame: 112,
            next: "zombie_painb11",
            operations: OPS_ZOMBIE_PAINB10,
        },
    ),
    (
        "zombie_painb11",
        MonsterFrame {
            frame: 113,
            next: "zombie_painb12",
            operations: OPS_ZOMBIE_PAINB11,
        },
    ),
    (
        "zombie_painb12",
        MonsterFrame {
            frame: 114,
            next: "zombie_painb13",
            operations: OPS_ZOMBIE_PAINB12,
        },
    ),
    (
        "zombie_painb13",
        MonsterFrame {
            frame: 115,
            next: "zombie_painb14",
            operations: OPS_ZOMBIE_PAINB13,
        },
    ),
    (
        "zombie_painb14",
        MonsterFrame {
            frame: 116,
            next: "zombie_painb15",
            operations: OPS_ZOMBIE_PAINB14,
        },
    ),
    (
        "zombie_painb15",
        MonsterFrame {
            frame: 117,
            next: "zombie_painb16",
            operations: OPS_ZOMBIE_PAINB15,
        },
    ),
    (
        "zombie_painb16",
        MonsterFrame {
            frame: 118,
            next: "zombie_painb17",
            operations: OPS_ZOMBIE_PAINB16,
        },
    ),
    (
        "zombie_painb17",
        MonsterFrame {
            frame: 119,
            next: "zombie_painb18",
            operations: OPS_ZOMBIE_PAINB17,
        },
    ),
    (
        "zombie_painb18",
        MonsterFrame {
            frame: 120,
            next: "zombie_painb19",
            operations: OPS_ZOMBIE_PAINB18,
        },
    ),
    (
        "zombie_painb19",
        MonsterFrame {
            frame: 121,
            next: "zombie_painb20",
            operations: OPS_ZOMBIE_PAINB19,
        },
    ),
    (
        "zombie_painb2",
        MonsterFrame {
            frame: 104,
            next: "zombie_painb3",
            operations: OPS_ZOMBIE_PAINB2,
        },
    ),
    (
        "zombie_painb20",
        MonsterFrame {
            frame: 122,
            next: "zombie_painb21",
            operations: OPS_ZOMBIE_PAINB20,
        },
    ),
    (
        "zombie_painb21",
        MonsterFrame {
            frame: 123,
            next: "zombie_painb22",
            operations: OPS_ZOMBIE_PAINB21,
        },
    ),
    (
        "zombie_painb22",
        MonsterFrame {
            frame: 124,
            next: "zombie_painb23",
            operations: OPS_ZOMBIE_PAINB22,
        },
    ),
    (
        "zombie_painb23",
        MonsterFrame {
            frame: 125,
            next: "zombie_painb24",
            operations: OPS_ZOMBIE_PAINB23,
        },
    ),
    (
        "zombie_painb24",
        MonsterFrame {
            frame: 126,
            next: "zombie_painb25",
            operations: OPS_ZOMBIE_PAINB24,
        },
    ),
    (
        "zombie_painb25",
        MonsterFrame {
            frame: 127,
            next: "zombie_painb26",
            operations: OPS_ZOMBIE_PAINB25,
        },
    ),
    (
        "zombie_painb26",
        MonsterFrame {
            frame: 128,
            next: "zombie_painb27",
            operations: OPS_ZOMBIE_PAINB26,
        },
    ),
    (
        "zombie_painb27",
        MonsterFrame {
            frame: 129,
            next: "zombie_painb28",
            operations: OPS_ZOMBIE_PAINB27,
        },
    ),
    (
        "zombie_painb28",
        MonsterFrame {
            frame: 130,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_PAINB28,
        },
    ),
    (
        "zombie_painb3",
        MonsterFrame {
            frame: 105,
            next: "zombie_painb4",
            operations: OPS_ZOMBIE_PAINB3,
        },
    ),
    (
        "zombie_painb4",
        MonsterFrame {
            frame: 106,
            next: "zombie_painb5",
            operations: OPS_ZOMBIE_PAINB4,
        },
    ),
    (
        "zombie_painb5",
        MonsterFrame {
            frame: 107,
            next: "zombie_painb6",
            operations: OPS_ZOMBIE_PAINB5,
        },
    ),
    (
        "zombie_painb6",
        MonsterFrame {
            frame: 108,
            next: "zombie_painb7",
            operations: OPS_ZOMBIE_PAINB6,
        },
    ),
    (
        "zombie_painb7",
        MonsterFrame {
            frame: 109,
            next: "zombie_painb8",
            operations: OPS_ZOMBIE_PAINB7,
        },
    ),
    (
        "zombie_painb8",
        MonsterFrame {
            frame: 110,
            next: "zombie_painb9",
            operations: OPS_ZOMBIE_PAINB8,
        },
    ),
    (
        "zombie_painb9",
        MonsterFrame {
            frame: 111,
            next: "zombie_painb10",
            operations: OPS_ZOMBIE_PAINB9,
        },
    ),
    (
        "zombie_painc1",
        MonsterFrame {
            frame: 131,
            next: "zombie_painc2",
            operations: OPS_ZOMBIE_PAINC1,
        },
    ),
    (
        "zombie_painc10",
        MonsterFrame {
            frame: 140,
            next: "zombie_painc11",
            operations: OPS_ZOMBIE_PAINC10,
        },
    ),
    (
        "zombie_painc11",
        MonsterFrame {
            frame: 141,
            next: "zombie_painc12",
            operations: OPS_ZOMBIE_PAINC11,
        },
    ),
    (
        "zombie_painc12",
        MonsterFrame {
            frame: 142,
            next: "zombie_painc13",
            operations: OPS_ZOMBIE_PAINC12,
        },
    ),
    (
        "zombie_painc13",
        MonsterFrame {
            frame: 143,
            next: "zombie_painc14",
            operations: OPS_ZOMBIE_PAINC13,
        },
    ),
    (
        "zombie_painc14",
        MonsterFrame {
            frame: 144,
            next: "zombie_painc15",
            operations: OPS_ZOMBIE_PAINC14,
        },
    ),
    (
        "zombie_painc15",
        MonsterFrame {
            frame: 145,
            next: "zombie_painc16",
            operations: OPS_ZOMBIE_PAINC15,
        },
    ),
    (
        "zombie_painc16",
        MonsterFrame {
            frame: 146,
            next: "zombie_painc17",
            operations: OPS_ZOMBIE_PAINC16,
        },
    ),
    (
        "zombie_painc17",
        MonsterFrame {
            frame: 147,
            next: "zombie_painc18",
            operations: OPS_ZOMBIE_PAINC17,
        },
    ),
    (
        "zombie_painc18",
        MonsterFrame {
            frame: 148,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_PAINC18,
        },
    ),
    (
        "zombie_painc2",
        MonsterFrame {
            frame: 132,
            next: "zombie_painc3",
            operations: OPS_ZOMBIE_PAINC2,
        },
    ),
    (
        "zombie_painc3",
        MonsterFrame {
            frame: 133,
            next: "zombie_painc4",
            operations: OPS_ZOMBIE_PAINC3,
        },
    ),
    (
        "zombie_painc4",
        MonsterFrame {
            frame: 134,
            next: "zombie_painc5",
            operations: OPS_ZOMBIE_PAINC4,
        },
    ),
    (
        "zombie_painc5",
        MonsterFrame {
            frame: 135,
            next: "zombie_painc6",
            operations: OPS_ZOMBIE_PAINC5,
        },
    ),
    (
        "zombie_painc6",
        MonsterFrame {
            frame: 136,
            next: "zombie_painc7",
            operations: OPS_ZOMBIE_PAINC6,
        },
    ),
    (
        "zombie_painc7",
        MonsterFrame {
            frame: 137,
            next: "zombie_painc8",
            operations: OPS_ZOMBIE_PAINC7,
        },
    ),
    (
        "zombie_painc8",
        MonsterFrame {
            frame: 138,
            next: "zombie_painc9",
            operations: OPS_ZOMBIE_PAINC8,
        },
    ),
    (
        "zombie_painc9",
        MonsterFrame {
            frame: 139,
            next: "zombie_painc10",
            operations: OPS_ZOMBIE_PAINC9,
        },
    ),
    (
        "zombie_paind1",
        MonsterFrame {
            frame: 149,
            next: "zombie_paind2",
            operations: OPS_ZOMBIE_PAIND1,
        },
    ),
    (
        "zombie_paind10",
        MonsterFrame {
            frame: 158,
            next: "zombie_paind11",
            operations: OPS_ZOMBIE_PAIND10,
        },
    ),
    (
        "zombie_paind11",
        MonsterFrame {
            frame: 159,
            next: "zombie_paind12",
            operations: OPS_ZOMBIE_PAIND11,
        },
    ),
    (
        "zombie_paind12",
        MonsterFrame {
            frame: 160,
            next: "zombie_paind13",
            operations: OPS_ZOMBIE_PAIND12,
        },
    ),
    (
        "zombie_paind13",
        MonsterFrame {
            frame: 161,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_PAIND13,
        },
    ),
    (
        "zombie_paind2",
        MonsterFrame {
            frame: 150,
            next: "zombie_paind3",
            operations: OPS_ZOMBIE_PAIND2,
        },
    ),
    (
        "zombie_paind3",
        MonsterFrame {
            frame: 151,
            next: "zombie_paind4",
            operations: OPS_ZOMBIE_PAIND3,
        },
    ),
    (
        "zombie_paind4",
        MonsterFrame {
            frame: 152,
            next: "zombie_paind5",
            operations: OPS_ZOMBIE_PAIND4,
        },
    ),
    (
        "zombie_paind5",
        MonsterFrame {
            frame: 153,
            next: "zombie_paind6",
            operations: OPS_ZOMBIE_PAIND5,
        },
    ),
    (
        "zombie_paind6",
        MonsterFrame {
            frame: 154,
            next: "zombie_paind7",
            operations: OPS_ZOMBIE_PAIND6,
        },
    ),
    (
        "zombie_paind7",
        MonsterFrame {
            frame: 155,
            next: "zombie_paind8",
            operations: OPS_ZOMBIE_PAIND7,
        },
    ),
    (
        "zombie_paind8",
        MonsterFrame {
            frame: 156,
            next: "zombie_paind9",
            operations: OPS_ZOMBIE_PAIND8,
        },
    ),
    (
        "zombie_paind9",
        MonsterFrame {
            frame: 157,
            next: "zombie_paind10",
            operations: OPS_ZOMBIE_PAIND9,
        },
    ),
    (
        "zombie_paine1",
        MonsterFrame {
            frame: 162,
            next: "zombie_paine2",
            operations: OPS_ZOMBIE_PAINE1,
        },
    ),
    (
        "zombie_paine10",
        MonsterFrame {
            frame: 171,
            next: "zombie_paine11",
            operations: OPS_ZOMBIE_PAINE10,
        },
    ),
    (
        "zombie_paine11",
        MonsterFrame {
            frame: 172,
            next: "zombie_paine12",
            operations: OPS_ZOMBIE_PAINE11,
        },
    ),
    (
        "zombie_paine12",
        MonsterFrame {
            frame: 173,
            next: "zombie_paine13",
            operations: OPS_ZOMBIE_PAINE12,
        },
    ),
    (
        "zombie_paine13",
        MonsterFrame {
            frame: 174,
            next: "zombie_paine14",
            operations: OPS_ZOMBIE_PAINE13,
        },
    ),
    (
        "zombie_paine14",
        MonsterFrame {
            frame: 175,
            next: "zombie_paine15",
            operations: OPS_ZOMBIE_PAINE14,
        },
    ),
    (
        "zombie_paine15",
        MonsterFrame {
            frame: 176,
            next: "zombie_paine16",
            operations: OPS_ZOMBIE_PAINE15,
        },
    ),
    (
        "zombie_paine16",
        MonsterFrame {
            frame: 177,
            next: "zombie_paine17",
            operations: OPS_ZOMBIE_PAINE16,
        },
    ),
    (
        "zombie_paine17",
        MonsterFrame {
            frame: 178,
            next: "zombie_paine18",
            operations: OPS_ZOMBIE_PAINE17,
        },
    ),
    (
        "zombie_paine18",
        MonsterFrame {
            frame: 179,
            next: "zombie_paine19",
            operations: OPS_ZOMBIE_PAINE18,
        },
    ),
    (
        "zombie_paine19",
        MonsterFrame {
            frame: 180,
            next: "zombie_paine20",
            operations: OPS_ZOMBIE_PAINE19,
        },
    ),
    (
        "zombie_paine2",
        MonsterFrame {
            frame: 163,
            next: "zombie_paine3",
            operations: OPS_ZOMBIE_PAINE2,
        },
    ),
    (
        "zombie_paine20",
        MonsterFrame {
            frame: 181,
            next: "zombie_paine21",
            operations: OPS_ZOMBIE_PAINE20,
        },
    ),
    (
        "zombie_paine21",
        MonsterFrame {
            frame: 182,
            next: "zombie_paine22",
            operations: OPS_ZOMBIE_PAINE21,
        },
    ),
    (
        "zombie_paine22",
        MonsterFrame {
            frame: 183,
            next: "zombie_paine23",
            operations: OPS_ZOMBIE_PAINE22,
        },
    ),
    (
        "zombie_paine23",
        MonsterFrame {
            frame: 184,
            next: "zombie_paine24",
            operations: OPS_ZOMBIE_PAINE23,
        },
    ),
    (
        "zombie_paine24",
        MonsterFrame {
            frame: 185,
            next: "zombie_paine25",
            operations: OPS_ZOMBIE_PAINE24,
        },
    ),
    (
        "zombie_paine25",
        MonsterFrame {
            frame: 186,
            next: "zombie_paine26",
            operations: OPS_ZOMBIE_PAINE25,
        },
    ),
    (
        "zombie_paine26",
        MonsterFrame {
            frame: 187,
            next: "zombie_paine27",
            operations: OPS_ZOMBIE_PAINE26,
        },
    ),
    (
        "zombie_paine27",
        MonsterFrame {
            frame: 188,
            next: "zombie_paine28",
            operations: OPS_ZOMBIE_PAINE27,
        },
    ),
    (
        "zombie_paine28",
        MonsterFrame {
            frame: 189,
            next: "zombie_paine29",
            operations: OPS_ZOMBIE_PAINE28,
        },
    ),
    (
        "zombie_paine29",
        MonsterFrame {
            frame: 190,
            next: "zombie_paine30",
            operations: OPS_ZOMBIE_PAINE29,
        },
    ),
    (
        "zombie_paine3",
        MonsterFrame {
            frame: 164,
            next: "zombie_paine4",
            operations: OPS_ZOMBIE_PAINE3,
        },
    ),
    (
        "zombie_paine30",
        MonsterFrame {
            frame: 191,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_PAINE30,
        },
    ),
    (
        "zombie_paine4",
        MonsterFrame {
            frame: 165,
            next: "zombie_paine5",
            operations: OPS_ZOMBIE_PAINE4,
        },
    ),
    (
        "zombie_paine5",
        MonsterFrame {
            frame: 166,
            next: "zombie_paine6",
            operations: OPS_ZOMBIE_PAINE5,
        },
    ),
    (
        "zombie_paine6",
        MonsterFrame {
            frame: 167,
            next: "zombie_paine7",
            operations: OPS_ZOMBIE_PAINE6,
        },
    ),
    (
        "zombie_paine7",
        MonsterFrame {
            frame: 168,
            next: "zombie_paine8",
            operations: OPS_ZOMBIE_PAINE7,
        },
    ),
    (
        "zombie_paine8",
        MonsterFrame {
            frame: 169,
            next: "zombie_paine9",
            operations: OPS_ZOMBIE_PAINE8,
        },
    ),
    (
        "zombie_paine9",
        MonsterFrame {
            frame: 170,
            next: "zombie_paine10",
            operations: OPS_ZOMBIE_PAINE9,
        },
    ),
    (
        "zombie_run1",
        MonsterFrame {
            frame: 34,
            next: "zombie_run2",
            operations: OPS_ZOMBIE_RUN1,
        },
    ),
    (
        "zombie_run10",
        MonsterFrame {
            frame: 43,
            next: "zombie_run11",
            operations: OPS_ZOMBIE_RUN10,
        },
    ),
    (
        "zombie_run11",
        MonsterFrame {
            frame: 44,
            next: "zombie_run12",
            operations: OPS_ZOMBIE_RUN11,
        },
    ),
    (
        "zombie_run12",
        MonsterFrame {
            frame: 45,
            next: "zombie_run13",
            operations: OPS_ZOMBIE_RUN12,
        },
    ),
    (
        "zombie_run13",
        MonsterFrame {
            frame: 46,
            next: "zombie_run14",
            operations: OPS_ZOMBIE_RUN13,
        },
    ),
    (
        "zombie_run14",
        MonsterFrame {
            frame: 47,
            next: "zombie_run15",
            operations: OPS_ZOMBIE_RUN14,
        },
    ),
    (
        "zombie_run15",
        MonsterFrame {
            frame: 48,
            next: "zombie_run16",
            operations: OPS_ZOMBIE_RUN15,
        },
    ),
    (
        "zombie_run16",
        MonsterFrame {
            frame: 49,
            next: "zombie_run17",
            operations: OPS_ZOMBIE_RUN16,
        },
    ),
    (
        "zombie_run17",
        MonsterFrame {
            frame: 50,
            next: "zombie_run18",
            operations: OPS_ZOMBIE_RUN17,
        },
    ),
    (
        "zombie_run18",
        MonsterFrame {
            frame: 51,
            next: "zombie_run1",
            operations: OPS_ZOMBIE_RUN18,
        },
    ),
    (
        "zombie_run2",
        MonsterFrame {
            frame: 35,
            next: "zombie_run3",
            operations: OPS_ZOMBIE_RUN2,
        },
    ),
    (
        "zombie_run3",
        MonsterFrame {
            frame: 36,
            next: "zombie_run4",
            operations: OPS_ZOMBIE_RUN3,
        },
    ),
    (
        "zombie_run4",
        MonsterFrame {
            frame: 37,
            next: "zombie_run5",
            operations: OPS_ZOMBIE_RUN4,
        },
    ),
    (
        "zombie_run5",
        MonsterFrame {
            frame: 38,
            next: "zombie_run6",
            operations: OPS_ZOMBIE_RUN5,
        },
    ),
    (
        "zombie_run6",
        MonsterFrame {
            frame: 39,
            next: "zombie_run7",
            operations: OPS_ZOMBIE_RUN6,
        },
    ),
    (
        "zombie_run7",
        MonsterFrame {
            frame: 40,
            next: "zombie_run8",
            operations: OPS_ZOMBIE_RUN7,
        },
    ),
    (
        "zombie_run8",
        MonsterFrame {
            frame: 41,
            next: "zombie_run9",
            operations: OPS_ZOMBIE_RUN8,
        },
    ),
    (
        "zombie_run9",
        MonsterFrame {
            frame: 42,
            next: "zombie_run10",
            operations: OPS_ZOMBIE_RUN9,
        },
    ),
    (
        "zombie_stand1",
        MonsterFrame {
            frame: 0,
            next: "zombie_stand2",
            operations: OPS_ZOMBIE_STAND1,
        },
    ),
    (
        "zombie_stand10",
        MonsterFrame {
            frame: 9,
            next: "zombie_stand11",
            operations: OPS_ZOMBIE_STAND10,
        },
    ),
    (
        "zombie_stand11",
        MonsterFrame {
            frame: 10,
            next: "zombie_stand12",
            operations: OPS_ZOMBIE_STAND11,
        },
    ),
    (
        "zombie_stand12",
        MonsterFrame {
            frame: 11,
            next: "zombie_stand13",
            operations: OPS_ZOMBIE_STAND12,
        },
    ),
    (
        "zombie_stand13",
        MonsterFrame {
            frame: 12,
            next: "zombie_stand14",
            operations: OPS_ZOMBIE_STAND13,
        },
    ),
    (
        "zombie_stand14",
        MonsterFrame {
            frame: 13,
            next: "zombie_stand15",
            operations: OPS_ZOMBIE_STAND14,
        },
    ),
    (
        "zombie_stand15",
        MonsterFrame {
            frame: 14,
            next: "zombie_stand1",
            operations: OPS_ZOMBIE_STAND15,
        },
    ),
    (
        "zombie_stand2",
        MonsterFrame {
            frame: 1,
            next: "zombie_stand3",
            operations: OPS_ZOMBIE_STAND2,
        },
    ),
    (
        "zombie_stand3",
        MonsterFrame {
            frame: 2,
            next: "zombie_stand4",
            operations: OPS_ZOMBIE_STAND3,
        },
    ),
    (
        "zombie_stand4",
        MonsterFrame {
            frame: 3,
            next: "zombie_stand5",
            operations: OPS_ZOMBIE_STAND4,
        },
    ),
    (
        "zombie_stand5",
        MonsterFrame {
            frame: 4,
            next: "zombie_stand6",
            operations: OPS_ZOMBIE_STAND5,
        },
    ),
    (
        "zombie_stand6",
        MonsterFrame {
            frame: 5,
            next: "zombie_stand7",
            operations: OPS_ZOMBIE_STAND6,
        },
    ),
    (
        "zombie_stand7",
        MonsterFrame {
            frame: 6,
            next: "zombie_stand8",
            operations: OPS_ZOMBIE_STAND7,
        },
    ),
    (
        "zombie_stand8",
        MonsterFrame {
            frame: 7,
            next: "zombie_stand9",
            operations: OPS_ZOMBIE_STAND8,
        },
    ),
    (
        "zombie_stand9",
        MonsterFrame {
            frame: 8,
            next: "zombie_stand10",
            operations: OPS_ZOMBIE_STAND9,
        },
    ),
    (
        "zombie_walk1",
        MonsterFrame {
            frame: 15,
            next: "zombie_walk2",
            operations: OPS_ZOMBIE_WALK1,
        },
    ),
    (
        "zombie_walk10",
        MonsterFrame {
            frame: 24,
            next: "zombie_walk11",
            operations: OPS_ZOMBIE_WALK10,
        },
    ),
    (
        "zombie_walk11",
        MonsterFrame {
            frame: 25,
            next: "zombie_walk12",
            operations: OPS_ZOMBIE_WALK11,
        },
    ),
    (
        "zombie_walk12",
        MonsterFrame {
            frame: 26,
            next: "zombie_walk13",
            operations: OPS_ZOMBIE_WALK12,
        },
    ),
    (
        "zombie_walk13",
        MonsterFrame {
            frame: 27,
            next: "zombie_walk14",
            operations: OPS_ZOMBIE_WALK13,
        },
    ),
    (
        "zombie_walk14",
        MonsterFrame {
            frame: 28,
            next: "zombie_walk15",
            operations: OPS_ZOMBIE_WALK14,
        },
    ),
    (
        "zombie_walk15",
        MonsterFrame {
            frame: 29,
            next: "zombie_walk16",
            operations: OPS_ZOMBIE_WALK15,
        },
    ),
    (
        "zombie_walk16",
        MonsterFrame {
            frame: 30,
            next: "zombie_walk17",
            operations: OPS_ZOMBIE_WALK16,
        },
    ),
    (
        "zombie_walk17",
        MonsterFrame {
            frame: 31,
            next: "zombie_walk18",
            operations: OPS_ZOMBIE_WALK17,
        },
    ),
    (
        "zombie_walk18",
        MonsterFrame {
            frame: 32,
            next: "zombie_walk19",
            operations: OPS_ZOMBIE_WALK18,
        },
    ),
    (
        "zombie_walk19",
        MonsterFrame {
            frame: 33,
            next: "zombie_walk1",
            operations: OPS_ZOMBIE_WALK19,
        },
    ),
    (
        "zombie_walk2",
        MonsterFrame {
            frame: 16,
            next: "zombie_walk3",
            operations: OPS_ZOMBIE_WALK2,
        },
    ),
    (
        "zombie_walk3",
        MonsterFrame {
            frame: 17,
            next: "zombie_walk4",
            operations: OPS_ZOMBIE_WALK3,
        },
    ),
    (
        "zombie_walk4",
        MonsterFrame {
            frame: 18,
            next: "zombie_walk5",
            operations: OPS_ZOMBIE_WALK4,
        },
    ),
    (
        "zombie_walk5",
        MonsterFrame {
            frame: 19,
            next: "zombie_walk6",
            operations: OPS_ZOMBIE_WALK5,
        },
    ),
    (
        "zombie_walk6",
        MonsterFrame {
            frame: 20,
            next: "zombie_walk7",
            operations: OPS_ZOMBIE_WALK6,
        },
    ),
    (
        "zombie_walk7",
        MonsterFrame {
            frame: 21,
            next: "zombie_walk8",
            operations: OPS_ZOMBIE_WALK7,
        },
    ),
    (
        "zombie_walk8",
        MonsterFrame {
            frame: 22,
            next: "zombie_walk9",
            operations: OPS_ZOMBIE_WALK8,
        },
    ),
    (
        "zombie_walk9",
        MonsterFrame {
            frame: 23,
            next: "zombie_walk10",
            operations: OPS_ZOMBIE_WALK9,
        },
    ),
];

/// Look up a monster frame by name.
#[must_use]
pub fn monster_frame(name: &str) -> Option<&'static MonsterFrame> {
    MONSTER_FRAMES
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| &MONSTER_FRAMES[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_cover_all_continuations() {
        assert!(MONSTER_FRAMES.len() > 1300);
        let mut names: Vec<&str> = MONSTER_FRAMES.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        assert!(MONSTER_FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for (_, frame) in MONSTER_FRAMES {
            assert!(monster_frame(frame.next).is_some(), "dangling {}", frame.next);
            for op in frame.operations {
                if let MonsterOperation::Sound {
                    chance: Some(chance), ..
                } = op
                {
                    assert!((0.0..=1.0).contains(chance));
                }
            }
        }
        let stand = monster_frame("knight_stand1").expect("knight stand");
        assert_eq!((stand.frame, stand.next), (0, "knight_stand2"));
        let thrash = monster_frame("old_thrash20").expect("thrash end");
        assert_eq!(thrash.next, "old_thrash20");
    }
}
