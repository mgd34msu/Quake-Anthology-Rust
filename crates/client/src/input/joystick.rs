//! Source joystick state for both platform profiles.
//!
//! Donor provenance: `src/input/source-joystick.ts` (profiles from
//! Quake III `linux_joystick.c` and `win_input.c`).
//!
//! Events arrive as [`qa_platform::sdl::SdlJoystickEvent`]; synthetic
//! events drive headless tests.

use qa_platform::sdl::SdlJoystickEvent;
use thiserror::Error;

use super::KeyCode;

/// Source joystick profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceJoystickProfile {
    /// Linux threshold profile.
    Linux,
    /// Windows absolute U/V polling profile.
    Windows,
}

/// Source joystick error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JoystickError {
    /// Button index must fit a byte.
    #[error("Joystick button requires an unsigned byte")]
    BadButton,
    /// Windows profile sampled a missing axis.
    #[error("Missing Windows joystick axis")]
    MissingAxis,
    /// Windows profile sampled missing U/V axes.
    #[error("Missing Windows joystick U/V axes")]
    MissingBallAxes,
}

/// `IN_JoyMove` diagnostic row for the consumed sample.
#[must_use]
pub fn windows_joystick_debug(events: &[SdlJoystickEvent]) -> String {
    let mut buttons = 0u32;
    let mut pov = 65535;
    let mut axes = [0i16; 6];
    for event in events {
        match event {
            SdlJoystickEvent::Button { button, down, .. } => {
                if *button < 32 {
                    if *down {
                        buttons |= 1 << button;
                    } else {
                        buttons &= !(1 << button);
                    }
                }
            }
            SdlJoystickEvent::Axis { axis, value, .. } => {
                if usize::from(*axis) < axes.len() {
                    axes[usize::from(*axis)] = *value;
                }
            }
            SdlJoystickEvent::Hat { hat, value, .. } => {
                if *hat == 0 {
                    pov = match value {
                        1 => 0,
                        3 => 4500,
                        2 => 9000,
                        6 => 13500,
                        4 => 18000,
                        12 => 22500,
                        8 => 27000,
                        9 => 31500,
                        _ => 65535,
                    };
                }
            }
            SdlJoystickEvent::Removed { .. } => {}
        }
    }
    let mut fields = vec![format!("{buttons:08x}"), format!("{pov:>5}")];
    for (axis, value) in axes.iter().enumerate() {
        if axis < 4 {
            fields.push(format!("{:>5.2}", f32::from(*value) / 32768.0));
        } else {
            fields.push(format!("{value:>6}"));
        }
    }
    fields.join(" ") + "\n"
}

/// Linux threshold bitmask for up to 16 axes.
#[must_use]
pub fn sdl_joystick_axes(values: &[i16; 16], threshold: f64) -> u32 {
    let mut axes = 0u32;
    for (index, value) in values.iter().enumerate() {
        let fraction = f64::from(f32::from(*value) / 32767.0);
        if fraction < -threshold {
            axes |= 1 << (index * 2);
        } else if fraction > threshold {
            axes |= 1 << (index * 2 + 1);
        }
    }
    axes
}

fn joystick_keys() -> [i32; 16] {
    [
        KeyCode::Left as i32,
        KeyCode::Right as i32,
        KeyCode::Up as i32,
        KeyCode::Down as i32,
        KeyCode::Joy16 as i32,
        KeyCode::Joy17 as i32,
        KeyCode::Joy18 as i32,
        KeyCode::Joy19 as i32,
        KeyCode::Joy20 as i32,
        KeyCode::Joy21 as i32,
        KeyCode::Joy22 as i32,
        KeyCode::Joy23 as i32,
        KeyCode::Joy24 as i32,
        KeyCode::Joy25 as i32,
        KeyCode::Joy26 as i32,
        KeyCode::Joy27 as i32,
    ]
}

/// Held buttons, sampled axes, and edge-published axis keys.
#[derive(Debug, Clone)]
pub struct SourceJoystickState {
    held_buttons: std::collections::BTreeSet<i32>,
    axes: [i16; 16],
    old_axes: u32,
    hat: u8,
}

