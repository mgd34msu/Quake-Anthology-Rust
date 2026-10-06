//! Stock Quake I view blends (`view.c:254-518`): the four client color
//! shifts (contents, damage, bonus, powerup) combined GL-style into one
//! fullscreen `v_blend` rgba quad, plus the damage view kick
//! (`v_dmg_roll`/`v_dmg_pitch`/`v_dmg_time`).
//!
//! Pure state machine over live inputs: the damage funnel records armor
//! save / blood take plus the inflictor position, pickups raise the
//! bonus flash, the eye contents select the water/slime/lava shift, and
//! the worn powerup bits select the glow. Damage decays 150 percent per
//! second, bonus 100 per second (`view.c:557-565`).

use super::q1_native::{Q1_IT_INVISIBILITY, Q1_IT_INVULNERABILITY, Q1_IT_QUAD, Q1_IT_SUIT};

/// Damage shift decay, percent per second (`view.c:557`).
pub const Q1_DAMAGE_DECAY_PER_SECOND: f32 = 150.0;

/// Bonus shift decay, percent per second (`view.c:562`).
pub const Q1_BONUS_DECAY_PER_SECOND: f32 = 100.0;

/// Damage shift ceiling (`view.c:340-341`).
pub const Q1_DAMAGE_PERCENT_MAX: f32 = 150.0;

/// Minimum damage count: light hits still flash (`view.c:330-331`).
pub const Q1_DAMAGE_COUNT_MIN: f32 = 10.0;

/// Bonus flash percent (`V_BonusFlash_f`, `view.c:408`).
pub const Q1_BONUS_PERCENT: f32 = 50.0;

/// Pain face hold after damage (`V_ParseDamage`, `view.c:335`): the sbar
/// face stays in its pain frame this long. Feeds the bar's
/// `face_anim_until`, closing the damage-to-face loop stock draws.
pub const Q1_FACE_PAIN_SECONDS: f32 = 0.2;

/// Damage view-kick roll scale (`v_kickroll`, `view.c:49`).
pub const Q1_KICK_ROLL: f32 = 0.6;

/// Damage view-kick pitch scale (`v_kickpitch`, `view.c:50`).
pub const Q1_KICK_PITCH: f32 = 0.6;

/// Damage view-kick duration (`v_kicktime`, `view.c:48`).
pub const Q1_KICK_TIME: f32 = 0.5;

/// Stock Quake I point contents at the eye (`bspfile.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q1EyeContents {
    /// Open air (or unset).
    #[default]
    Empty,
    /// Inside solid geometry.
    Solid,
    /// Under water.
    Water,
    /// Under slime.
    Slime,
    /// Under lava.
    Lava,
    /// Under sky (stock `default:` arm treats it as water).
    Sky,
}

impl Q1EyeContents {
    /// Raw BSP contents value to eye contents. Stock `V_SetContentsColor`
    /// (`view.c:418-436`) matches empty/solid, lava, slime, and defaults
    /// everything else (water, sky) to the water shift.
    #[must_use]
    pub fn from_raw(contents: i32) -> Self {
        match contents {
            -1 => Q1EyeContents::Empty,
            -2 => Q1EyeContents::Solid,
            -3 => Q1EyeContents::Water,
            -4 => Q1EyeContents::Slime,
            -5 => Q1EyeContents::Lava,
            _ => Q1EyeContents::Sky,
        }
    }
}

/// One client color shift (`cshift_t`, `client.h`): target color plus
/// blend percent.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q1Cshift {
    /// Target color, 0-255 per channel.
    pub destcolor: [u8; 3],
    /// Blend percent.
    pub percent: f32,
}

/// The four stock shifts plus damage-kick state (`client.h:56-60` order:
/// contents, damage, bonus, powerup).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1ViewBlends {
    /// Contents shift (water/slime/lava).
    pub contents: Q1Cshift,
    /// Damage shift.
    pub damage: Q1Cshift,
    /// Pickup bonus shift.
    pub bonus: Q1Cshift,
    /// Powerup glow shift.
    pub powerup: Q1Cshift,
    /// Pending damage-kick roll, degrees at full strength.
    pub dmg_roll: f32,
    /// Pending damage-kick pitch, degrees at full strength.
    pub dmg_pitch: f32,
    /// Remaining kick time, seconds.
    pub dmg_time: f32,
}

