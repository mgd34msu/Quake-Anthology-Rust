//! Client sound traps for cgame and ui modules.
//!
//! Provenance: `src/compat/qvm/client-audio-syscalls.ts` (Q3
//! `cl_cgame.c`/`cl_ui.c` sound traps, adapted from quake-3-ts). Donor
//! promises become direct returns; [`ClientAudioHost`] is a local mirror of
//! the `Q3ClientSound` bank surface plus its print callback.

use qa_core::math::Vec3;

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{
    CG_S_ADDLOOPINGSOUND, CG_S_ADDREALLOOPINGSOUND, CG_S_CLEARLOOPINGSOUNDS, CG_S_REGISTERSOUND, CG_S_RESPATIALIZE,
    CG_S_STARTBACKGROUNDTRACK, CG_S_STARTLOCALSOUND, CG_S_STARTSOUND, CG_S_STOPBACKGROUNDTRACK, CG_S_STOPLOOPINGSOUND,
    CG_S_UPDATEENTITYPOSITION, UI_S_REGISTERSOUND, UI_S_STARTBACKGROUNDTRACK, UI_S_STARTLOCALSOUND,
    UI_S_STOPBACKGROUNDTRACK,
};
use crate::error::GuestError;

/// Validated sound-bank handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundHandle(pub i32);

/// Host client-sound surface used by the traps.
pub trait ClientAudioHost {
    /// Resolve a guest sound index, printing the invalid-handle marker when absent.
    fn resolve_sound(&mut self, index: i32) -> Option<SoundHandle>;
    /// Register a sound, returning its bank index.
    fn register_sound(&mut self, name: Option<&str>, compressed: bool) -> i32;
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: SoundHandle, channel: i32);
    /// Start (or, with empty names, stop) the background track.
    fn start_background_track(&mut self, intro: &str, loop_track: &str);
    /// Start a positional sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: SoundHandle);
    /// Clear looping sounds.
    fn clear_looping_sounds(&mut self, clear: bool);
    /// Add a looping sound.
    fn add_loop_sound(&mut self, entity: i32, origin: Vec3, velocity: Vec3, sound: SoundHandle, real: bool);
    /// Update a sound position.
    fn update_sound_position(&mut self, entity: i32, origin: Vec3);
    /// Stop a looping sound.
    fn stop_looping_sound(&mut self, entity: i32);
    /// Set the listener.
    fn set_listener(&mut self, entity: i32, origin: Vec3, axis: [Vec3; 3]);
    /// Print text (invalid-handle marker and diagnostics).
    fn print(&mut self, text: &str);
}

fn pcm(host: &mut dyn ClientAudioHost, index: i32) -> Option<SoundHandle> {
    match host.resolve_sound(index) {
        Some(sound) => Some(sound),
        None => {
            host.print("^3");
            None
        }
    }
}

