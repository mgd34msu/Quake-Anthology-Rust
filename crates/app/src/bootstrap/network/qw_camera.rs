//! QuakeWorld spectator camera (`QW/client/cl_cam.c`).
//!
//! Port of `src/app/bootstrap/network/qw-camera.ts` (`QwSpectatorCamera`).
//! Donor `QwPlayerState`/`QwUserCommand` map to the existing
//! [`QwPlayerState`](qa_net::q1_net::QwPlayerState) and
//! [`QwUsercmd`](qa_net::qw::QwUsercmd) codec types; camera math converts
//! their `f64` origins/angles to [`Vec3`](qa_core::math::Vec3), matching the
//! donor's `Math.fround` rounding. Lengths and `atan2` run in `f64` exactly
//! like the donor's double arithmetic. The donor surface is total, so
//! [`QwCameraError`] only reserves the module's error domain.

use std::collections::HashMap;
use std::f64::consts::PI;

use qa_core::math::{vec3, Vec3};
use qa_net::q1_net::QwPlayerState;
use qa_net::qw::QwUsercmd;
use thiserror::Error;

/// Spectator camera options (`QwCameraOptions`).
pub struct QwCameraOptions {
    /// `hightrack` cvar value.
    pub hightrack: Box<dyn Fn() -> i32>,
    /// `chasecam` cvar value.
    pub chasecam: Box<dyn Fn() -> i32>,
}

/// Spectator camera player metadata (`QwCameraPlayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwCameraPlayer {
    /// Player name.
    pub name: String,
    /// Whether the player spectates.
    pub spectator: bool,
    /// Frag count.
    pub frags: i32,
}

/// Spectator camera trace result (`QwCameraTrace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QwCameraTrace {
    /// Trace fraction.
    pub fraction: f64,
    /// Trace end.
    pub end: Vec3,
    /// Whether the trace ended in water.
    pub in_water: bool,
}

/// Spectator camera frame (`QwCameraFrame`).
pub struct QwCameraFrame {
    /// Local player state (donor `self`).
    pub viewer: QwPlayerState,
    /// Player states by slot.
    pub players: HashMap<i32, QwPlayerState>,
    /// Player metadata by slot.
    pub users: HashMap<i32, QwCameraPlayer>,
    /// Frame time in seconds.
    pub seconds: f64,
}

/// Spectator camera view (`QwCameraView`).
#[derive(Debug, Clone, PartialEq)]
pub struct QwCameraView {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera angles.
    pub angles: Vec3,
    /// Tracked target.
    pub target: QwPlayerState,
    /// Whether chasecam follows the target origin.
    pub chase: bool,
}

/// Error for the QuakeWorld spectator camera.
///
/// The donor surface is total; this reserves the module's error domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QwCameraError {}

/// Maximum tracked slots (`32`).
const MAX_SLOTS: i32 = 32;

/// Single-precision vector from double components (donor `vector`).
fn vector(x: f64, y: f64, z: f64) -> Vec3 {
    vec3(x as f32, y as f32, z as f32)
}

/// Single-precision subtraction (donor `subtract`).
fn subtract(a: Vec3, b: Vec3) -> Vec3 {
    vector(
        f64::from(a.x) - f64::from(b.x),
        f64::from(a.y) - f64::from(b.y),
        f64::from(a.z) - f64::from(b.z),
    )
}

/// Single-precision addition through `vector` rounding (donor `add`).
fn add(a: Vec3, b: Vec3) -> Vec3 {
    vector(
        f64::from(a.x) + f64::from(b.x),
        f64::from(a.y) + f64::from(b.y),
        f64::from(a.z) + f64::from(b.z),
    )
}

/// Double-precision length (donor `length`).
fn length(value: Vec3) -> f64 {
    let x = f64::from(value.x);
    let y = f64::from(value.y);
    let z = f64::from(value.z);
    (x * x + y * y + z * z).sqrt()
}

/// Player-state origin as a camera vector.
fn origin_of(state: &QwPlayerState) -> Vec3 {
    vector(state.origin[0], state.origin[1], state.origin[2])
}

/// User-command angles as a camera vector.
fn angles_of(command: &QwUsercmd) -> Vec3 {
    vector(command.angles[0], command.angles[1], command.angles[2])
}

/// Camera vector as wire angles.
fn wire_angles(angles: Vec3) -> [f64; 3] {
    [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)]
}

