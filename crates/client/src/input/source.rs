//! Source joystick/mouse lifecycle.
//!
//! Donor provenance: `src/input/source-input.ts`
//! (`SourceInputState`, Quake III `IN_Init`/`IN_JoyMove` flow).
//!
//! The joystick backend is injected; [`open_platform_joystick`]
//! opens the real `qa-platform` device.

use qa_core::cvar::{CvarError, CvarRegistry};
use qa_platform::sdl::{JoystickProfile, SdlJoystick, SdlJoystickEvent};
use thiserror::Error;

use super::device::register_source_input_settings;
use super::joystick::{windows_joystick_debug, SourceJoystickProfile, SourceJoystickState};

/// Source input error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceInputError {
    /// Source input is closed.
    #[error("Source input is closed")]
    Closed,
    /// An input cvar no longer exists.
    #[error("Input cvar {0} no longer exists")]
    MissingCvar(String),
    /// The joystick profile must be linux or windows.
    #[error("in_joystickProfile must be linux or windows")]
    BadProfile,
    /// Windows joystick input requires a mouse event consumer.
    #[error("Windows joystick input requires a mouse event consumer")]
    MissingMouseConsumer,
    /// Joystick state error.
    #[error(transparent)]
    Joystick(#[from] super::joystick::JoystickError),
    /// Cvar error.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Platform joystick error.
    #[error("Platform joystick error: {0}")]
    Platform(String),
}

/// Source joystick device behind [`SourceInputState`].
pub trait SourceJoystickDevice {
    /// SDL instance id.
    fn instance(&self) -> i32;
    /// Device name.
    fn name(&self) -> &str;
    /// Axis count.
    fn axes(&self) -> i32;
    /// Button count.
    fn buttons(&self) -> i32;
    /// Poll fresh events.
    fn poll_events(&mut self, profile: SourceJoystickProfile) -> Result<Vec<SdlJoystickEvent>, SourceInputError>;
    /// Close the device.
    fn close(&mut self);
}

fn platform_profile(profile: SourceJoystickProfile) -> JoystickProfile {
    match profile {
        SourceJoystickProfile::Linux => JoystickProfile::Linux,
        SourceJoystickProfile::Windows => JoystickProfile::Windows,
    }
}

impl SourceJoystickDevice for SdlJoystick {
    fn instance(&self) -> i32 {
        SdlJoystick::instance(self)
    }

    fn name(&self) -> &str {
        SdlJoystick::name(self)
    }

    fn axes(&self) -> i32 {
        SdlJoystick::axes(self)
    }

    fn buttons(&self) -> i32 {
        SdlJoystick::buttons(self)
    }

    fn poll_events(&mut self, profile: SourceJoystickProfile) -> Result<Vec<SdlJoystickEvent>, SourceInputError> {
        SdlJoystick::poll_events(self, platform_profile(profile)).map_err(|error| SourceInputError::Platform(error.to_string()))
    }

    fn close(&mut self) {
        SdlJoystick::close(self);
    }
}

/// Open the first platform joystick, printing discovery like the source.
pub fn open_platform_joystick(
    print: &mut dyn FnMut(&str),
    profile: SourceJoystickProfile,
) -> Result<Option<Box<dyn SourceJoystickDevice>>, SourceInputError> {
    let print = std::cell::RefCell::new(print);
    SdlJoystick::open_first(|text| print.borrow_mut()(text), platform_profile(profile))
        .map(|joystick| joystick.map(|joystick| Box::new(joystick) as Box<dyn SourceJoystickDevice>))
        .map_err(|error| SourceInputError::Platform(error.to_string()))
}

/// Joystick opener for [`SourceInputState`].
pub type JoystickOpener = Box<dyn FnMut(&mut dyn FnMut(&str), SourceJoystickProfile) -> Result<Option<Box<dyn SourceJoystickDevice>>, SourceInputError>>;

/// Source mouse availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceMouse {
    /// Mouse input available.
    pub available: bool,
    /// Mouse input active.
    pub active: bool,
    /// Reset timestamp.
    pub reset_time: i64,
}

/// Client-build input: mouse availability plus joystick lifecycle.
pub struct SourceInputState {
    print: Box<dyn FnMut(&str)>,
    opener: JoystickOpener,
    /// Mouse state.
    pub mouse: SourceMouse,
    /// Joystick button/axis state (retained across restarts).
    pub joystick_state: SourceJoystickState,
    closed: bool,
    joystick: Option<Box<dyn SourceJoystickDevice>>,
    joystick_events: Vec<SdlJoystickEvent>,
    joystick_profile: SourceJoystickProfile,
}