/// Dispatch a client-audio trap for `role`. Returns `Ok(None)` when unhandled.
pub fn client_audio_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    role: QvmRole,
    host: &mut dyn ClientAudioHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != role || role == QvmRole::Qagame {
        return Ok(None);
    }
    let ui = role == QvmRole::Ui;
    if call.code == if ui { UI_S_REGISTERSOUND } else { CG_S_REGISTERSOUND } {
        let name_word = call.int(1)?;
        let compressed = call.abi_profile.is_modern() && call.int(2)? != 0;
        let name = if name_word == 0 {
            None
        } else {
            Some(memory.read_string(name_word)?)
        };
        return Ok(Some(host.register_sound(name.as_deref(), compressed)));
    }
    if call.code == if ui { UI_S_STARTLOCALSOUND } else { CG_S_STARTLOCALSOUND } {
        if let Some(sound) = pcm(host, call.int(1)?) {
            host.start_local_sound(sound, call.int(2)?);
        }
        return Ok(Some(0));
    }
    if call.code
        == if ui {
            UI_S_STARTBACKGROUNDTRACK
        } else {
            CG_S_STARTBACKGROUNDTRACK
        }
    {
        let intro_word = call.int(1)?;
        let loop_word = call.int(2)?;
        let intro = if intro_word == 0 {
            String::new()
        } else {
            memory.read_string(intro_word)?
        };
        let loop_track = if loop_word == 0 {
            String::new()
        } else {
            memory.read_string(loop_word)?
        };
        host.start_background_track(&intro, &loop_track);
        return Ok(Some(0));
    }
    if call.code
        == if ui {
            UI_S_STOPBACKGROUNDTRACK
        } else {
            CG_S_STOPBACKGROUNDTRACK
        }
    {
        host.start_background_track("", "");
        return Ok(Some(0));
    }
    if call.role != QvmRole::Cgame {
        return Ok(None);
    }
    match call.code {
        CG_S_STARTSOUND => {
            let origin_word = call.int(1)?;
            let entity = call.int(2)?;
            let channel = call.int(3)?;
            if origin_word == 0 && !(0..=1024).contains(&entity) {
                return Err(GuestError::runtime(format!("S_StartSound: bad entitynum {entity}")));
            }
            if let Some(sound) = pcm(host, call.int(4)?) {
                let origin = if origin_word == 0 {
                    None
                } else {
                    Some(memory.read_vec3_ptr(origin_word)?)
                };
                host.start_sound(origin, entity, channel, sound);
            }
            Ok(Some(0))
        }
        CG_S_CLEARLOOPINGSOUNDS => {
            host.clear_looping_sounds(!call.abi_profile.is_modern() || call.int(1)? != 0);
            Ok(Some(0))
        }
        CG_S_ADDLOOPINGSOUND | CG_S_ADDREALLOOPINGSOUND => {
            if let Some(sound) = pcm(host, call.int(4)?) {
                let origin = memory.read_vec3_ptr(call.int(2)?)?;
                let velocity = memory.read_vec3_ptr(call.int(3)?)?;
                host.add_loop_sound(
                    call.int(1)?,
                    origin,
                    velocity,
                    sound,
                    call.code == CG_S_ADDREALLOOPINGSOUND,
                );
            }
            Ok(Some(0))
        }
        CG_S_UPDATEENTITYPOSITION => {
            host.update_sound_position(call.int(1)?, memory.read_vec3_ptr(call.int(2)?)?);
            Ok(Some(0))
        }
        CG_S_STOPLOOPINGSOUND => {
            host.stop_looping_sound(call.int(1)?);
            Ok(Some(0))
        }
        CG_S_RESPATIALIZE => {
            let entity = call.int(1)?;
            let origin = memory.read_vec3_ptr(call.int(2)?)?;
            let axes_word = call.int(3)?;
            let _ = call.int(4)?;
            let base = memory
                .pointer(axes_word)
                .ok_or_else(|| GuestError::invalid("QVM respatialize axes require a nonnull pointer"))?;
            let axis = [
                memory.read_vec3(base)?,
                memory.read_vec3(base + 12)?,
                memory.read_vec3(base + 24)?,
            ];
            host.set_listener(entity, origin, axis);
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeAudio {
        log: Vec<String>,
    }

    impl ClientAudioHost for FakeAudio {
        fn resolve_sound(&mut self, index: i32) -> Option<SoundHandle> {
            (0..64).contains(&index).then_some(SoundHandle(index))
        }
        fn register_sound(&mut self, name: Option<&str>, compressed: bool) -> i32 {
            self.log.push(format!("register {name:?} {compressed}"));
            5
        }
        fn start_local_sound(&mut self, sound: SoundHandle, channel: i32) {
            self.log.push(format!("local {} {channel}", sound.0));
        }
        fn start_background_track(&mut self, intro: &str, loop_track: &str) {
            self.log.push(format!("bg {intro} {loop_track}"));
        }
        fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: SoundHandle) {
            self.log
                .push(format!("start {} {entity} {channel} {}", sound.0, origin.is_some()));
        }
        fn clear_looping_sounds(&mut self, clear: bool) {
            self.log.push(format!("clear {clear}"));
        }
        fn add_loop_sound(&mut self, entity: i32, _origin: Vec3, _velocity: Vec3, sound: SoundHandle, real: bool) {
            self.log.push(format!("loop {entity} {} {real}", sound.0));
        }
        fn update_sound_position(&mut self, entity: i32, _origin: Vec3) {
            self.log.push(format!("pos {entity}"));
        }
        fn stop_looping_sound(&mut self, entity: i32) {
            self.log.push(format!("stop {entity}"));
        }
        fn set_listener(&mut self, entity: i32, _origin: Vec3, _axis: [Vec3; 3]) {
            self.log.push(format!("listener {entity}"));
        }
        fn print(&mut self, text: &str) {
            self.log.push(format!("print {text}"));
        }
    }

    fn cg(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, code, args, AbiProfile::Modern)
    }

    #[test]
    fn shared_traps_both_roles() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "sound/jump.wav", 15).unwrap();
        memory.write_string(512, "intro", 6).unwrap();
        memory.write_string(640, "loop", 5).unwrap();
        let mut host = FakeAudio { log: Vec::new() };
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_REGISTERSOUND, &[256, 1]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(5)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_STARTLOCALSOUND, &[5, 1]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_STARTLOCALSOUND, &[99, 1]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_STARTBACKGROUNDTRACK, &[512, 640]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_STOPBACKGROUNDTRACK, &[]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        let ui = HostCall::engine(QvmRole::Ui, UI_S_REGISTERSOUND, &[0, 0], AbiProfile::Modern);
        assert_eq!(
            client_audio_syscall(&ui, &mut memory, QvmRole::Ui, &mut host).unwrap(),
            Some(5)
        );
        assert_eq!(
            host.log,
            vec![
                "register Some(\"sound/jump.wav\") true".to_string(),
                "local 5 1".to_string(),
                "print ^3".to_string(),
                "bg intro loop".to_string(),
                "bg  ".to_string(),
                "register None false".to_string(),
            ]
        );
    }

    #[test]
    fn cgame_positional_traps() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_vec3(256, &Vec3 { x: 1.0, y: 2.0, z: 3.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(640, &Vec3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(652, &Vec3 { x: 0.0, y: 1.0, z: 0.0 }).unwrap();
        memory.write_vec3(664, &Vec3 { x: 0.0, y: 0.0, z: 1.0 }).unwrap();
        let mut host = FakeAudio { log: Vec::new() };
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_STARTSOUND, &[256, 3, 1, 5]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_STARTSOUND, &[0, 3, 1, 5]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert!(client_audio_syscall(
            &cg(CG_S_STARTSOUND, &[0, 5000, 1, 5]),
            &mut memory,
            QvmRole::Cgame,
            &mut host
        )
        .is_err());
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_CLEARLOOPINGSOUNDS, &[1]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_ADDLOOPINGSOUND, &[2, 256, 512, 5]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_ADDREALLOOPINGSOUND, &[2, 256, 512, 5]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_UPDATEENTITYPOSITION, &[2, 256]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(&cg(CG_S_STOPLOOPINGSOUND, &[2]), &mut memory, QvmRole::Cgame, &mut host).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_audio_syscall(
                &cg(CG_S_RESPATIALIZE, &[1, 256, 640, 0]),
                &mut memory,
                QvmRole::Cgame,
                &mut host
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            host.log,
            vec![
                "start 5 3 1 true".to_string(),
                "start 5 3 1 false".to_string(),
                "clear true".to_string(),
                "loop 2 5 false".to_string(),
                "loop 2 5 true".to_string(),
                "pos 2".to_string(),
                "stop 2".to_string(),
                "listener 1".to_string(),
            ]
        );
    }

    #[test]
    fn ui_rejects_cgame_only_and_routes_role() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut host = FakeAudio { log: Vec::new() };
        let ui = HostCall::engine(QvmRole::Ui, CG_S_STARTSOUND, &[0, 0, 0, 0], AbiProfile::Modern);
        assert_eq!(
            client_audio_syscall(&ui, &mut memory, QvmRole::Ui, &mut host).unwrap(),
            None
        );
        assert_eq!(
            client_audio_syscall(&cg(999, &[]), &mut memory, QvmRole::Cgame, &mut host).unwrap(),
            None
        );
        let wrong = HostCall::engine(QvmRole::Cgame, CG_S_STARTLOCALSOUND, &[5, 1], AbiProfile::Modern);
        assert_eq!(
            client_audio_syscall(&wrong, &mut memory, QvmRole::Ui, &mut host).unwrap(),
            None
        );
    }
}
