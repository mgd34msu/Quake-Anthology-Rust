//! Port of `src/compat/q2/native-mod-client-stages.ts`.
//! Bridges client call stages: exclusion regions apply only while their
//! declaring original call is active, and client input lowers to user commands.

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::Vec3;
use qa_guest::core::contracts::GuestAddress;
use thiserror::Error;

/// Failures staging client calls or encoding user commands.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum StageError {
    /// The output buffer is too small.
    #[error("native user command output exceeds its buffer")]
    BufferTooSmall,
    /// A value exceeds the float32 command ABI.
    #[error("native input output exceeds the float32 command ABI")]
    FloatRange,
    /// An aim value exceeds the integer command ABI.
    #[error("native input aim exceeds the integer command ABI")]
    AimRange,
    /// A movement value exceeds the int16 command ABI.
    #[error("native input movement exceeds the int16 command ABI")]
    MoveRange,
    /// A command input is missing or not finite.
    #[error("missing native command input {0}")]
    MissingInput(&'static str),
    /// The command interval exceeds its byte ABI.
    #[error("native command interval exceeds its original byte ABI")]
    IntervalRange,
}

/// One exclusion region inside an original call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkipRegion {
    /// Region entry RVA.
    pub entry: u64,
    /// Region join RVA.
    pub join: u64,
}

/// Source call with its exclusion regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageCall {
    /// Call id.
    pub id: String,
    /// Exclusion regions.
    pub skips: Vec<SkipRegion>,
}

/// Inline-region host surface.
pub trait InlineRegionHost {
    /// Image base address.
    fn image_base(&self) -> GuestAddress;
    /// Bind an inline region; returns a binding id.
    fn bind_inline_region(&mut self, entry: GuestAddress, join: GuestAddress) -> u64;
    /// Release an inline binding.
    fn unbind_inline_region(&mut self, id: u64);
}

/// Headless stage host recording active inline regions.
#[derive(Debug)]
pub struct SyntheticStageHost {
    image_base: GuestAddress,
    next_id: u64,
    active: HashMap<u64, (u64, u64)>,
}

impl Default for SyntheticStageHost {
    fn default() -> Self {
        Self {
            image_base: GuestAddress::new(0, 0),
            next_id: 1,
            active: HashMap::new(),
        }
    }
}

impl SyntheticStageHost {
    /// Build a host with an image base.
    #[must_use]
    pub fn new(image_base: GuestAddress) -> Self {
        Self {
            image_base,
            next_id: 1,
            active: HashMap::new(),
        }
    }

    /// Active bindings as (entry, join) offset pairs.
    #[must_use]
    pub fn active(&self) -> Vec<(u64, u64)> {
        self.active.values().copied().collect()
    }
}

impl InlineRegionHost for SyntheticStageHost {
    fn image_base(&self) -> GuestAddress {
        self.image_base
    }

    fn bind_inline_region(&mut self, entry: GuestAddress, join: GuestAddress) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.active.insert(id, (entry.offset, join.offset));
        id
    }

    fn unbind_inline_region(&mut self, id: u64) {
        self.active.remove(&id);
    }
}

struct ActiveScope {
    call: StageCall,
    removals: Vec<u64>,
    teardown: Option<Box<dyn FnMut()>>,
    extra: Option<Rc<dyn Fn() -> Box<dyn FnMut()>>>,
}

/// Client call stages: exclusions follow the innermost active original call.
#[derive(Default)]
pub struct NativeModClientStages {
    active: Option<ActiveScope>,
}

impl NativeModClientStages {
    /// Build idle stages.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Id of the active call, if any.
    #[must_use]
    pub fn active_call(&self) -> Option<&str> {
        self.active.as_ref().map(|scope| scope.call.id.as_str())
    }

    fn bind<H: InlineRegionHost>(scope: &mut ActiveScope, host: &mut H) {
        for region in &scope.call.skips {
            let base = host.image_base();
            // Synthetic offsets stay within the image mapping by construction.
            let entry = GuestAddress::new(base.space, base.offset + region.entry);
            let join = GuestAddress::new(base.space, base.offset + region.join);
            scope.removals.push(host.bind_inline_region(entry, join));
        }
        if let Some(setup) = scope.extra.clone() {
            scope.teardown = Some(setup());
        }
    }