/// View angles toward a delta (donor `viewAngles`).
fn view_angles(v: Vec3) -> Vec3 {
    if v.x == 0.0 && v.y == 0.0 {
        return vector(if v.z > 0.0 { -90.0 } else { -270.0 }, 0.0, 0.0);
    }
    let yaw = (f64::from(v.y).atan2(f64::from(v.x)) * 180.0 / PI).trunc();
    let pitch = (f64::from(v.z).atan2((f64::from(v.x).powi(2) + f64::from(v.y).powi(2)).sqrt()) * 180.0 / PI).trunc();
    vector(
        -(if pitch < 0.0 { pitch + 360.0 } else { pitch }),
        if yaw < 0.0 { yaw + 360.0 } else { yaw },
        0.0,
    )
}

/// QuakeWorld spectator camera (donor `QwSpectatorCamera`).
pub struct QwSpectatorCamera {
    options: QwCameraOptions,
    send_command: Box<dyn FnMut(&str)>,
    trace: Box<dyn FnMut(Vec3, Vec3) -> QwCameraTrace>,
    print: Box<dyn FnMut(&str)>,
    tracking: bool,
    locked: bool,
    slot: i32,
    old_buttons: u8,
    desired: Vec3,
    last_view_seconds: f64,
    teleport: Option<Vec3>,
    current: Option<QwCameraView>,
}

impl QwSpectatorCamera {
    /// Create a spectator camera.
    pub fn new(
        options: QwCameraOptions,
        send_command: impl FnMut(&str) + 'static,
        trace: impl FnMut(Vec3, Vec3) -> QwCameraTrace + 'static,
        print: impl FnMut(&str) + 'static,
    ) -> Self {
        Self {
            options,
            send_command: Box::new(send_command),
            trace: Box::new(trace),
            print: Box::new(print),
            tracking: false,
            locked: false,
            slot: 0,
            old_buttons: 0,
            desired: vector(0.0, 0.0, 0.0),
            last_view_seconds: 0.0,
            teleport: None,
            current: None,
        }
    }

    /// Current view while tracking and locked, else `None` (donor `view`).
    pub fn view(&self) -> Option<&QwCameraView> {
        if self.tracking && self.locked {
            self.current.as_ref()
        } else {
            None
        }
    }

    /// Take a pending spectator teleport (donor `takeTeleport`).
    pub fn take_teleport(&mut self) -> Option<Vec3> {
        self.teleport.take()
    }

    /// Reset tracking state (donor `reset`).
    pub fn reset(&mut self) {
        self.tracking = false;
        self.locked = false;
        self.slot = 0;
        self.old_buttons = 0;
        self.current = None;
        self.teleport = None;
    }

    /// Lock onto a slot (donor `lock`).
    fn lock(&mut self, slot: i32) {
        let text = format!("ptrack {slot}");
        (self.send_command)(&text);
        self.slot = slot;
        self.locked = false;
        self.current = None;
    }

    /// Stop tracking (donor `unlock`).
    fn unlock(&mut self) {
        if self.tracking {
            (self.send_command)("ptrack");
        }
        self.tracking = false;
        self.locked = false;
        self.current = None;
    }

    /// Whether a user is a trackable player (donor `eligible`).
    fn eligible(user: Option<&QwCameraPlayer>) -> bool {
        matches!(user, Some(user) if !user.name.is_empty() && !user.spectator)
    }

    /// Track the highest-frag player (donor `highTarget`).
    fn high_target(&mut self, users: &HashMap<i32, QwCameraPlayer>) {
        let mut best = -1;
        let mut frags = -9999;
        for slot in 0..MAX_SLOTS {
            if let Some(user) = users.get(&slot) {
                if Self::eligible(Some(user)) && user.frags > frags {
                    best = slot;
                    frags = user.frags;
                }
            }
        }
        if best < 0 {
            self.unlock();
        } else if !self.locked || frags > users.get(&self.slot).map(|user| user.frags).unwrap_or(-9999) {
            self.lock(best);
        }
    }

    /// Whether the desired camera position sees the target (donor `visible`).
    fn is_visible(&mut self, target: &QwPlayerState) -> bool {
        let origin = origin_of(target);
        let result = (self.trace)(origin, self.desired);
        result.fraction == 1.0 && !result.in_water && length(subtract(origin, self.desired)) >= 16.0
    }

