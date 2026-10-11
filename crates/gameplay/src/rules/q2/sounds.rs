//! Quake II client cl_fx.c and q2repro client effects.c muzzle sound rules.
//! Resource paths are bound to common SoundIds at load, not at cue dispatch.
use qa_core::primitives::{IndexSpan, RuleSetId};

#[derive(Clone, Copy, Debug)]
pub struct MuzzleSound {
    pub choices: IndexSpan,
    pub channel: u16,
    pub attenuation: f32,
    pub delay: f32,
    pub full_volume: bool,
}
impl MuzzleSound {
    pub fn volume(self, rules: RuleSetId, silenced: bool) -> f32 {
        if self.full_volume || !silenced {
            1.0
        } else if rules == RuleSetId::Quake2Rerelease {
            // q2repro's subtraction narrows below the classic 0.2 literal.
            1.0 - 0.8
        } else {
            0.2
        }
    }
}
const fn sound(
    first: u32,
    count: u32,
    channel: u16,
    attenuation: f32,
    delay: f32,
    full_volume: bool,
) -> MuzzleSound {
    MuzzleSound {
        choices: IndexSpan { first, count },
        channel,
        attenuation,
        delay,
        full_volume,
    }
}
const fn monster_table(attenuation: f32) -> [[MuzzleSound; 1]; PATHS.len()] {
    let mut rows = [[sound(0, 1, 1, attenuation, 0.0, true)]; PATHS.len()];
    let mut i = 0;
    while i < rows.len() {
        rows[i][0].choices.first = i as u32;
        i += 1;
    }
    rows
}
const NORMAL: [[MuzzleSound; 1]; PATHS.len()] = monster_table(1.0);
const DISTANT: [[MuzzleSound; 1]; PATHS.len()] = monster_table(0.0);
fn one(first: u32, count: u32, attenuation: f32) -> &'static [MuzzleSound] {
    if count == 5 {
        const { &[sound(23, 5, 1, 1.0, 0.0, true)] }
    } else if attenuation == 0.0 {
        &DISTANT[first as usize]
    } else {
        &NORMAL[first as usize]
    }
}

pub const PATHS: &[&str] = &[
    "sound/weapons/blastf1a.wav",
    "sound/weapons/hyprbf1a.wav",
    "sound/weapons/machgf1b.wav",
    "sound/weapons/machgf2b.wav",
    "sound/weapons/machgf3b.wav",
    "sound/weapons/machgf4b.wav",
    "sound/weapons/machgf5b.wav",
    "sound/weapons/shotgf1b.wav",
    "sound/weapons/shotgr1b.wav",
    "sound/weapons/sshotf1b.wav",
    "sound/weapons/railgf1a.wav",
    "sound/weapons/railgr1b.wav",
    "sound/weapons/rocklf1a.wav",
    "sound/weapons/rocklr1b.wav",
    "sound/weapons/grenlf1a.wav",
    "sound/weapons/grenlr1b.wav",
    "sound/weapons/bfg__f1y.wav",
    "sound/weapons/plasshot.wav",
    "sound/weapons/rippfire.wav",
    "sound/weapons/proxlr1a.wav",
    "sound/weapons/nail1.wav",
    "sound/weapons/shotg2.wav",
    "sound/weapons/disint2.wav",
    "sound/tank/tnkatk2a.wav",
    "sound/tank/tnkatk2b.wav",
    "sound/tank/tnkatk2c.wav",
    "sound/tank/tnkatk2d.wav",
    "sound/tank/tnkatk2e.wav",
    "sound/infantry/infatck1.wav",
    "sound/soldier/solatck3.wav",
    "sound/gunner/gunatck2.wav",
    "sound/flyer/flyatck3.wav",
    "sound/soldier/solatck2.wav",
    "sound/medic/medatck1.wav",
    "sound/hover/hovatck1.wav",
    "sound/floater/fltatck1.wav",
    "sound/soldier/solatck1.wav",
    "sound/tank/tnkatck3.wav",
    "sound/chick/chkatck2.wav",
    "sound/tank/tnkatck1.wav",
    "sound/tank/rocket.wav",
    "sound/gunner/gunatck3.wav",
    "sound/makron/blaster.wav",
    "sound/boss3/xfire.wav",
    "sound/guncmdr/gcdratck2.wav",
    "sound/guncmdr/gcdratck3.wav",
];

