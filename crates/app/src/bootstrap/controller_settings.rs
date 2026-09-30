//! Per-controller gyro settings on disk.
//!
//! Donor provenance: `src/app/bootstrap/controller-settings.ts`
//! (`ControllerSettings`).
//!
//! Sync adaptation: the donor loads and saves gyro profiles through async
//! store calls tracked in a `pending` set with `busy` flags and `settle()`.
//! The sync [`crate::settings::config::ConfigStore`] completes inside
//! [`ControllerSettings::update`] and [`ControllerSettings::save`], so there
//! is never a pending load: `settle()` is a no-op and the
//! "still loading" guards are vacuous. Failures are still recorded in
//! [`ControllerSettings::message`] and reported with a trailing newline,
//! exactly as the donor's `failed()` does, and `save` still never surfaces
//! errors to the caller.
//!
//! Gap report: `qa-app` does not depend on `qa-platform`, and `qa-client`
//! does not re-export controller types, so the device descriptor and the
//! gyro-enable result are module-local mirrors ([`ControllerDeviceInfo`],
//! [`GyroEnableResult`]). Likewise the real
//! [`qa_client::input::router::InputRouter`] exposes seats by shared
//! reference only (`seat()` returns `&Seat`, no `seat_mut`), so live tuning
//! mutation goes through the minimal [`ControllerRouter`] trait and no
//! blanket impl is provided. Wiring the real router needs either a
//! `seat_mut` (or tuning-setter) upstream plus a `qa-platform` dependency
//! edge, at which point the mirrors collapse into
//! `qa_platform::controller::{ControllerDevice, ControllerOperationResult}`.

use std::collections::HashMap;
use std::path::PathBuf;

use qa_client::input::gamepad::{
    default_gamepad_tuning, GamepadTuning as ClientGamepadTuning, GyroTuning as ClientGyroTuning,
};
use qa_content::hash::sha256_hex;
use qa_core::identity::SeatId;
use thiserror::Error;

use crate::settings::config::{
    ConfigStore, GyroProfile, GyroProfileIdentity, GyroTuning as StoredGyroTuning, GyroYawAxis,
};
use crate::settings::SettingsError;

/// Failure while applying a gyro profile (recorded in the seat message).
#[derive(Debug, Error)]
pub enum ControllerSettingsError {
    /// A stored profile belongs to a different controller.
    #[error("Gyro settings belong to another controller")]
    ForeignController,
    /// The router rejected enabling the gyro (carries its reason).
    #[error("{0}")]
    Rejected(String),
    /// Settings store failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// Minimal controller descriptor (mirror of the platform `ControllerDevice`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerDeviceInfo {
    /// Platform instance id.
    pub instance: i32,
    /// Device name.
    pub name: String,
    /// Stable GUID, when reported.
    pub guid: Option<String>,
    /// Serial string, when reported.
    pub serial: Option<String>,
}

/// Minimal gyro-enable outcome (mirror of `ControllerOperationResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GyroEnableResult {
    /// Accepted by the backend.
    Accepted,
    /// Rejected with a reason.
    Rejected {
        /// Cause.
        reason: String,
    },
}

impl GyroEnableResult {
    /// Whether the backend accepted the change.
    #[must_use]
    pub fn accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// Rejection reason, or empty when accepted.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Accepted => "",
            Self::Rejected { reason } => reason,
        }
    }
}

/// Minimal router surface `ControllerSettings` needs.
pub trait ControllerRouter {
    /// Live gamepad tuning for a seat, or [`None`] when the seat is gone.
    fn seat_tuning(&self, seat: &SeatId) -> Option<ClientGamepadTuning>;
    /// Replace live gamepad tuning for a seat.
    fn set_seat_tuning(&mut self, seat: &SeatId, tuning: ClientGamepadTuning);
    /// Bound controller instance for a seat.
    fn controller_for(&self, seat: &SeatId) -> Option<i32>;
    /// Enable or disable gyro aiming for a seat.
    fn set_gyro_enabled(&mut self, seat: &SeatId, enabled: bool) -> GyroEnableResult;
}