impl Q1ViewBlends {
    /// Stock damage event (`V_ParseDamage`, `view.c:316-379`): `armor` is
    /// the absorbed save, `blood` the health taken (both cross the
    /// `svc_damage` wire as bytes, hence the 255 clamp). Returns the
    /// damage count, which also scales the view kick.
    pub fn damage(&mut self, armor: f32, blood: f32) -> f32 {
        let armor = armor.clamp(0.0, 255.0);
        let blood = blood.clamp(0.0, 255.0);
        let count = (blood * 0.5 + armor * 0.5).max(Q1_DAMAGE_COUNT_MIN);
        self.damage.percent = (self.damage.percent + 3.0 * count).clamp(0.0, Q1_DAMAGE_PERCENT_MAX);
        self.damage.destcolor = if armor > blood {
            [200, 100, 100]
        } else if armor > 0.0 {
            [220, 50, 50]
        } else {
            [255, 0, 0]
        };
        count
    }

    /// Stock view kick from a damage direction (`view.c:361-378`): `from`
    /// points from the view entity toward the inflictor (need not be
    /// normalized), `forward`/`right` are the view axes. Returns the
    /// (roll, pitch) kick in degrees and arms `dmg_time`.
    pub fn damage_kick(&mut self, from: [f32; 3], forward: [f32; 3], right: [f32; 3], count: f32) -> (f32, f32) {
        let len = (from[0] * from[0] + from[1] * from[1] + from[2] * from[2]).sqrt();
        if len == 0.0 {
            return (0.0, 0.0);
        }
        let dir = [from[0] / len, from[1] / len, from[2] / len];
        let side_roll = dir[0] * right[0] + dir[1] * right[1] + dir[2] * right[2];
        let side_pitch = dir[0] * forward[0] + dir[1] * forward[1] + dir[2] * forward[2];
        self.dmg_roll = count * side_roll * Q1_KICK_ROLL;
        self.dmg_pitch = count * side_pitch * Q1_KICK_PITCH;
        self.dmg_time = Q1_KICK_TIME;
        (self.dmg_roll, self.dmg_pitch)
    }

    /// Kick offsets for this frame, decaying the timer (`view.c:815-819`):
    /// while `dmg_time` runs, the view rolls/pitches by the time fraction
    /// of the armed kick.
    pub fn kick_offsets(&mut self, dt: f32) -> (f32, f32) {
        if self.dmg_time <= 0.0 {
            return (0.0, 0.0);
        }
        let fraction = (self.dmg_time / Q1_KICK_TIME).clamp(0.0, 1.0);
        let offsets = (fraction * self.dmg_roll, fraction * self.dmg_pitch);
        self.dmg_time -= dt;
        offsets
    }

    /// Stock pickup flash (`V_BonusFlash_f`, `view.c:403-409`): 50
    /// percent gold.
    pub fn bonus(&mut self) {
        self.bonus.destcolor = [215, 186, 69];
        self.bonus.percent = Q1_BONUS_PERCENT;
    }

    /// Stock eye-contents shift (`V_SetContentsColor`, `view.c:418-436`).
    pub fn set_contents(&mut self, contents: Q1EyeContents) {
        self.contents = match contents {
            Q1EyeContents::Empty | Q1EyeContents::Solid => Q1Cshift {
                destcolor: [130, 80, 50],
                percent: 0.0,
            },
            Q1EyeContents::Lava => Q1Cshift {
                destcolor: [255, 80, 0],
                percent: 150.0,
            },
            Q1EyeContents::Slime => Q1Cshift {
                destcolor: [0, 25, 5],
                percent: 150.0,
            },
            Q1EyeContents::Water | Q1EyeContents::Sky => Q1Cshift {
                destcolor: [130, 80, 50],
                percent: 128.0,
            },
        };
    }

