//! Shared QVM scalar traps: input, time, memory, and lighting.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/qvm-scalars.ts`
//! (`QvmApplicationScalars`, `qvmClientInputSyscall`). Display traps
//! dispatch first through [`qvm_display_syscall`](super::qvm_display);
//! the remaining ui/cgame scalar traps read through injected input,
//! clock, catcher, and lighting seams. `UPDATESCREEN` reports
//! [`QvmScalarResult::UpdateScreen`] so the QVM owner runs the screen
//! update and re-asserts currency itself.

use qa_client::input::keycodes::keynum_to_string;
use qa_client::input::{physical_mouse_button, quake_mouse_button};
use qa_client::input::{InputAction, InputBinding, InputBindingTarget, KeyCode, PhysicalInput};
use qa_client::materials::q3_lighting::LightingSample;
use qa_core::math::Vec3;
use qa_guest::qvm::abi::{QvmCgameImport, QvmUiImport};
use qa_guest::qvm::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use qa_guest::GuestError;

use super::qvm_display::{qvm_display_syscall, QvmDisplayOptions, QvmDisplayRenderer, QvmFontServices};

/// Client-state record for `GETCLIENTSTATE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmClientStateRecord {
    /// Connection phase.
    pub phase: i32,
    /// Connect packets sent.
    pub connect_packet_count: i32,
    /// Client slot.
    pub client_number: i32,
    /// Server name.
    pub server_name: String,
    /// Connection message.
    pub message: String,
}

/// Broken-down real time for `REAL_TIME`: seconds, minutes, hours,
/// day-of-month, month, year minus 1900, weekday, day-of-year, and
/// daylight-saving flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmRealTime {
    /// Unix seconds.
    pub unix_seconds: i32,
    /// Calendar fields.
    pub calendar: [i32; 9],
}

/// Seat input backing the key traps.
pub trait QvmClientInput {
    /// Whether an input is held.
    fn is_down(&mut self, input: &PhysicalInput) -> bool;
    /// Binding for an input, if any.
    fn binding(&mut self, input: &PhysicalInput) -> Option<InputBindingTarget>;
    /// All bindings.
    fn bindings(&self) -> Vec<InputBinding>;
    /// Bind an input to command text.
    fn bind(&mut self, input: PhysicalInput, command: &str);
    /// Clear transient states.
    fn clear_states(&mut self);
}

/// Key-catcher word backing the catcher traps.
pub trait QvmKeyCatcher {
    /// Read the catcher word.
    fn get(&mut self) -> i32;
    /// Write the catcher word.
    fn set(&mut self, value: i32);
}

/// Console overstrike mode backing the overstrike traps.
pub trait QvmOverstrike {
    /// Read overstrike mode.
    fn get(&mut self) -> bool;
    /// Write overstrike mode.
    fn set(&mut self, value: bool);
}

/// Scalar dispatch outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmScalarResult {
    /// Trap handled with a guest result.
    Handled(i32),
    /// The owner must run the screen update, re-assert currency, and
    /// return 0.
    UpdateScreen,
    /// Another owner handles the trap.
    Unhandled,
}

/// Scalar trap stage for the QVM host chain.
pub trait QvmScalarStage {
    /// Dispatch a scalar trap.
    fn dispatch_scalar(&mut self, call: &HostCall, memory: &mut SyscallMemory) -> Result<QvmScalarResult, GuestError>;
}

/// Read the system clipboard as source bytes, or `None` when SDL has no
/// text (donor `readSdlClipboard`).
#[must_use]
pub fn sdl_clipboard() -> Option<Vec<u8>> {
    qa_platform::sdl::read_sdl_clipboard().unwrap_or(None)
}

/// Console command for a binding target.
#[must_use]
pub fn binding_command(target: &InputBindingTarget) -> String {
    match target {
        InputBindingTarget::Command(text) => text.clone(),
        InputBindingTarget::Action(action) => action_command(*action).to_string(),
    }
}

/// Console command for a named action (donor `actionCommands`).
#[must_use]
pub const fn action_command(action: InputAction) -> &'static str {
    match action {
        InputAction::Attack => "+attack",
        InputAction::Jump | InputAction::MoveUp => "+moveup",
        InputAction::Forward => "+forward",
        InputAction::Back => "+back",
        InputAction::MoveLeft => "+moveleft",
        InputAction::MoveRight => "+moveright",
        InputAction::MoveDown | InputAction::Crouch => "+movedown",
        InputAction::Use => "+use",
        InputAction::Walk => "+speed",
        InputAction::Scores => "+scores",
        InputAction::NextWeapon => "weapnext",
        InputAction::PreviousWeapon => "weapprev",
        InputAction::Menu => "togglemenu",
    }
}