pub fn player(rules: RuleSetId, flash: u16) -> &'static [MuzzleSound] {
    let rerelease = match rules {
        RuleSetId::Quake2 => false,
        RuleSetId::Quake2Rerelease => true,
        _ => return &[],
    };
    match flash {
        0 | 34 => const { &[sound(0, 1, 1, 1.0, 0.0, false)] },
        14 | 17 => const { &[sound(1, 1, 1, 1.0, 0.0, false)] },
        1 | 3 => const { &[sound(2, 5, 1, 1.0, 0.0, false)] },
        2 if rerelease => {
            const {
                &[
                    sound(7, 1, 1, 1.0, 0.0, false),
                    sound(8, 1, 0, 1.0, 0.35, false),
                ]
            }
        }
        2 => {
            const {
                &[
                    sound(7, 1, 1, 1.0, 0.0, false),
                    sound(8, 1, 0, 1.0, 0.1, false),
                ]
            }
        }
        13 => const { &[sound(9, 1, 1, 1.0, 0.0, false)] },
        4 => {
            const {
                &[
                    sound(2, 5, 1, 1.0, 0.0, false),
                    sound(2, 5, 1, 1.0, 0.05, false),
                ]
            }
        }
        5 => {
            const {
                &[
                    sound(2, 5, 1, 1.0, 0.0, false),
                    sound(2, 5, 1, 1.0, 0.033, false),
                    sound(2, 5, 1, 1.0, 0.066, false),
                ]
            }
        }
        6 if rerelease => {
            const {
                &[
                    sound(10, 1, 1, 1.0, 0.0, false),
                    sound(11, 1, 7, 1.0, 0.4, false),
                ]
            }
        }
        6 => const { &[sound(10, 1, 1, 1.0, 0.0, false)] },
        7 if rerelease => {
            const {
                &[
                    sound(12, 1, 1, 1.0, 0.0, false),
                    sound(13, 1, 0, 1.0, 0.15, false),
                ]
            }
        }
        7 => {
            const {
                &[
                    sound(12, 1, 1, 1.0, 0.0, false),
                    sound(13, 1, 0, 1.0, 0.1, false),
                ]
            }
        }
        8 if rerelease => {
            const {
                &[
                    sound(14, 1, 1, 1.0, 0.0, false),
                    sound(15, 1, 0, 1.0, 0.15, false),
                ]
            }
        }
        8 => {
            const {
                &[
                    sound(14, 1, 1, 1.0, 0.0, false),
                    sound(15, 1, 0, 1.0, 0.1, false),
                ]
            }
        }
        9..=11 => const { &[sound(14, 1, 1, 1.0, 0.0, true)] },
        12 => const { &[sound(16, 1, 1, 1.0, 0.0, false)] },
        18 => const { &[sound(17, 1, 1, 1.0, 0.0, false)] },
        16 => const { &[sound(18, 1, 1, 1.0, 0.0, false)] },
        31 if rerelease => {
            const {
                &[
                    sound(14, 1, 1, 1.0, 0.0, false),
                    sound(19, 1, 0, 1.0, 0.15, false),
                ]
            }
        }
        30 => const { &[sound(20, 1, 1, 1.0, 0.0, false)] },
        32 if rerelease => const { &[sound(20, 1, 1, 1.0, 0.0, false)] },
        32 => const { &[sound(21, 1, 1, 1.0, 0.0, false)] },
        35 => const { &[sound(22, 1, 1, 1.0, 0.0, false)] },
        _ => &[],
    }
}

