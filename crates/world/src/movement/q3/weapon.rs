//! Quake III weapon states (`PM_Weapon` from `bg_pmove.c`).
//!
//! Donor provenance: `src/movement/q3/weapon.ts` (from id Software
//! `code/game/bg_pmove.c`).

use std::collections::HashMap;

use super::constants::{
    command_buttons as B, entity_event, holdable, move_flags as F, player_animation as A, powerup, weapon as W,
    weapon_state as S,
};
use super::types::Q3Product;

/// External weapon slot phase, mirroring donor `Q3ExternalWeaponSlot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ExternalWeaponSlot {
    /// Active.
    Active,
    /// Holster requested.
    HolsterRequested,
    /// Dropping.
    Dropping,
    /// Holstered.
    Holstered,
    /// Resume requested.
    ResumeRequested,
}

/// Source weapon-step state, mirroring donor `Q3SourceWeaponState`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourceWeaponState {
    /// Product selector.
    pub product: Q3Product,
    /// Movement flags.
    pub pm_flags: i32,
    /// Active source weapon.
    pub weapon: i32,
    /// Weapon state word.
    pub weapon_state: i32,
    /// Weapon timer in milliseconds.
    pub weapon_time: i32,
    /// Owned-weapons bitmask.
    pub owned_weapons: i32,
    /// Health.
    pub health: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Spectator flag.
    pub spectator: bool,
    /// Haste granted.
    pub haste: bool,
    /// Persistent powerup tag.
    pub persistent_powerup_tag: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Holdable tag.
    pub holdable_tag: i32,
    /// Ammo counters (-1 is infinite).
    pub ammo: HashMap<i32, i32>,
}

/// Weapon-step command view, mirroring donor `Pick<Q3Command, ...>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WeaponCommand {
    /// Button bitmask.
    pub buttons: i32,
    /// Requested weapon.
    pub weapon: i32,
}

/// Source weapon-step options, mirroring donor `Q3SourceWeaponOptions`.
pub struct Q3SourceWeaponOptions<'a> {
    /// Step in milliseconds.
    pub msec: i32,
    /// Gauntlet hit flag.
    pub gauntlet_hit: bool,
    /// External weapon slot phase.
    pub external_slot: Option<Q3ExternalWeaponSlot>,
    /// Firing-delay resolver.
    pub firing_delay: Option<Box<dyn Fn(i32) -> i32 + 'a>>,
    /// Event callback.
    pub event: Box<dyn FnMut(i32) + 'a>,
    /// Torso-start callback.
    pub start_torso: Box<dyn FnMut(i32) + 'a>,
}

/// Haste/scout firing-delay scaling.
#[must_use]
pub fn q3_weapon_delay(milliseconds: i32, persistent: i32, haste: bool) -> i32 {
    if persistent == powerup::SCOUT {
        (f64::from(milliseconds) / 1.5).trunc() as i32
    } else if persistent == powerup::AMMOREGEN || haste {
        (f64::from(milliseconds) / 1.3).trunc() as i32
    } else {
        milliseconds
    }
}

/// Holdable-step state, mirroring donor `Q3HoldableState`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3HoldableState {
    /// Movement flags.
    pub pm_flags: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Holdable tag.
    pub holdable_tag: i32,
    /// Health.
    pub health: f64,
    /// Maximum health.
    pub max_health: f64,
}

/// True consumes this weapon step; an already-held button permits normal
/// primary processing.
pub fn step_q3_holdable(state: &mut Q3HoldableState, pressed: bool, event: &mut dyn FnMut(i32)) -> bool {
    if pressed {
        if state.pm_flags & F::USE_ITEM_HELD == 0 {
            let tag = state.holdable_tag;
            if tag != holdable::MEDKIT || state.health < state.max_health + 25.0 {
                state.pm_flags |= F::USE_ITEM_HELD;
                event(entity_event::USE_ITEM0 + tag);
                state.holdable_item = 0;
                state.holdable_tag = 0;
            }
            return true;
        }
    } else {
        state.pm_flags &= !F::USE_ITEM_HELD;
    }
    false
}

struct WeaponStep<'a, 'b, 'c> {
    state: &'a mut Q3SourceWeaponState,
    cmd: Q3WeaponCommand,
    options: &'b mut Q3SourceWeaponOptions<'c>,
}