impl SourceInputState {
    /// Inert construction; [`SourceInputState::initialize`] prints.
    #[must_use]
    pub fn new(print: Box<dyn FnMut(&str)>, opener: JoystickOpener) -> Self {
        Self {
            print,
            opener,
            mouse: SourceMouse::default(),
            joystick_state: SourceJoystickState::new(),
            closed: false,
            joystick: None,
            joystick_events: Vec::new(),
            joystick_profile: SourceJoystickProfile::Linux,
        }
    }

    /// Open joystick instance, if any.
    #[must_use]
    pub fn instance(&self) -> Option<i32> {
        self.joystick.as_ref().map(|joystick| joystick.instance())
    }

    fn cvar(cvars: &CvarRegistry, name: &str) -> Result<qa_core::cvar::CvarSnapshot, SourceInputError> {
        cvars.get(name).ok_or_else(|| SourceInputError::MissingCvar(name.to_string()))
    }

    /// Register source cvars, apply the joystick latch, discover hardware.
    pub fn initialize(&mut self, cvars: &mut CvarRegistry) -> Result<(), SourceInputError> {
        if self.closed {
            return Err(SourceInputError::Closed);
        }
        (self.print)("\n------- Input Initialization -------\n");
        register_source_input_settings(cvars)?;
        cvars.apply_latched(Some("in_joystick"))?;
        cvars.apply_latched(Some("in_joystickProfile"))?;
        let profile = Self::cvar(cvars, "in_joystickProfile")?.value.clone();
        self.joystick_profile = match profile.as_str() {
            "linux" => SourceJoystickProfile::Linux,
            "windows" => SourceJoystickProfile::Windows,
            _ => return Err(SourceInputError::BadProfile),
        };
        self.mouse.available = Self::cvar(cvars, "in_mouse")?.numeric_value != 0.0;
        // The source abandons the old descriptor but retains
        // IN_JoyMove's static axis state.
        if let Some(mut previous) = self.joystick.take() {
            previous.close();
        }
        self.joystick_events.clear();
        if Self::cvar(cvars, "in_joystick")?.integer_value != 0 {
            if self.joystick_profile == SourceJoystickProfile::Windows {
                self.joystick_state.clear();
            }
            let joystick = (self.opener)(&mut self.print, self.joystick_profile)?;
            match joystick {
                None => (self.print)("No joystick found.\n"),
                Some(mut joystick) => {
                    (self.print)(&format!(
                        "Joystick SDL instance {} found\nName:    {}\nAxes:    {}\nButtons: {}\n",
                        joystick.instance(),
                        joystick.name(),
                        joystick.axes(),
                        joystick.buttons()
                    ));
                    // Linux discards JS_EVENT_INIT; Windows samples
                    // current values in each frame.
                    joystick.poll_events(self.joystick_profile)?;
                    self.joystick = Some(joystick);
                }
            }
        } else {
            (self.print)("Joystick is not active.\n");
        }
        (self.print)("------------------------------------\n");
        Ok(())
    }

    /// Restart without retiring the window.
    pub fn restart(&mut self, cvars: &mut CvarRegistry) -> Result<(), SourceInputError> {
        if self.closed {
            return Err(SourceInputError::Closed);
        }
        self.mouse.available = false;
        self.initialize(cvars)
    }

    /// Queue an externally routed event for this device.
    pub fn queue_joystick_event(&mut self, event: SdlJoystickEvent) -> Result<(), SourceInputError> {
        if self.closed {
            return Err(SourceInputError::Closed);
        }
        let owned = match (&self.joystick, &event) {
            (Some(joystick), SdlJoystickEvent::Axis { instance, .. })
            | (Some(joystick), SdlJoystickEvent::Hat { instance, .. })
            | (Some(joystick), SdlJoystickEvent::Button { instance, .. })
            | (Some(joystick), SdlJoystickEvent::Removed { instance, .. }) => *instance == joystick.instance(),
            _ => false,
        };
        if owned {
            self.joystick_events.push(event);
        }
        Ok(())
    }

