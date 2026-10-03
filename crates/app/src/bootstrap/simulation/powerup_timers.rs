//! Active powerup HUD timers across the Quake families.
//!
//! Port of donor `src/app/bootstrap/simulation/powerup-timers.ts`
//! (`q2PowerupTimers`, `q1PowerupTimers`, `q3PublicPowerupTimers`,
//! `q3PowerupTimers`).

use std::collections::HashMap;

use qa_content::contract::ItemId;
use qa_content::q1::foundation::types::Q1Powerup;
use qa_content::q2::foundation::items::Q2PlayerPowerups;
use qa_content::q2::missionpacks::items::Q2MissionPackPowerups;
use qa_content::q3::base::game::state::GameClient;
use qa_content::q3::base::shared::definitions::Powerup;
use qa_world::movement::q3::constants::move_flags;

fn active(item: &str, label: &str, remaining_seconds: f64) -> ActivePowerupTimer {
    ActivePowerupTimer {
        item: item.to_string(),
        label: label.to_string(),
        remaining_seconds,
    }
}

/// Active powerup HUD timer, mirroring donor `ActivePowerupTimer`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivePowerupTimer {
    /// Timer item.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Seconds remaining.
    pub remaining_seconds: f64,
}

/// Quake II base and mission-pack powerup timers.
pub fn q2_powerup_timers(
    base: &Q2PlayerPowerups,
    expansion: Option<&Q2MissionPackPowerups>,
    now: f64,
) -> Vec<ActivePowerupTimer> {
    [
        active("q2:item_quad", "Quad Damage", base.quad_until - now),
        active(
            "q2:item_quadfire",
            "DualFire Damage",
            expansion.map_or(0.0, |timers| timers.quad_fire_until) - now,
        ),
        active(
            "q2:item_double",
            "Double Damage",
            expansion.map_or(0.0, |timers| timers.double_until) - now,
        ),
        active(
            "q2:item_invulnerability",
            "Invulnerability",
            base.invulnerability_until - now,
        ),
        active("q2:item_enviro", "Environment Suit", base.enviro_until - now),
        active("q2:item_breather", "Rebreather", base.breather_until - now),
        active(
            "q2:item_ir_goggles",
            "IR Goggles",
            expansion.map_or(0.0, |timers| timers.ir_until) - now,
        ),
    ]
    .into_iter()
    .filter(|timer| timer.remaining_seconds > 0.0)
    .collect()
}

fn q1_timer(powerup: Q1Powerup) -> (&'static str, &'static str) {
    match powerup {
        Q1Powerup::Quad => ("q1:item_artifact_super_damage", "Quad Damage"),
        Q1Powerup::Invulnerability => ("q1:item_artifact_invulnerability", "Invulnerability"),
        Q1Powerup::Invisibility => ("q1:item_artifact_invisibility", "Invisibility"),
        Q1Powerup::Suit => ("q1:item_artifact_envirosuit", "Environment Suit"),
        Q1Powerup::Mg3Lavasuit => ("q1:item_artifact_lavasuit", "Lava Suit"),
        Q1Powerup::HipnoticWetsuit => ("q1:item_artifact_wetsuit", "Wetsuit"),
        Q1Powerup::HipnoticEmpathy => ("q1:item_artifact_empathy_shields", "Empathy Shields"),
        Q1Powerup::RogueShield => ("q1:item_powerup_shield", "Power Shield"),
        Q1Powerup::RogueAntigrav => ("q1:item_powerup_belt", "Anti-gravity Belt"),
    }
}

/// Quake powerup timers from per-kind expiry seconds.
pub fn q1_powerup_timers(powerups: &HashMap<Q1Powerup, f64>, now_seconds: f64) -> Vec<ActivePowerupTimer> {
    powerups
        .iter()
        .filter(|(_, expires)| **expires > now_seconds)
        .map(|(kind, expires)| {
            let (item, label) = q1_timer(*kind);
            active(item, label, expires - now_seconds)
        })
        .collect()
}

const Q3_TIMERS: [(Powerup, &str, &str); 6] = [
    (Powerup::PwQuad, "q3:item_quad", "Quad Damage"),
    (Powerup::PwBattlesuit, "q3:item_enviro", "Battle Suit"),
    (Powerup::PwHaste, "q3:item_haste", "Haste"),
    (Powerup::PwInvis, "q3:item_invis", "Invisibility"),
    (Powerup::PwRegen, "q3:item_regen", "Regeneration"),
    (Powerup::PwFlight, "q3:item_flight", "Flight"),
];

/// Quake III powerup timers from a raw expiry lookup.
pub fn q3_public_powerup_timers(expires: impl Fn(i32) -> i32, now_milliseconds: i32) -> Vec<ActivePowerupTimer> {
    Q3_TIMERS
        .iter()
        .map(|(powerup, item, label)| {
            active(
                item,
                label,
                f64::from(expires(*powerup as i32) - now_milliseconds) / 1000.0,
            )
        })
        .filter(|timer| timer.remaining_seconds > 0.0)
        .collect()
}

/// Client state read by Quake III powerup timers. The donor reads these
/// `GameClient` fields directly; the trait keeps timer reads testable.
pub trait Q3PowerupTimerView {
    /// Player-state flag word.
    fn timer_pm_flags(&self) -> i32;
    /// Followed client number.
    fn timer_client_num(&self) -> i32;
    /// Powerup expiry in milliseconds.
    fn timer_powerup_expiry(&self, powerup: i32) -> i32;
    /// Holdable invulnerability expiry in milliseconds.
    fn timer_invulnerability_time(&self) -> i32;
}

