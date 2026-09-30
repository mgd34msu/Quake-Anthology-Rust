//! Rogue mummy frames (src/content/q1/missionpacks/monsters/tables/mummy.ts).
//!
//! quakec_rogue/mummy.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation, SoundComparison};
use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};

static OPS_MUMMY_ATTA1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mummy:mummy_atta13",
}];
static OPS_MUMMY_ATTA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTA9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mummy:mummy_attb14",
}];
static OPS_MUMMY_ATTB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTB9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mummy:mummy_attc12",
}];
static OPS_MUMMY_ATTC2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_ATTC9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Face,
    distance: 0.0,
}];
static OPS_MUMMY_PAINA1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_MUMMY_PAINA10: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINA11: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINA12: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINA2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 3.0,
}];
static OPS_MUMMY_PAINA3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_MUMMY_PAINA4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINA5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 3.0,
}];
static OPS_MUMMY_PAINA6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINA7: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINA8: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINA9: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_MUMMY_PAINB10: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB11: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB12: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB13: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB14: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB15: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB16: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB17: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB18: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB19: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_MUMMY_PAINB20: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB21: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB22: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB23: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB24: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB25: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_MUMMY_PAINB26: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB27: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB28: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 8.0,
}];
static OPS_MUMMY_PAINB4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 6.0,
}];
static OPS_MUMMY_PAINB5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_MUMMY_PAINB6: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB7: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB8: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINB9: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_fall.wav",
    channel: Q1SoundChannel::Body,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_MUMMY_PAINC1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain1.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_MUMMY_PAINC10: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_MUMMY_PAINC12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_MUMMY_PAINC13: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC14: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC15: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC16: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC17: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC18: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC2: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 3.0,
}];
static OPS_MUMMY_PAINC4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINC5: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC6: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC7: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC8: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINC9: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_MUMMY_PAIND10: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND11: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND12: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND13: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND2: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND3: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND4: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND5: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND6: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND7: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND8: &[MonsterOperation] = &[];
static OPS_MUMMY_PAIND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINE1: &[MonsterOperation] = &[MonsterOperation::Sound {
    path: "zombie/z_pain.wav",
    channel: Q1SoundChannel::Voice,
    attenuation: 1.0,
    comparison: SoundComparison::Less,
    chance: None,
}];
static OPS_MUMMY_PAINE10: &[MonsterOperation] = &[
    MonsterOperation::Sound {
        path: "zombie/z_fall.wav",
        channel: Q1SoundChannel::Body,
        attenuation: 1.0,
        comparison: SoundComparison::Less,
        chance: None,
    },
    MonsterOperation::Solid { solid: Q1Solid::None },
];
static OPS_MUMMY_PAINE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mummy:mummy_paine11",
}];
static OPS_MUMMY_PAINE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mummy:mummy_paine12",
}];
static OPS_MUMMY_PAINE13: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE14: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE15: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE16: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE17: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE18: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE19: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 8.0,
}];
static OPS_MUMMY_PAINE20: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE21: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE22: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE23: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE24: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE25: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 5.0,
}];
static OPS_MUMMY_PAINE26: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 3.0,
}];
static OPS_MUMMY_PAINE27: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Painforward,
    distance: 1.0,
}];
static OPS_MUMMY_PAINE28: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINE29: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 5.0,
}];
static OPS_MUMMY_PAINE30: &[MonsterOperation] = &[];
static OPS_MUMMY_PAINE4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 3.0,
}];
static OPS_MUMMY_PAINE5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINE6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_MUMMY_PAINE7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINE8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 1.0,
}];
static OPS_MUMMY_PAINE9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Pain,
    distance: 2.0,
}];
static OPS_MUMMY_RUN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "mummy:mummy_run1",
}];
static OPS_MUMMY_RUN10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_MUMMY_RUN11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_MUMMY_RUN12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_MUMMY_RUN13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_MUMMY_RUN14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_MUMMY_RUN15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 12.0,
}];
static OPS_MUMMY_RUN16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 14.0,
}];
static OPS_MUMMY_RUN17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 6.0,
}];
static OPS_MUMMY_RUN18: &[MonsterOperation] = &[
    MonsterOperation::Ai {
        mode: MonsterAi::Run,
        distance: 16.0,
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
static OPS_MUMMY_RUN2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_MUMMY_RUN3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 0.0,
}];
static OPS_MUMMY_RUN4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 2.0,
}];
static OPS_MUMMY_RUN5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_MUMMY_RUN6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 6.0,
}];
static OPS_MUMMY_RUN7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_MUMMY_RUN8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 8.0,
}];
static OPS_MUMMY_RUN9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Run,
    distance: 4.0,
}];
static OPS_MUMMY_SLEEP: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_STAND9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Stand,
    distance: 0.0,
}];
static OPS_MUMMY_WALK1: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK10: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK11: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_MUMMY_WALK12: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_MUMMY_WALK13: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_MUMMY_WALK14: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK15: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK16: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK17: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK18: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK19: &[MonsterOperation] = &[
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
static OPS_MUMMY_WALK2: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 3.0,
}];
static OPS_MUMMY_WALK3: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 5.0,
}];
static OPS_MUMMY_WALK4: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 4.0,
}];
static OPS_MUMMY_WALK5: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 2.0,
}];
static OPS_MUMMY_WALK6: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK7: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK8: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];
static OPS_MUMMY_WALK9: &[MonsterOperation] = &[MonsterOperation::Ai {
    mode: MonsterAi::Walk,
    distance: 0.0,
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "mummy_atta1",
        MonsterFrame {
            frame: 52,
            next: "mummy_atta2",
            operations: OPS_MUMMY_ATTA1,
        },
    ),
    (
        "mummy_atta10",
        MonsterFrame {
            frame: 61,
            next: "mummy_atta11",
            operations: OPS_MUMMY_ATTA10,
        },
    ),
    (
        "mummy_atta11",
        MonsterFrame {
            frame: 62,
            next: "mummy_atta12",
            operations: OPS_MUMMY_ATTA11,
        },
    ),
    (
        "mummy_atta12",
        MonsterFrame {
            frame: 63,
            next: "mummy_atta13",
            operations: OPS_MUMMY_ATTA12,
        },
    ),
    (
        "mummy_atta13",
        MonsterFrame {
            frame: 64,
            next: "mummy_run1",
            operations: OPS_MUMMY_ATTA13,
        },
    ),
    (
        "mummy_atta2",
        MonsterFrame {
            frame: 53,
            next: "mummy_atta3",
            operations: OPS_MUMMY_ATTA2,
        },
    ),
    (
        "mummy_atta3",
        MonsterFrame {
            frame: 54,
            next: "mummy_atta4",
            operations: OPS_MUMMY_ATTA3,
        },
    ),
    (
        "mummy_atta4",
        MonsterFrame {
            frame: 55,
            next: "mummy_atta5",
            operations: OPS_MUMMY_ATTA4,
        },
    ),
    (
        "mummy_atta5",
        MonsterFrame {
            frame: 56,
            next: "mummy_atta6",
            operations: OPS_MUMMY_ATTA5,
        },
    ),
    (
        "mummy_atta6",
        MonsterFrame {
            frame: 57,
            next: "mummy_atta7",
            operations: OPS_MUMMY_ATTA6,
        },
    ),
    (
        "mummy_atta7",
        MonsterFrame {
            frame: 58,
            next: "mummy_atta8",
            operations: OPS_MUMMY_ATTA7,
        },
    ),
    (
        "mummy_atta8",
        MonsterFrame {
            frame: 59,
            next: "mummy_atta9",
            operations: OPS_MUMMY_ATTA8,
        },
    ),
    (
        "mummy_atta9",
        MonsterFrame {
            frame: 60,
            next: "mummy_atta10",
            operations: OPS_MUMMY_ATTA9,
        },
    ),
    (
        "mummy_attb1",
        MonsterFrame {
            frame: 65,
            next: "mummy_attb2",
            operations: OPS_MUMMY_ATTB1,
        },
    ),
    (
        "mummy_attb10",
        MonsterFrame {
            frame: 74,
            next: "mummy_attb11",
            operations: OPS_MUMMY_ATTB10,
        },
    ),
    (
        "mummy_attb11",
        MonsterFrame {
            frame: 75,
            next: "mummy_attb12",
            operations: OPS_MUMMY_ATTB11,
        },
    ),
    (
        "mummy_attb12",
        MonsterFrame {
            frame: 76,
            next: "mummy_attb13",
            operations: OPS_MUMMY_ATTB12,
        },
    ),
    (
        "mummy_attb13",
        MonsterFrame {
            frame: 77,
            next: "mummy_attb14",
            operations: OPS_MUMMY_ATTB13,
        },
    ),
    (
        "mummy_attb14",
        MonsterFrame {
            frame: 77,
            next: "mummy_run1",
            operations: OPS_MUMMY_ATTB14,
        },
    ),
    (
        "mummy_attb2",
        MonsterFrame {
            frame: 66,
            next: "mummy_attb3",
            operations: OPS_MUMMY_ATTB2,
        },
    ),
    (
        "mummy_attb3",
        MonsterFrame {
            frame: 67,
            next: "mummy_attb4",
            operations: OPS_MUMMY_ATTB3,
        },
    ),
    (
        "mummy_attb4",
        MonsterFrame {
            frame: 68,
            next: "mummy_attb5",
            operations: OPS_MUMMY_ATTB4,
        },
    ),
    (
        "mummy_attb5",
        MonsterFrame {
            frame: 69,
            next: "mummy_attb6",
            operations: OPS_MUMMY_ATTB5,
        },
    ),
    (
        "mummy_attb6",
        MonsterFrame {
            frame: 70,
            next: "mummy_attb7",
            operations: OPS_MUMMY_ATTB6,
        },
    ),
    (
        "mummy_attb7",
        MonsterFrame {
            frame: 71,
            next: "mummy_attb8",
            operations: OPS_MUMMY_ATTB7,
        },
    ),
    (
        "mummy_attb8",
        MonsterFrame {
            frame: 72,
            next: "mummy_attb9",
            operations: OPS_MUMMY_ATTB8,
        },
    ),
    (
        "mummy_attb9",
        MonsterFrame {
            frame: 73,
            next: "mummy_attb10",
            operations: OPS_MUMMY_ATTB9,
        },
    ),
    (
        "mummy_attc1",
        MonsterFrame {
            frame: 79,
            next: "mummy_attc2",
            operations: OPS_MUMMY_ATTC1,
        },
    ),
    (
        "mummy_attc10",
        MonsterFrame {
            frame: 88,
            next: "mummy_attc11",
            operations: OPS_MUMMY_ATTC10,
        },
    ),
    (
        "mummy_attc11",
        MonsterFrame {
            frame: 89,
            next: "mummy_attc12",
            operations: OPS_MUMMY_ATTC11,
        },
    ),
    (
        "mummy_attc12",
        MonsterFrame {
            frame: 90,
            next: "mummy_run1",
            operations: OPS_MUMMY_ATTC12,
        },
    ),
    (
        "mummy_attc2",
        MonsterFrame {
            frame: 80,
            next: "mummy_attc3",
            operations: OPS_MUMMY_ATTC2,
        },
    ),
    (
        "mummy_attc3",
        MonsterFrame {
            frame: 81,
            next: "mummy_attc4",
            operations: OPS_MUMMY_ATTC3,
        },
    ),
    (
        "mummy_attc4",
        MonsterFrame {
            frame: 82,
            next: "mummy_attc5",
            operations: OPS_MUMMY_ATTC4,
        },
    ),
    (
        "mummy_attc5",
        MonsterFrame {
            frame: 83,
            next: "mummy_attc6",
            operations: OPS_MUMMY_ATTC5,
        },
    ),
    (
        "mummy_attc6",
        MonsterFrame {
            frame: 84,
            next: "mummy_attc7",
            operations: OPS_MUMMY_ATTC6,
        },
    ),
    (
        "mummy_attc7",
        MonsterFrame {
            frame: 85,
            next: "mummy_attc8",
            operations: OPS_MUMMY_ATTC7,
        },
    ),
    (
        "mummy_attc8",
        MonsterFrame {
            frame: 86,
            next: "mummy_attc9",
            operations: OPS_MUMMY_ATTC8,
        },
    ),
    (
        "mummy_attc9",
        MonsterFrame {
            frame: 87,
            next: "mummy_attc10",
            operations: OPS_MUMMY_ATTC9,
        },
    ),
    (
        "mummy_paina1",
        MonsterFrame {
            frame: 91,
            next: "mummy_paina2",
            operations: OPS_MUMMY_PAINA1,
        },
    ),
    (
        "mummy_paina10",
        MonsterFrame {
            frame: 100,
            next: "mummy_paina11",
            operations: OPS_MUMMY_PAINA10,
        },
    ),
    (
        "mummy_paina11",
        MonsterFrame {
            frame: 101,
            next: "mummy_paina12",
            operations: OPS_MUMMY_PAINA11,
        },
    ),
    (
        "mummy_paina12",
        MonsterFrame {
            frame: 102,
            next: "mummy_run1",
            operations: OPS_MUMMY_PAINA12,
        },
    ),
    (
        "mummy_paina2",
        MonsterFrame {
            frame: 92,
            next: "mummy_paina3",
            operations: OPS_MUMMY_PAINA2,
        },
    ),
    (
        "mummy_paina3",
        MonsterFrame {
            frame: 93,
            next: "mummy_paina4",
            operations: OPS_MUMMY_PAINA3,
        },
    ),
    (
        "mummy_paina4",
        MonsterFrame {
            frame: 94,
            next: "mummy_paina5",
            operations: OPS_MUMMY_PAINA4,
        },
    ),
    (
        "mummy_paina5",
        MonsterFrame {
            frame: 95,
            next: "mummy_paina6",
            operations: OPS_MUMMY_PAINA5,
        },
    ),
    (
        "mummy_paina6",
        MonsterFrame {
            frame: 96,
            next: "mummy_paina7",
            operations: OPS_MUMMY_PAINA6,
        },
    ),
    (
        "mummy_paina7",
        MonsterFrame {
            frame: 97,
            next: "mummy_paina8",
            operations: OPS_MUMMY_PAINA7,
        },
    ),
    (
        "mummy_paina8",
        MonsterFrame {
            frame: 98,
            next: "mummy_paina9",
            operations: OPS_MUMMY_PAINA8,
        },
    ),
    (
        "mummy_paina9",
        MonsterFrame {
            frame: 99,
            next: "mummy_paina10",
            operations: OPS_MUMMY_PAINA9,
        },
    ),
    (
        "mummy_painb1",
        MonsterFrame {
            frame: 103,
            next: "mummy_painb2",
            operations: OPS_MUMMY_PAINB1,
        },
    ),
    (
        "mummy_painb10",
        MonsterFrame {
            frame: 112,
            next: "mummy_painb11",
            operations: OPS_MUMMY_PAINB10,
        },
    ),
    (
        "mummy_painb11",
        MonsterFrame {
            frame: 113,
            next: "mummy_painb12",
            operations: OPS_MUMMY_PAINB11,
        },
    ),
    (
        "mummy_painb12",
        MonsterFrame {
            frame: 114,
            next: "mummy_painb13",
            operations: OPS_MUMMY_PAINB12,
        },
    ),
    (
        "mummy_painb13",
        MonsterFrame {
            frame: 115,
            next: "mummy_painb14",
            operations: OPS_MUMMY_PAINB13,
        },
    ),
    (
        "mummy_painb14",
        MonsterFrame {
            frame: 116,
            next: "mummy_painb15",
            operations: OPS_MUMMY_PAINB14,
        },
    ),
    (
        "mummy_painb15",
        MonsterFrame {
            frame: 117,
            next: "mummy_painb16",
            operations: OPS_MUMMY_PAINB15,
        },
    ),
    (
        "mummy_painb16",
        MonsterFrame {
            frame: 118,
            next: "mummy_painb17",
            operations: OPS_MUMMY_PAINB16,
        },
    ),
    (
        "mummy_painb17",
        MonsterFrame {
            frame: 119,
            next: "mummy_painb18",
            operations: OPS_MUMMY_PAINB17,
        },
    ),
    (
        "mummy_painb18",
        MonsterFrame {
            frame: 120,
            next: "mummy_painb19",
            operations: OPS_MUMMY_PAINB18,
        },
    ),
    (
        "mummy_painb19",
        MonsterFrame {
            frame: 121,
            next: "mummy_painb20",
            operations: OPS_MUMMY_PAINB19,
        },
    ),
    (
        "mummy_painb2",
        MonsterFrame {
            frame: 104,
            next: "mummy_painb3",
            operations: OPS_MUMMY_PAINB2,
        },
    ),
    (
        "mummy_painb20",
        MonsterFrame {
            frame: 122,
            next: "mummy_painb21",
            operations: OPS_MUMMY_PAINB20,
        },
    ),
    (
        "mummy_painb21",
        MonsterFrame {
            frame: 123,
            next: "mummy_painb22",
            operations: OPS_MUMMY_PAINB21,
        },
    ),
    (
        "mummy_painb22",
        MonsterFrame {
            frame: 124,
            next: "mummy_painb23",
            operations: OPS_MUMMY_PAINB22,
        },
    ),
    (
        "mummy_painb23",
        MonsterFrame {
            frame: 125,
            next: "mummy_painb24",
            operations: OPS_MUMMY_PAINB23,
        },
    ),
    (
        "mummy_painb24",
        MonsterFrame {
            frame: 126,
            next: "mummy_painb25",
            operations: OPS_MUMMY_PAINB24,
        },
    ),
    (
        "mummy_painb25",
        MonsterFrame {
            frame: 127,
            next: "mummy_painb26",
            operations: OPS_MUMMY_PAINB25,
        },
    ),
    (
        "mummy_painb26",
        MonsterFrame {
            frame: 128,
            next: "mummy_painb27",
            operations: OPS_MUMMY_PAINB26,
        },
    ),
    (
        "mummy_painb27",
        MonsterFrame {
            frame: 129,
            next: "mummy_painb28",
            operations: OPS_MUMMY_PAINB27,
        },
    ),
    (
        "mummy_painb28",
        MonsterFrame {
            frame: 130,
            next: "mummy_run1",
            operations: OPS_MUMMY_PAINB28,
        },
    ),
    (
        "mummy_painb3",
        MonsterFrame {
            frame: 105,
            next: "mummy_painb4",
            operations: OPS_MUMMY_PAINB3,
        },
    ),
    (
        "mummy_painb4",
        MonsterFrame {
            frame: 106,
            next: "mummy_painb5",
            operations: OPS_MUMMY_PAINB4,
        },
    ),
    (
        "mummy_painb5",
        MonsterFrame {
            frame: 107,
            next: "mummy_painb6",
            operations: OPS_MUMMY_PAINB5,
        },
    ),
    (
        "mummy_painb6",
        MonsterFrame {
            frame: 108,
            next: "mummy_painb7",
            operations: OPS_MUMMY_PAINB6,
        },
    ),
    (
        "mummy_painb7",
        MonsterFrame {
            frame: 109,
            next: "mummy_painb8",
            operations: OPS_MUMMY_PAINB7,
        },
    ),
    (
        "mummy_painb8",
        MonsterFrame {
            frame: 110,
            next: "mummy_painb9",
            operations: OPS_MUMMY_PAINB8,
        },
    ),
    (
        "mummy_painb9",
        MonsterFrame {
            frame: 111,
            next: "mummy_painb10",
            operations: OPS_MUMMY_PAINB9,
        },
    ),
    (
        "mummy_painc1",
        MonsterFrame {
            frame: 131,
            next: "mummy_painc2",
            operations: OPS_MUMMY_PAINC1,
        },
    ),
    (
        "mummy_painc10",
        MonsterFrame {
            frame: 140,
            next: "mummy_painc11",
            operations: OPS_MUMMY_PAINC10,
        },
    ),
    (
        "mummy_painc11",
        MonsterFrame {
            frame: 141,
            next: "mummy_painc12",
            operations: OPS_MUMMY_PAINC11,
        },
    ),
    (
        "mummy_painc12",
        MonsterFrame {
            frame: 142,
            next: "mummy_painc13",
            operations: OPS_MUMMY_PAINC12,
        },
    ),
    (
        "mummy_painc13",
        MonsterFrame {
            frame: 143,
            next: "mummy_painc14",
            operations: OPS_MUMMY_PAINC13,
        },
    ),
    (
        "mummy_painc14",
        MonsterFrame {
            frame: 144,
            next: "mummy_painc15",
            operations: OPS_MUMMY_PAINC14,
        },
    ),
    (
        "mummy_painc15",
        MonsterFrame {
            frame: 145,
            next: "mummy_painc16",
            operations: OPS_MUMMY_PAINC15,
        },
    ),
    (
        "mummy_painc16",
        MonsterFrame {
            frame: 146,
            next: "mummy_painc17",
            operations: OPS_MUMMY_PAINC16,
        },
    ),
    (
        "mummy_painc17",
        MonsterFrame {
            frame: 147,
            next: "mummy_painc18",
            operations: OPS_MUMMY_PAINC17,
        },
    ),
    (
        "mummy_painc18",
        MonsterFrame {
            frame: 148,
            next: "mummy_run1",
            operations: OPS_MUMMY_PAINC18,
        },
    ),
    (
        "mummy_painc2",
        MonsterFrame {
            frame: 132,
            next: "mummy_painc3",
            operations: OPS_MUMMY_PAINC2,
        },
    ),
    (
        "mummy_painc3",
        MonsterFrame {
            frame: 133,
            next: "mummy_painc4",
            operations: OPS_MUMMY_PAINC3,
        },
    ),
    (
        "mummy_painc4",
        MonsterFrame {
            frame: 134,
            next: "mummy_painc5",
            operations: OPS_MUMMY_PAINC4,
        },
    ),
    (
        "mummy_painc5",
        MonsterFrame {
            frame: 135,
            next: "mummy_painc6",
            operations: OPS_MUMMY_PAINC5,
        },
    ),
    (
        "mummy_painc6",
        MonsterFrame {
            frame: 136,
            next: "mummy_painc7",
            operations: OPS_MUMMY_PAINC6,
        },
    ),
    (
        "mummy_painc7",
        MonsterFrame {
            frame: 137,
            next: "mummy_painc8",
            operations: OPS_MUMMY_PAINC7,
        },
    ),
    (
        "mummy_painc8",
        MonsterFrame {
            frame: 138,
            next: "mummy_painc9",
            operations: OPS_MUMMY_PAINC8,
        },
    ),
    (
        "mummy_painc9",
        MonsterFrame {
            frame: 139,
            next: "mummy_painc10",
            operations: OPS_MUMMY_PAINC9,
        },
    ),
    (
        "mummy_paind1",
        MonsterFrame {
            frame: 149,
            next: "mummy_paind2",
            operations: OPS_MUMMY_PAIND1,
        },
    ),
    (
        "mummy_paind10",
        MonsterFrame {
            frame: 158,
            next: "mummy_paind11",
            operations: OPS_MUMMY_PAIND10,
        },
    ),
    (
        "mummy_paind11",
        MonsterFrame {
            frame: 159,
            next: "mummy_paind12",
            operations: OPS_MUMMY_PAIND11,
        },
    ),
    (
        "mummy_paind12",
        MonsterFrame {
            frame: 160,
            next: "mummy_paind13",
            operations: OPS_MUMMY_PAIND12,
        },
    ),
    (
        "mummy_paind13",
        MonsterFrame {
            frame: 161,
            next: "mummy_run1",
            operations: OPS_MUMMY_PAIND13,
        },
    ),
    (
        "mummy_paind2",
        MonsterFrame {
            frame: 150,
            next: "mummy_paind3",
            operations: OPS_MUMMY_PAIND2,
        },
    ),
    (
        "mummy_paind3",
        MonsterFrame {
            frame: 151,
            next: "mummy_paind4",
            operations: OPS_MUMMY_PAIND3,
        },
    ),
    (
        "mummy_paind4",
        MonsterFrame {
            frame: 152,
            next: "mummy_paind5",
            operations: OPS_MUMMY_PAIND4,
        },
    ),
    (
        "mummy_paind5",
        MonsterFrame {
            frame: 153,
            next: "mummy_paind6",
            operations: OPS_MUMMY_PAIND5,
        },
    ),
    (
        "mummy_paind6",
        MonsterFrame {
            frame: 154,
            next: "mummy_paind7",
            operations: OPS_MUMMY_PAIND6,
        },
    ),
    (
        "mummy_paind7",
        MonsterFrame {
            frame: 155,
            next: "mummy_paind8",
            operations: OPS_MUMMY_PAIND7,
        },
    ),
    (
        "mummy_paind8",
        MonsterFrame {
            frame: 156,
            next: "mummy_paind9",
            operations: OPS_MUMMY_PAIND8,
        },
    ),
    (
        "mummy_paind9",
        MonsterFrame {
            frame: 157,
            next: "mummy_paind10",
            operations: OPS_MUMMY_PAIND9,
        },
    ),
    (
        "mummy_paine1",
        MonsterFrame {
            frame: 162,
            next: "mummy_paine2",
            operations: OPS_MUMMY_PAINE1,
        },
    ),
    (
        "mummy_paine10",
        MonsterFrame {
            frame: 171,
            next: "mummy_paine11",
            operations: OPS_MUMMY_PAINE10,
        },
    ),
    (
        "mummy_paine11",
        MonsterFrame {
            frame: 172,
            next: "mummy_paine12",
            operations: OPS_MUMMY_PAINE11,
        },
    ),
    (
        "mummy_paine12",
        MonsterFrame {
            frame: 173,
            next: "mummy_paine13",
            operations: OPS_MUMMY_PAINE12,
        },
    ),
    (
        "mummy_paine13",
        MonsterFrame {
            frame: 174,
            next: "mummy_paine14",
            operations: OPS_MUMMY_PAINE13,
        },
    ),
    (
        "mummy_paine14",
        MonsterFrame {
            frame: 175,
            next: "mummy_paine15",
            operations: OPS_MUMMY_PAINE14,
        },
    ),
    (
        "mummy_paine15",
        MonsterFrame {
            frame: 176,
            next: "mummy_paine16",
            operations: OPS_MUMMY_PAINE15,
        },
    ),
    (
        "mummy_paine16",
        MonsterFrame {
            frame: 177,
            next: "mummy_paine17",
            operations: OPS_MUMMY_PAINE16,
        },
    ),
    (
        "mummy_paine17",
        MonsterFrame {
            frame: 178,
            next: "mummy_paine18",
            operations: OPS_MUMMY_PAINE17,
        },
    ),
    (
        "mummy_paine18",
        MonsterFrame {
            frame: 179,
            next: "mummy_paine19",
            operations: OPS_MUMMY_PAINE18,
        },
    ),
    (
        "mummy_paine19",
        MonsterFrame {
            frame: 180,
            next: "mummy_paine20",
            operations: OPS_MUMMY_PAINE19,
        },
    ),
    (
        "mummy_paine2",
        MonsterFrame {
            frame: 163,
            next: "mummy_paine3",
            operations: OPS_MUMMY_PAINE2,
        },
    ),
    (
        "mummy_paine20",
        MonsterFrame {
            frame: 181,
            next: "mummy_paine21",
            operations: OPS_MUMMY_PAINE20,
        },
    ),
    (
        "mummy_paine21",
        MonsterFrame {
            frame: 182,
            next: "mummy_paine22",
            operations: OPS_MUMMY_PAINE21,
        },
    ),
    (
        "mummy_paine22",
        MonsterFrame {
            frame: 183,
            next: "mummy_paine23",
            operations: OPS_MUMMY_PAINE22,
        },
    ),
    (
        "mummy_paine23",
        MonsterFrame {
            frame: 184,
            next: "mummy_paine24",
            operations: OPS_MUMMY_PAINE23,
        },
    ),
    (
        "mummy_paine24",
        MonsterFrame {
            frame: 185,
            next: "mummy_paine25",
            operations: OPS_MUMMY_PAINE24,
        },
    ),
    (
        "mummy_paine25",
        MonsterFrame {
            frame: 186,
            next: "mummy_paine26",
            operations: OPS_MUMMY_PAINE25,
        },
    ),
    (
        "mummy_paine26",
        MonsterFrame {
            frame: 187,
            next: "mummy_paine27",
            operations: OPS_MUMMY_PAINE26,
        },
    ),
    (
        "mummy_paine27",
        MonsterFrame {
            frame: 188,
            next: "mummy_paine28",
            operations: OPS_MUMMY_PAINE27,
        },
    ),
    (
        "mummy_paine28",
        MonsterFrame {
            frame: 189,
            next: "mummy_paine29",
            operations: OPS_MUMMY_PAINE28,
        },
    ),
    (
        "mummy_paine29",
        MonsterFrame {
            frame: 190,
            next: "mummy_paine30",
            operations: OPS_MUMMY_PAINE29,
        },
    ),
    (
        "mummy_paine3",
        MonsterFrame {
            frame: 164,
            next: "mummy_paine4",
            operations: OPS_MUMMY_PAINE3,
        },
    ),
    (
        "mummy_paine30",
        MonsterFrame {
            frame: 191,
            next: "mummy_run1",
            operations: OPS_MUMMY_PAINE30,
        },
    ),
    (
        "mummy_paine4",
        MonsterFrame {
            frame: 165,
            next: "mummy_paine5",
            operations: OPS_MUMMY_PAINE4,
        },
    ),
    (
        "mummy_paine5",
        MonsterFrame {
            frame: 166,
            next: "mummy_paine6",
            operations: OPS_MUMMY_PAINE5,
        },
    ),
    (
        "mummy_paine6",
        MonsterFrame {
            frame: 167,
            next: "mummy_paine7",
            operations: OPS_MUMMY_PAINE6,
        },
    ),
    (
        "mummy_paine7",
        MonsterFrame {
            frame: 168,
            next: "mummy_paine8",
            operations: OPS_MUMMY_PAINE7,
        },
    ),
    (
        "mummy_paine8",
        MonsterFrame {
            frame: 169,
            next: "mummy_paine9",
            operations: OPS_MUMMY_PAINE8,
        },
    ),
    (
        "mummy_paine9",
        MonsterFrame {
            frame: 170,
            next: "mummy_paine10",
            operations: OPS_MUMMY_PAINE9,
        },
    ),
    (
        "mummy_run1",
        MonsterFrame {
            frame: 34,
            next: "mummy_run2",
            operations: OPS_MUMMY_RUN1,
        },
    ),
    (
        "mummy_run10",
        MonsterFrame {
            frame: 43,
            next: "mummy_run11",
            operations: OPS_MUMMY_RUN10,
        },
    ),
    (
        "mummy_run11",
        MonsterFrame {
            frame: 44,
            next: "mummy_run12",
            operations: OPS_MUMMY_RUN11,
        },
    ),
    (
        "mummy_run12",
        MonsterFrame {
            frame: 45,
            next: "mummy_run13",
            operations: OPS_MUMMY_RUN12,
        },
    ),
    (
        "mummy_run13",
        MonsterFrame {
            frame: 46,
            next: "mummy_run14",
            operations: OPS_MUMMY_RUN13,
        },
    ),
    (
        "mummy_run14",
        MonsterFrame {
            frame: 47,
            next: "mummy_run15",
            operations: OPS_MUMMY_RUN14,
        },
    ),
    (
        "mummy_run15",
        MonsterFrame {
            frame: 48,
            next: "mummy_run16",
            operations: OPS_MUMMY_RUN15,
        },
    ),
    (
        "mummy_run16",
        MonsterFrame {
            frame: 49,
            next: "mummy_run17",
            operations: OPS_MUMMY_RUN16,
        },
    ),
    (
        "mummy_run17",
        MonsterFrame {
            frame: 50,
            next: "mummy_run18",
            operations: OPS_MUMMY_RUN17,
        },
    ),
    (
        "mummy_run18",
        MonsterFrame {
            frame: 51,
            next: "mummy_run1",
            operations: OPS_MUMMY_RUN18,
        },
    ),
    (
        "mummy_run2",
        MonsterFrame {
            frame: 35,
            next: "mummy_run3",
            operations: OPS_MUMMY_RUN2,
        },
    ),
    (
        "mummy_run3",
        MonsterFrame {
            frame: 36,
            next: "mummy_run4",
            operations: OPS_MUMMY_RUN3,
        },
    ),
    (
        "mummy_run4",
        MonsterFrame {
            frame: 37,
            next: "mummy_run5",
            operations: OPS_MUMMY_RUN4,
        },
    ),
    (
        "mummy_run5",
        MonsterFrame {
            frame: 38,
            next: "mummy_run6",
            operations: OPS_MUMMY_RUN5,
        },
    ),
    (
        "mummy_run6",
        MonsterFrame {
            frame: 39,
            next: "mummy_run7",
            operations: OPS_MUMMY_RUN6,
        },
    ),
    (
        "mummy_run7",
        MonsterFrame {
            frame: 40,
            next: "mummy_run8",
            operations: OPS_MUMMY_RUN7,
        },
    ),
    (
        "mummy_run8",
        MonsterFrame {
            frame: 41,
            next: "mummy_run9",
            operations: OPS_MUMMY_RUN8,
        },
    ),
    (
        "mummy_run9",
        MonsterFrame {
            frame: 42,
            next: "mummy_run10",
            operations: OPS_MUMMY_RUN9,
        },
    ),
    (
        "mummy_sleep",
        MonsterFrame {
            frame: 172,
            next: "mummy_sleep",
            operations: OPS_MUMMY_SLEEP,
        },
    ),
    (
        "mummy_stand1",
        MonsterFrame {
            frame: 0,
            next: "mummy_stand2",
            operations: OPS_MUMMY_STAND1,
        },
    ),
    (
        "mummy_stand10",
        MonsterFrame {
            frame: 9,
            next: "mummy_stand11",
            operations: OPS_MUMMY_STAND10,
        },
    ),
    (
        "mummy_stand11",
        MonsterFrame {
            frame: 10,
            next: "mummy_stand12",
            operations: OPS_MUMMY_STAND11,
        },
    ),
    (
        "mummy_stand12",
        MonsterFrame {
            frame: 11,
            next: "mummy_stand13",
            operations: OPS_MUMMY_STAND12,
        },
    ),
    (
        "mummy_stand13",
        MonsterFrame {
            frame: 12,
            next: "mummy_stand14",
            operations: OPS_MUMMY_STAND13,
        },
    ),
    (
        "mummy_stand14",
        MonsterFrame {
            frame: 13,
            next: "mummy_stand15",
            operations: OPS_MUMMY_STAND14,
        },
    ),
    (
        "mummy_stand15",
        MonsterFrame {
            frame: 14,
            next: "mummy_stand1",
            operations: OPS_MUMMY_STAND15,
        },
    ),
    (
        "mummy_stand2",
        MonsterFrame {
            frame: 1,
            next: "mummy_stand3",
            operations: OPS_MUMMY_STAND2,
        },
    ),
    (
        "mummy_stand3",
        MonsterFrame {
            frame: 2,
            next: "mummy_stand4",
            operations: OPS_MUMMY_STAND3,
        },
    ),
    (
        "mummy_stand4",
        MonsterFrame {
            frame: 3,
            next: "mummy_stand5",
            operations: OPS_MUMMY_STAND4,
        },
    ),
    (
        "mummy_stand5",
        MonsterFrame {
            frame: 4,
            next: "mummy_stand6",
            operations: OPS_MUMMY_STAND5,
        },
    ),
    (
        "mummy_stand6",
        MonsterFrame {
            frame: 5,
            next: "mummy_stand7",
            operations: OPS_MUMMY_STAND6,
        },
    ),
    (
        "mummy_stand7",
        MonsterFrame {
            frame: 6,
            next: "mummy_stand8",
            operations: OPS_MUMMY_STAND7,
        },
    ),
    (
        "mummy_stand8",
        MonsterFrame {
            frame: 7,
            next: "mummy_stand9",
            operations: OPS_MUMMY_STAND8,
        },
    ),
    (
        "mummy_stand9",
        MonsterFrame {
            frame: 8,
            next: "mummy_stand10",
            operations: OPS_MUMMY_STAND9,
        },
    ),
    (
        "mummy_walk1",
        MonsterFrame {
            frame: 15,
            next: "mummy_walk2",
            operations: OPS_MUMMY_WALK1,
        },
    ),
    (
        "mummy_walk10",
        MonsterFrame {
            frame: 24,
            next: "mummy_walk11",
            operations: OPS_MUMMY_WALK10,
        },
    ),
    (
        "mummy_walk11",
        MonsterFrame {
            frame: 25,
            next: "mummy_walk12",
            operations: OPS_MUMMY_WALK11,
        },
    ),
    (
        "mummy_walk12",
        MonsterFrame {
            frame: 26,
            next: "mummy_walk13",
            operations: OPS_MUMMY_WALK12,
        },
    ),
    (
        "mummy_walk13",
        MonsterFrame {
            frame: 27,
            next: "mummy_walk14",
            operations: OPS_MUMMY_WALK13,
        },
    ),
    (
        "mummy_walk14",
        MonsterFrame {
            frame: 28,
            next: "mummy_walk15",
            operations: OPS_MUMMY_WALK14,
        },
    ),
    (
        "mummy_walk15",
        MonsterFrame {
            frame: 29,
            next: "mummy_walk16",
            operations: OPS_MUMMY_WALK15,
        },
    ),
    (
        "mummy_walk16",
        MonsterFrame {
            frame: 30,
            next: "mummy_walk17",
            operations: OPS_MUMMY_WALK16,
        },
    ),
    (
        "mummy_walk17",
        MonsterFrame {
            frame: 31,
            next: "mummy_walk18",
            operations: OPS_MUMMY_WALK17,
        },
    ),
    (
        "mummy_walk18",
        MonsterFrame {
            frame: 32,
            next: "mummy_walk19",
            operations: OPS_MUMMY_WALK18,
        },
    ),
    (
        "mummy_walk19",
        MonsterFrame {
            frame: 33,
            next: "mummy_walk1",
            operations: OPS_MUMMY_WALK19,
        },
    ),
    (
        "mummy_walk2",
        MonsterFrame {
            frame: 16,
            next: "mummy_walk3",
            operations: OPS_MUMMY_WALK2,
        },
    ),
    (
        "mummy_walk3",
        MonsterFrame {
            frame: 17,
            next: "mummy_walk4",
            operations: OPS_MUMMY_WALK3,
        },
    ),
    (
        "mummy_walk4",
        MonsterFrame {
            frame: 18,
            next: "mummy_walk5",
            operations: OPS_MUMMY_WALK4,
        },
    ),
    (
        "mummy_walk5",
        MonsterFrame {
            frame: 19,
            next: "mummy_walk6",
            operations: OPS_MUMMY_WALK5,
        },
    ),
    (
        "mummy_walk6",
        MonsterFrame {
            frame: 20,
            next: "mummy_walk7",
            operations: OPS_MUMMY_WALK6,
        },
    ),
    (
        "mummy_walk7",
        MonsterFrame {
            frame: 21,
            next: "mummy_walk8",
            operations: OPS_MUMMY_WALK7,
        },
    ),
    (
        "mummy_walk8",
        MonsterFrame {
            frame: 22,
            next: "mummy_walk9",
            operations: OPS_MUMMY_WALK8,
        },
    ),
    (
        "mummy_walk9",
        MonsterFrame {
            frame: 23,
            next: "mummy_walk10",
            operations: OPS_MUMMY_WALK9,
        },
    ),
];

/// Look up a mummy frame by name.
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
        assert_eq!(FRAMES.len(), 193);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("mummy_atta1").expect("first frame");
        assert_eq!((head.frame, head.next), (52, "mummy_atta2"));
        let tail = frame("mummy_walk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (23, "mummy_walk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