#[derive(Debug, Clone)]
struct Profile {
    instance: i32,
    identity: GyroProfileIdentity,
    path: String,
    fallback: ClientGyroTuning,
    busy: bool,
    message: String,
}

/// Owns gyro disk I/O only; live tuning stays in the router.
pub struct ControllerSettings<R, D> {
    router: R,
    seats: Vec<SeatId>,
    devices: D,
    store: ConfigStore,
    report: Box<dyn FnMut(&str)>,
    profiles: HashMap<SeatId, Profile>,
    seat_fallbacks: HashMap<SeatId, ClientGyroTuning>,
    closed: bool,
}

impl<R: ControllerRouter, D: Fn() -> Vec<ControllerDeviceInfo>> ControllerSettings<R, D> {
    /// Create settings over a router, seats, device list, store, and reporter.
    pub fn new(
        router: R,
        seats: Vec<SeatId>,
        devices: D,
        store: ConfigStore,
        report: impl FnMut(&str) + 'static,
    ) -> Self {
        let mut settings = Self {
            router,
            seats,
            devices,
            store,
            report: Box::new(report),
            profiles: HashMap::new(),
            seat_fallbacks: HashMap::new(),
            closed: false,
        };
        for seat in settings.seats.clone() {
            let fallback = settings
                .router
                .seat_tuning(&seat)
                .map(|tuning| tuning.gyro)
                .unwrap_or_else(|| default_gamepad_tuning().gyro);
            settings.seat_fallbacks.insert(seat, fallback);
        }
        settings
    }

    /// No-op: sync loads always settle inside [`Self::update`].
    pub fn settle(&self) {}

    /// Republish the seat list, pruning profiles and fallbacks.
    pub fn publish_seats(&mut self, seats: Vec<SeatId>) {
        self.seats = seats.clone();
        self.profiles.retain(|seat, _| seats.contains(seat));
        self.seat_fallbacks.retain(|seat, _| seats.contains(seat));
        for seat in &seats {
            if !self.seat_fallbacks.contains_key(seat) {
                let fallback = self
                    .router
                    .seat_tuning(seat)
                    .map(|tuning| tuning.gyro)
                    .unwrap_or_else(|| default_gamepad_tuning().gyro);
                self.seat_fallbacks.insert(seat.clone(), fallback);
            }
        }
    }

    /// Copy settled profiles from a previous instance for matching seats.
    pub fn copy_settled_profiles_from<R2, D2>(&mut self, previous: &ControllerSettings<R2, D2>) {
        for seat in self.seats.clone() {
            if let Some(profile) = previous.profiles.get(&seat) {
                self.profiles.insert(seat, profile.clone());
            }
        }
    }

    /// Detect controller changes and synchronously load their profiles.
    pub fn update(&mut self) {
        if self.closed {
            return;
        }
        for seat in self.seats.clone() {
            let Some(instance) = self.router.controller_for(&seat) else {
                self.profiles.remove(&seat);
                continue;
            };
            if self
                .profiles
                .get(&seat)
                .is_some_and(|profile| profile.instance == instance)
            {
                continue;
            }
            let devices = (self.devices)();
            let Some(device) = devices.iter().find(|device| device.instance == instance) else {
                continue;
            };
            let identity = match (&device.guid, &device.serial) {
                (Some(guid), Some(serial)) if !serial.is_empty() => {
                    GyroProfileIdentity::Device {
                        guid: guid.clone(),
                        serial: serial.clone(),
                    }
                }
                _ => GyroProfileIdentity::Seat,
            };
            let file = match &identity {
                GyroProfileIdentity::Seat => "seat".to_string(),
                GyroProfileIdentity::Device { guid, serial } => {
                    format!("{guid}-{}", sha256_hex(serial.as_bytes()))
                }
            };
            let fallback = self
                .seat_fallbacks
                .get(&seat)
                .copied()
                .unwrap_or_else(|| default_gamepad_tuning().gyro);
            self.profiles.insert(
                seat.clone(),
                Profile {
                    instance,
                    identity,
                    path: format!("controllers/seat-{}/{file}.json", seat.index() + 1),
                    fallback,
                    busy: true,
                    message: "Loading settings...".to_string(),
                },
            );
            if let Some(mut tuning) = self.router.seat_tuning(&seat) {
                tuning.gyro = default_gamepad_tuning().gyro;
                self.router.set_seat_tuning(&seat, tuning);
            }
            let _ = self.router.set_gyro_enabled(&seat, false);
            if let Err(error) = self.load(&seat) {
                if self.current(&seat) {
                    let message = error.to_string();
                    self.failed(&seat, &message);
                }
            }
            if self.current(&seat) {
                if let Some(profile) = self.profiles.get_mut(&seat) {
                    profile.busy = false;
                }
            }
        }
    }

