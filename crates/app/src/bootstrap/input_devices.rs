//! MIDI and source-joystick device ownership over the input router.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/input-devices.ts`
//! (`inputDeviceStore`, `loadInputDeviceSettings`, `InputDevices`). The
//! console root (`config-scripts.ts`) and cvar-archive loading
//! (`cvar-archives.ts`) are absorbed with citation (both donors unported).
//! MIDI and source-joystick reuse the workspace inputs; seat routing stays
//! behind [`DeviceRouter`] (the router's seat-input surface drifted).

use qa_client::input::device::{input_device_cvar_names, register_input_device_cvars};
use qa_client::input::midi::{MidiError, MidiInputBoundary, SourceMidiInput};
use qa_client::input::source::{JoystickOpener, SourceInputError, SourceInputState};
use qa_client::ui::settings::{SettingBinding, SettingBindingKind, SettingCategory};
use qa_client::ui::types::{UiChoice, UiControlId};
use qa_content::user_data::default_user_content_root;
use qa_core::cvar::{CvarArchiveEntry, CvarError, CvarRegistry};
use qa_core::identity::SeatId;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use thiserror::Error;

use crate::settings::config::ConfigStore;
use crate::settings::SettingsError;

/// Failure of input device ownership.
#[derive(Debug, Error)]
pub enum InputDeviceError {
    /// Device misuse.
    #[error("{0}")]
    Device(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Settings store failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// MIDI failure.
    #[error(transparent)]
    Midi(#[from] MidiError),
    /// Source input failure.
    #[error(transparent)]
    Source(#[from] SourceInputError),
}

/// Console config root absorbed from `config-scripts.ts`.
#[must_use]
pub fn input_device_store(user_content_root: Option<&str>) -> ConfigStore {
    let root: PathBuf = user_content_root
        .map(PathBuf::from)
        .unwrap_or_else(default_user_content_root)
        .join("console");
    ConfigStore::new(root)
}

/// Cvar-archive loading absorbed from `cvar-archives.ts` (`loadCvarArchive`).
pub fn load_input_device_settings(store: &ConfigStore) -> Result<Vec<CvarArchiveEntry>, InputDeviceError> {
    let Some(text) = store.load_text("cvars/input/devices.json")? else {
        return Ok(Vec::new());
    };
    let value = qa_content::value::parse_save_json(&text)
        .map_err(|error| InputDeviceError::Device(format!("cvar archive: {error}")))?;
    let reader = qa_content::value::SaveReader::new(&value);
    reader
        .field("version")
        .literal_i64(1)
        .map_err(|error| InputDeviceError::Device(format!("cvar archive: {error}")))?;
    reader
        .field("dialect")
        .literal_str("q3")
        .map_err(|error| InputDeviceError::Device(format!("cvar archive: {error}")))?;
    let entries: Vec<CvarArchiveEntry> = reader
        .field("entries")
        .list(|entry| {
            Ok::<CvarArchiveEntry, qa_content::value::ValueError>(CvarArchiveEntry {
                name: entry.field("name").string()?,
                value: entry.field("value").string()?,
            })
        })
        .map_err(|error| InputDeviceError::Device(format!("cvar archive: {error}")))?;
    let mut names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    if names.len() != entries.len() {
        return Err(InputDeviceError::Device(
            "cvar archive: duplicate archived cvar".to_owned(),
        ));
    }
    Ok(entries)
}

/// Seat routing surface for device ownership.
pub trait DeviceRouter {
    /// Seat id at a router index, if present.
    fn seat_id(&self, index: u32) -> Option<SeatId>;
    /// Router seat count.
    fn seat_count(&self) -> u32;
    /// Deliver a key to a router seat.
    fn key(&mut self, index: u32, code: i32, down: bool, time_ms: i64);
    /// Deliver mouse motion to a router seat.
    fn mouse_motion(&mut self, index: u32, dx: i32, dy: i32, time_ms: i64);
    /// Release a router seat, appending console releases when given.
    fn release_seat(&mut self, index: u32, time_ms: i64, append: Option<&mut dyn FnMut(&str)>);
    /// Whether any seat is focused.
    fn any_focused(&self) -> bool;
    /// Set the source joystick instance and seat.
    fn set_source_joystick(&mut self, instance: Option<i32>, seat: Option<SeatId>);
    /// Player choices for seat bindings: (id, label).
    fn player_choices(&self) -> Vec<(String, String)>;
}

type PrintSink = Rc<RefCell<dyn FnMut(&str)>>;

struct DevicesShared<R> {
    cvars: CvarRegistry,
    store: ConfigStore,
    print: PrintSink,
    midi: SourceMidiInput,
    joystick: SourceInputState,
    router: Option<R>,
    midi_seat: Option<u32>,
    joystick_seat: Option<u32>,
    signature: String,
    retry_at: i64,
    closed: bool,
}

/// One device owner follows the published input router; prepared worlds never open a second device.
pub struct InputDevices<R> {
    shared: Rc<RefCell<DevicesShared<R>>>,
}

impl<R: DeviceRouter + 'static> InputDevices<R> {
    /// Create the owner, registering device cvars.
    pub fn new(
        cvars: CvarRegistry,
        store: ConfigStore,
        print: PrintSink,
        boundary: Box<dyn MidiInputBoundary>,
        opener: JoystickOpener,
    ) -> Result<Self, InputDeviceError> {
        let mut cvars = cvars;
        register_input_device_cvars(&mut cvars)?;
        let midi = SourceMidiInput::new(
            boundary,
            Box::new({
                let print = Rc::clone(&print);
                move |text: &str| print.borrow_mut()(text)
            }),
        );
        let joystick = SourceInputState::new(
            Box::new({
                let print = Rc::clone(&print);
                move |text: &str| print.borrow_mut()(text)
            }),
            opener,
        );
        Ok(Self {
            shared: Rc::new(RefCell::new(DevicesShared {
                cvars,
                store,
                print,
                midi,
                joystick,
                router: None,
                midi_seat: None,
                joystick_seat: None,
                signature: String::new(),
                retry_at: 0,
                closed: false,
            })),
        })
    }

    fn selection(shared: &DevicesShared<R>) -> String {
        [
            "in_midi",
            "in_mididevice",
            "in_midichannel",
            "in_midiseat",
            "in_joystick",
            "in_joystickProfile",
            "in_joystickSeat",
        ]
        .iter()
        .map(|name| shared.cvars.variable_string(name))
        .collect::<Vec<_>>()
        .join("/")
    }

    fn target(shared: &DevicesShared<R>, name: &str) -> Option<u32> {
        let index = shared.cvars.variable_value(name) as i64 - 1;
        if index < 0 {
            return None;
        }
        let index = index as u32;
        if shared.router.as_ref().is_some_and(|router| index < router.seat_count()) {
            Some(index)
        } else {
            None
        }
    }

    /// Activate the owner on a router.
    pub fn activate(&mut self, router: R, now_ms: i64) -> Result<(), InputDeviceError> {
        if self.shared.borrow().closed {
            return Err(InputDeviceError::Device("Input device owner is closed".to_owned()));
        }
        self.release(now_ms, None)?;
        if let Some(mut old) = self.shared.borrow_mut().router.take() {
            old.set_source_joystick(None, None);
        }
        {
            let mut shared = self.shared.borrow_mut();
            shared.router = Some(router);
            shared.midi_seat = Self::target(&shared, "in_midiseat");
            shared.joystick_seat = Self::target(&shared, "in_joystickSeat");
        }
        if self.shared.borrow().signature.is_empty() {
            self.restart(now_ms)?;
        }
        let mut shared = self.shared.borrow_mut();
        let seat = shared
            .joystick_seat
            .and_then(|index| shared.router.as_ref().and_then(|router| router.seat_id(index)));
        let instance = shared.joystick.instance();
        if let Some(router) = shared.router.as_mut() {
            router.set_source_joystick(instance, seat);
        }
        Ok(())
    }

    /// Release held device state, appending console releases when given.
    pub fn release(&mut self, time_ms: i64, append: Option<&mut dyn FnMut(&str)>) -> Result<(), InputDeviceError> {
        let (midi_seat, joystick_seat) = {
            let shared = self.shared.borrow();
            (shared.midi_seat, shared.joystick_seat)
        };
        let console = append.is_some();
        if let Some(append) = append {
            let mut shared = self.shared.borrow_mut();
            if let (Some(router), Some(seat)) = (shared.router.as_mut(), midi_seat) {
                router.release_seat(seat, time_ms, Some(&mut *append));
            }
            if joystick_seat != midi_seat {
                if let (Some(router), Some(seat)) = (shared.router.as_mut(), joystick_seat) {
                    router.release_seat(seat, time_ms, Some(&mut *append));
                }
            }
        }
        let mut midi_keys = Vec::new();
        self.shared.borrow_mut().midi.release(time_ms, &mut |code, down, time| {
            midi_keys.push((code, down, time));
        });
        let mut joystick_keys = Vec::new();
        self.shared
            .borrow_mut()
            .joystick
            .release_joystick_state(time_ms, &mut |code, down, time| {
                joystick_keys.push((code, down, time));
            })?;
        if !console {
            let mut shared = self.shared.borrow_mut();
            if let Some(router) = shared.router.as_mut() {
                if let Some(seat) = midi_seat {
                    for (code, down, time) in midi_keys {
                        router.key(seat, code, down, time);
                    }
                }
                if let Some(seat) = joystick_seat {
                    for (code, down, time) in joystick_keys {
                        router.key(seat, code, down, time);
                    }
                }
            }
        }
        Ok(())
    }

    /// Restart device selection.
    pub fn restart(&mut self, now_ms: i64) -> Result<(), InputDeviceError> {
        if self.shared.borrow().closed {
            return Err(InputDeviceError::Device("Input device owner is closed".to_owned()));
        }
        self.release(now_ms, None)?;
        let mut shared = self.shared.borrow_mut();
        shared.midi_seat = Self::target(&shared, "in_midiseat");
        shared.joystick_seat = Self::target(&shared, "in_joystickSeat");
        {
            let DevicesShared {
                midi, joystick, cvars, ..
            } = &mut *shared;
            midi.restart(cvars)?;
            joystick.restart(cvars)?;
        }
        shared.signature = Self::selection(&shared);
        let seat = shared
            .joystick_seat
            .and_then(|index| shared.router.as_ref().and_then(|router| router.seat_id(index)));
        let instance = shared.joystick.instance();
        if let Some(router) = shared.router.as_mut() {
            router.set_source_joystick(instance, seat);
        }
        Ok(())
    }

    /// Pump device state for one frame.
    pub fn frame(&mut self, now_ms: i64) -> Result<(), InputDeviceError> {
        if self.shared.borrow().closed || self.shared.borrow().router.is_none() {
            return Ok(());
        }
        if Self::selection(&self.shared.borrow()) != self.shared.borrow().signature {
            self.restart(now_ms)?;
        }
        if now_ms >= self.shared.borrow().retry_at {
            self.shared.borrow_mut().retry_at = now_ms + 1000;
            let retry_midi =
                self.shared.borrow().cvars.variable_value("in_midi") != 0.0 && !self.shared.borrow().midi.connected();
            if retry_midi {
                let mut shared = self.shared.borrow_mut();
                let DevicesShared { midi, cvars, .. } = &mut *shared;
                midi.restart(cvars)?;
            }
            let retry_joystick = self.shared.borrow().cvars.variable_value("in_joystick") != 0.0
                && self.shared.borrow().joystick.instance().is_none();
            if retry_joystick {
                let mut shared = self.shared.borrow_mut();
                let DevicesShared { joystick, cvars, .. } = &mut *shared;
                joystick.restart(cvars)?;
            }
            let mut shared = self.shared.borrow_mut();
            let seat = shared
                .joystick_seat
                .and_then(|index| shared.router.as_ref().and_then(|router| router.seat_id(index)));
            let instance = shared.joystick.instance();
            if let Some(router) = shared.router.as_mut() {
                router.set_source_joystick(instance, seat);
            }
        }
        if !self
            .shared
            .borrow()
            .router
            .as_ref()
            .is_some_and(|router| router.any_focused())
        {
            self.release(now_ms, None)?;
        }
        let midi_seat = self.shared.borrow().midi_seat;
        let joystick_seat = self.shared.borrow().joystick_seat;
        let mut midi_keys = Vec::new();
        {
            let mut borrowed = self.shared.borrow_mut();
            let DevicesShared { midi, cvars, .. } = &mut *borrowed;
            midi.frame(
                cvars,
                &mut |code, down, time| {
                    midi_keys.push((code, down, time));
                },
                now_ms,
            )?;
        }
        if let Some(seat) = midi_seat {
            let mut shared = self.shared.borrow_mut();
            if let Some(router) = shared.router.as_mut() {
                for (code, down, time) in midi_keys {
                    router.key(seat, code, down, time);
                }
            }
        }
        let mut joystick_keys = Vec::new();
        let mut motions = Vec::new();
        {
            let mut borrowed = self.shared.borrow_mut();
            let DevicesShared { joystick, cvars, .. } = &mut *borrowed;
            joystick.joystick_frame(
                cvars,
                &mut |code: i32, down: bool, _time: i64| {
                    joystick_keys.push((code, down));
                },
                Some(&mut |x: i32, y: i32, _time: i64| {
                    motions.push((x, y));
                }),
            )?;
        }
        if let Some(seat) = joystick_seat {
            let mut shared = self.shared.borrow_mut();
            if let Some(router) = shared.router.as_mut() {
                for (code, down) in joystick_keys {
                    router.key(seat, code, down, now_ms);
                }
                for (x, y) in motions {
                    router.mouse_motion(seat, x, y, now_ms);
                }
            }
        }
        let mut shared = self.shared.borrow_mut();
        let seat = shared
            .joystick_seat
            .and_then(|index| shared.router.as_ref().and_then(|router| router.seat_id(index)));
        let instance = shared.joystick.instance();
        if let Some(router) = shared.router.as_mut() {
            router.set_source_joystick(instance, seat);
        }
        Ok(())
    }

    /// Print MIDI status.
    pub fn info(&mut self) -> Result<(), InputDeviceError> {
        let mut shared = self.shared.borrow_mut();
        let DevicesShared { midi, cvars, .. } = &mut *shared;
        midi.info(cvars)?;
        Ok(())
    }

    /// Persist device cvars.
    pub fn save(&mut self) -> Result<(), InputDeviceError> {
        let names = input_device_cvar_names();
        let entries: Vec<CvarArchiveEntry> = {
            let shared = self.shared.borrow();
            shared
                .cvars
                .archive_entries(&|name| names.iter().any(|owned| owned == name))
        };
        let mut text = String::from("{\"version\":1,\"dialect\":\"q3\",\"entries\":[");
        for (index, entry) in entries.iter().enumerate() {
            if index != 0 {
                text.push(',');
            }
            text.push_str(&format!(
                "{{\"name\":{},\"value\":{}}}",
                json_string(&entry.name),
                json_string(&entry.value)
            ));
        }
        text.push_str("]}\n");
        self.shared.borrow().store.dump("cvars/input/devices.json", &text)?;
        Ok(())
    }

    /// Settings bindings for device selection.
    #[must_use]
    pub fn bindings(&self) -> Vec<SettingBinding> {
        let shared = Rc::clone(&self.shared);
        let toggle = |name: &'static str, label: &'static str| {
            let read_shared = Rc::clone(&shared);
            let write_shared = Rc::clone(&shared);
            SettingBinding {
                id: UiControlId::new(&format!("ui:input:{name}")).expect("binding id"),
                label: label.to_owned(),
                category: SettingCategory::Input,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Toggle {
                    read: Rc::new(move || read_shared.borrow().cvars.variable_value(name) != 0.0),
                    write: Rc::new(move |value| {
                        let _ = write_shared
                            .borrow_mut()
                            .cvars
                            .set(name, if value { "1" } else { "0" }, true);
                    }),
                },
            }
        };
        let seat = |name: &'static str, label: &'static str| {
            let read_shared = Rc::clone(&shared);
            let write_shared = Rc::clone(&shared);
            let choices_shared = Rc::clone(&shared);
            SettingBinding {
                id: UiControlId::new(&format!("ui:input:{name}")).expect("binding id"),
                label: label.to_owned(),
                category: SettingCategory::Input,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Choice {
                    read: Rc::new(move || read_shared.borrow().cvars.variable_string(name)),
                    write: Rc::new(move |value| {
                        let _ = write_shared.borrow_mut().cvars.set(name, value, false);
                    }),
                    choices: Rc::new(move || {
                        choices_shared
                            .borrow()
                            .router
                            .as_ref()
                            .map(|router| {
                                router
                                    .player_choices()
                                    .into_iter()
                                    .map(|(id, label)| UiChoice { id, label })
                                    .collect()
                            })
                            .unwrap_or_default()
                    }),
                },
            }
        };
        let midi_device = {
            let read_shared = Rc::clone(&shared);
            let write_shared = Rc::clone(&shared);
            let choices_shared = Rc::clone(&shared);
            SettingBinding {
                id: UiControlId::new("ui:input:midi-device").expect("binding id"),
                label: "MIDI device".to_owned(),
                category: SettingCategory::Input,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Choice {
                    read: Rc::new(move || read_shared.borrow().cvars.variable_string("in_mididevice")),
                    write: Rc::new(move |value| {
                        let _ = write_shared.borrow_mut().cvars.set("in_mididevice", value, false);
                    }),
                    choices: Rc::new(move || match choices_shared.borrow().midi.available_devices() {
                        Ok(devices) => devices
                            .into_iter()
                            .enumerate()
                            .map(|(index, device)| UiChoice {
                                id: index.to_string(),
                                label: device.name,
                            })
                            .collect(),
                        Err(error) => {
                            choices_shared.borrow().print.borrow_mut()(&format!("MIDI devices unavailable: {error}\n"));
                            Vec::new()
                        }
                    }),
                },
            }
        };
        let midi_channel = {
            let read_shared = Rc::clone(&shared);
            let write_shared = Rc::clone(&shared);
            SettingBinding {
                id: UiControlId::new("ui:input:midi-channel").expect("binding id"),
                label: "MIDI channel".to_owned(),
                category: SettingCategory::Input,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Slider {
                    read: Rc::new(move || read_shared.borrow().cvars.variable_value("in_midichannel")),
                    write: Rc::new(move |value| {
                        let _ = write_shared
                            .borrow_mut()
                            .cvars
                            .set("in_midichannel", &value.to_string(), false);
                    }),
                    minimum: 1.0,
                    maximum: 16.0,
                    step: 1.0,
                    format_value: None,
                },
            }
        };
        let joystick_profile = {
            let read_shared = Rc::clone(&shared);
            let write_shared = Rc::clone(&shared);
            SettingBinding {
                id: UiControlId::new("ui:input:joystick-profile").expect("binding id"),
                label: "Joystick profile".to_owned(),
                category: SettingCategory::Input,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Choice {
                    read: Rc::new(move || read_shared.borrow().cvars.variable_string("in_joystickProfile")),
                    write: Rc::new(move |value| {
                        let _ = write_shared.borrow_mut().cvars.set("in_joystickProfile", value, true);
                    }),
                    choices: Rc::new(|| {
                        vec![
                            UiChoice {
                                id: "linux".to_owned(),
                                label: "Linux axes".to_owned(),
                            },
                            UiChoice {
                                id: "windows".to_owned(),
                                label: "Windows POV and ball".to_owned(),
                            },
                        ]
                    }),
                },
            }
        };
        let joystick_threshold = {
            let read_shared = Rc::clone(&shared);
            let write_shared = Rc::clone(&shared);
            SettingBinding {
                id: UiControlId::new("ui:input:joystick-threshold").expect("binding id"),
                label: "Joystick axis threshold".to_owned(),
                category: SettingCategory::Input,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Slider {
                    read: Rc::new(move || read_shared.borrow().cvars.variable_value("joy_threshold")),
                    write: Rc::new(move |value| {
                        let _ = write_shared
                            .borrow_mut()
                            .cvars
                            .set("joy_threshold", &value.to_string(), false);
                    }),
                    minimum: 0.01,
                    maximum: 1.0,
                    step: 0.01,
                    format_value: None,
                },
            }
        };
        vec![
            toggle("in_midi", "MIDI input"),
            seat("in_midiseat", "MIDI player"),
            midi_device,
            midi_channel,
            toggle("in_joystick", "Source joystick input"),
            seat("in_joystickSeat", "Joystick player"),
            joystick_profile,
            joystick_threshold,
        ]
    }

    /// Release devices and close.
    pub fn close(&mut self, now_ms: i64) -> Result<(), InputDeviceError> {
        if self.shared.borrow().closed {
            return Ok(());
        }
        self.release(now_ms, None)?;
        if let Some(mut router) = self.shared.borrow_mut().router.take() {
            router.set_source_joystick(None, None);
        }
        let mut shared = self.shared.borrow_mut();
        shared.closed = true;
        shared.router = None;
        shared.midi.close();
        shared.joystick.close();
        Ok(())
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    struct FakeRouter {
        seats: u32,
        keys: Vec<(u32, i32, bool, i64)>,
        joystick: Vec<(Option<i32>, Option<SeatId>)>,
    }

    impl DeviceRouter for FakeRouter {
        fn seat_id(&self, index: u32) -> Option<SeatId> {
            (index < self.seats).then(|| {
                qa_core::identity::IdentityOwner::create("devices")
                    .expect("owner")
                    .seat(index)
            })
        }

        fn seat_count(&self) -> u32 {
            self.seats
        }

        fn key(&mut self, index: u32, code: i32, down: bool, time_ms: i64) {
            self.keys.push((index, code, down, time_ms));
        }

        fn mouse_motion(&mut self, _index: u32, _dx: i32, _dy: i32, _time_ms: i64) {}

        fn release_seat(&mut self, _index: u32, _time_ms: i64, _append: Option<&mut dyn FnMut(&str)>) {}

        fn any_focused(&self) -> bool {
            true
        }

        fn set_source_joystick(&mut self, instance: Option<i32>, seat: Option<SeatId>) {
            self.joystick.push((instance, seat));
        }

        fn player_choices(&self) -> Vec<(String, String)> {
            (0..self.seats)
                .map(|index| (format!("{}", index + 1), format!("Player {}", index + 1)))
                .collect()
        }
    }

    struct NullBoundary;

    impl MidiInputBoundary for NullBoundary {
        fn list(&self) -> Result<Vec<qa_client::input::midi::MidiDevice>, MidiError> {
            Ok(Vec::new())
        }

        fn open(
            &self,
            _device: &qa_client::input::midi::MidiDevice,
        ) -> Result<Box<dyn qa_client::input::midi::MidiInputHandle>, MidiError> {
            Err(MidiError::Closed)
        }
    }

    fn devices(name: &str) -> (InputDevices<FakeRouter>, PathBuf) {
        let root = std::env::temp_dir().join(format!("qa-input-dev-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let store = input_device_store(Some(&root.to_string_lossy()));
        let print: PrintSink = Rc::new(RefCell::new(|_: &str| {}));
        let devices = InputDevices::new(
            CvarRegistry::new(Dialect::Q3),
            store,
            print,
            Box::new(NullBoundary),
            Box::new(|_, _| Err(SourceInputError::Closed)),
        )
        .expect("devices");
        (devices, root)
    }

    #[test]
    fn activates_restarts_and_closes() {
        let (mut devices, root) = devices("activate");
        devices
            .activate(
                FakeRouter {
                    seats: 2,
                    keys: Vec::new(),
                    joystick: Vec::new(),
                },
                100,
            )
            .expect("activate");
        devices.frame(200).expect("frame");
        devices.frame(1500).expect("retry");
        assert_eq!(devices.bindings().len(), 8);
        devices.info().expect("info");
        devices.save().expect("save");
        devices.close(2000).expect("close");
        assert!(devices
            .activate(
                FakeRouter {
                    seats: 1,
                    keys: Vec::new(),
                    joystick: Vec::new()
                },
                3000
            )
            .is_err());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn archive_roundtrip_validates() {
        let (mut devices, root) = devices("archive");
        devices
            .activate(
                FakeRouter {
                    seats: 1,
                    keys: Vec::new(),
                    joystick: Vec::new(),
                },
                0,
            )
            .expect("activate");
        devices.save().expect("save");
        let store = input_device_store(Some(&root.to_string_lossy()));
        let entries = load_input_device_settings(&store).expect("load");
        assert!(!entries.is_empty());
        assert!(entries
            .iter()
            .all(|entry| input_device_cvar_names().contains(&entry.name)));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