    /// Pick a flyby camera position (donor `flyby`).
    fn flyby(&mut self, viewer: &QwPlayerState, target: &QwPlayerState, check_visibility: bool) -> bool {
        let yaw = target.command.angles[1] * PI / 180.0;
        let roll = target.command.angles[2] * PI / 180.0;
        let forward = vector(yaw.cos(), yaw.sin(), 0.0);
        let right = vector(roll.cos() * yaw.sin(), -roll.cos() * yaw.cos(), -roll.sin());
        let up = vector(roll.sin() * yaw.sin(), -roll.sin() * yaw.cos(), roll.cos());
        let directions = [
            add(add(forward, up), right),
            subtract(add(forward, up), right),
            add(forward, right),
            subtract(forward, right),
            add(forward, up),
            subtract(forward, up),
            subtract(add(up, right), forward),
            subtract(subtract(up, right), forward),
            vector(-f64::from(forward.x), -f64::from(forward.y), -f64::from(forward.z)),
            forward,
            vector(-f64::from(right.x), -f64::from(right.y), -f64::from(right.z)),
            right,
        ];
        let target_origin = origin_of(target);
        let viewer_origin = origin_of(viewer);
        let mut best = 1000.0;
        let mut position: Option<Vec3> = None;
        for direction in directions {
            let magnitude = length(direction);
            let unit = vector(
                f64::from(direction.x) / magnitude,
                f64::from(direction.y) / magnitude,
                f64::from(direction.z) / magnitude,
            );
            let end = vector(
                f64::from(target_origin.x) + 800.0 * f64::from(unit.x),
                f64::from(target_origin.y) + 800.0 * f64::from(unit.y),
                f64::from(target_origin.z) + 800.0 * f64::from(unit.z),
            );
            let result = (self.trace)(target_origin, end);
            if result.in_water {
                continue;
            }
            let mut distance = length(subtract(result.end, target_origin));
            if distance < 32.0 || distance > 800.0 {
                continue;
            }
            if check_visibility {
                distance = length(subtract(result.end, viewer_origin));
                let visible = (self.trace)(viewer_origin, result.end);
                if visible.fraction != 1.0 || visible.in_water {
                    continue;
                }
            }
            if distance < best {
                best = distance;
                position = Some(result.end);
            }
        }
        if let Some(position) = position {
            self.locked = true;
            self.desired = position;
            true
        } else {
            false
        }
    }

    /// Filter a user command through the camera (donor `command`).
    pub fn command(&mut self, command: &QwUsercmd, frame: &QwCameraFrame) -> QwUsercmd {
        let mut result = command.clone();
        if (self.options.hightrack)() != 0 && !self.locked {
            self.high_target(&frame.users);
        }
        if self.tracking {
            if self.locked && !Self::eligible(frame.users.get(&self.slot)) {
                self.locked = false;
                if (self.options.hightrack)() != 0 {
                    self.high_target(&frame.users);
                } else {
                    self.unlock();
                }
            } else if let Some(target) = frame.players.get(&self.slot) {
                if !self.locked || !self.is_visible(target) {
                    if !self.locked || frame.seconds - self.last_view_seconds > 0.1 {
                        if !self.flyby(&frame.viewer, target, true) {
                            self.flyby(&frame.viewer, target, false);
                        }
                        self.last_view_seconds = frame.seconds;
                    }
                } else {
                    self.last_view_seconds = frame.seconds;
                }
                if self.locked {
                    let chase = (self.options.chasecam)() != 0;
                    if chase {
                        self.desired = origin_of(target);
                    }
                    let delta = subtract(self.desired, origin_of(&frame.viewer));
                    let moved = if chase {
                        delta.x != 0.0 || delta.y != 0.0 || delta.z != 0.0
                    } else {
                        length(delta) > 16.0
                    };
                    if moved {
                        self.teleport = Some(self.desired);
                    }
                    let angles = if chase {
                        angles_of(&target.command)
                    } else {
                        view_angles(subtract(origin_of(target), self.desired))
                    };
                    self.current = Some(QwCameraView {
                        origin: self.desired,
                        angles,
                        target: target.clone(),
                        chase,
                    });
                    result.angles = wire_angles(angles);
                    result.forwardmove = 0;
                    result.sidemove = 0;
                    result.upmove = 0;
                }
            }
        }
        self.finish(&result, &frame.users);
        result
    }