pub fn monster(rules: RuleSetId, flash: u16) -> &'static [MuzzleSound] {
    let rerelease = match rules {
        RuleSetId::Quake2 if flash <= 210 => false,
        RuleSetId::Quake2Rerelease => true,
        _ => return &[],
    };
    match flash {
        26..=38 | 232..=239 | 260 => one(28, 1, 1.0), // MZ2_INFANTRY_MACHINEGUN_1
        43..=44 | 85 | 88 | 91 | 94 | 97 | 100 | 253 => one(29, 1, 1.0), // MZ2_SOLDIER_MACHINEGUN_1
        45..=52 => one(30, 1, 1.0),                   // MZ2_GUNNER_MACHINEGUN_1
        63..=69 | 141 => one(28, 1, 1.0),             // MZ2_ACTOR_MACHINEGUN_1
        73..=77 | 138 | 152 => one(if rerelease && flash == 74 { 31 } else { 28 }, 1, 0.0), // MZ2_BOSS2_MACHINEGUN_L1
        39..=40 | 83 | 86 | 89 | 92 | 95 | 98 | 143 | 251 => one(32, 1, 1.0), // MZ2_SOLDIER_BLASTER_1
        58..=59 => one(31, 1, 1.0),                                           // MZ2_FLYER_BLASTER_1
        60 | 265..=276 => one(33, 1, 1.0),                                    // MZ2_MEDIC_BLASTER_1
        62 | 263 => one(34, 1, 1.0),                                          // MZ2_HOVER_BLASTER_1
        82 => one(35, 1, 1.0),                                                // MZ2_FLOAT_BLASTER_1
        41..=42 | 84 | 87 | 90 | 93 | 96 | 99 | 252 => one(36, 1, 1.0), // MZ2_SOLDIER_SHOTGUN_1
        1..=3 => one(37, 1, 1.0),                                       // MZ2_TANK_BLASTER_1
        4..=22 => one(23, 5, 1.0),                                      // MZ2_TANK_MACHINEGUN_1
        57 | 142 => one(38, 1, 1.0),                                    // MZ2_CHICK_ROCKET_1
        23..=25 => one(39, 1, 1.0),                                     // MZ2_TANK_ROCKET_1
        70..=72 | 78..=81 | 191 => one(40, 1, 1.0),                     // MZ2_SUPERTANK_ROCKET_1
        53..=56 | 256..=259 | 261..=262 => one(41, 1, 1.0),             // MZ2_GUNNER_GRENADE_1
        102..=118 => one(42, 1, 1.0),                                   // MZ2_MAKRON_BLASTER_1
        120..=125 => one(43, 1, 1.0),                                   // MZ2_JORG_MACHINEGUN_L1
        133..=137 | 139 | 153 => {
            if rerelease && flash == 134 {
                one(31, 1, 0.0)
            } else {
                &[]
            }
        } // MZ2_BOSS2_MACHINEGUN_R1
        144..=146 | 149 | 156..=190 | 264 | 277..=288 => one(37, 1, 1.0), // MZ2_STALKER_BLASTER
        148 => one(22, 1, 1.0),                                         // MZ2_WIDOW_DISRUPTOR
        211..=218 | 254 => one(18, 1, 1.0),                             // MZ2_SOLDIER_RIPPER_1
        219..=226 | 255 => one(1, 1, 1.0),                              // MZ2_SOLDIER_HYPERGUN_1
        227 => one(1, 1, 1.0),                                          // MZ2_GUARDIAN_BLASTER
        240..=241 => one(44, 1, 1.0),                                   // MZ2_GUNCMDR_CHAINGUN_1
        242..=247 => one(45, 1, 1.0), // MZ2_GUNCMDR_GRENADE_MORTAR_1
        _ => &[],
    }
}
