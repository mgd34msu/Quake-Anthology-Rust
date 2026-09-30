//! Cinematic traps for cgame and ui modules.
//!
//! Provenance: `src/compat/qvm/client-cinematic-syscalls.ts` (cinematic
//! traps from id Software `cl_cgame.c`/`cl_ui.c`). Donor promises become
//! direct returns; the `Draw2D` handle the donor passes to `drawGuest` stays
//! host-owned inside [`CinematicHost::draw`].

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{CG_CIN_PLAYCINEMATIC, UI_CIN_PLAYCINEMATIC};
use crate::error::GuestError;

/// Integer 2D rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CineRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// Host cinematic surface used by the traps.
pub trait CinematicHost {
    /// Play a cinematic, returning its handle.
    fn play(&mut self, path: &str, rect: CineRect, bits: i32) -> i32;
    /// Run a cinematic handle.
    fn run(&mut self, handle: i32) -> i32;
    /// Stop a cinematic handle.
    fn stop(&mut self, handle: i32) -> i32;
    /// Draw a cinematic handle.
    fn draw(&mut self, handle: i32);
    /// Set a cinematic handle's extents.
    fn set_extents(&mut self, handle: i32, rect: CineRect);
    /// Developer-channel print (UI play only).
    fn developer_print(&mut self, text: &str);
}

/// Dispatch a cinematic trap. Returns `Ok(None)` when unhandled.
pub fn client_cinematic_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn CinematicHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role == QvmRole::Qagame {
        return Ok(None);
    }
    let play = if call.role == QvmRole::Ui {
        UI_CIN_PLAYCINEMATIC
    } else {
        CG_CIN_PLAYCINEMATIC
    };
    if call.code < play || call.code > play + 4 {
        return Ok(None);
    }
    if call.code == play {
        let path = memory.read_string(call.int(1)?)?;
        let rect = CineRect {
            x: call.int(2)?,
            y: call.int(3)?,
            width: call.int(4)?,
            height: call.int(5)?,
        };
        let bits = call.int(6)?;
        if call.role == QvmRole::Ui {
            services.developer_print("UI_CIN_PlayCinematic\n");
        }
        return Ok(Some(services.play(&path, rect, bits)));
    }
    let handle = call.int(1)?;
    if call.code == play + 1 {
        return Ok(Some(services.stop(handle)));
    }
    if call.code == play + 2 {
        return Ok(Some(services.run(handle)));
    }
    if call.code == play + 3 {
        services.draw(handle);
    } else {
        services.set_extents(
            handle,
            CineRect {
                x: call.int(2)?,
                y: call.int(3)?,
                width: call.int(4)?,
                height: call.int(5)?,
            },
        );
    }
    Ok(Some(0))
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeCine {
        log: Vec<String>,
    }

    impl CinematicHost for FakeCine {
        fn play(&mut self, path: &str, rect: CineRect, bits: i32) -> i32 {
            self.log
                .push(format!("play {path} {}x{} bits={bits}", rect.width, rect.height));
            11
        }
        fn run(&mut self, handle: i32) -> i32 {
            self.log.push(format!("run {handle}"));
            1
        }
        fn stop(&mut self, handle: i32) -> i32 {
            self.log.push(format!("stop {handle}"));
            2
        }
        fn draw(&mut self, handle: i32) {
            self.log.push(format!("draw {handle}"));
        }
        fn set_extents(&mut self, handle: i32, rect: CineRect) {
            self.log.push(format!("extents {handle} {},{}", rect.x, rect.y));
        }
        fn developer_print(&mut self, text: &str) {
            self.log.push(format!("dev {text:?}"));
        }
    }

    fn cg(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, code, args, AbiProfile::Modern)
    }

    fn ui(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Ui, code, args, AbiProfile::Modern)
    }

    #[test]
    fn cgame_sequence() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "intro.roq", 10).unwrap();
        let mut cine = FakeCine { log: Vec::new() };
        assert_eq!(
            client_cinematic_syscall(&cg(74, &[512, 1, 2, 320, 240, 4]), &mut memory, &mut cine).unwrap(),
            Some(11)
        );
        assert_eq!(
            client_cinematic_syscall(&cg(75, &[11]), &mut memory, &mut cine).unwrap(),
            Some(2)
        );
        assert_eq!(
            client_cinematic_syscall(&cg(76, &[11]), &mut memory, &mut cine).unwrap(),
            Some(1)
        );
        assert_eq!(
            client_cinematic_syscall(&cg(77, &[11]), &mut memory, &mut cine).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_cinematic_syscall(&cg(78, &[11, 5, 6, 100, 80]), &mut memory, &mut cine).unwrap(),
            Some(0)
        );
        assert_eq!(
            cine.log,
            vec![
                "play intro.roq 320x240 bits=4".to_string(),
                "stop 11".to_string(),
                "run 11".to_string(),
                "draw 11".to_string(),
                "extents 11 5,6".to_string(),
            ]
        );
    }

    #[test]
    fn ui_play_prints_developer_line() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "idlog.roq", 10).unwrap();
        let mut cine = FakeCine { log: Vec::new() };
        assert_eq!(
            client_cinematic_syscall(&ui(75, &[512, 0, 0, 640, 480, 0]), &mut memory, &mut cine).unwrap(),
            Some(11)
        );
        assert_eq!(
            client_cinematic_syscall(&ui(79, &[11, 1, 1, 2, 2]), &mut memory, &mut cine).unwrap(),
            Some(0)
        );
        assert_eq!(cine.log[0], "dev \"UI_CIN_PlayCinematic\\n\"".to_string());
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut cine = FakeCine { log: Vec::new() };
        let game = HostCall::engine(QvmRole::Qagame, 74, &[0, 0, 0, 0, 0, 0], AbiProfile::Modern);
        assert_eq!(client_cinematic_syscall(&game, &mut memory, &mut cine).unwrap(), None);
        assert_eq!(
            client_cinematic_syscall(&cg(73, &[]), &mut memory, &mut cine).unwrap(),
            None
        );
        assert_eq!(
            client_cinematic_syscall(&cg(79, &[11]), &mut memory, &mut cine).unwrap(),
            None
        );
        assert_eq!(
            client_cinematic_syscall(&ui(74, &[0, 0, 0, 0, 0, 0]), &mut memory, &mut cine).unwrap(),
            None
        );
    }
}
