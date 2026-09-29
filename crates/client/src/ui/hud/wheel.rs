//! Weapon and powerup wheels.
//!
//! Ported from the TypeScript donor's `src/ui/hud/wheel.ts`: per-seat weapon
//! and powerup wheel plus the weapon carousel, adapted from q2repro
//! `client/wheel.c`. All angles, fades, timeouts, and the q2 slot-zero
//! deselection quirk keep the donor's behavior.

use std::f64::consts::PI;

use qa_core::identity::SeatId;
use qa_core::math::Vec2;

use crate::error::ClientError;
use crate::input::ControllerAxis;
use crate::ui::types::{ResourceId, SeatInputEvent, SeatInputEventKind};

/// One selectable wheel entry.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelItem {
    /// Stable item identity.
    pub id: String,
    /// Source ordering before sorting.
    pub source_ordinal: i32,
    /// Primary sort key.
    pub sort_order: i32,
    /// Display label.
    pub label: String,
    /// Whether the seat owns this item.
    pub owned: bool,
    /// Whether the item currently has ammo.
    pub has_ammo: bool,
    /// Remaining count, when the item tracks one.
    pub count: Option<f32>,
    /// Count below which the HUD warns.
    pub warning_count: i32,
    /// Unselected icon.
    pub icon: Option<ResourceId>,
    /// Selected icon.
    pub selected_icon: Option<ResourceId>,
}

/// Which item list the wheel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WheelMode {
    /// Weapons.
    Weapons,
    /// Powerups.
    Powerups,
}

/// Drawable wheel state.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelPresentation {
    /// Active mode.
    pub mode: WheelMode,
    /// Sorted items.
    pub items: Vec<WheelItem>,
    /// Selected item id, if any.
    pub selected: Option<String>,
    /// Fade opacity in `0..=1`.
    pub opacity: f32,
    /// Cursor in `-1..=1` wheel units.
    pub cursor: Vec2,
}

/// Drawable carousel state.
#[derive(Debug, Clone, PartialEq)]
pub struct CarouselPresentation {
    /// Owned weapon items.
    pub items: Vec<WheelItem>,
    /// Carousel-selected item id, if any.
    pub selected: Option<String>,
}

/// Outcome of [`SeatWeaponWheel::command`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Whether the weapon should render holstered.
    pub holster: bool,
    /// Whether the attack press was consumed by carousel selection.
    pub consume_attack: bool,
}

/// Drawable wheel plus carousel state.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawState {
    /// Wheel presentation while open or fading out.
    pub wheel: Option<WheelPresentation>,
    /// Carousel presentation while the carousel is open.
    pub carousel: Option<CarouselPresentation>,
}

/// Services the wheel needs from its seat.
pub trait WeaponWheelServices {
    /// Owning seat.
    fn seat(&self) -> SeatId;
    /// Items for a mode, in source order.
    fn items(&self, mode: WheelMode) -> Vec<WheelItem>;
    /// Currently active item id, if any.
    fn active_item(&self) -> Option<String>;
    /// Select an item for a seat.
    fn select(&mut self, id: &str, mode: WheelMode, seat: SeatId);
    /// Current time in milliseconds.
    fn now(&self) -> i64;
    /// Notify that wheel presentation changed for a seat.
    fn changed(&mut self, seat: SeatId);
}

/// Wheel geometry and timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelOptions {
    /// Wheel radius in cursor units.
    pub radius: f32,
    /// Cursor distance past which a slice selects.
    pub selection_distance: f32,
    /// Opacity change per second.
    pub fade_per_second: f32,
    /// Milliseconds the carousel stays open.
    pub carousel_timeout: i64,
    /// Milliseconds the weapon stays locked after carousel selection.
    pub carousel_lock: i64,
    /// Retains q2repro's slot-zero deselection quirk for its source profile.
    pub q2_slot_zero_deselect: bool,
}