    fn unbind<H: InlineRegionHost>(scope: &mut ActiveScope, host: &mut H) {
        if let Some(mut teardown) = scope.teardown.take() {
            teardown();
        }
        for id in scope.removals.drain(..).rev() {
            host.unbind_inline_region(id);
        }
    }

    /// Run a call with its exclusions (plus an optional extra binding) active.
    ///
    /// Nested runs unbind the outer scope while the inner call executes and
    /// rebind it afterwards.
    pub fn run<T, H: InlineRegionHost>(
        &mut self,
        host: &mut H,
        call: &StageCall,
        invoke: impl FnOnce(&mut Self, &mut H) -> T,
        extra: Option<Rc<dyn Fn() -> Box<dyn FnMut()>>>,
    ) -> T {
        let mut previous = self.active.take();
        if let Some(scope) = previous.as_mut() {
            Self::unbind(scope, host);
        }
        self.active = Some(ActiveScope {
            call: call.clone(),
            removals: Vec::new(),
            teardown: None,
            extra,
        });
        if let Some(scope) = self.active.as_mut() {
            Self::bind(scope, host);
        }
        let result = invoke(&mut *self, &mut *host);
        if let Some(mut scope) = self.active.take() {
            Self::unbind(&mut scope, host);
        }
        if let Some(mut scope) = previous {
            Self::bind(&mut scope, host);
            self.active = Some(scope);
        }
        result
    }
}

/// Classic user command (16 bytes on the wire).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicUserCommand {
    /// Frame milliseconds.
    pub milliseconds: u8,
    /// Button bits.
    pub buttons: u8,
    /// Aim as short fractions of a turn.
    pub angle_shorts: [i16; 3],
    /// Forward movement.
    pub forward_move: i16,
    /// Side movement.
    pub side_move: i16,
    /// Up movement.
    pub up_move: i16,
    /// Impulse.
    pub impulse: u8,
    /// Light level.
    pub light_level: u8,
}

/// Rerelease user command (28 bytes on the wire).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseUserCommand {
    /// Frame milliseconds.
    pub milliseconds: u8,
    /// Button bits.
    pub buttons: u8,
    /// Absolute aim angles.
    pub angles: Vec3,
    /// Forward movement.
    pub forward_move: f32,
    /// Side movement.
    pub side_move: f32,
    /// Server frame.
    pub server_frame: i32,
}

/// User command across editions.
#[derive(Debug, Clone, PartialEq)]
pub enum UserCommand {
    /// Classic command.
    Classic(ClassicUserCommand),
    /// Rerelease command.
    Rerelease(RereleaseUserCommand),
}

/// Encode a user command into a caller-provided buffer.
pub fn write_native_user_command(bytes: &mut [u8], command: &UserCommand) -> Result<(), StageError> {
    match command {
        UserCommand::Rerelease(command) => {
            if bytes.len() < 28 {
                return Err(StageError::BufferTooSmall);
            }
            for value in [
                command.angles.x,
                command.angles.y,
                command.angles.z,
                command.forward_move,
                command.side_move,
            ] {
                if !value.is_finite() {
                    return Err(StageError::FloatRange);
                }
            }
            bytes[0] = command.milliseconds;
            bytes[1] = command.buttons;
            bytes[2] = 0;
            bytes[3] = 0;
            for (index, component) in [command.angles.x, command.angles.y, command.angles.z]
                .iter()
                .enumerate()
            {
                bytes[4 + index * 4..8 + index * 4].copy_from_slice(&component.to_le_bytes());
            }
            bytes[16..20].copy_from_slice(&command.forward_move.to_le_bytes());
            bytes[20..24].copy_from_slice(&command.side_move.to_le_bytes());
            bytes[24..28].copy_from_slice(&command.server_frame.to_le_bytes());
            Ok(())
        }
        UserCommand::Classic(command) => {
            if bytes.len() < 16 {
                return Err(StageError::BufferTooSmall);
            }
            bytes[0] = command.milliseconds;
            bytes[1] = command.buttons;
            for (axis, value) in command.angle_shorts.iter().enumerate() {
                bytes[2 + axis * 2..4 + axis * 2].copy_from_slice(&value.to_le_bytes());
            }
            bytes[8..10].copy_from_slice(&command.forward_move.to_le_bytes());
            bytes[10..12].copy_from_slice(&command.side_move.to_le_bytes());
            bytes[12..14].copy_from_slice(&command.up_move.to_le_bytes());
            bytes[14] = command.impulse;
            bytes[15] = command.light_level;
            Ok(())
        }
    }
}