    /// Stock powerup glow (`V_CalcPowerupCshift`, `view.c:444-478`):
    /// quad beats suit beats ring beats pent, else clear.
    pub fn set_powerup(&mut self, items: u32) {
        if items & Q1_IT_QUAD != 0 {
            self.powerup.destcolor = [0, 0, 255];
            self.powerup.percent = 30.0;
        } else if items & Q1_IT_SUIT != 0 {
            self.powerup.destcolor = [0, 255, 0];
            self.powerup.percent = 20.0;
        } else if items & Q1_IT_INVISIBILITY != 0 {
            self.powerup.destcolor = [100, 100, 100];
            self.powerup.percent = 100.0;
        } else if items & Q1_IT_INVULNERABILITY != 0 {
            self.powerup.destcolor = [255, 255, 0];
            self.powerup.percent = 30.0;
        } else {
            self.powerup.percent = 0.0;
        }
    }

    /// Per-frame decay (`view.c:557-565`): damage drops 150 percent per
    /// second, bonus 100. Contents and powerup are recomputed by the
    /// caller every frame instead.
    pub fn tick(&mut self, dt: f32) {
        self.damage.percent = (self.damage.percent - dt * Q1_DAMAGE_DECAY_PER_SECOND).max(0.0);
        self.bonus.percent = (self.bonus.percent - dt * Q1_BONUS_DECAY_PER_SECOND).max(0.0);
    }