impl WeaponStep<'_, '_, '_> {
    fn event(&mut self, event: i32) {
        (self.options.event)(event);
    }

    fn start_torso(&mut self, animation: i32) {
        (self.options.start_torso)(animation);
    }

    fn begin_drop(&mut self) {
        self.event(entity_event::CHANGE_WEAPON);
        self.state.weapon_state = S::DROPPING;
        self.state.weapon_time += 200;
        self.start_torso(A::TORSO_DROP);
    }

    fn begin_weapon_change(&mut self, weapon: i32) {
        let cap = if self.state.product == Q3Product::MissionPack {
            14
        } else {
            11
        };
        if weapon <= W::NONE
            || weapon >= cap
            || self.state.owned_weapons & (1 << weapon) == 0
            || self.state.weapon_state == S::DROPPING
        {
            return;
        }
        self.begin_drop();
    }

    fn finish_weapon_change(&mut self) {
        let requested = self.cmd.weapon;
        let mut weapon = match requested {
            W::NONE
            | W::GAUNTLET
            | W::MACHINEGUN
            | W::SHOTGUN
            | W::GRENADE_LAUNCHER
            | W::ROCKET_LAUNCHER
            | W::LIGHTNING
            | W::RAILGUN
            | W::PLASMAGUN
            | W::BFG
            | W::GRAPPLING_HOOK
            | W::NAILGUN
            | W::PROX_LAUNCHER
            | W::CHAINGUN => requested,
            _ => W::NONE,
        };
        let cap = if self.state.product == Q3Product::MissionPack {
            14
        } else {
            11
        };
        if weapon >= cap {
            weapon = W::NONE;
        }
        if self.state.owned_weapons & (1 << weapon) == 0 {
            weapon = W::NONE;
        }
        self.state.weapon = weapon;
        self.state.weapon_state = S::RAISING;
        self.state.weapon_time += 250;
        self.start_torso(A::TORSO_RAISE);
    }

    fn run(&mut self) {
        if self.state.pm_flags & F::RESPAWNED != 0 || self.state.spectator {
            return;
        }
        if self.state.health <= 0.0 {
            self.state.weapon = W::NONE;
            return;
        }
        let pressed = self.cmd.buttons & B::USE_HOLDABLE != 0;
        let mut holdable = Q3HoldableState {
            pm_flags: self.state.pm_flags,
            holdable_item: self.state.holdable_item,
            holdable_tag: self.state.holdable_tag,
            health: self.state.health,
            max_health: self.state.max_health,
        };
        let consumed = {
            let event = &mut *self.options.event;
            step_q3_holdable(&mut holdable, pressed, event)
        };
        self.state.pm_flags = holdable.pm_flags;
        self.state.holdable_item = holdable.holdable_item;
        self.state.holdable_tag = holdable.holdable_tag;
        if consumed {
            return;
        }
        if self.state.weapon_time > 0 {
            self.state.weapon_time -= self.options.msec;
        }
        match self.options.external_slot {
            Some(Q3ExternalWeaponSlot::Holstered) => return,
            Some(Q3ExternalWeaponSlot::Dropping) => {
                if self.state.weapon_time <= 0 {
                    self.options.external_slot = Some(Q3ExternalWeaponSlot::Holstered);
                }
                return;
            }
            Some(Q3ExternalWeaponSlot::ResumeRequested) => {
                if self.state.weapon_time > 0 {
                    return;
                }
                self.finish_weapon_change();
                self.options.external_slot = Some(Q3ExternalWeaponSlot::Active);
                return;
            }
            Some(Q3ExternalWeaponSlot::HolsterRequested)
                if self.state.weapon_time <= 0
                    && (self.state.weapon_state == S::READY || self.state.weapon_state == S::FIRING) =>
            {
                let requested = self.cmd.weapon;
                let cap = if self.state.product == Q3Product::MissionPack {
                    14
                } else {
                    11
                };
                let native_switch = requested != self.state.weapon
                    && requested > W::NONE
                    && requested < cap
                    && self.state.owned_weapons & (1 << requested) != 0;
                if !native_switch {
                    self.begin_drop();
                    self.options.external_slot = Some(Q3ExternalWeaponSlot::Dropping);
                    return;
                }
            }
            _ => {}
        }
        if self.state.weapon_time <= 0 || self.state.weapon_state != S::FIRING {
            if self.state.weapon != self.cmd.weapon {
                let requested = self.cmd.weapon;
                self.begin_weapon_change(requested);
            }
        }
        if self.state.weapon_time > 0 {
            return;
        }
        if self.state.weapon_state == S::DROPPING {
            self.finish_weapon_change();
            return;
        }
        if self.state.weapon_state == S::RAISING {
            self.state.weapon_state = S::READY;
            let animation = if self.state.weapon == W::GAUNTLET {
                A::TORSO_STAND2
            } else {
                A::TORSO_STAND
            };
            self.start_torso(animation);
            return;
        }
        if self.cmd.buttons & B::ATTACK == 0 || (self.state.weapon == W::GAUNTLET && !self.options.gauntlet_hit) {
            self.state.weapon_time = 0;
            self.state.weapon_state = S::READY;
            return;
        }
        let animation = if self.state.weapon == W::GAUNTLET {
            A::TORSO_ATTACK2
        } else {
            A::TORSO_ATTACK
        };
        self.start_torso(animation);
        self.state.weapon_state = S::FIRING;
        let ammo = self.state.ammo.get(&self.state.weapon).copied().unwrap_or(0);
        if ammo == 0 {
            self.event(entity_event::NOAMMO);
            self.state.weapon_time += 500;
            return;
        }
        if ammo != -1 {
            self.state.ammo.insert(self.state.weapon, ammo - 1);
        }
        self.event(entity_event::FIRE_WEAPON);
        let mission = self.state.product == Q3Product::MissionPack;
        let add_time = match self.state.weapon {
            W::LIGHTNING => 50,
            W::SHOTGUN => 1000,
            W::MACHINEGUN | W::PLASMAGUN => 100,
            W::GRENADE_LAUNCHER | W::ROCKET_LAUNCHER => 800,
            W::RAILGUN => 1500,
            W::BFG => 200,
            W::NAILGUN => {
                if mission {
                    1000
                } else {
                    400
                }
            }
            W::PROX_LAUNCHER => {
                if mission {
                    800
                } else {
                    400
                }
            }
            W::CHAINGUN => {
                if mission {
                    30
                } else {
                    400
                }
            }
            _ => 400,
        };
        let persistent = if mission { self.state.persistent_powerup_tag } else { 0 };
        let delay = match self.options.firing_delay.as_ref() {
            Some(delay) => delay(add_time),
            None => q3_weapon_delay(add_time, persistent, self.state.haste),
        };
        self.state.weapon_time += delay;
    }
}

