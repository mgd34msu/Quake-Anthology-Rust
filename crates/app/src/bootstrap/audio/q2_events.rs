//! Quake II entity-event and muzzle-flash sounds.
//!
//! Port of donor `src/app/bootstrap/audio/q2-events.ts`
//! (`q2EntitySound`, `q2MuzzleSounds`, `q2MonsterMuzzleSounds`),
//! following `cl_fx.c` `CL_EntityEvent` and `CL_ParseMuzzleFlash`.
//! Copyright id Software. SPDX-License-Identifier: GPL-2.0-or-later.

use super::output_settings::js_number_string;

/// One Quake II event sound.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2EventSound {
    /// Sound path.
    pub path: String,
    /// Mix channel.
    pub channel: i32,
    /// Distance attenuation.
    pub attenuation: f64,
    /// Gain multiplier.
    pub volume: f64,
    /// Delay in seconds.
    pub delay_seconds: f64,
}

/// JavaScript `ToInt32` conversion.
fn to_int32(value: f64) -> i32 {
    const TWO_32: f64 = 4_294_967_296.0;
    const TWO_31: f64 = 2_147_483_648.0;
    if !value.is_finite() {
        return 0;
    }
    let wrapped = value % TWO_32;
    let positive = if wrapped < 0.0 { wrapped + TWO_32 } else { wrapped };
    let signed = if positive >= TWO_31 { positive - TWO_32 } else { positive };
    signed.trunc() as i32
}