    /// Handle tracking buttons after filtering (donor `finish`).
    fn finish(&mut self, command: &QwUsercmd, users: &HashMap<i32, QwCameraPlayer>) {
        if command.buttons & 1 != 0 {
            if self.old_buttons & 1 != 0 {
                return;
            }
            self.old_buttons |= 1;
            if self.tracking {
                self.unlock();
                return;
            }
            self.tracking = true;
        } else {
            self.old_buttons &= !1;
            if !self.tracking {
                return;
            }
        }
        if (self.options.hightrack)() != 0 {
            self.high_target(users);
            return;
        }
        if self.locked {
            if command.buttons & 2 != 0 && self.old_buttons & 2 != 0 {
                return;
            }
            if command.buttons & 2 == 0 {
                self.old_buttons &= !2;
                return;
            }
            self.old_buttons |= 2;
        }
        let start = if self.locked {
            (self.slot + 1) % MAX_SLOTS
        } else {
            self.slot
        };
        for offset in 0..MAX_SLOTS {
            let slot = (start + offset) % MAX_SLOTS;
            if Self::eligible(users.get(&slot)) {
                self.lock(slot);
                return;
            }
        }
        (self.print)("No target found ...\n");
        self.tracking = false;
        self.locked = false;
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn player(origin: [f64; 3], angles: [f64; 3]) -> QwPlayerState {
        QwPlayerState {
            origin,
            command: QwUsercmd {
                angles,
                ..QwUsercmd::default()
            },
            ..QwPlayerState::default()
        }
    }

    fn user(name: &str, spectator: bool, frags: i32) -> QwCameraPlayer {
        QwCameraPlayer {
            name: name.to_string(),
            spectator,
            frags,
        }
    }

    fn frame(viewer: QwPlayerState) -> QwCameraFrame {
        QwCameraFrame {
            viewer,
            players: HashMap::new(),
            users: HashMap::new(),
            seconds: 1.0,
        }
    }

    struct Harness {
        camera: QwSpectatorCamera,
        sent: Rc<RefCell<Vec<String>>>,
        printed: Rc<RefCell<Vec<String>>>,
    }

    fn harness(hightrack: i32, chasecam: i32, trace: impl FnMut(Vec3, Vec3) -> QwCameraTrace + 'static) -> Harness {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sent_out = Rc::clone(&sent);
        let printed_out = Rc::clone(&printed);
        let camera = QwSpectatorCamera::new(
            QwCameraOptions {
                hightrack: Box::new(move || hightrack),
                chasecam: Box::new(move || chasecam),
            },
            move |text: &str| sent_out.borrow_mut().push(text.to_string()),
            trace,
            move |text: &str| printed_out.borrow_mut().push(text.to_string()),
        );
        Harness { camera, sent, printed }
    }

    fn open_trace(_start: Vec3, _end: Vec3) -> QwCameraTrace {
        QwCameraTrace {
            fraction: 1.0,
            end: vec3(100.0, 0.0, 0.0),
            in_water: false,
        }
    }

    #[test]
    fn attack_press_starts_tracking_and_locks_first_player() {
        let mut test = harness(0, 0, open_trace);
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.players.insert(1, player([100.0, 0.0, 50.0], [0.0, 90.0, 0.0]));
        frame.users.insert(1, user("duke", false, 3));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        assert_eq!(*test.sent.borrow(), vec!["ptrack 1".to_string()]);
        assert!(test.camera.view().is_none());

        let released = QwUsercmd::default();
        let filtered = test.camera.command(&released, &frame);
        let view = test.camera.view().expect("flyby locks the target");
        assert!(!view.chase);
        assert_eq!(view.origin, vec3(100.0, 0.0, 0.0));
        assert_eq!(filtered.angles, [-90.0, 0.0, 0.0]);
        assert_eq!(filtered.forwardmove, 0);
        assert_eq!(filtered.sidemove, 0);
        assert_eq!(filtered.upmove, 0);
        assert_eq!(test.camera.take_teleport(), Some(vec3(100.0, 0.0, 0.0)));
        assert!(test.camera.take_teleport().is_none());
    }

    #[test]
    fn attack_press_while_tracking_unlocks() {
        let mut test = harness(0, 0, open_trace);
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.players.insert(0, player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.users.insert(0, user("duke", false, 0));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        test.camera.command(&QwUsercmd::default(), &frame);
        assert!(test.camera.view().is_some());
        test.camera.command(&pressed, &frame);
        assert!(test.camera.view().is_none());
        assert_eq!(*test.sent.borrow(), vec!["ptrack 0".to_string(), "ptrack".to_string()]);
    }

    #[test]
    fn held_attack_does_not_retoggle() {
        let mut test = harness(0, 0, open_trace);
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.users.insert(0, user("duke", false, 0));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        test.camera.command(&pressed, &frame);
        assert_eq!(*test.sent.borrow(), vec!["ptrack 0".to_string()]);
    }

    #[test]
    fn no_eligible_target_prints_and_stops_tracking() {
        let mut test = harness(0, 0, open_trace);
        let frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        assert!(test.sent.borrow().is_empty());
        assert_eq!(*test.printed.borrow(), vec!["No target found ...\n".to_string()]);
        test.camera.command(&QwUsercmd::default(), &frame);
        assert!(test.camera.view().is_none());
    }

    #[test]
    fn hightrack_selects_highest_frags() {
        let mut test = harness(1, 0, open_trace);
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.users.insert(0, user("low", false, 1));
        frame.users.insert(3, user("high", false, 9));
        frame.users.insert(5, user("spec", true, 99));
        frame.users.insert(6, user("", false, 50));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        // The donor locks twice per press: `command` runs `highTarget` before
        // tracking starts, and `finish` runs it again after arming tracking.
        assert_eq!(
            *test.sent.borrow(),
            vec!["ptrack 3".to_string(), "ptrack 3".to_string()]
        );
    }

    #[test]
    fn hightrack_with_no_target_unlocks() {
        let mut test = harness(1, 0, open_trace);
        let frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        assert_eq!(*test.sent.borrow(), vec!["ptrack".to_string()]);
    }

    #[test]
    fn chasecam_follows_target_origin_and_angles() {
        let mut test = harness(0, 1, open_trace);
        let mut frame = frame(player([10.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.players.insert(0, player([100.0, 20.0, 30.0], [5.0, 90.0, 0.0]));
        frame.users.insert(0, user("duke", false, 0));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        let filtered = test.camera.command(&QwUsercmd::default(), &frame);
        let view = test.camera.view().expect("chase locks the target");
        assert!(view.chase);
        assert_eq!(view.origin, vec3(100.0, 20.0, 30.0));
        assert_eq!(filtered.angles, [5.0, 90.0, 0.0]);
        assert_eq!(test.camera.take_teleport(), Some(vec3(100.0, 20.0, 30.0)));
    }

    #[test]
    fn water_blocks_every_flyby_direction() {
        let mut test = harness(0, 0, |_start, end| QwCameraTrace {
            fraction: 1.0,
            end,
            in_water: true,
        });
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.players.insert(0, player([64.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.users.insert(0, user("duke", false, 0));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        let filtered = test.camera.command(&QwUsercmd::default(), &frame);
        assert!(test.camera.view().is_none());
        assert_eq!(filtered, QwUsercmd::default());
    }

    #[test]
    fn ineligible_locked_user_retargets_without_hightrack() {
        let mut test = harness(0, 0, open_trace);
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.players.insert(0, player([64.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.users.insert(0, user("duke", false, 0));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        test.camera.command(&QwUsercmd::default(), &frame);
        assert!(test.camera.view().is_some());
        frame.users.insert(0, user("duke", true, 0));
        test.camera.command(&QwUsercmd::default(), &frame);
        assert!(test.camera.view().is_none());
        assert_eq!(*test.sent.borrow(), vec!["ptrack 0".to_string(), "ptrack".to_string()]);
    }

    #[test]
    fn view_angles_handles_vertical_deltas() {
        assert_eq!(view_angles(vec3(0.0, 0.0, 10.0)), vec3(-90.0, 0.0, 0.0));
        assert_eq!(view_angles(vec3(0.0, 0.0, -10.0)), vec3(-270.0, 0.0, 0.0));
        let angles = view_angles(vec3(100.0, 0.0, 0.0));
        assert_eq!(angles.x, 0.0);
        assert_eq!(angles.y, 0.0);
        assert_eq!(angles.z, 0.0);
    }

    #[test]
    fn reset_clears_tracking_and_view() {
        let mut test = harness(0, 0, open_trace);
        let mut frame = frame(player([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.players.insert(0, player([64.0, 0.0, 0.0], [0.0, 0.0, 0.0]));
        frame.users.insert(0, user("duke", false, 0));
        let pressed = QwUsercmd {
            buttons: 1,
            ..QwUsercmd::default()
        };
        test.camera.command(&pressed, &frame);
        test.camera.command(&QwUsercmd::default(), &frame);
        assert!(test.camera.view().is_some());
        test.camera.reset();
        assert!(test.camera.view().is_none());
        assert!(test.camera.take_teleport().is_none());
    }
}