/// Default wheel options: 180 radius, 140 selection distance, 3/second fade,
/// 400ms carousel timeout, 300ms carousel lock, slot-zero quirk on.
pub const DEFAULT_WHEEL_OPTIONS: WheelOptions = WheelOptions {
    radius: 180.0,
    selection_distance: 140.0,
    fade_per_second: 3.0,
    carousel_timeout: 400,
    carousel_lock: 300,
    q2_slot_zero_deselect: true,
};

/// Milliseconds the cursor must rest inside the dead zone before deselecting.
const DESELECT_DELAY_MS: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WheelState {
    Closed,
    Open { mode: WheelMode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CarouselState {
    Closed,
    Open { until: i64 },
    Closing { until: i64 },
}

/// Per-seat weapon wheel and carousel.
pub struct SeatWeaponWheel<S: WeaponWheelServices> {
    services: S,
    options: WheelOptions,
    state: WheelState,
    carousel: CarouselState,
    mode: WheelMode,
    selected: Option<String>,
    carousel_selected: Option<String>,
    position: Vec2,
    analog: Vec2,
    deselect_time: i64,
    opacity: f32,
    last_update: i64,
    lock_until: i64,
}

impl<S: WeaponWheelServices> SeatWeaponWheel<S> {
    /// Build a wheel, requiring the radius to exceed the selection distance.
    pub fn new(services: S, options: WheelOptions) -> Result<Self, ClientError> {
        if options.radius <= 0.0 || options.selection_distance < 0.0 || options.selection_distance >= options.radius {
            return Err(ClientError::BadUi(
                "Wheel radius must exceed selection distance".to_string(),
            ));
        }
        let last_update = services.now();
        Ok(Self {
            services,
            options,
            state: WheelState::Closed,
            carousel: CarouselState::Closed,
            mode: WheelMode::Weapons,
            selected: None,
            carousel_selected: None,
            position: Vec2 { x: 0.0, y: 0.0 },
            analog: Vec2 { x: 0.0, y: 0.0 },
            deselect_time: 0,
            opacity: 0.0,
            last_update,
            lock_until: 0,
        })
    }

    /// Owning seat.
    #[must_use]
    pub fn seat(&self) -> SeatId {
        self.services.seat()
    }

    /// Whether the wheel is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state, WheelState::Open { .. })
    }

    /// Time scale while the wheel fades in or out.
    #[must_use]
    pub fn time_scale(&self) -> f32 {
        (1.0 - self.opacity).max(0.1)
    }

    /// Time in milliseconds until which the weapon stays locked.
    #[must_use]
    pub fn weapon_lock_until(&self) -> i64 {
        self.lock_until
    }

    /// Whether the weapon should render holstered.
    #[must_use]
    pub fn holster(&self) -> bool {
        (self.state != WheelState::Closed && self.mode == WheelMode::Weapons)
            || matches!(self.carousel, CarouselState::Open { .. })
    }

    /// Wheel options.
    #[must_use]
    pub fn options(&self) -> WheelOptions {
        self.options
    }

    fn items(&self, mode: WheelMode) -> Vec<WheelItem> {
        let mut items = self.services.items(mode);
        items.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then(a.source_ordinal.cmp(&b.source_ordinal))
        });
        items
    }

    /// Open the wheel on a mode; returns false when the mode has no items.
    pub fn open(&mut self, mode: WheelMode) -> bool {
        if self.items(mode).is_empty() {
            return false;
        }
        self.mode = mode;
        self.state = WheelState::Open { mode };
        self.selected = None;
        self.deselect_time = 0;
        self.position = Vec2 { x: 0.0, y: 0.0 };
        self.analog = Vec2 { x: 0.0, y: 0.0 };
        let seat = self.services.seat();
        self.services.changed(seat);
        true
    }

    /// Close the wheel, selecting the highlighted owned item when asked.
    pub fn close(&mut self, select: bool) {
        if !matches!(self.state, WheelState::Open { .. }) {
            return;
        }
        self.state = WheelState::Closed;
        let selected = self
            .items(self.mode)
            .into_iter()
            .find(|item| Some(item.id.as_str()) == self.selected.as_deref());
        if select {
            if let Some(item) = selected {
                if item.owned {
                    let seat = self.services.seat();
                    self.services.select(&item.id, self.mode, seat);
                }
            }
        }
        let seat = self.services.seat();
        self.services.changed(seat);
    }

    /// Feed a seat input event; returns whether the wheel consumed it.
    pub fn input(&mut self, event: &SeatInputEvent) -> Result<bool, ClientError> {
        if event.seat != self.services.seat() {
            return Err(ClientError::BadUi(
                "Weapon wheel input belongs to another seat".to_string(),
            ));
        }
        if !matches!(self.state, WheelState::Open { .. }) {
            return Ok(false);
        }
        match &event.kind {
            SeatInputEventKind::Focus { focused } => {
                if !focused {
                    self.close(false);
                    return Ok(true);
                }
            }
            SeatInputEventKind::MouseMotion { delta, .. } => {
                let next = Vec2 {
                    x: self.position.x + delta.x,
                    y: self.position.y + delta.y,
                };
                self.slide(next);
            }
            SeatInputEventKind::ControllerAxis { axis, value, .. } => {
                if *axis == ControllerAxis::RightX {
                    self.analog.x = *value;
                    let next = Vec2 {
                        x: self.analog.x * self.options.radius,
                        y: self.analog.y * self.options.radius,
                    };
                    self.slide(next);
                } else if *axis == ControllerAxis::RightY {
                    self.analog.y = *value;
                    let next = Vec2 {
                        x: self.analog.x * self.options.radius,
                        y: self.analog.y * self.options.radius,
                    };
                    self.slide(next);
                }
            }
            _ => {}
        }
        Ok(matches!(
            event.kind,
            SeatInputEventKind::MouseMotion { .. } | SeatInputEventKind::ControllerAxis { .. }
        ))
    }

    fn slide(&mut self, position: Vec2) {
        let distance = position.x.hypot(position.y);
        let factor = if distance > self.options.radius {
            self.options.radius / distance
        } else {
            1.0
        };
        self.position = Vec2 {
            x: position.x * factor,
            y: position.y * factor,
        };
    }

    /// Step the weapon carousel, skipping owned items without ammo.
    ///
    /// Positive directions move forward, negative directions move backward.
    pub fn cycle(&mut self, direction: i32) {
        let items: Vec<WheelItem> = self
            .items(WheelMode::Weapons)
            .into_iter()
            .filter(|item| item.owned)
            .collect();
        if items.is_empty() {
            self.carousel = CarouselState::Closed;
            return;
        }
        if !matches!(self.carousel, CarouselState::Open { .. }) {
            self.carousel_selected = self.services.active_item();
        }
        let start = items
            .iter()
            .position(|item| Some(item.id.as_str()) == self.carousel_selected.as_deref())
            .map_or(if direction > 0 { -1 } else { 0 }, |index| index as i32);
        let len = items.len() as i32;
        for offset in 1..=len {
            let index = (start + offset * direction).rem_euclid(len) as usize;
            if let Some(candidate) = items.get(index) {
                if candidate.has_ammo {
                    self.carousel_selected = Some(candidate.id.clone());
                    break;
                }
            }
        }
        let until = self.services.now() + self.options.carousel_timeout;
        self.carousel = CarouselState::Open { until };
        let seat = self.services.seat();
        self.services.changed(seat);
    }

    /// Called once for the seat's command, before its provider encodes
    /// source button bits.
    pub fn command(&mut self, attack: bool, now_ms: i64) -> CommandOutcome {
        if let CarouselState::Closing { until } = self.carousel {
            if now_ms >= until {
                self.carousel = CarouselState::Closed;
            }
        }
        let selecting = matches!(self.carousel, CarouselState::Open { .. });
        if let CarouselState::Open { until } = self.carousel {
            if attack || now_ms >= until {
                let selected = self.items(WheelMode::Weapons).into_iter().find(|item| {
                    Some(item.id.as_str()) == self.carousel_selected.as_deref() && item.owned && item.has_ammo
                });
                let active = self.services.active_item();
                if selected
                    .as_ref()
                    .is_some_and(|item| Some(item.id.as_str()) != active.as_deref())
                {
                    let id = selected.map(|item| item.id).unwrap_or_default();
                    let seat = self.services.seat();
                    self.services.select(&id, WheelMode::Weapons, seat);
                    self.lock_until = now_ms + self.options.carousel_lock;
                    self.carousel = CarouselState::Closing { until: self.lock_until };
                } else {
                    self.carousel = CarouselState::Closed;
                }
            }
        }
        CommandOutcome {
            holster: self.holster() || selecting,
            consume_attack: attack && selecting,
        }
    }

    /// Run [`SeatWeaponWheel::command`] with the attack button held.
    pub fn attack(&mut self, now_ms: i64) -> CommandOutcome {
        self.command(true, now_ms)
    }

    /// Advance fades and selection using the services clock.
    pub fn update(&mut self) {
        let now = self.services.now();
        self.update_at(now);
    }

    /// Advance fades and selection at an explicit time in milliseconds.
    pub fn update_at(&mut self, now_ms: i64) {
        let elapsed = (now_ms - self.last_update).max(0) as f32 / 1000.0;
        self.last_update = now_ms;
        let drift = elapsed * self.options.fade_per_second * if self.is_open() { 1.0 } else { -1.0 };
        self.opacity = (self.opacity + drift).clamp(0.0, 1.0);
        if !self.is_open() {
            return;
        }
        let items = self.items(self.mode);
        if items.is_empty() {
            self.close(false);
            return;
        }
        let distance = f64::from(self.position.x.hypot(self.position.y));
        let slice = PI * 2.0 / items.len() as f64;
        let prior = self.selected.clone();
        if distance > f64::from(self.options.selection_distance) {
            for (index, item) in items.iter().enumerate() {
                if !item.owned {
                    continue;
                }
                let angle = slice * index as f64;
                let dot = f64::from(self.position.x) / distance * angle.sin()
                    - f64::from(self.position.y) / distance * angle.cos();
                if dot > (slice / 2.0).cos() {
                    self.selected = Some(item.id.clone());
                    self.deselect_time = 0;
                }
            }
        } else {
            let selected_index = items
                .iter()
                .position(|item| Some(item.id.as_str()) == self.selected.as_deref());
            let armed = if self.options.q2_slot_zero_deselect {
                selected_index.is_some_and(|index| index != 0)
            } else {
                selected_index.is_some()
            };
            if armed && self.deselect_time == 0 {
                self.deselect_time = now_ms + DESELECT_DELAY_MS;
            }
        }
        if self.deselect_time != 0 && self.deselect_time < now_ms {
            self.selected = None;
            self.deselect_time = 0;
        }
        if prior != self.selected {
            let seat = self.services.seat();
            self.services.changed(seat);
        }
    }

    /// Current drawable wheel and carousel state.
    #[must_use]
    pub fn draw_state(&self) -> DrawState {
        let wheel = if self.opacity > 0.0 || self.is_open() {
            Some(WheelPresentation {
                mode: self.mode,
                items: self.items(self.mode),
                selected: self.selected.clone(),
                opacity: self.opacity,
                cursor: Vec2 {
                    x: self.position.x / self.options.radius,
                    y: self.position.y / self.options.radius,
                },
            })
        } else {
            None
        };
        let carousel = if matches!(self.carousel, CarouselState::Open { .. }) {
            Some(CarouselPresentation {
                items: self
                    .items(WheelMode::Weapons)
                    .into_iter()
                    .filter(|item| item.owned)
                    .collect(),
                selected: self.carousel_selected.clone(),
            })
        } else {
            None
        };
        DrawState { wheel, carousel }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    #[derive(Debug, Clone)]
    struct FakeServices {
        seat: SeatId,
        weapons: Vec<WheelItem>,
        powerups: Vec<WheelItem>,
        active: Option<String>,
        now_ms: i64,
        selected: Vec<(String, WheelMode)>,
        changed_count: usize,
    }

    fn item(id: &str, order: i32, owned: bool, has_ammo: bool) -> WheelItem {
        WheelItem {
            id: id.to_string(),
            source_ordinal: order,
            sort_order: order,
            label: id.to_string(),
            owned,
            has_ammo,
            count: None,
            warning_count: 0,
            icon: None,
            selected_icon: None,
        }
    }

    impl FakeServices {
        fn new(seat: SeatId) -> Self {
            Self {
                seat,
                weapons: vec![
                    item("blaster", 0, true, true),
                    item("shotgun", 1, true, true),
                    item("machinegun", 2, true, false),
                    item("rocket", 3, false, false),
                ],
                powerups: vec![item("quad", 0, true, true)],
                active: Some("blaster".to_string()),
                now_ms: 1_000,
                selected: Vec::new(),
                changed_count: 0,
            }
        }
    }

    impl WeaponWheelServices for FakeServices {
        fn seat(&self) -> SeatId {
            self.seat.clone()
        }

        fn items(&self, mode: WheelMode) -> Vec<WheelItem> {
            match mode {
                WheelMode::Weapons => self.weapons.clone(),
                WheelMode::Powerups => self.powerups.clone(),
            }
        }

        fn active_item(&self) -> Option<String> {
            self.active.clone()
        }

        fn select(&mut self, id: &str, mode: WheelMode, _seat: SeatId) {
            self.selected.push((id.to_string(), mode));
            if mode == WheelMode::Weapons {
                self.active = Some(id.to_string());
            }
        }

        fn now(&self) -> i64 {
            self.now_ms
        }

        fn changed(&mut self, _seat: SeatId) {
            self.changed_count += 1;
        }
    }

    fn seat() -> SeatId {
        IdentityOwner::create("wheel-test").expect("session").seat(0)
    }

    fn make_wheel() -> SeatWeaponWheel<FakeServices> {
        SeatWeaponWheel::new(FakeServices::new(seat()), DEFAULT_WHEEL_OPTIONS).expect("wheel")
    }

    fn mouse(seat: SeatId, dx: f32, dy: f32) -> SeatInputEvent {
        SeatInputEvent {
            seat,
            time_ms: 1_000,
            kind: SeatInputEventKind::MouseMotion {
                position: Vec2 { x: 0.0, y: 0.0 },
                delta: Vec2 { x: dx, y: dy },
            },
        }
    }

    #[test]
    fn rejects_radius_at_or_below_selection_distance() {
        let bad = WheelOptions {
            radius: 100.0,
            selection_distance: 100.0,
            ..DEFAULT_WHEEL_OPTIONS
        };
        assert!(SeatWeaponWheel::new(FakeServices::new(seat()), bad).is_err());
        let negative = WheelOptions {
            selection_distance: -1.0,
            ..DEFAULT_WHEEL_OPTIONS
        };
        assert!(SeatWeaponWheel::new(FakeServices::new(seat()), negative).is_err());
    }

    #[test]
    fn open_refuses_empty_modes_and_close_selects_owned() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        assert!(wheel.is_open());
        assert!(wheel.holster());

        wheel.close(true);
        assert!(!wheel.is_open());
        assert!(wheel.services.selected.is_empty());

        let mut empty = FakeServices::new(seat());
        empty.powerups.clear();
        let mut wheel = SeatWeaponWheel::new(empty, DEFAULT_WHEEL_OPTIONS).expect("wheel");
        assert!(!wheel.open(WheelMode::Powerups));
        assert!(!wheel.is_open());
    }

    #[test]
    fn selects_slice_by_angle() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        let seat = wheel.seat();
        wheel.input(&mouse(seat, 0.0, -170.0)).expect("input");
        wheel.update_at(1_100);
        assert_eq!(
            wheel.draw_state().wheel.expect("wheel").selected.as_deref(),
            Some("blaster")
        );

        let mut side = make_wheel();
        assert!(side.open(WheelMode::Weapons));
        let seat = side.seat();
        side.input(&mouse(seat, 170.0, 0.0)).expect("input");
        side.update_at(1_100);
        assert_eq!(
            side.draw_state().wheel.expect("wheel").selected.as_deref(),
            Some("shotgun")
        );
    }

    #[test]
    fn q2_slot_zero_resists_deselect_but_later_slots_release() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        wheel.selected = Some("blaster".to_string());
        wheel.update_at(1_100);
        assert_eq!(wheel.selected.as_deref(), Some("blaster"));
        assert_eq!(wheel.deselect_time, 0);

        wheel.selected = Some("shotgun".to_string());
        wheel.update_at(1_100);
        assert!(wheel.deselect_time > 0);
        wheel.update_at(wheel.deselect_time + 1);
        assert_eq!(wheel.selected, None);
    }

    #[test]
    fn opacity_fades_in_and_out() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        wheel.update_at(1_000 + 1_000);
        assert!((wheel.draw_state().wheel.expect("wheel").opacity - 1.0).abs() < f32::EPSILON);
        assert!((wheel.time_scale() - 0.1).abs() < f32::EPSILON);
        wheel.close(false);
        wheel.update_at(1_000 + 2_000);
        assert!(wheel.draw_state().wheel.is_none());
        assert!((wheel.time_scale() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn command_consumes_attack_while_carousel_selects() {
        let mut wheel = make_wheel();
        wheel.cycle(1);
        assert!(wheel.draw_state().carousel.is_some());
        let outcome = wheel.command(true, 1_100);
        assert!(outcome.holster);
        assert!(outcome.consume_attack);
        assert_eq!(
            wheel.services.selected.last().map(|entry| entry.0.as_str()),
            Some("shotgun")
        );
        assert_eq!(wheel.weapon_lock_until(), 1_100 + DEFAULT_WHEEL_OPTIONS.carousel_lock);
        assert!(wheel.draw_state().carousel.is_none());

        let outcome = wheel.command(false, wheel.weapon_lock_until());
        assert!(!outcome.consume_attack);
    }

    #[test]
    fn cycle_skips_items_without_ammo_and_wraps() {
        let mut wheel = make_wheel();
        wheel.cycle(1);
        assert_eq!(wheel.carousel_selected.as_deref(), Some("shotgun"));
        wheel.services.now_ms += 10;
        wheel.cycle(1);
        assert_eq!(wheel.carousel_selected.as_deref(), Some("blaster"));
        wheel.services.now_ms += 10;
        wheel.cycle(-1);
        assert_eq!(wheel.carousel_selected.as_deref(), Some("shotgun"));
    }

    #[test]
    fn carousel_times_out_without_attack() {
        let mut wheel = make_wheel();
        wheel.cycle(1);
        let until = wheel.services.now_ms + DEFAULT_WHEEL_OPTIONS.carousel_timeout;
        let outcome = wheel.command(false, until);
        assert!(outcome.holster);
        assert!(!outcome.consume_attack);
        assert_eq!(
            wheel.services.selected.last().map(|entry| entry.0.as_str()),
            Some("shotgun")
        );
    }

    #[test]
    fn close_selects_highlighted_owned_item() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        wheel.selected = Some("shotgun".to_string());
        wheel.close(true);
        assert_eq!(
            wheel.services.selected.last().map(|entry| entry.0.as_str()),
            Some("shotgun")
        );
    }

    #[test]
    fn wrong_seat_input_errors() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        let other = IdentityOwner::create("other").expect("session").seat(0);
        assert!(wheel.input(&mouse(other, 5.0, 5.0)).is_err());
    }

    #[test]
    fn focus_loss_closes_without_selecting() {
        let mut wheel = make_wheel();
        assert!(wheel.open(WheelMode::Weapons));
        let seat = wheel.seat();
        let event = SeatInputEvent {
            seat,
            time_ms: 1_000,
            kind: SeatInputEventKind::Focus { focused: false },
        };
        assert!(wheel.input(&event).expect("input"));
        assert!(!wheel.is_open());
        assert!(wheel.services.selected.is_empty());
    }
}