/// Called only by the selected Q3 arsenal adapter at its movement weapon
/// stage.
pub fn run_q3_weapon_step(
    state: &mut Q3SourceWeaponState,
    command: Q3WeaponCommand,
    options: &mut Q3SourceWeaponOptions<'_>,
) {
    WeaponStep {
        state,
        cmd: command,
        options,
    }
    .run();
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    fn state() -> Q3SourceWeaponState {
        Q3SourceWeaponState {
            product: Q3Product::BaseQ3,
            pm_flags: 0,
            weapon: W::MACHINEGUN,
            weapon_state: S::READY,
            weapon_time: 0,
            owned_weapons: (1 << W::GAUNTLET) | (1 << W::MACHINEGUN),
            health: 100.0,
            max_health: 100.0,
            spectator: false,
            haste: false,
            persistent_powerup_tag: 0,
            holdable_item: 0,
            holdable_tag: 0,
            ammo: [(W::MACHINEGUN, 50), (W::GAUNTLET, -1)].into_iter().collect(),
        }
    }

    fn options(events: Rc<RefCell<Vec<i32>>>, torsos: Rc<RefCell<Vec<i32>>>) -> Q3SourceWeaponOptions<'static> {
        Q3SourceWeaponOptions {
            msec: 16,
            gauntlet_hit: false,
            external_slot: None,
            firing_delay: None,
            event: Box::new(move |event| events.borrow_mut().push(event)),
            start_torso: Box::new(move |animation| torsos.borrow_mut().push(animation)),
        }
    }

    fn recorders() -> (Rc<RefCell<Vec<i32>>>, Rc<RefCell<Vec<i32>>>) {
        (Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(Vec::new())))
    }

    #[test]
    fn attack_fires_and_consumes_ammo() {
        let mut state = state();
        let (events, torsos) = recorders();
        let mut options = options(Rc::clone(&events), Rc::clone(&torsos));
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: B::ATTACK,
                weapon: W::MACHINEGUN,
            },
            &mut options,
        );
        assert_eq!(state.weapon_state, S::FIRING);
        assert_eq!(state.weapon_time, 100);
        assert_eq!(state.ammo[&W::MACHINEGUN], 49);
        assert_eq!(*events.borrow(), vec![entity_event::FIRE_WEAPON]);
        assert_eq!(*torsos.borrow(), vec![A::TORSO_ATTACK]);
    }

    #[test]
    fn empty_weapon_reports_noammo() {
        let mut state = state();
        state.ammo.insert(W::MACHINEGUN, 0);
        let (events, _torsos) = recorders();
        let mut options = options(Rc::clone(&events), Rc::new(RefCell::new(Vec::new())));
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: B::ATTACK,
                weapon: W::MACHINEGUN,
            },
            &mut options,
        );
        assert_eq!(*events.borrow(), vec![entity_event::NOAMMO]);
        assert_eq!(state.weapon_time, 500);
    }

    #[test]
    fn weapon_change_drops_then_finishes() {
        let mut state = state();
        state.owned_weapons |= 1 << W::SHOTGUN;
        let (events, torsos) = recorders();
        let mut options = options(Rc::clone(&events), Rc::clone(&torsos));
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: 0,
                weapon: W::SHOTGUN,
            },
            &mut options,
        );
        assert_eq!(state.weapon_state, S::DROPPING);
        assert_eq!(*events.borrow(), vec![entity_event::CHANGE_WEAPON]);
        state.weapon_time = 0;
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: 0,
                weapon: W::SHOTGUN,
            },
            &mut options,
        );
        assert_eq!((state.weapon, state.weapon_state), (W::SHOTGUN, S::RAISING));
    }

    #[test]
    fn gauntlet_needs_hit_and_uses_fixed_delay() {
        let mut state = state();
        state.weapon = W::GAUNTLET;
        let (events, torsos) = recorders();
        let mut options = options(Rc::clone(&events), Rc::clone(&torsos));
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: B::ATTACK,
                weapon: W::GAUNTLET,
            },
            &mut options,
        );
        assert_eq!(state.weapon_state, S::READY);
        assert!(events.borrow().is_empty());
        options.gauntlet_hit = true;
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: B::ATTACK,
                weapon: W::GAUNTLET,
            },
            &mut options,
        );
        assert_eq!((state.weapon_state, state.weapon_time), (S::FIRING, 400));
        assert_eq!(torsos.borrow().last(), Some(&A::TORSO_ATTACK2));
    }

    #[test]
    fn holdable_use_consumes_step_and_emits_tag() {
        let mut state = state();
        state.holdable_item = 1;
        state.holdable_tag = holdable::TELEPORTER;
        let (events, torsos) = recorders();
        let mut options = options(Rc::clone(&events), Rc::clone(&torsos));
        run_q3_weapon_step(
            &mut state,
            Q3WeaponCommand {
                buttons: B::ATTACK | B::USE_HOLDABLE,
                weapon: W::MACHINEGUN,
            },
            &mut options,
        );
        assert_eq!(*events.borrow(), vec![entity_event::USE_ITEM0 + holdable::TELEPORTER]);
        assert_eq!((state.holdable_item, state.holdable_tag), (0, 0));
        assert_eq!(state.weapon_state, S::READY);
    }

    #[test]
    fn delay_scaling_matches_source() {
        assert_eq!(q3_weapon_delay(150, powerup::SCOUT, false), 100);
        assert_eq!(q3_weapon_delay(130, powerup::AMMOREGEN, false), 100);
        assert_eq!(q3_weapon_delay(130, 0, true), 100);
        assert_eq!(q3_weapon_delay(130, 0, false), 130);
    }
}