    /// Combined GL blend (`V_CalcBlend`, `view.c:486-518` with
    /// `gl_cshiftpercent` 100): rgba 0.0-1.0 for one fullscreen quad.
    #[must_use]
    pub fn blend(&self) -> [f32; 4] {
        let shifts = [&self.contents, &self.damage, &self.bonus, &self.powerup];
        let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
        for shift in shifts {
            let blend_a = shift.percent / 255.0;
            if blend_a == 0.0 {
                continue;
            }
            a += blend_a * (1.0 - a);
            let mix = blend_a / a;
            r = r * (1.0 - mix) + f32::from(shift.destcolor[0]) * mix;
            g = g * (1.0 - mix) + f32::from(shift.destcolor[1]) * mix;
            b = b * (1.0 - mix) + f32::from(shift.destcolor[2]) * mix;
        }
        [r / 255.0, g / 255.0, b / 255.0, a.clamp(0.0, 1.0)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_scales_clamps_and_picks_color() {
        let mut blends = Q1ViewBlends::default();
        // Pure blood: full red, count floors at 10 for light hits.
        let count = blends.damage(0.0, 4.0);
        assert_eq!(count, 10.0);
        assert_eq!(blends.damage.destcolor, [255, 0, 0]);
        assert_eq!(blends.damage.percent, 30.0);
        // Mostly armor: pale red; mostly blood with some armor: mid red.
        blends.damage(30.0, 10.0);
        assert_eq!(blends.damage.destcolor, [200, 100, 100]);
        blends.damage(10.0, 30.0);
        assert_eq!(blends.damage.destcolor, [220, 50, 50]);
        // The ceiling holds under heavy fire.
        blends.damage(200.0, 200.0);
        assert_eq!(blends.damage.percent, 150.0);
    }

    #[test]
    fn shifts_decay_at_stock_rates() {
        let mut blends = Q1ViewBlends::default();
        blends.damage(0.0, 100.0);
        blends.bonus();
        assert_eq!(blends.damage.percent, 150.0);
        assert_eq!(blends.bonus.percent, 50.0);
        blends.tick(0.5);
        assert_eq!(blends.damage.percent, 75.0);
        assert_eq!(blends.bonus.percent, 0.0);
        blends.tick(10.0);
        assert_eq!(blends.damage.percent, 0.0);
    }

    #[test]
    fn bonus_is_gold_at_fifty_percent() {
        let mut blends = Q1ViewBlends::default();
        blends.bonus();
        assert_eq!(blends.bonus.destcolor, [215, 186, 69]);
        assert_eq!(blends.bonus.percent, 50.0);
    }

    #[test]
    fn contents_select_stock_presets() {
        let mut blends = Q1ViewBlends::default();
        blends.set_contents(Q1EyeContents::Water);
        assert_eq!(blends.contents.destcolor, [130, 80, 50]);
        assert_eq!(blends.contents.percent, 128.0);
        blends.set_contents(Q1EyeContents::Slime);
        assert_eq!(blends.contents.destcolor, [0, 25, 5]);
        assert_eq!(blends.contents.percent, 150.0);
        blends.set_contents(Q1EyeContents::Lava);
        assert_eq!(blends.contents.destcolor, [255, 80, 0]);
        assert_eq!(blends.contents.percent, 150.0);
        blends.set_contents(Q1EyeContents::Empty);
        assert_eq!(blends.contents.percent, 0.0);
        blends.set_contents(Q1EyeContents::Solid);
        assert_eq!(blends.contents.percent, 0.0);
        // Stock `default:` arm: sky shifts like water.
        blends.set_contents(Q1EyeContents::from_raw(-6));
        assert_eq!(blends.contents.percent, 128.0);
        assert_eq!(Q1EyeContents::from_raw(-1), Q1EyeContents::Empty);
        assert_eq!(Q1EyeContents::from_raw(-4), Q1EyeContents::Slime);
        assert_eq!(Q1EyeContents::from_raw(-5), Q1EyeContents::Lava);
    }

    #[test]
    fn powerup_precedence_is_quad_suit_ring_pent() {
        let mut blends = Q1ViewBlends::default();
        blends.set_powerup(Q1_IT_QUAD | Q1_IT_SUIT | Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY);
        assert_eq!(blends.powerup.destcolor, [0, 0, 255]);
        assert_eq!(blends.powerup.percent, 30.0);
        blends.set_powerup(Q1_IT_SUIT | Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY);
        assert_eq!(blends.powerup.destcolor, [0, 255, 0]);
        assert_eq!(blends.powerup.percent, 20.0);
        blends.set_powerup(Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY);
        assert_eq!(blends.powerup.destcolor, [100, 100, 100]);
        assert_eq!(blends.powerup.percent, 100.0);
        blends.set_powerup(Q1_IT_INVULNERABILITY);
        assert_eq!(blends.powerup.destcolor, [255, 255, 0]);
        assert_eq!(blends.powerup.percent, 30.0);
        blends.set_powerup(0);
        assert_eq!(blends.powerup.percent, 0.0);
    }

    #[test]
    fn blend_combines_shifts_gl_style() {
        let mut blends = Q1ViewBlends::default();
        assert_eq!(blends.blend(), [0.0, 0.0, 0.0, 0.0]);
        blends.bonus();
        let gold = blends.blend();
        assert!((gold[3] - 50.0 / 255.0).abs() < 1e-6);
        assert!((gold[0] - 215.0 / 255.0).abs() < 1e-6);
        assert!((gold[1] - 186.0 / 255.0).abs() < 1e-6);
        assert!((gold[2] - 69.0 / 255.0).abs() < 1e-6);
        // A later shift folds over the earlier one without exceeding 1.
        blends.damage(0.0, 200.0);
        let mixed = blends.blend();
        assert!(mixed[3] > gold[3] && mixed[3] <= 1.0);
        assert!(mixed[0] > gold[0]);
    }

    #[test]
    fn kick_points_away_from_the_hit_and_decays() {
        let mut blends = Q1ViewBlends::default();
        // Hit from the left-front: negative roll, positive pitch.
        let (roll, pitch) = blends.damage_kick([-1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 20.0);
        assert!(roll < 0.0 && pitch > 0.0);
        assert_eq!(blends.dmg_time, 0.5);
        // Full strength on the first frame, then linear decay.
        assert_eq!(blends.kick_offsets(0.0), (roll, pitch));
        assert_eq!(blends.kick_offsets(0.25), (roll, pitch));
        let (half_roll, half_pitch) = blends.kick_offsets(0.0);
        assert!((half_roll - roll * 0.5).abs() < 1e-6);
        assert!((half_pitch - pitch * 0.5).abs() < 1e-6);
        let _ = blends.kick_offsets(0.25);
        assert_eq!(blends.kick_offsets(0.0), (0.0, 0.0));
        // A zero-length direction never kicks.
        assert_eq!(
            blends.damage_kick([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 20.0),
            (0.0, 0.0)
        );
    }
}
