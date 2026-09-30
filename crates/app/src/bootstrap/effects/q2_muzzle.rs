//! Quake II monster muzzle-flash admissions from `cl_fx.c`.
//!
//! Donor: `src/app/bootstrap/effects/q2-muzzle.ts`
//! (Copyright (C) Id Software, Inc. GPL-2.0-or-later).
//! Values retain the classic source MZ2 indices.

use qa_core::math::{vec3, Vec3};

/// Light color plus particle/smoke admissions for one MZ2 flash.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonsterMuzzle {
    /// Muzzle light color.
    pub color: Vec3,
    /// Whether the flash emits impact particles.
    pub particles: bool,
    /// Whether the flash emits smoke.
    pub smoke: bool,
    /// Muzzle light radius.
    pub radius: f32,
    /// Particle admission mask.
    pub mask: u32,
}

/// Look up a monster muzzle flash. `rerelease` remaps rerelease flashes onto
/// their classic effect (q2repro `effects.c` grouping).
#[must_use]
pub fn q2_monster_muzzle(flash: u16, rerelease: bool) -> Option<MonsterMuzzle> {
    let flash = if rerelease {
        match flash {
            232..=239 | 260 => 26,
            251 => 39,
            252 => 41,
            253 => 43,
            256..=259 => 53,
            263 => 62,
            74 | 134 => 58,
            other => other,
        }
    } else {
        flash
    };
    let (color, particles, smoke, radius, mask) = match flash {
        4..=22
        | 26..=38
        | 43..=52
        | 63..=69
        | 73..=77
        | 85
        | 88
        | 91
        | 94
        | 97
        | 100
        | 120..=131
        | 133..=139
        | 141
        | 152
        | 153 => ([1.0, 1.0, 0.0], true, true, 200.0, 31),
        1..=3 | 39 | 40 | 58..=60 | 62 | 82 | 83 | 86 | 89 | 92 | 95 | 98 | 102..=118 | 143 => {
            ([1.0, 1.0, 0.0], false, false, 200.0, 31)
        }
        41 | 42 | 84 | 87 | 90 | 93 | 96 | 99 => ([1.0, 1.0, 0.0], false, true, 200.0, 31),
        23..=25 | 57 | 70..=72 | 78..=81 | 142 | 191 => ([1.0, 0.5, 0.2], false, false, 200.0, 31),
        53..=56 => ([1.0, 0.5, 0.0], false, false, 200.0, 31),
        61 | 147 | 150 => ([0.5, 0.5, 1.0], false, false, 200.0, 31),
        101 | 132 => ([0.5, 1.0, 0.5], false, false, 200.0, 31),
        144..=146 | 149 | 156..=190 => ([0.0, 1.0, 0.0], false, false, 200.0, 31),
        148 => ([-1.0, -1.0, -1.0], false, false, 200.0, 31),
        151 | 195..=210 => ([1.0, 1.0, 0.0], false, false, 300.0, 100),
        _ => return None,
    };
    Some(MonsterMuzzle {
        color: vec3(color[0], color[1], color[2]),
        particles,
        smoke,
        radius,
        mask,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machinegun_admits_particles_and_smoke() {
        assert_eq!(
            q2_monster_muzzle(26, false),
            Some(MonsterMuzzle {
                color: vec3(1.0, 1.0, 0.0),
                particles: true,
                smoke: true,
                radius: 200.0,
                mask: 31,
            })
        );
    }

    #[test]
    fn blaster_admits_neither_particles_nor_smoke() {
        let muzzle = q2_monster_muzzle(39, false).expect("soldier blaster");
        assert!(!muzzle.particles && !muzzle.smoke);
        assert_eq!((muzzle.radius, muzzle.mask), (200.0, 31));
    }

    #[test]
    fn shotgun_admits_smoke_only() {
        let muzzle = q2_monster_muzzle(41, false).expect("soldier shotgun");
        assert!(!muzzle.particles && muzzle.smoke);
    }

    #[test]
    fn rerelease_flashes_remap_to_classic() {
        assert_eq!(q2_monster_muzzle(232, true), q2_monster_muzzle(26, false));
        assert_eq!(q2_monster_muzzle(263, true), q2_monster_muzzle(62, false));
        let classic = q2_monster_muzzle(74, false).expect("boss2 machinegun");
        assert!(classic.particles);
        let rerelease = q2_monster_muzzle(74, true).expect("remapped flyer blaster");
        assert!(!rerelease.particles);
        assert_eq!(q2_monster_muzzle(74, true), q2_monster_muzzle(58, false));
    }

    #[test]
    fn plasmabeam_and_disruptor_keep_distinct_values() {
        let beam = q2_monster_muzzle(151, false).expect("plasmabeam");
        assert_eq!((beam.radius, beam.mask), (300.0, 100));
        let disruptor = q2_monster_muzzle(148, false).expect("disruptor");
        assert_eq!(disruptor.color, vec3(-1.0, -1.0, -1.0));
    }

    #[test]
    fn unknown_flashes_are_absent() {
        for flash in [0, 119, 140, 194, 211, 255, 263] {
            assert_eq!(q2_monster_muzzle(flash, false), None, "flash {flash}");
        }
    }
}