/// Physical input for a key number.
#[must_use]
pub fn physical_input(key: i32) -> PhysicalInput {
    if (KeyCode::Mouse1 as i32..=KeyCode::Mouse5 as i32).contains(&key) {
        PhysicalInput::MouseButton(physical_mouse_button(key - KeyCode::Mouse1 as i32 + 1))
    } else {
        PhysicalInput::Key(key)
    }
}

/// Key number for a bound input, or -1 when unrepresentable.
#[must_use]
pub fn binding_key(binding: &InputBinding) -> i32 {
    match &binding.input {
        PhysicalInput::Key(code) => *code,
        PhysicalInput::MouseButton(button) => KeyCode::Mouse1 as i32 + quake_mouse_button(*button) - 1,
        _ => -1,
    }
}

/// Dispatch the key traps shared by ui and cgame roles.
pub fn qvm_client_input_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    input: &mut dyn QvmClientInput,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || (call.role != QvmRole::Ui && call.role != QvmRole::Cgame) {
        return Ok(None);
    }
    let ui = call.role == QvmRole::Ui;
    let is_down = if ui {
        QvmUiImport::UiKeyIsdown as i32
    } else {
        QvmCgameImport::CgKeyIsdown as i32
    };
    if call.code == is_down {
        return Ok(Some(i32::from(input.is_down(&physical_input(call.int(1)?)))));
    }
    if ui {
        match call.code {
            code if code == QvmUiImport::UiKeyGetbindingbuf as i32 => {
                let binding = input.binding(&physical_input(call.int(1)?));
                memory.write_string(
                    call.int(2)?,
                    &binding.as_ref().map_or_else(String::new, binding_command),
                    call.int(3)? as usize,
                )?;
                return Ok(Some(0));
            }
            code if code == QvmUiImport::UiKeySetbinding as i32 => {
                let target = memory.read_string(call.int(2)?)?;
                input.bind(physical_input(call.int(1)?), &target);
                return Ok(Some(0));
            }
            code if code == QvmUiImport::UiKeyClearstates as i32 => {
                input.clear_states();
                return Ok(Some(0));
            }
            _ => {}
        }
    } else if call.code == QvmCgameImport::CgKeyGetkey as i32 {
        let name = memory.read_string(call.int(1)?)?;
        let found = input
            .bindings()
            .iter()
            .find(|binding| binding_command(&binding.target).eq_ignore_ascii_case(&name))
            .map(binding_key)
            .unwrap_or(-1);
        return Ok(Some(found));
    }
    Ok(None)
}

/// Options for [`QvmApplicationScalars`].
pub struct QvmApplicationScalarOptions {
    /// Renderer record for the display traps.
    pub renderer: QvmDisplayRenderer,
    /// Font services for the display traps.
    pub fonts: Box<dyn QvmFontServices>,
    /// Current viewport in pixels.
    pub viewport: Box<dyn FnMut() -> (i32, i32)>,
    /// Seat input.
    pub input: Box<dyn QvmClientInput>,
    /// Overstrike mode.
    pub overstrike: Box<dyn QvmOverstrike>,
    /// Key catcher.
    pub key_catcher: Box<dyn QvmKeyCatcher>,
    /// Client-state record.
    pub client_state: Box<dyn FnMut() -> QvmClientStateRecord>,
    /// Lighting sample for a point.
    pub light_for_point: Box<dyn FnMut(Vec3) -> LightingSample>,
    /// Real time.
    pub real_time: Box<dyn FnMut() -> QvmRealTime>,
    /// Free memory in bytes (donor `freemem`).
    pub free_memory: Box<dyn FnMut() -> u64>,
    /// Clipboard bytes, if any.
    pub clipboard: Box<dyn FnMut() -> Option<Vec<u8>>>,
    /// Panic when the seat moved past this client.
    pub assert_current: Box<dyn FnMut()>,
}

/// Shared scalar traps for ui and cgame roles.
pub struct QvmApplicationScalars {
    options: QvmApplicationScalarOptions,
}