impl Q3PowerupTimerView for GameClient {
    fn timer_pm_flags(&self) -> i32 {
        self.ps.pm_flags
    }

    fn timer_client_num(&self) -> i32 {
        self.ps.client_num
    }

    fn timer_powerup_expiry(&self, powerup: i32) -> i32 {
        self.ps.powerups.get(powerup as usize)
    }

    fn timer_invulnerability_time(&self) -> i32 {
        self.invulnerability_time
    }
}

impl Q3PowerupTimerView for qa_content::q3::base::game::entities::GameClient {
    fn timer_pm_flags(&self) -> i32 {
        self.ps.pm_flags
    }

    fn timer_client_num(&self) -> i32 {
        self.ps.client_num
    }

    fn timer_powerup_expiry(&self, powerup: i32) -> i32 {
        self.ps.powerups.get(powerup)
    }

    fn timer_invulnerability_time(&self) -> i32 {
        self.invulnerability_time
    }
}

/// Quake III powerup timers, following the spectated client when set.
pub fn q3_powerup_timers<'a, C: Q3PowerupTimerView>(
    client: &'a C,
    now_milliseconds: i32,
    client_at: impl Fn(i32) -> &'a C,
) -> Vec<ActivePowerupTimer> {
    let viewed: &C = if client.timer_pm_flags() & move_flags::FOLLOW != 0 {
        client_at(client.timer_client_num())
    } else {
        client
    };
    let mut timers = q3_public_powerup_timers(|powerup| viewed.timer_powerup_expiry(powerup), now_milliseconds);
    timers.push(active(
        "q3:holdable_invulnerability",
        "Invulnerability",
        f64::from(viewed.timer_invulnerability_time() - now_milliseconds) / 1000.0,
    ));
    timers
        .into_iter()
        .filter(|timer| timer.remaining_seconds > 0.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClient {
        pm_flags: i32,
        client_num: i32,
        expiries: HashMap<i32, i32>,
        invulnerability_time: i32,
    }

    impl Q3PowerupTimerView for FakeClient {
        fn timer_pm_flags(&self) -> i32 {
            self.pm_flags
        }

        fn timer_client_num(&self) -> i32 {
            self.client_num
        }

        fn timer_powerup_expiry(&self, powerup: i32) -> i32 {
            self.expiries.get(&powerup).copied().unwrap_or(0)
        }

        fn timer_invulnerability_time(&self) -> i32 {
            self.invulnerability_time
        }
    }

    #[test]
    fn q2_filters_expired_and_missing_expansion() {
        let base = Q2PlayerPowerups {
            quad_until: 12.0,
            invulnerability_until: 3.0,
            breather_until: 0.0,
            enviro_until: 9.0,
        };
        let timers = q2_powerup_timers(&base, None, 5.0);
        let items: Vec<&str> = timers.iter().map(|timer| timer.item.as_str()).collect();
        assert_eq!(items, vec!["q2:item_quad", "q2:item_enviro"]);
        assert_eq!(timers[0].remaining_seconds, 7.0);
        let expansion = Q2MissionPackPowerups {
            quad_fire_until: 20.0,
            double_until: 0.0,
            ir_until: 6.0,
        };
        let timers = q2_powerup_timers(&base, Some(&expansion), 5.0);
        assert!(timers.iter().any(|timer| timer.item == "q2:item_quadfire"));
        assert!(timers.iter().any(|timer| timer.item == "q2:item_ir_goggles"));
        assert!(!timers.iter().any(|timer| timer.item == "q2:item_double"));
    }

    #[test]
    fn q1_maps_kinds_to_labeled_timers() {
        let powerups = HashMap::from([
            (Q1Powerup::Quad, 12.0),
            (Q1Powerup::Suit, 4.0),
            (Q1Powerup::RogueShield, 9.0),
        ]);
        let mut timers = q1_powerup_timers(&powerups, 5.0);
        timers.sort_by(|a, b| a.item.cmp(&b.item));
        assert_eq!(timers.len(), 2);
        assert_eq!(timers[0].label, "Quad Damage");
        assert_eq!(timers[0].remaining_seconds, 7.0);
        assert_eq!(timers[1].label, "Power Shield");
    }

    #[test]
    fn q3_public_converts_milliseconds() {
        let timers = q3_public_powerup_timers(|powerup| powerup * 1000, 500);
        assert_eq!(timers.len(), 6);
        assert!(timers.iter().all(|timer| timer.remaining_seconds > 0.0));
        let quad = timers.iter().find(|timer| timer.item == "q3:item_quad").unwrap();
        assert_eq!(quad.remaining_seconds, 0.5);
    }

    #[test]
    fn q3_follows_spectated_client() {
        let followed = FakeClient {
            pm_flags: 0,
            client_num: 3,
            expiries: HashMap::from([(Powerup::PwQuad as i32, 9000)]),
            invulnerability_time: 0,
        };
        let spectator = FakeClient {
            pm_flags: move_flags::FOLLOW,
            client_num: 3,
            expiries: HashMap::new(),
            invulnerability_time: 0,
        };
        let store = HashMap::from([(3, followed)]);
        let timers = q3_powerup_timers(&spectator, 4000, |num| &store[&num]);
        assert_eq!(timers.len(), 1);
        assert_eq!(timers[0].item, "q3:item_quad");
        assert_eq!(timers[0].remaining_seconds, 5.0);
    }
}
