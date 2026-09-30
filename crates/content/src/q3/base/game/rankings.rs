//! Quake III base/game: rankings.
//!
//! Donor provenance: `src/content/q3/base/game/rankings.ts`.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::{failure, Q3GameError};
use crate::q3::base::shared::definitions::{Holdable, Powerup, Weapon};

// ---------------------------------------------------------------------------
// rankings.ts: ranking reports (g_rankings.c)
// ---------------------------------------------------------------------------

/// Ranking report (`Q3RankingReport`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3RankingReport {
    /// Integer report.
    Integer {
        /// Self.
        slf: i32,
        /// Other.
        other: i32,
        /// Key.
        key: i32,
        /// Value.
        value: i32,
        /// Accumulate.
        accumulate: bool,
    },
    /// String report.
    String {
        /// Self.
        slf: i32,
        /// Other.
        other: i32,
        /// Key.
        key: i32,
        /// Value.
        value: String,
    },
}

pub(crate) struct RankingInner {
    sink: Option<Rc<dyn Fn(Q3RankingReport)>>,
    warmup: Rc<dyn Fn() -> bool>,
    last_hit: String,
}

/// Ranking reports (`Q3RankingReports`).
#[derive(Clone)]
pub struct Q3RankingReports {
    inner: Rc<RefCell<RankingInner>>,
}

impl std::fmt::Debug for Q3RankingReports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3RankingReports")
            .field("attached", &self.inner.borrow().sink.is_some())
            .finish_non_exhaustive()
    }
}