    fn current(&self, seat: &SeatId) -> bool {
        !self.closed
            && self.profiles.get(seat).is_some_and(|profile| {
                self.router.controller_for(seat) == Some(profile.instance)
            })
    }

    fn load(&mut self, seat: &SeatId) -> Result<(), ControllerSettingsError> {
        let (path, identity) = match self.profiles.get(seat) {
            Some(profile) => (profile.path.clone(), profile.identity.clone()),
            None => return Ok(()),
        };
        let saved = self.store.load_gyro(&path)?;
        if !self.current(seat) {
            return Ok(());
        }
        if self.router.seat_tuning(seat).is_some() {
            if let Some(saved) = saved.as_ref() {
                if !same_identity(&saved.identity, &identity) {
                    return Err(ControllerSettingsError::ForeignController);
                }
            }
            let fallback = self
                .profiles
                .get(seat)
                .map(|profile| profile.fallback)
                .unwrap_or_else(|| default_gamepad_tuning().gyro);
            let want = saved
                .as_ref()
                .map(|saved| client_from_stored(&saved.tuning))
                .unwrap_or(fallback);
            let mut tuning = self
                .router
                .seat_tuning(seat)
                .expect("seat tuning checked above");
            tuning.gyro = ClientGyroTuning {
                enabled: false,
                ..want
            };
            self.router.set_seat_tuning(seat, tuning);
            let result = self.router.set_gyro_enabled(seat, want.enabled);
            if let GyroEnableResult::Rejected { reason } = result {
                return Err(ControllerSettingsError::Rejected(reason));
            }
        }
        if self.current(seat) {
            let text = if matches!(identity, GyroProfileIdentity::Seat) {
                "Saved for this seat."
            } else {
                "Saved for this controller and seat."
            };
            if let Some(profile) = self.profiles.get_mut(seat) {
                profile.message = text.to_string();
            }
        }
        Ok(())
    }

    fn failed(&mut self, seat: &SeatId, message: &str) {
        if let Some(profile) = self.profiles.get_mut(seat) {
            profile.message = message.to_string();
        }
        (self.report)(&format!("{message}\n"));
    }

    /// Whether the seat profile is busy.
    #[must_use]
    pub fn busy(&self, seat: &SeatId) -> bool {
        self.profiles.get(seat).is_some_and(|profile| profile.busy)
    }

    /// Status message for the seat.
    #[must_use]
    pub fn message(&self, seat: &SeatId) -> String {
        self.profiles
            .get(seat)
            .map(|profile| profile.message.clone())
            .unwrap_or_else(|| "Connect a controller with a gyroscope.".to_string())
    }

    /// Save live gyro tuning for the seat; failures land in the message.
    pub fn save(&mut self, seat: &SeatId) {
        let Some(profile) = self.profiles.get(seat) else {
            return;
        };
        if profile.busy || !self.current(seat) {
            return;
        }
        let Some(input) = self.router.seat_tuning(seat) else {
            return;
        };
        let tuning = input.gyro;
        let (path, identity) = (profile.path.clone(), profile.identity.clone());
        if let Some(profile) = self.profiles.get_mut(seat) {
            profile.busy = true;
        }
        let result = self.store.save_gyro(
            &path,
            &GyroProfile {
                version: 1,
                identity,
                tuning: stored_from_client(&tuning),
            },
        );
        if let Err(error) = result {
            if self.current(seat) {
                let message = error.to_string();
                self.failed(seat, &message);
            }
        }
        if self.current(seat) {
            if let Some(profile) = self.profiles.get_mut(seat) {
                profile.busy = false;
            }
        }
    }