    /// Run `IN_JoyMove` after console polling.
    pub fn joystick_frame(
        &mut self,
        cvars: &CvarRegistry,
        queue_key: &mut dyn FnMut(i32, bool, i64),
        queue_mouse: Option<&mut dyn FnMut(i32, i32, i64)>,
    ) -> Result<(), SourceInputError> {
        if self.closed {
            return Err(SourceInputError::Closed);
        }
        if self.joystick.is_none() {
            return Ok(());
        }
        let mut events = std::mem::take(&mut self.joystick_events);
        if let Some(joystick) = self.joystick.as_mut() {
            events.extend(joystick.poll_events(self.joystick_profile)?);
        }
        if self.joystick_profile == SourceJoystickProfile::Windows
            && Self::cvar(cvars, "in_debugjoystick")?.integer_value != 0
            && !events.iter().any(|event| matches!(event, SdlJoystickEvent::Removed { .. }))
        {
            let debug = windows_joystick_debug(&events);
            (self.print)(&debug);
        }
        for event in events {
            if self.joystick.is_none() {
                break;
            }
            match event {
                SdlJoystickEvent::Button { button, down, .. } => {
                    let windows = self.joystick_profile == SourceJoystickProfile::Windows;
                    self.joystick_state.button(i32::from(button), down, queue_key, windows)?;
                }
                SdlJoystickEvent::Axis { axis, value, .. } => self.joystick_state.axis(axis, value),
                SdlJoystickEvent::Hat { hat, value, .. } => self.joystick_state.pov(hat, value),
                SdlJoystickEvent::Removed { .. } => {
                    (self.print)("SDL joystick disconnected.\n");
                    if let Some(mut joystick) = self.joystick.take() {
                        joystick.close();
                    }
                    self.joystick_state.remove_device(queue_key);
                }
            }
        }
        let threshold = if self.joystick.is_none() {
            None
        } else {
            Some(f64::from(Self::cvar(cvars, "joy_threshold")?.numeric_value))
        };
        if self.joystick_profile == SourceJoystickProfile::Windows {
            let Some(queue_mouse) = queue_mouse else {
                return Err(SourceInputError::MissingMouseConsumer);
            };
            let axes = self.joystick.as_ref().map_or(0, |joystick| joystick.axes());
            let ball = f64::from(Self::cvar(cvars, "in_joyBallScale")?.numeric_value);
            self.joystick_state.windows_frame(threshold, axes, ball, queue_key, queue_mouse)?;
        } else {
            self.joystick_state.frame(threshold, queue_key);
        }
        Ok(())
    }

    /// Release joystick keys on SDL focus loss.
    pub fn release_joystick_state(&mut self, time: i64, queue_key: &mut dyn FnMut(i32, bool, i64)) -> Result<(), SourceInputError> {
        if self.closed {
            return Err(SourceInputError::Closed);
        }
        self.joystick_state.release(time, queue_key);
        Ok(())
    }

    /// Final disposal releases the joystick.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.mouse.available = false;
        if let Some(mut joystick) = self.joystick.take() {
            joystick.close();
        }
        self.joystick_events.clear();
        self.joystick_state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    struct FakeJoystick {
        events: Vec<SdlJoystickEvent>,
        closed: bool,
    }

    impl SourceJoystickDevice for FakeJoystick {
        fn instance(&self) -> i32 {
            11
        }

        fn name(&self) -> &str {
            "fake"
        }

        fn axes(&self) -> i32 {
            4
        }

        fn buttons(&self) -> i32 {
            12
        }

        fn poll_events(&mut self, _profile: SourceJoystickProfile) -> Result<Vec<SdlJoystickEvent>, SourceInputError> {
            Ok(std::mem::take(&mut self.events))
        }

        fn close(&mut self) {
            self.closed = true;
        }
    }

    fn harness() -> (SourceInputState, CvarRegistry, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
        let printed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = printed.clone();
        let state = SourceInputState::new(
            Box::new(move |text| sink.borrow_mut().push(text.to_string())),
            Box::new(|_, _| {
                Ok(Some(Box::new(FakeJoystick {
                    events: Vec::new(),
                    closed: false,
                }) as Box<dyn SourceJoystickDevice>))
            }),
        );
        (state, CvarRegistry::new(Dialect::Q3), printed)
    }

    #[test]
    fn initializes_discovers_and_pumps() {
        let (mut state, mut cvars, printed) = harness();
        cvars.register("in_joystick", "0", qa_core::cvar::flags::ARCHIVE).unwrap();
        state.initialize(&mut cvars).unwrap();
        assert!(state.mouse.available);
        assert!(state.instance().is_none());
        assert!(printed.borrow().iter().any(|line| line.contains("not active")));
        cvars.set("in_joystick", "1", false).unwrap();
        state.restart(&mut cvars).unwrap();
        assert_eq!(state.instance(), Some(11));
        let mut keys = Vec::new();
        state
            .queue_joystick_event(SdlJoystickEvent::Button {
                timestamp: 0,
                instance: 11,
                button: 2,
                down: true,
            })
            .unwrap();
        state.joystick_frame(&cvars, &mut |key, down, _| keys.push((key, down)), None).unwrap();
        assert_eq!(keys, vec![(super::super::KeyCode::Joy1 as i32 + 2, true)]);
        state.release_joystick_state(9, &mut |key, down, _| keys.push((key, down))).unwrap();
        assert!(keys.iter().any(|(_, down)| !down));
        state.close();
        assert!(state.initialize(&mut cvars).is_err());
    }

    #[test]
    fn rejects_bad_profiles() {
        let (mut state, mut cvars, _) = harness();
        state.initialize(&mut cvars).unwrap();
        cvars.set("in_joystickProfile", "mac", false).unwrap();
        assert_eq!(state.restart(&mut cvars), Err(SourceInputError::BadProfile));
    }
}