impl QvmApplicationScalars {
    /// Build scalar traps over injected seams.
    pub fn new(options: QvmApplicationScalarOptions) -> Self {
        Self { options }
    }

    fn display(&mut self, call: &HostCall, memory: &mut SyscallMemory) -> Result<Option<i32>, GuestError> {
        let options = &mut self.options;
        qvm_display_syscall(
            call,
            memory,
            QvmDisplayOptions {
                renderer: &options.renderer,
                fonts: &mut *options.fonts,
                viewport: &mut *options.viewport,
                assert_current: &mut *options.assert_current,
            },
        )
    }
}

impl QvmScalarStage for QvmApplicationScalars {
    fn dispatch_scalar(&mut self, call: &HostCall, memory: &mut SyscallMemory) -> Result<QvmScalarResult, GuestError> {
        (self.options.assert_current)();
        if call.kind != CallKind::Engine || (call.role != QvmRole::Ui && call.role != QvmRole::Cgame) {
            return Ok(QvmScalarResult::Unhandled);
        }
        if let Some(handled) = self.display(call, memory)? {
            return Ok(QvmScalarResult::Handled(handled));
        }
        let ui = call.role == QvmRole::Ui;
        if ui && call.code == QvmUiImport::UiGetclientstate as i32 {
            let pointer = call.int(1)?;
            let base = memory
                .pointer(pointer)
                .ok_or_else(|| GuestError::invalid("GETCLIENTSTATE requires a record pointer"))?;
            memory.span(pointer, 3084, 0)?;
            memory.fill(base, 3084, 0)?;
            let state = (self.options.client_state)();
            memory.write_i32(base, state.phase)?;
            memory.write_i32(base + 4, state.connect_packet_count)?;
            memory.write_i32(base + 8, state.client_number)?;
            memory.write_string(pointer + 12, &state.server_name, 1024)?;
            memory.write_string(pointer + 2060, &state.message, 1024)?;
            return Ok(QvmScalarResult::Handled(0));
        }
        let memory_remaining = if ui {
            QvmUiImport::UiMemoryRemaining as i32
        } else {
            QvmCgameImport::CgMemoryRemaining as i32
        };
        if call.code == memory_remaining {
            let free = (self.options.free_memory)();
            return Ok(QvmScalarResult::Handled(free.min(0x7fff_ffff) as i32));
        }
        let update_screen = if ui {
            QvmUiImport::UiUpdatescreen as i32
        } else {
            QvmCgameImport::CgUpdatescreen as i32
        };
        if call.code == update_screen {
            return Ok(QvmScalarResult::UpdateScreen);
        }
        if let Some(handled) = qvm_client_input_syscall(call, memory, &mut *self.options.input)? {
            return Ok(QvmScalarResult::Handled(handled));
        }
        let get_catcher = if ui {
            QvmUiImport::UiKeyGetcatcher as i32
        } else {
            QvmCgameImport::CgKeyGetcatcher as i32
        };
        if call.code == get_catcher {
            let catcher = self.options.key_catcher.get();
            return Ok(QvmScalarResult::Handled(catcher));
        }
        let set_catcher = if ui {
            QvmUiImport::UiKeySetcatcher as i32
        } else {
            QvmCgameImport::CgKeySetcatcher as i32
        };
        if call.code == set_catcher {
            self.options.key_catcher.set(call.int(1)?);
            return Ok(QvmScalarResult::Handled(0));
        }
        if ui {
            if call.code == QvmUiImport::UiKeyKeynumtostringbuf as i32 {
                memory.write_string(call.int(2)?, &keynum_to_string(call.int(1)?), call.int(3)? as usize)?;
                return Ok(QvmScalarResult::Handled(0));
            }
            if call.code == QvmUiImport::UiKeyGetoverstrikemode as i32 {
                let overstrike = self.options.overstrike.get();
                return Ok(QvmScalarResult::Handled(i32::from(overstrike)));
            }
            if call.code == QvmUiImport::UiKeySetoverstrikemode as i32 {
                self.options.overstrike.set(call.int(1)? != 0);
                return Ok(QvmScalarResult::Handled(0));
            }
            if call.code == QvmUiImport::UiGetclipboarddata as i32 {
                let bytes = (self.options.clipboard)().unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes).into_owned();
                memory.write_string(call.int(1)?, &text, call.int(2)? as usize)?;
                return Ok(QvmScalarResult::Handled(0));
            }
        }
        let real_time = if ui {
            QvmUiImport::UiRealTime as i32
        } else {
            QvmCgameImport::CgRealTime as i32
        };
        if call.code == real_time {
            let time = (self.options.real_time)();
            let pointer = call.int(1)?;
            if pointer != 0 {
                let base = memory
                    .pointer(pointer)
                    .ok_or_else(|| GuestError::invalid("REAL_TIME requires a calendar pointer"))?;
                memory.span(pointer, 36, 0)?;
                for (index, value) in time.calendar.iter().enumerate() {
                    memory.write_i32(base + index * 4, *value)?;
                }
            }
            return Ok(QvmScalarResult::Handled(time.unix_seconds));
        }
        if !ui && call.code == QvmCgameImport::CgRLightforpoint as i32 {
            let point = memory.read_vec3_ptr(call.int(1)?)?;
            let light = (self.options.light_for_point)(point);
            for (word, value) in [
                (call.int(2)?, light.ambient_light),
                (call.int(3)?, light.directed_light),
                (call.int(4)?, light.light_dir),
            ] {
                let base = memory
                    .pointer(word)
                    .ok_or_else(|| GuestError::invalid("R_LIGHTFORPOINT requires a vector pointer"))?;
                memory.span(word, 12, 0)?;
                memory.write_vec3(base, &value)?;
            }
            return Ok(QvmScalarResult::Handled(1));
        }
        Ok(QvmScalarResult::Unhandled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::input::InputBinding;
    use qa_core::math::vec3;
    use qa_guest::qvm::client_state::AbiProfile;
    use std::collections::HashMap;

    use super::super::qvm_display::{QvmDisplayGlConfig, QvmDisplayRenderer};

    struct StubInput {
        down: Vec<PhysicalInput>,
        bindings: HashMap<PhysicalInput, InputBindingTarget>,
        cleared: usize,
    }

    impl StubInput {
        fn new() -> Self {
            Self {
                down: Vec::new(),
                bindings: HashMap::new(),
                cleared: 0,
            }
        }
    }

    impl QvmClientInput for StubInput {
        fn is_down(&mut self, input: &PhysicalInput) -> bool {
            self.down.contains(input)
        }

        fn binding(&mut self, input: &PhysicalInput) -> Option<InputBindingTarget> {
            self.bindings.get(input).cloned()
        }

        fn bindings(&self) -> Vec<InputBinding> {
            self.bindings
                .iter()
                .map(|(input, target)| InputBinding {
                    input: input.clone(),
                    target: target.clone(),
                })
                .collect()
        }

        fn bind(&mut self, input: PhysicalInput, command: &str) {
            self.bindings
                .insert(input, InputBindingTarget::Command(command.to_string()));
        }

        fn clear_states(&mut self) {
            self.cleared += 1;
        }
    }

    struct StubFonts;

    impl QvmFontServices for StubFonts {
        fn register_font(&mut self, _name: Option<&str>, _point_size: i32, _glyph_table: &mut [u8]) -> bool {
            false
        }

        fn register_shader_no_mip(&mut self, _path: &str) -> i32 {
            0
        }
    }

    struct Stubs {
        catcher: i32,
        overstrike: bool,
    }

    fn scalars() -> (QvmApplicationScalars, Stubs) {
        let stubs = Stubs {
            catcher: 3,
            overstrike: true,
        };
        let catcher = std::rc::Rc::new(std::cell::RefCell::new(stubs.catcher));
        let overstrike = std::rc::Rc::new(std::cell::RefCell::new(stubs.overstrike));
        struct Catcher(std::rc::Rc<std::cell::RefCell<i32>>);
        impl QvmKeyCatcher for Catcher {
            fn get(&mut self) -> i32 {
                *self.0.borrow()
            }
            fn set(&mut self, value: i32) {
                *self.0.borrow_mut() = value;
            }
        }
        struct Strike(std::rc::Rc<std::cell::RefCell<bool>>);
        impl QvmOverstrike for Strike {
            fn get(&mut self) -> bool {
                *self.0.borrow()
            }
            fn set(&mut self, value: bool) {
                *self.0.borrow_mut() = value;
            }
        }
        let scalars = QvmApplicationScalars::new(QvmApplicationScalarOptions {
            renderer: QvmDisplayRenderer {
                stencil_bits: 0,
                driver: None,
                gl_config: Some(QvmDisplayGlConfig {
                    max_texture_size: 256,
                    texture_units: 1,
                    color_bits: 24,
                    depth_bits: 24,
                    stereo_enabled: false,
                }),
            },
            fonts: Box::new(StubFonts),
            viewport: Box::new(|| (640, 480)),
            input: Box::new(StubInput::new()),
            overstrike: Box::new(Strike(overstrike)),
            key_catcher: Box::new(Catcher(catcher)),
            client_state: Box::new(|| QvmClientStateRecord {
                phase: 2,
                connect_packet_count: 7,
                client_number: 1,
                server_name: "local".to_string(),
                message: "hello".to_string(),
            }),
            light_for_point: Box::new(|_| LightingSample {
                ambient_light: vec3(1.0, 0.0, 0.0),
                directed_light: vec3(0.0, 1.0, 0.0),
                light_dir: vec3(0.0, 0.0, 1.0),
            }),
            real_time: Box::new(|| QvmRealTime {
                unix_seconds: 42,
                calendar: [1, 2, 3, 4, 5, 6, 7, 8, 9],
            }),
            free_memory: Box::new(|| u64::MAX),
            clipboard: Box::new(|| Some(b"clip".to_vec())),
            assert_current: Box::new(|| {}),
        });
        (scalars, stubs)
    }

    fn memory() -> SyscallMemory {
        SyscallMemory::new(65536).expect("memory")
    }

    fn dispatch(
        scalars: &mut QvmApplicationScalars,
        memory: &mut SyscallMemory,
        call: &HostCall,
    ) -> Result<QvmScalarResult, GuestError> {
        scalars.dispatch_scalar(call, memory)
    }

    #[test]
    fn action_commands_cover_named_actions() {
        assert_eq!(action_command(InputAction::Attack), "+attack");
        assert_eq!(action_command(InputAction::Jump), "+moveup");
        assert_eq!(action_command(InputAction::Crouch), "+movedown");
        assert_eq!(action_command(InputAction::PreviousWeapon), "weapprev");
        assert_eq!(
            binding_command(&InputBindingTarget::Command("say hi".to_string())),
            "say hi".to_string()
        );
        assert_eq!(
            binding_command(&InputBindingTarget::Action(InputAction::Menu)),
            "togglemenu".to_string()
        );
    }

    #[test]
    fn physical_input_maps_mouse_range() {
        assert_eq!(physical_input(KeyCode::Mouse1 as i32), PhysicalInput::MouseButton(1));
        assert_eq!(
            physical_input(KeyCode::Mouse1 as i32 + 1),
            PhysicalInput::MouseButton(physical_mouse_button(2))
        );
        assert_eq!(physical_input(65), PhysicalInput::Key(65));
    }

    #[test]
    fn key_traps_read_and_write_bindings() {
        let (mut scalars, _) = scalars();
        let mut memory = memory();
        let is_down = HostCall::engine(QvmRole::Ui, QvmUiImport::UiKeyIsdown as i32, &[65], AbiProfile::Modern);
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &is_down).expect("isdown"),
            QvmScalarResult::Handled(0)
        );
        memory.write_string(512, "+attack", 32).expect("command");
        let set = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeySetbinding as i32,
            &[65, 512],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &set).expect("set"),
            QvmScalarResult::Handled(0)
        );
        let get = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeyGetbindingbuf as i32,
            &[65, 1024, 32],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &get).expect("get"),
            QvmScalarResult::Handled(0)
        );
        assert_eq!(memory.read_string(1024).expect("binding"), "+attack");
        memory.write_string(1536, "+ATTACK", 32).expect("name");
        let key = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgKeyGetkey as i32,
            &[1536],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &key).expect("key"),
            QvmScalarResult::Handled(65)
        );
        let clear = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeyClearstates as i32,
            &[],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &clear).expect("clear"),
            QvmScalarResult::Handled(0)
        );
    }

    #[test]
    fn client_state_writes_its_record() {
        let (mut scalars, _) = scalars();
        let mut memory = memory();
        let call = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiGetclientstate as i32,
            &[4096],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &call).expect("state"),
            QvmScalarResult::Handled(0)
        );
        let base = memory.pointer(4096).expect("record");
        assert_eq!(memory.read_i32(base).expect("phase"), 2);
        assert_eq!(memory.read_i32(base + 4).expect("packets"), 7);
        assert_eq!(memory.read_i32(base + 8).expect("client"), 1);
        assert_eq!(memory.read_string(4096 + 12).expect("server"), "local");
        assert_eq!(memory.read_string(4096 + 2060).expect("message"), "hello");
    }

    #[test]
    fn memory_and_screen_and_catcher_traps() {
        let (mut scalars, _) = scalars();
        let mut memory = memory();
        let free = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgMemoryRemaining as i32,
            &[],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &free).expect("free"),
            QvmScalarResult::Handled(0x7fff_ffff)
        );
        let screen = HostCall::engine(QvmRole::Ui, QvmUiImport::UiUpdatescreen as i32, &[], AbiProfile::Modern);
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &screen).expect("screen"),
            QvmScalarResult::UpdateScreen
        );
        let get = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeyGetcatcher as i32,
            &[],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &get).expect("get"),
            QvmScalarResult::Handled(3)
        );
        let set = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgKeySetcatcher as i32,
            &[9],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &set).expect("set"),
            QvmScalarResult::Handled(0)
        );
        let get = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgKeyGetcatcher as i32,
            &[],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &get).expect("get"),
            QvmScalarResult::Handled(9)
        );
    }

    #[test]
    fn ui_key_helpers_and_clipboard() {
        let (mut scalars, _) = scalars();
        let mut memory = memory();
        let name = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeyKeynumtostringbuf as i32,
            &[KeyCode::Mouse1 as i32, 512, 32],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &name).expect("name"),
            QvmScalarResult::Handled(0)
        );
        assert_eq!(
            memory.read_string(512).expect("name"),
            keynum_to_string(KeyCode::Mouse1 as i32)
        );
        let get = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeyGetoverstrikemode as i32,
            &[],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &get).expect("get"),
            QvmScalarResult::Handled(1)
        );
        let set = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiKeySetoverstrikemode as i32,
            &[0],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &set).expect("set"),
            QvmScalarResult::Handled(0)
        );
        let clip = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiGetclipboarddata as i32,
            &[1024, 32],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &clip).expect("clip"),
            QvmScalarResult::Handled(0)
        );
        assert_eq!(memory.read_string(1024).expect("clip"), "clip");
    }

    #[test]
    fn real_time_and_light_point() {
        let (mut scalars, _) = scalars();
        let mut memory = memory();
        let time = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgRealTime as i32,
            &[512],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &time).expect("time"),
            QvmScalarResult::Handled(42)
        );
        let base = memory.pointer(512).expect("calendar");
        for (index, value) in [1, 2, 3, 4, 5, 6, 7, 8, 9].iter().enumerate() {
            assert_eq!(memory.read_i32(base + index * 4).expect("field"), *value);
        }
        let skipped = HostCall::engine(QvmRole::Ui, QvmUiImport::UiRealTime as i32, &[0], AbiProfile::Modern);
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &skipped).expect("time"),
            QvmScalarResult::Handled(42)
        );
        memory
            .write_vec3(memory.pointer(1024).expect("point"), &vec3(1.0, 2.0, 3.0))
            .expect("point");
        let light = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgRLightforpoint as i32,
            &[1024, 2048, 2060, 2072],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &light).expect("light"),
            QvmScalarResult::Handled(1)
        );
        let ambient = memory.pointer(2048).expect("ambient");
        assert_eq!(memory.read_vec3(ambient).expect("ambient"), vec3(1.0, 0.0, 0.0));
        let directed = memory.pointer(2060).expect("directed");
        assert_eq!(memory.read_vec3(directed).expect("directed"), vec3(0.0, 1.0, 0.0));
    }

    #[test]
    fn display_and_foreign_calls_route() {
        let (mut scalars, _) = scalars();
        let mut memory = memory();
        let gl = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiGetglconfig as i32,
            &[8192],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &gl).expect("gl"),
            QvmScalarResult::Handled(0)
        );
        let other = HostCall::engine(QvmRole::Ui, 999, &[], AbiProfile::Modern);
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &other).expect("other"),
            QvmScalarResult::Unhandled
        );
        let game = HostCall::engine(
            QvmRole::Qagame,
            QvmUiImport::UiKeyIsdown as i32,
            &[65],
            AbiProfile::Modern,
        );
        assert_eq!(
            dispatch(&mut scalars, &mut memory, &game).expect("game"),
            QvmScalarResult::Unhandled
        );
    }
}