impl SourceJoystickState {
    /// Fresh state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held_buttons: std::collections::BTreeSet::new(),
            axes: [0; 16],
            old_axes: 0,
            hat: 0,
        }
    }

    /// Queue a button edge (`Joy1 + button`).
    pub fn button(
        &mut self,
        button: i32,
        down: bool,
        queue_key: &mut dyn FnMut(i32, bool, i64),
        transitions_only: bool,
    ) -> Result<(), JoystickError> {
        if !(0..=255).contains(&button) {
            return Err(JoystickError::BadButton);
        }
        let key = KeyCode::Joy1 as i32 + button;
        if transitions_only && self.held_buttons.contains(&key) == down {
            return Ok(());
        }
        queue_key(key, down, 0);
        if down {
            self.held_buttons.insert(key);
        } else {
            self.held_buttons.remove(&key);
        }
        Ok(())
    }

    /// Sample an axis value.
    pub fn axis(&mut self, axis: u8, value: i16) {
        if usize::from(axis) < self.axes.len() {
            self.axes[usize::from(axis)] = value;
        }
    }

    /// Sample POV hat zero.
    pub fn pov(&mut self, hat: u8, value: u8) {
        if hat == 0 {
            self.hat = value;
        }
    }

    /// Linux frame: publish threshold edges.
    pub fn frame(&mut self, threshold: Option<f64>, queue_key: &mut dyn FnMut(i32, bool, i64)) {
        let axes = threshold.map_or(0, |threshold| sdl_joystick_axes(&self.axes, threshold));
        self.publish_axes(axes, queue_key);
    }

    /// Windows frame: publish edges and ball motion.
    pub fn windows_frame(
        &mut self,
        threshold: Option<f64>,
        axis_count: i32,
        ball_scale: f64,
        queue_key: &mut dyn FnMut(i32, bool, i64),
        queue_mouse: &mut dyn FnMut(i32, i32, i64),
    ) -> Result<(), JoystickError> {
        let mut axes = 0u32;
        if let Some(threshold) = threshold {
            for index in 0..axis_count.min(4) {
                let value = self
                    .axes
                    .get(index as usize)
                    .copied()
                    .ok_or(JoystickError::MissingAxis)?;
                let fraction = f64::from(f32::from(value) / 32768.0);
                if fraction < -threshold {
                    axes |= 1 << (index * 2);
                } else if fraction > threshold {
                    axes |= 1 << (index * 2 + 1);
                }
            }
            match self.hat {
                1 => axes |= 1 << 12,
                4 => axes |= 1 << 13,
                2 => axes |= 1 << 14,
                8 => axes |= 1 << 15,
                _ => {}
            }
        }
        self.publish_axes(axes, queue_key);
        if threshold.is_some() && axis_count >= 6 {
            let u = self.axes.get(4).copied().ok_or(JoystickError::MissingBallAxes)?;
            let v = self.axes.get(5).copied().ok_or(JoystickError::MissingBallAxes)?;
            let dx = (f32::from(u) * ball_scale as f32).trunc() as i32;
            let dy = (f32::from(v) * ball_scale as f32).trunc() as i32;
            if dx != 0 || dy != 0 {
                queue_mouse(dx, dy, 0);
            }
        }
        Ok(())
    }

    fn publish_axes(&mut self, axes: u32, queue_key: &mut dyn FnMut(i32, bool, i64)) {
        for (bit, key) in joystick_keys().iter().enumerate() {
            let mask = 1 << bit;
            if axes & mask != self.old_axes & mask {
                queue_key(*key, axes & mask != 0, 0);
            }
        }
        self.old_axes = axes;
    }

    /// Release buttons and clear samples after device removal.
    pub fn remove_device(&mut self, queue_key: &mut dyn FnMut(i32, bool, i64)) {
        for key in std::mem::take(&mut self.held_buttons) {
            queue_key(key, false, 0);
        }
        self.axes = [0; 16];
        self.hat = 0;
    }

    /// Release every held key with the shared timestamp.
    pub fn release(&mut self, time: i64, queue_key: &mut dyn FnMut(i32, bool, i64)) {
        let mut held = std::mem::take(&mut self.held_buttons);
        for (bit, key) in joystick_keys().iter().enumerate() {
            if self.old_axes & (1 << bit) != 0 {
                held.insert(*key);
            }
        }
        for key in held {
            queue_key(key, false, time);
        }
        self.clear();
    }

    /// Clear buttons, samples, and edges.
    pub fn clear(&mut self) {
        self.held_buttons.clear();
        self.axes = [0; 16];
        self.old_axes = 0;
        self.hat = 0;
    }
}

impl Default for SourceJoystickState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_axes_publish_edges() {
        let mut state = SourceJoystickState::new();
        let mut keys = Vec::new();
        state.axis(0, 20000);
        state.frame(Some(0.15), &mut |key, down, _| keys.push((key, down)));
        assert_eq!(keys, vec![(KeyCode::Right as i32, true)]);
        state.axis(0, 0);
        state.frame(Some(0.15), &mut |key, down, _| keys.push((key, down)));
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1], (KeyCode::Right as i32, false));
        state.frame(None, &mut |_, _, _| panic!("no threshold means no edges"));
    }

    #[test]
    fn buttons_validate_and_track() {
        let mut state = SourceJoystickState::new();
        let mut keys = Vec::new();
        state
            .button(0, true, &mut |key, down, _| keys.push((key, down)), false)
            .unwrap();
        assert_eq!(keys, vec![(KeyCode::Joy1 as i32, true)]);
        state
            .button(0, true, &mut |_, _, _| panic!("transition swallowed"), true)
            .unwrap();
        assert!(state.button(256, true, &mut |_, _, _| {}, false).is_err());
        state.remove_device(&mut |key, down, _| keys.push((key, down)));
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn windows_frame_maps_hat_and_ball() {
        let mut state = SourceJoystickState::new();
        let mut keys = Vec::new();
        let mut mouse = Vec::new();
        state.pov(0, 1);
        state.axis(4, 1000);
        state
            .windows_frame(
                Some(0.15),
                6,
                0.02,
                &mut |key, down, _| keys.push((key, down)),
                &mut |dx, dy, _| {
                    mouse.push((dx, dy));
                },
            )
            .unwrap();
        assert_eq!(keys, vec![(KeyCode::Joy24 as i32, true)]);
        assert_eq!(mouse, vec![(20, 0)]);
        let debug = windows_joystick_debug(&[
            SdlJoystickEvent::Button {
                timestamp: 0,
                instance: 1,
                button: 0,
                down: true,
            },
            SdlJoystickEvent::Axis {
                timestamp: 0,
                instance: 1,
                axis: 0,
                value: 16384,
            },
        ]);
        assert!(debug.starts_with("00000001 65535  0.50"), "{debug}");
    }
}