/// JavaScript `String.fromCharCode` for one code.
fn from_char_code(code: f64) -> char {
    const MODULO: f64 = 65536.0;
    let wrapped = ((code % MODULO) + MODULO) % MODULO;
    char::from_u32(wrapped as u32).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// Entity-event sound for a Quake II event id, if the event has one.
pub fn q2_entity_sound(event: i32, random: &mut dyn FnMut() -> f64) -> Option<Q2EventSound> {
    let sound = |path: String, channel: i32, attenuation: f64| Q2EventSound {
        path,
        channel,
        attenuation,
        volume: 1.0,
        delay_seconds: 0.0,
    };
    match event {
        1 => Some(sound("items/respawn1.wav".to_string(), 1, 2.0)),
        2 => Some(sound(format!("player/step{}.wav", (to_int32(random()) & 3) + 1), 4, 1.0)),
        3 => Some(sound("player/land1.wav".to_string(), 0, 1.0)),
        4 => Some(sound("*fall2.wav".to_string(), 0, 1.0)),
        5 => Some(sound("*fall1.wav".to_string(), 0, 1.0)),
        6 => Some(sound("misc/tele1.wav".to_string(), 1, 2.0)),
        _ => None,
    }
}

/// Muzzle-flash sounds for a Quake II weapon flash id.
pub fn q2_muzzle_sounds(
    flash: i32,
    silenced: bool,
    random: &mut dyn FnMut() -> f64,
    rerelease: bool,
) -> Vec<Q2EventSound> {
    let volume = if silenced { 0.2 } else { 1.0 };
    let sound = |path: &str, channel: i32, delay_seconds: f64, gain: f64| Q2EventSound {
        path: path.to_string(),
        channel,
        attenuation: 1.0,
        volume: gain,
        delay_seconds,
    };
    let machinegun = |random: &mut dyn FnMut() -> f64, delay: f64| {
        sound(
            &format!("weapons/machgf{}b.wav", js_number_string(random() % 5.0 + 1.0)),
            1,
            delay,
            volume,
        )
    };
    match flash & !128 {
        0 | 34 => vec![sound("weapons/blastf1a.wav", 1, 0.0, volume)],
        1 | 3 => vec![machinegun(random, 0.0)],
        2 => vec![
            sound("weapons/shotgf1b.wav", 1, 0.0, volume),
            sound("weapons/shotgr1b.wav", 0, 0.1, volume),
        ],
        4 => vec![machinegun(random, 0.0), machinegun(random, 0.05)],
        5 => vec![
            machinegun(random, 0.0),
            machinegun(random, 0.033),
            machinegun(random, 0.066),
        ],
        6 => {
            if rerelease {
                vec![
                    sound("weapons/railgf1a.wav", 1, 0.0, volume),
                    sound("weapons/railgr1b.wav", 7, 0.4, volume),
                ]
            } else {
                vec![sound("weapons/railgf1a.wav", 1, 0.0, volume)]
            }
        }
        7 => vec![
            sound("weapons/rocklf1a.wav", 1, 0.0, volume),
            sound("weapons/rocklr1b.wav", 0, 0.1, volume),
        ],
        8 => vec![
            sound("weapons/grenlf1a.wav", 1, 0.0, volume),
            sound("weapons/grenlr1b.wav", 0, 0.1, volume),
        ],
        9 | 10 | 11 => vec![sound("weapons/grenlf1a.wav", 1, 0.0, 1.0)],
        12 => vec![sound("weapons/bfg__f1y.wav", 1, 0.0, volume)],
        13 => vec![sound("weapons/sshotf1b.wav", 1, 0.0, volume)],
        14 | 17 => vec![sound("weapons/hyprbf1a.wav", 1, 0.0, volume)],
        16 => vec![sound("weapons/rippfire.wav", 1, 0.0, volume)],
        18 => vec![sound("weapons/plasshot.wav", 1, 0.0, volume)],
        30 => vec![sound("weapons/nail1.wav", 1, 0.0, volume)],
        32 => vec![sound("weapons/shotg2.wav", 1, 0.0, volume)],
        35 => vec![sound("weapons/disint2.wav", 1, 0.0, volume)],
        _ => vec![],
    }
}

/// Monster muzzle-flash sounds; `None` is an uncovered flash.
pub fn q2_monster_muzzle_sounds(
    flash: i32,
    random: &mut dyn FnMut() -> f64,
    rerelease: bool,
) -> Option<Vec<Q2EventSound>> {
    let sound = |path: &str, attenuation: f64| {
        vec![Q2EventSound {
            path: path.to_string(),
            channel: 1,
            attenuation,
            volume: 1.0,
            delay_seconds: 0.0,
        }]
    };
    if rerelease {
        match flash {
            232 | 233 | 234 | 235 | 236 | 237 | 238 | 239 | 260 => {
                return Some(sound("infantry/infatck1.wav", 1.0))
            }
            251 => return Some(sound("soldier/solatck2.wav", 1.0)),
            252 => return Some(sound("soldier/solatck1.wav", 1.0)),
            253 => return Some(sound("soldier/solatck3.wav", 1.0)),
            256 | 257 | 258 | 259 => return Some(sound("gunner/gunatck3.wav", 1.0)),
            263 => return Some(sound("hover/hovatck1.wav", 1.0)),
            _ => {}
        }
    }
    match flash {
        26 | 27 | 28 | 29 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 => {
            Some(sound("infantry/infatck1.wav", 1.0))
        }
        43 | 44 | 85 | 88 | 91 | 94 | 97 | 100 => Some(sound("soldier/solatck3.wav", 1.0)),
        45 | 46 | 47 | 48 | 49 | 50 | 51 | 52 => Some(sound("gunner/gunatck2.wav", 1.0)),
        63 | 64 | 65 | 66 | 67 | 68 | 69 | 141 => Some(sound("infantry/infatck1.wav", 1.0)),
        73 | 74 | 75 | 76 | 77 | 138 | 152 => Some(sound(
            if rerelease && flash == 74 {
                "flyer/flyatck3.wav"
            } else {
                "infantry/infatck1.wav"
            },
            0.0,
        )),
        39 | 40 | 83 | 86 | 89 | 92 | 95 | 98 | 143 => {
            Some(sound("soldier/solatck2.wav", 1.0))
        }
        58 | 59 => Some(sound("flyer/flyatck3.wav", 1.0)),
        60 => Some(sound("medic/medatck1.wav", 1.0)),
        62 => Some(sound("hover/hovatck1.wav", 1.0)),
        82 => Some(sound("floater/fltatck1.wav", 1.0)),
        41 | 42 | 84 | 87 | 90 | 93 | 96 | 99 => Some(sound("soldier/solatck1.wav", 1.0)),
        1 | 2 | 3 => Some(sound("tank/tnkatck3.wav", 1.0)),
        4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 => {
            Some(sound(
                &format!("tank/tnkatk2{}.wav", from_char_code(97.0 + random() % 5.0)),
                1.0,
            ))
        }
        57 | 142 => Some(sound("chick/chkatck2.wav", 1.0)),
        23 | 24 | 25 => Some(sound("tank/tnkatck1.wav", 1.0)),
        70 | 71 | 72 | 78 | 79 | 80 | 81 | 191 => Some(sound("tank/rocket.wav", 1.0)),
        53 | 54 | 55 | 56 => Some(sound("gunner/gunatck3.wav", 1.0)),
        61 | 147 | 150 | 101 => Some(vec![]),
        102 | 103 | 104 | 105 | 106 | 107 | 108 | 109 | 110 | 111 | 112 | 113 | 114 | 115
        | 116 | 117 | 118 => Some(sound("makron/blaster.wav", 1.0)),
        120 | 121 | 122 | 123 | 124 | 125 => Some(sound("boss3/xfire.wav", 1.0)),
        126 | 127 | 128 | 129 | 130 | 131 | 132 => Some(vec![]),
        133 | 134 | 135 | 136 | 137 | 139 | 153 => {
            if rerelease && flash == 134 {
                Some(sound("flyer/flyatck3.wav", 0.0))
            } else {
                Some(vec![])
            }
        }
        144 | 145 | 146 | 149 | 156 | 157 | 158 | 159 | 160 | 161 | 162 | 163 | 164 | 165 | 166
        | 167 | 168 | 169 | 170 | 171 | 172 | 173 | 174 | 175 | 176 | 177 | 178 | 179 | 180
        | 181 | 182 | 183 | 184 | 185 | 186 | 187 | 188 | 189 | 190 => {
            Some(sound("tank/tnkatck3.wav", 1.0))
        }
        148 => Some(sound("weapons/disint2.wav", 1.0)),
        151 | 195 | 196 | 197 | 198 | 199 | 200 | 201 | 202 | 203 | 204 | 205 | 206 | 207 | 208
        | 209 | 210 => Some(vec![]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_events_cover_table() {
        assert_eq!(
            q2_entity_sound(1, &mut || 0.0).unwrap().path,
            "items/respawn1.wav"
        );
        assert_eq!(q2_entity_sound(2, &mut || 6.0).unwrap().path, "player/step3.wav");
        assert_eq!(q2_entity_sound(4, &mut || 0.0).unwrap().path, "*fall2.wav");
        assert_eq!(q2_entity_sound(6, &mut || 0.0).unwrap().attenuation, 2.0);
        assert_eq!(q2_entity_sound(0, &mut || 0.0), None);
        assert_eq!(q2_entity_sound(7, &mut || 0.0), None);
    }

    #[test]
    fn muzzle_covers_guns_and_silence() {
        let blast = q2_muzzle_sounds(0, false, &mut || 0.0, false);
        assert_eq!(blast.len(), 1);
        assert_eq!(blast[0].path, "weapons/blastf1a.wav");
        // Silenced bit 128 clears through the mask.
        let masked = q2_muzzle_sounds(128, false, &mut || 0.0, false);
        assert_eq!(masked[0].path, "weapons/blastf1a.wav");
        let burst = q2_muzzle_sounds(5, true, &mut || 2.0, false);
        assert_eq!(burst.len(), 3);
        assert_eq!(burst[0].volume, 0.2);
        assert_eq!(burst[0].path, "weapons/machgf3b.wav");
        assert_eq!(burst[1].delay_seconds, 0.033);
        let classic = q2_muzzle_sounds(6, false, &mut || 0.0, false);
        assert_eq!(classic.len(), 1);
        let remastered = q2_muzzle_sounds(6, false, &mut || 0.0, true);
        assert_eq!(remastered.len(), 2);
        assert_eq!(remastered[1].channel, 7);
        assert!(q2_muzzle_sounds(99, false, &mut || 0.0, false).is_empty());
    }

    #[test]
    fn monster_covers_rerelease_and_random() {
        let tank = q2_monster_muzzle_sounds(4, &mut || 2.0, false).unwrap();
        assert_eq!(tank[0].path, "tank/tnkatk2c.wav");
        let rerelease = q2_monster_muzzle_sounds(232, &mut || 0.0, true).unwrap();
        assert_eq!(rerelease[0].path, "infantry/infatck1.wav");
        assert_eq!(q2_monster_muzzle_sounds(232, &mut || 0.0, false), None);
        let flyer = q2_monster_muzzle_sounds(74, &mut || 0.0, true).unwrap();
        assert_eq!(flyer[0].path, "flyer/flyatck3.wav");
        assert_eq!(flyer[0].attenuation, 0.0);
        let silent = q2_monster_muzzle_sounds(61, &mut || 0.0, false).unwrap();
        assert!(silent.is_empty());
        assert_eq!(q2_monster_muzzle_sounds(999, &mut || 0.0, false), None);
    }
}