    /// Borrow UI bindings for a seat (donor `ui()`).
    pub fn ui(&mut self, seat: SeatId) -> GyroSettingsUi<'_, R, D> {
        GyroSettingsUi {
            settings: self,
            seat,
        }
    }

    /// Close settings, dropping all profiles.
    pub fn close(&mut self) {
        self.closed = true;
        self.profiles.clear();
    }
}

/// UI bindings for one seat (donor `GyroSettingsUi`).
pub struct GyroSettingsUi<'a, R: ControllerRouter, D: Fn() -> Vec<ControllerDeviceInfo>> {
    settings: &'a mut ControllerSettings<R, D>,
    seat: SeatId,
}

impl<R: ControllerRouter, D: Fn() -> Vec<ControllerDeviceInfo>> GyroSettingsUi<'_, R, D> {
    /// Bound seat.
    #[must_use]
    pub fn seat(&self) -> &SeatId {
        &self.seat
    }

    /// Mutable router access for calibration controls.
    pub fn router(&mut self) -> &mut R {
        &mut self.settings.router
    }

    /// Currently bound device, or [`None`] when closed or missing.
    #[must_use]
    pub fn device(&self) -> Option<ControllerDeviceInfo> {
        if self.settings.closed {
            return None;
        }
        let instance = self.settings.router.controller_for(&self.seat)?;
        (self.settings.devices)()
            .into_iter()
            .find(|device| device.instance == instance)
    }

    /// Whether the seat profile is busy.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.settings.busy(&self.seat)
    }

    /// Status message for the seat.
    #[must_use]
    pub fn message(&self) -> String {
        self.settings.message(&self.seat)
    }

    /// Save live gyro tuning for the seat.
    pub fn save(&mut self) {
        let seat = self.seat.clone();
        self.settings.save(&seat);
    }
}

fn same_identity(saved: &GyroProfileIdentity, current: &GyroProfileIdentity) -> bool {
    match (saved, current) {
        (GyroProfileIdentity::Seat, GyroProfileIdentity::Seat) => true,
        (
            GyroProfileIdentity::Device {
                guid: saved_guid,
                serial: saved_serial,
            },
            GyroProfileIdentity::Device { guid, serial },
        ) => saved_guid == guid && saved_serial == serial,
        _ => false,
    }
}

fn stored_from_client(tuning: &ClientGyroTuning) -> StoredGyroTuning {
    StoredGyroTuning {
        enabled: tuning.enabled,
        yaw_axis: if tuning.yaw_axis_y {
            GyroYawAxis::Y
        } else {
            GyroYawAxis::Z
        },
        yaw_sensitivity: tuning.yaw_sensitivity,
        pitch_sensitivity: tuning.pitch_sensitivity,
    }
}

fn client_from_stored(tuning: &StoredGyroTuning) -> ClientGyroTuning {
    ClientGyroTuning {
        enabled: tuning.enabled,
        yaw_sensitivity: tuning.yaw_sensitivity,
        pitch_sensitivity: tuning.pitch_sensitivity,
        yaw_axis_y: matches!(tuning.yaw_axis, GyroYawAxis::Y),
    }
}