/// Client input application lowered into one user command.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientApplication {
    /// Frame elapsed time in seconds.
    pub elapsed_secs: f64,
    /// Absolute aim angles.
    pub aim: Vec3,
    /// Attack input.
    pub attack: f64,
    /// Jump input.
    pub jump: f64,
    /// Impulse input.
    pub impulse: f64,
    /// Forward input.
    pub forward_move: f64,
    /// Side input.
    pub side_move: f64,
    /// Up input.
    pub up_move: f64,
}

fn to_short(value: f64) -> Result<i16, StageError> {
    if !value.is_finite() || value.trunc().abs() > 9_007_199_254_740_992.0 {
        return Err(StageError::AimRange);
    }
    Ok(value.trunc().rem_euclid(65536.0) as u16 as i16)
}

fn to_move(value: f64) -> Result<i16, StageError> {
    if !value.is_finite() || value < -32768.0 || value > 32767.0 {
        return Err(StageError::MoveRange);
    }
    Ok(value.trunc() as i16)
}

/// Lower one client application into wire command bytes.
pub fn native_mod_user_command(
    application: &ModClientApplication,
    rerelease: bool,
    frame: u32,
) -> Result<Vec<u8>, StageError> {
    for (name, value) in [
        ("attack", application.attack),
        ("jump", application.jump),
        ("impulse", application.impulse),
        ("forward-move", application.forward_move),
        ("side-move", application.side_move),
        ("up-move", application.up_move),
    ] {
        if !value.is_finite() {
            return Err(StageError::MissingInput(name));
        }
    }
    let milliseconds = (application.elapsed_secs * 1000.0).round();
    if !(0.0..=255.0).contains(&milliseconds) {
        return Err(StageError::IntervalRange);
    }
    let milliseconds = milliseconds as u8;
    let forward = application.forward_move * 200.0;
    let side = application.side_move * 200.0;
    let up = application.up_move * 200.0;
    let command = if rerelease {
        UserCommand::Rerelease(RereleaseUserCommand {
            milliseconds,
            buttons: (u8::from(application.attack != 0.0))
                | (u8::from(application.jump != 0.0) * 8)
                | (u8::from(up < 0.0) * 16),
            angles: application.aim,
            forward_move: forward as f32,
            side_move: side as f32,
            server_frame: frame as i32,
        })
    } else {
        let aim = application.aim;
        UserCommand::Classic(ClassicUserCommand {
            milliseconds,
            buttons: u8::from(application.attack != 0.0),
            angle_shorts: [
                to_short(f64::from(aim.x) * 65536.0 / 360.0)?,
                to_short(f64::from(aim.y) * 65536.0 / 360.0)?,
                to_short(f64::from(aim.z) * 65536.0 / 360.0)?,
            ],
            forward_move: to_move(forward)?,
            side_move: to_move(side)?,
            up_move: to_move(if application.jump != 0.0 { up.max(200.0) } else { up })?,
            impulse: application.impulse.trunc().rem_euclid(256.0) as u8,
            light_level: 0,
        })
    };
    let mut bytes = vec![0u8; if rerelease { 28 } else { 16 }];
    write_native_user_command(&mut bytes, &command)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn application() -> ModClientApplication {
        ModClientApplication {
            elapsed_secs: 0.05,
            aim: Vec3 {
                x: 90.0,
                y: 0.0,
                z: 0.0,
            },
            attack: 1.0,
            jump: 0.0,
            impulse: 0.0,
            forward_move: 1.0,
            side_move: 0.0,
            up_move: 0.0,
        }
    }

    #[test]
    fn nested_runs_unbind_and_rebind_outer_scopes() {
        let mut host = SyntheticStageHost::new(GuestAddress::new(3, 0x10000));
        let mut stages = NativeModClientStages::new();
        let outer = StageCall {
            id: "outer".to_string(),
            skips: vec![SkipRegion {
                entry: 0x100,
                join: 0x180,
            }],
        };
        let inner = StageCall {
            id: "inner".to_string(),
            skips: vec![SkipRegion {
                entry: 0x200,
                join: 0x280,
            }],
        };
        let setups = Rc::new(RefCell::new(0u32));
        let teardowns = Rc::new(RefCell::new(0u32));
        let setup_count = setups.clone();
        let teardown_count = teardowns.clone();
        let extra: Rc<dyn Fn() -> Box<dyn FnMut()>> = Rc::new(move || {
            *setup_count.borrow_mut() += 1;
            let teardown_count = teardown_count.clone();
            Box::new(move || {
                *teardown_count.borrow_mut() += 1;
            })
        });
        let inner_call = inner.clone();
        stages.run(
            &mut host,
            &outer,
            |stages, host| {
                assert_eq!(stages.active_call(), Some("outer"));
                assert_eq!(host.active(), vec![(0x10100, 0x10180)]);
                stages.run(
                    &mut *host,
                    &inner_call,
                    |stages, host| {
                        assert_eq!(stages.active_call(), Some("inner"));
                        assert_eq!(host.active(), vec![(0x10200, 0x10280)]);
                    },
                    None,
                );
                assert_eq!(stages.active_call(), Some("outer"));
                assert_eq!(host.active(), vec![(0x10100, 0x10180)]);
            },
            Some(extra),
        );
        assert_eq!(stages.active_call(), None);
        assert!(host.active().is_empty());
        assert_eq!(*setups.borrow(), 2);
        assert_eq!(*teardowns.borrow(), 2);
    }

    #[test]
    fn classic_command_bytes_match_the_wire_layout() {
        let bytes = native_mod_user_command(&application(), false, 0).unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(bytes[0], 50);
        assert_eq!(bytes[1], 1);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), 16384);
        assert_eq!(i16::from_le_bytes([bytes[8], bytes[9]]), 200);
        assert_eq!(i16::from_le_bytes([bytes[12], bytes[13]]), 0);
        assert_eq!(bytes[14], 0);
        assert_eq!(bytes[15], 0);

        let mut jumped = application();
        jumped.jump = 1.0;
        jumped.up_move = -1.0;
        let bytes = native_mod_user_command(&jumped, false, 0).unwrap();
        assert_eq!(i16::from_le_bytes([bytes[12], bytes[13]]), 200);
    }

    #[test]
    fn rerelease_command_validates_ranges_and_reports_errors() {
        let bytes = native_mod_user_command(&application(), true, 41).unwrap();
        assert_eq!(bytes.len(), 28);
        assert_eq!(bytes[0], 50);
        assert_eq!(bytes[1], 1);
        assert_eq!(f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]), 90.0);
        assert_eq!(i32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]), 41);

        let mut slow = application();
        slow.elapsed_secs = 2.0;
        assert_eq!(native_mod_user_command(&slow, false, 0), Err(StageError::IntervalRange));
        let mut wild = application();
        wild.forward_move = f64::INFINITY;
        assert!(matches!(
            native_mod_user_command(&wild, false, 0),
            Err(StageError::MissingInput(_))
        ));
        let mut small = [0u8; 4];
        assert_eq!(
            write_native_user_command(
                &mut small,
                &UserCommand::Classic(ClassicUserCommand {
                    milliseconds: 0,
                    buttons: 0,
                    angle_shorts: [0, 0, 0],
                    forward_move: 0,
                    side_move: 0,
                    up_move: 0,
                    impulse: 0,
                    light_level: 0,
                }),
            ),
            Err(StageError::BufferTooSmall)
        );
    }
}