impl Q3RankingReports {
    /// New reports.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(RankingInner {
                sink: None,
                warmup: Rc::new(|| true),
                last_hit: String::new(),
            })),
        }
    }

    /// Attach a sink (`attach`), returning a detach closure.
    pub fn attach(
        &self,
        sink: Rc<dyn Fn(Q3RankingReport)>,
        warmup: Rc<dyn Fn() -> bool>,
    ) -> Result<Rc<dyn Fn()>, Q3GameError> {
        if self.inner.borrow().sink.is_some() {
            return Err(failure("Ranking report owner already attached"));
        }
        self.inner.borrow_mut().sink = Some(sink);
        self.inner.borrow_mut().warmup = warmup;
        let inner: Weak<RefCell<RankingInner>> = Rc::downgrade(&self.inner);
        Ok(Rc::new(move || {
            if let Some(inner) = inner.upgrade() {
                inner.borrow_mut().sink = None;
                inner.borrow_mut().warmup = Rc::new(|| true);
            }
        }))
    }

    fn is_warmup(&self) -> bool {
        (self.inner.borrow().warmup)()
    }

    fn emit(&self, report: Q3RankingReport) {
        if let Some(sink) = self.inner.borrow().sink.clone() {
            sink(report);
        }
    }

    /// Integer report (`integer`).
    pub fn integer(&self, slf: i32, other: i32, key: i32, value: i32, accumulate: bool) {
        if !self.is_warmup() {
            self.emit(Q3RankingReport::Integer {
                slf,
                other,
                key,
                value,
                accumulate,
            });
        }
    }

    /// String report (`string`).
    pub fn string(&self, slf: i32, other: i32, key: i32, value: &str) {
        if !self.is_warmup() {
            self.emit(Q3RankingReport::String {
                slf,
                other,
                key,
                value: value.to_string(),
            });
        }
    }

    /// Fire-weapon reports.
    pub fn fire_weapon(&self, slf: i32, weapon: i32) {
        if self.is_warmup() || weapon == Weapon::WpGauntlet as i32 {
            return;
        }
        self.integer(slf, -1, 1111020002, 1, true);
        match weapon {
            x if x == Weapon::WpMachinegun as i32 => self.integer(slf, -1, 1111020202, 1, true),
            x if x == Weapon::WpShotgun as i32 => self.integer(slf, -1, 1111020302, 1, true),
            x if x == Weapon::WpGrenadeLauncher as i32 => self.integer(slf, -1, 1111020402, 1, true),
            x if x == Weapon::WpRocketLauncher as i32 => self.integer(slf, -1, 1111020502, 1, true),
            x if x == Weapon::WpLightning as i32 => self.integer(slf, -1, 1111020802, 1, true),
            x if x == Weapon::WpRailgun as i32 => self.integer(slf, -1, 1111020702, 1, true),
            x if x == Weapon::WpPlasmagun as i32 => self.integer(slf, -1, 1111020602, 1, true),
            x if x == Weapon::WpBfg as i32 => self.integer(slf, -1, 1111020902, 1, true),
            x if x == Weapon::WpGrapplingHook as i32 => self.integer(slf, -1, 1111021002, 1, true),
            _ => {}
        }
    }

    /// Pickup-weapon reports.
    pub fn pickup_weapon(&self, slf: i32, weapon: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111020009, 1, true);
        match weapon {
            x if x == Weapon::WpGauntlet as i32 => self.integer(slf, -1, 1111020109, 1, true),
            x if x == Weapon::WpMachinegun as i32 => self.integer(slf, -1, 1111020209, 1, true),
            x if x == Weapon::WpShotgun as i32 => self.integer(slf, -1, 1111020309, 1, true),
            x if x == Weapon::WpGrenadeLauncher as i32 => self.integer(slf, -1, 1111020409, 1, true),
            x if x == Weapon::WpRocketLauncher as i32 => self.integer(slf, -1, 1111020509, 1, true),
            x if x == Weapon::WpLightning as i32 => self.integer(slf, -1, 1111020809, 1, true),
            x if x == Weapon::WpRailgun as i32 => self.integer(slf, -1, 1111020709, 1, true),
            x if x == Weapon::WpPlasmagun as i32 => self.integer(slf, -1, 1111020609, 1, true),
            x if x == Weapon::WpBfg as i32 => self.integer(slf, -1, 1111020909, 1, true),
            x if x == Weapon::WpGrapplingHook as i32 => self.integer(slf, -1, 1111021009, 1, true),
            _ => {}
        }
    }

    /// Pickup-ammo reports.
    pub fn pickup_ammo(&self, slf: i32, weapon: i32, quantity: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111030000, 1, true);
        self.integer(slf, -1, 1111030001, quantity, true);
        match weapon {
            x if x == Weapon::WpMachinegun as i32 => {
                self.integer(slf, -1, 1111030100, 1, true);
                self.integer(slf, -1, 1111030101, quantity, true);
            }
            x if x == Weapon::WpShotgun as i32 => {
                self.integer(slf, -1, 1111030200, 1, true);
                self.integer(slf, -1, 1111030201, quantity, true);
            }
            x if x == Weapon::WpGrenadeLauncher as i32 => {
                self.integer(slf, -1, 1111030300, 1, true);
                self.integer(slf, -1, 1111030301, quantity, true);
            }
            x if x == Weapon::WpRocketLauncher as i32 => {
                self.integer(slf, -1, 1111030400, 1, true);
                self.integer(slf, -1, 1111030401, quantity, true);
            }
            x if x == Weapon::WpLightning as i32 => {
                self.integer(slf, -1, 1111030700, 1, true);
                self.integer(slf, -1, 1111030701, quantity, true);
            }
            x if x == Weapon::WpRailgun as i32 => {
                self.integer(slf, -1, 1111030600, 1, true);
                self.integer(slf, -1, 1111030601, quantity, true);
            }
            x if x == Weapon::WpPlasmagun as i32 => {
                self.integer(slf, -1, 1111030500, 1, true);
                self.integer(slf, -1, 1111030501, quantity, true);
            }
            x if x == Weapon::WpBfg as i32 => {
                self.integer(slf, -1, 1111030800, 1, true);
                self.integer(slf, -1, 1111030801, quantity, true);
            }
            _ => {}
        }
    }

    /// Pickup-health reports.
    pub fn pickup_health(&self, slf: i32, quantity: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111040000, 1, true);
        self.integer(slf, -1, 1111040001, quantity, true);
        match quantity {
            5 => self.integer(slf, -1, 1111040100, 1, true),
            25 => self.integer(slf, -1, 1111040200, 1, true),
            50 => self.integer(slf, -1, 1111040300, 1, true),
            100 => self.integer(slf, -1, 1111040400, 1, true),
            _ => {}
        }
    }

    /// Pickup-armor reports.
    pub fn pickup_armor(&self, slf: i32, quantity: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111050000, 1, true);
        self.integer(slf, -1, 1111050001, quantity, true);
        match quantity {
            5 => self.integer(slf, -1, 1111050100, 1, true),
            50 => self.integer(slf, -1, 1111050200, 1, true),
            100 => self.integer(slf, -1, 1111050300, 1, true),
            _ => {}
        }
    }

    /// Pickup-powerup reports.
    pub fn pickup_powerup(&self, slf: i32, powerup: i32) {
        if self.is_warmup() {
            return;
        }
        if powerup == Powerup::PwRedflag as i32 || powerup == Powerup::PwBlueflag as i32 {
            self.integer(slf, -1, 1111110000, 1, true);
            return;
        }
        self.integer(slf, -1, 1111060000, 1, true);
        match powerup {
            x if x == Powerup::PwQuad as i32 => self.integer(slf, -1, 1111060100, 1, true),
            x if x == Powerup::PwBattlesuit as i32 => self.integer(slf, -1, 1111060200, 1, true),
            x if x == Powerup::PwHaste as i32 => self.integer(slf, -1, 1111060300, 1, true),
            x if x == Powerup::PwInvis as i32 => self.integer(slf, -1, 1111060400, 1, true),
            x if x == Powerup::PwRegen as i32 => self.integer(slf, -1, 1111060500, 1, true),
            x if x == Powerup::PwFlight as i32 => self.integer(slf, -1, 1111060600, 1, true),
            _ => {}
        }
    }

    /// Pickup-holdable reports.
    pub fn pickup_holdable(&self, slf: i32, holdable: i32) {
        if self.is_warmup() {
            return;
        }
        match holdable {
            x if x == Holdable::HiMedkit as i32 => self.integer(slf, -1, 1111070000, 1, true),
            x if x == Holdable::HiTeleporter as i32 => self.integer(slf, -1, 1111070100, 1, true),
            _ => {}
        }
    }

    /// Use-holdable reports.
    pub fn use_holdable(&self, slf: i32, holdable: i32) {
        if self.is_warmup() {
            return;
        }
        match holdable {
            x if x == Holdable::HiMedkit as i32 => self.integer(slf, -1, 1111070001, 1, true),
            x if x == Holdable::HiTeleporter as i32 => self.integer(slf, -1, 1111070101, 1, true),
            _ => {}
        }
    }

    /// Reward reports.
    pub fn reward(&self, slf: i32, award: i32) {
        if self.is_warmup() {
            return;
        }
        match award {
            0x8000 => self.integer(slf, -1, 1111090000, 1, true),
            0x8 => self.integer(slf, -1, 1111090100, 1, true),
            _ => {}
        }
    }

    /// Capture report.
    pub fn capture(&self, slf: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111110001, 1, true);
    }

    /// Damage reports.
    #[allow(clippy::too_many_arguments)]
    pub fn damage(
        &self,
        slf: i32,
        attacker: i32,
        damage: i32,
        means_of_death: i32,
        frame: i32,
        attacker_is_client: bool,
        same_team: bool,
    ) {
        if self.is_warmup() {
            return;
        }
        let hit = format!("{frame}:{slf}:{attacker}:{means_of_death}");
        let new_hit = hit != self.inner.borrow().last_hit;
        self.inner.borrow_mut().last_hit = hit;
        if attacker != 1022 && attacker != slf && means_of_death == 2 && attacker_is_client {
            self.integer(attacker, -1, 1111020102, 1, true);
        }
        match means_of_death {
            14 | 15 | 16 | 17 | 18 | 19 | 20 | 22 => return,
            _ => {}
        }
        let splash = match means_of_death {
            5 | 7 | 9 | 13 => damage,
            _ => 0,
        };
        let (key_hit, key_damage, mut key_splash) = match means_of_death {
            2 => (1111020104, 1111020106, -1),
            3 => (1111020204, 1111020206, -1),
            1 => (1111020304, 1111020306, -1),
            4 | 5 => (1111020404, 1111020406, 1111020408),
            6 | 7 => (1111020504, 1111020506, 1111020508),
            8 | 9 => (1111020604, 1111020606, 1111020608),
            10 => (1111020704, 1111020706, -1),
            11 => (1111020804, 1111020806, -1),
            12 | 13 => (1111020904, 1111020906, 1111020908),
            23 => (1111021004, 1111021006, -1),
            _ => (1111021104, 1111021106, -1),
        };
        if means_of_death != 5 && means_of_death != 7 && means_of_death != 9 && means_of_death != 13 {
            key_splash = -1;
        }
        if new_hit {
            self.integer(slf, -1, 1111020004, 1, true);
            self.integer(slf, -1, key_hit, 1, true);
        }
        self.integer(slf, -1, 1111020006, damage, true);
        self.integer(slf, -1, key_damage, damage, true);
        if splash != 0 {
            self.integer(slf, -1, 1111020008, splash, true);
            self.integer(slf, -1, key_splash, splash, true);
        }
        if attacker != 1022 && attacker != slf {
            let (key_hit, key_damage, key_splash) = match means_of_death {
                2 => (1111020103, 1111020105, -1),
                3 => (1111020203, 1111020205, -1),
                1 => (1111020303, 1111020305, -1),
                4 | 5 => (1111020403, 1111020405, 1111020407),
                6 | 7 => (1111020503, 1111020505, 1111020507),
                8 | 9 => (1111020603, 1111020605, 1111020607),
                10 => (1111020703, 1111020705, -1),
                11 => (1111020803, 1111020805, -1),
                12 | 13 => (1111020903, 1111020905, 1111020907),
                23 => (1111021003, 1111021005, -1),
                _ => (1111021103, 1111021105, -1),
            };
            if attacker_is_client {
                if new_hit {
                    self.integer(attacker, -1, 1111020003, 1, true);
                    self.integer(attacker, -1, key_hit, 1, true);
                }
                self.integer(attacker, -1, 1111020005, damage, true);
                self.integer(attacker, -1, key_damage, damage, true);
                if splash != 0 {
                    self.integer(attacker, -1, 1111020007, splash, true);
                    self.integer(attacker, -1, key_splash, splash, true);
                }
            }
        }
        if attacker != slf && same_team && attacker_is_client {
            if new_hit {
                self.integer(slf, -1, 1111100002, 1, true);
                self.integer(attacker, -1, 1111100001, 1, true);
            }
            self.integer(slf, -1, 1111100004, damage, true);
            self.integer(attacker, -1, 1111100003, damage, true);
            if splash != 0 {
                self.integer(slf, -1, 1111100006, splash, true);
                self.integer(attacker, -1, 1111100005, splash, true);
            }
        }
    }

    /// Player-die reports.
    pub fn player_die(&self, slf: i32, attacker: i32, means_of_death: i32) {
        if self.is_warmup() {
            return;
        }
        if attacker == 1022 {
            self.integer(slf, -1, 1111080000, 1, true);
            match means_of_death {
                14 => self.integer(slf, -1, 1111080100, 1, true),
                15 => self.integer(slf, -1, 1111080200, 1, true),
                16 => self.integer(slf, -1, 1111080300, 1, true),
                17 => self.integer(slf, -1, 1111080400, 1, true),
                18 => self.integer(slf, -1, 1111080500, 1, true),
                19 => self.integer(slf, -1, 1111080600, 1, true),
                20 => self.integer(slf, -1, 1111080700, 1, true),
                22 => self.integer(slf, -1, 1111080800, 1, true),
                _ => self.integer(slf, -1, 1111080900, 1, true),
            }
        } else if attacker == slf {
            self.integer(slf, -1, 1111020001, 1, true);
            match means_of_death {
                2 => self.integer(slf, -1, 1111020101, 1, true),
                3 => self.integer(slf, -1, 1111020201, 1, true),
                1 => self.integer(slf, -1, 1111020301, 1, true),
                4 | 5 => self.integer(slf, -1, 1111020401, 1, true),
                6 | 7 => self.integer(slf, -1, 1111020501, 1, true),
                8 | 9 => self.integer(slf, -1, 1111020601, 1, true),
                10 => self.integer(slf, -1, 1111020701, 1, true),
                11 => self.integer(slf, -1, 1111020801, 1, true),
                12 | 13 => self.integer(slf, -1, 1111020901, 1, true),
                23 => self.integer(slf, -1, 1111021001, 1, true),
                _ => self.integer(slf, -1, 1111021101, 1, true),
            }
        } else {
            self.integer(attacker, slf, 1211020000, 1, true);
            match means_of_death {
                2 => self.integer(attacker, slf, 1211020100, 1, true),
                3 => self.integer(attacker, slf, 1211020200, 1, true),
                1 => self.integer(attacker, slf, 1211020300, 1, true),
                4 | 5 => self.integer(attacker, slf, 1211020400, 1, true),
                6 | 7 => self.integer(attacker, slf, 1211020500, 1, true),
                8 | 9 => self.integer(attacker, slf, 1211020600, 1, true),
                10 => self.integer(attacker, slf, 1211020700, 1, true),
                11 => self.integer(attacker, slf, 1211020800, 1, true),
                12 | 13 => self.integer(attacker, slf, 1211020900, 1, true),
                23 => self.integer(attacker, slf, 1211021000, 1, true),
                _ => self.integer(attacker, slf, 1211021100, 1, true),
            }
        }
    }

    /// Weapon-time reports.
    pub fn weapon_time(&self, slf: i32, weapon: i32, time: i32) {
        if time <= 0 || self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111020010, time, true);
        match weapon {
            x if x == Weapon::WpGauntlet as i32 => self.integer(slf, -1, 1111020110, time, true),
            x if x == Weapon::WpMachinegun as i32 => self.integer(slf, -1, 1111020210, time, true),
            x if x == Weapon::WpShotgun as i32 => self.integer(slf, -1, 1111020310, time, true),
            x if x == Weapon::WpGrenadeLauncher as i32 => self.integer(slf, -1, 1111020410, time, true),
            x if x == Weapon::WpRocketLauncher as i32 => self.integer(slf, -1, 1111020510, time, true),
            x if x == Weapon::WpLightning as i32 => self.integer(slf, -1, 1111020810, time, true),
            x if x == Weapon::WpRailgun as i32 => self.integer(slf, -1, 1111020710, time, true),
            x if x == Weapon::WpPlasmagun as i32 => self.integer(slf, -1, 1111020610, time, true),
            x if x == Weapon::WpBfg as i32 => self.integer(slf, -1, 1111020910, time, true),
            x if x == Weapon::WpGrapplingHook as i32 => self.integer(slf, -1, 1111021010, time, true),
            _ => {}
        }
    }

    /// Team-name report.
    pub fn team_name(&self, slf: i32, name: &str) {
        self.string(slf, -1, 1100100007, name);
    }
}

impl Default for Q3RankingReports {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;

    use crate::q3::base::shared::definitions::Weapon;
    use std::rc::Rc;

    #[test]
    fn rankings_gate_accumulate_and_deduplicate() {
        let reports = Q3RankingReports::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = Rc::clone(&seen);
        let detach = reports
            .attach(
                Rc::new(move |report| seen_clone.borrow_mut().push(report)),
                Rc::new(|| false),
            )
            .unwrap();
        assert!(reports.attach(Rc::new(|_| {}), Rc::new(|| false)).is_err());
        reports.fire_weapon(0, Weapon::WpMachinegun as i32);
        assert_eq!(seen.borrow().len(), 2);
        reports.damage(1, 2, 10, 3, 99, true, false);
        let after_first = seen.borrow().len();
        reports.damage(1, 2, 10, 3, 99, true, false);
        let after_second = seen.borrow().len();
        assert!(after_second > after_first);
        reports.player_die(1, 1022, 14);
        reports.team_name(0, "red");
        detach();
        let before = seen.borrow().len();
        reports.capture(0);
        assert_eq!(seen.borrow().len(), before);
    }
}