/// Default writable settings root (`~/.local/share/quake-typescript/settings`).
#[must_use]
pub fn default_settings_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home).join(".local/share/quake-typescript/settings")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::cell::RefCell;
    use std::rc::Rc;

    const GUID: &str = "0123456789abcdef0123456789abcdef";

    #[derive(Default)]
    struct FakeRouter {
        tunings: HashMap<SeatId, ClientGamepadTuning>,
        controllers: HashMap<SeatId, i32>,
        gyro_enabled: HashMap<SeatId, bool>,
        reject_gyro: bool,
    }

    impl ControllerRouter for FakeRouter {
        fn seat_tuning(&self, seat: &SeatId) -> Option<ClientGamepadTuning> {
            self.tunings.get(seat).copied()
        }
        fn set_seat_tuning(&mut self, seat: &SeatId, tuning: ClientGamepadTuning) {
            self.tunings.insert(seat.clone(), tuning);
        }
        fn controller_for(&self, seat: &SeatId) -> Option<i32> {
            self.controllers.get(seat).copied()
        }
        fn set_gyro_enabled(&mut self, seat: &SeatId, enabled: bool) -> GyroEnableResult {
            if self.reject_gyro {
                return GyroEnableResult::Rejected {
                    reason: "no gyro".to_string(),
                };
            }
            self.gyro_enabled.insert(seat.clone(), enabled);
            GyroEnableResult::Accepted
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("test").unwrap()
    }

    fn test_store(name: &str) -> (ConfigStore, PathBuf) {
        let root: PathBuf = std::env::temp_dir().join(format!(
            "qa-controller-settings-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        (ConfigStore::new(root.clone()), root)
    }

    fn device(instance: i32) -> ControllerDeviceInfo {
        ControllerDeviceInfo {
            instance,
            name: "Pad".to_string(),
            guid: Some(GUID.to_string()),
            serial: Some("serial-1".to_string()),
        }
    }

    fn device_path() -> String {
        format!(
            "controllers/seat-1/{GUID}-{}.json",
            sha256_hex("serial-1".as_bytes())
        )
    }

    #[test]
    fn missing_controller_reports_default_message() {
        let owner = owner();
        let seat = owner.seat(0);
        let (store, _root) = test_store("missing");
        let mut settings =
            ControllerSettings::new(FakeRouter::default(), vec![seat.clone()], Vec::new, store, |_| {});
        settings.update();
        assert!(!settings.busy(&seat));
        assert_eq!(
            settings.message(&seat),
            "Connect a controller with a gyroscope."
        );
    }

    #[test]
    fn new_device_loads_fallback_and_marks_saved() {
        let owner = owner();
        let seat = owner.seat(0);
        let (store, _root) = test_store("fallback");
        let mut router = FakeRouter::default();
        router.controllers.insert(seat.clone(), 7);
        let mut tuning = default_gamepad_tuning();
        tuning.gyro.yaw_sensitivity = 3.0;
        router.tunings.insert(seat.clone(), tuning);
        let pad = device(7);
        let mut settings = ControllerSettings::new(
            router,
            vec![seat.clone()],
            move || vec![pad.clone()],
            store,
            |_| {},
        );
        settings.update();
        assert!(!settings.busy(&seat));
        assert_eq!(settings.message(&seat), "Saved for this controller and seat.");
    }

    #[test]
    fn seat_identity_without_guid_or_serial() {
        let owner = owner();
        let seat = owner.seat(0);
        let (store, root) = test_store("seat");
        let mut router = FakeRouter::default();
        router.controllers.insert(seat.clone(), 7);
        router
            .tunings
            .insert(seat.clone(), default_gamepad_tuning());
        let pad = ControllerDeviceInfo {
            instance: 7,
            name: "Pad".to_string(),
            guid: None,
            serial: None,
        };
        let mut settings = ControllerSettings::new(
            router,
            vec![seat.clone()],
            move || vec![pad.clone()],
            store,
            |_| {},
        );
        settings.update();
        assert_eq!(settings.message(&seat), "Saved for this seat.");
        settings.save(&seat);
        assert!(root.join("controllers/seat-1/seat.json").exists());
    }

    #[test]
    fn save_roundtrip_applies_stored_tuning() {
        let owner = owner();
        let seat = owner.seat(0);
        let (store, _root) = test_store("roundtrip");
        let mut router = FakeRouter::default();
        router.controllers.insert(seat.clone(), 7);
        router
            .tunings
            .insert(seat.clone(), default_gamepad_tuning());
        let pad = device(7);
        let mut settings = ControllerSettings::new(
            router,
            vec![seat.clone()],
            move || vec![pad.clone()],
            store,
            |_| {},
        );
        settings.update();
        let mut tuning = default_gamepad_tuning();
        tuning.gyro.yaw_sensitivity = 4.5;
        tuning.gyro.enabled = true;
        settings.router.tunings.insert(seat.clone(), tuning);
        settings.save(&seat);
        assert_eq!(settings.message(&seat), "Saved for this controller and seat.");

        let (store, _root) = test_store("roundtrip");
        let mut router = FakeRouter::default();
        router.controllers.insert(seat.clone(), 7);
        router
            .tunings
            .insert(seat.clone(), default_gamepad_tuning());
        let pad = device(7);
        let mut settings = ControllerSettings::new(
            router,
            vec![seat.clone()],
            move || vec![pad.clone()],
            store,
            |_| {},
        );
        settings.update();
        let applied = settings.router.tunings.get(&seat).unwrap();
        assert_eq!(applied.gyro.yaw_sensitivity, 4.5);
        assert_eq!(settings.router.gyro_enabled.get(&seat), Some(&true));
    }

    #[test]
    fn foreign_profile_is_reported() {
        let owner = owner();
        let seat = owner.seat(0);
        let (store, _root) = test_store("foreign");
        store
            .save_gyro(
                &device_path(),
                &GyroProfile {
                    version: 1,
                    identity: GyroProfileIdentity::Seat,
                    tuning: stored_from_client(&default_gamepad_tuning().gyro),
                },
            )
            .unwrap();
        let mut router = FakeRouter::default();
        router.controllers.insert(seat.clone(), 7);
        router
            .tunings
            .insert(seat.clone(), default_gamepad_tuning());
        let pad = device(7);
        let reported = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let mut settings = ControllerSettings::new(
            router,
            vec![seat.clone()],
            move || vec![pad.clone()],
            store,
            move |message: &str| sink.borrow_mut().push(message.to_string()),
        );
        settings.update();
        assert_eq!(
            settings.message(&seat),
            "Gyro settings belong to another controller"
        );
        assert_eq!(
            reported.borrow().as_slice(),
            ["Gyro settings belong to another controller\n"]
        );
        assert!(!settings.busy(&seat));
    }

    #[test]
    fn gyro_rejection_uses_router_reason() {
        let owner = owner();
        let seat = owner.seat(0);
        let (store, _root) = test_store("reject");
        let mut router = FakeRouter::default();
        router.controllers.insert(seat.clone(), 7);
        router
            .tunings
            .insert(seat.clone(), default_gamepad_tuning());
        router.reject_gyro = true;
        let pad = device(7);
        let mut settings = ControllerSettings::new(
            router,
            vec![seat.clone()],
            move || vec![pad.clone()],
            store,
            |_| {},
        );
        settings.update();
        assert_eq!(settings.message(&seat), "no gyro");
    }

    #[test]
    fn publish_copy_close_and_ui() {
        let owner = owner();
        let first = owner.seat(0);
        let second = owner.seat(1);
        let (store, _root) = test_store("seats");
        let mut router = FakeRouter::default();
        router.controllers.insert(first.clone(), 7);
        router
            .tunings
            .insert(first.clone(), default_gamepad_tuning());
        let pad = device(7);
        let mut settings = ControllerSettings::new(
            router,
            vec![first.clone()],
            move || vec![pad.clone()],
            store,
            |_| {},
        );
        settings.update();
        settings.publish_seats(vec![first.clone(), second.clone()]);
        settings.settle();
        let (store, _root) = test_store("seats-copy");
        let mut router = FakeRouter::default();
        router
            .tunings
            .insert(first.clone(), default_gamepad_tuning());
        let mut next = ControllerSettings::new(
            router,
            vec![first.clone()],
            Vec::new,
            store,
            |_| {},
        );
        next.copy_settled_profiles_from(&settings);
        assert_eq!(
            next.message(&first),
            "Saved for this controller and seat."
        );
        let mut ui = settings.ui(first.clone());
        assert_eq!(ui.device().unwrap().instance, 7);
        assert!(!ui.busy());
        ui.save();
        settings.close();
        let ui = settings.ui(first.clone());
        assert_eq!(ui.device(), None);
        assert_eq!(
            ui.message(),
            "Connect a controller with a gyroscope."
        );
    }
}
