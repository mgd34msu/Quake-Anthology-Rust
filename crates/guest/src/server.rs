//! VM game runner: [`ServerLogic`](qa_world::server::ServerLogic) driven by a
//! guest game module.
//!
//! A loaded game exports `int32 vmMain(int32 command, uintptr args)`, where
//! `args` points at three pointer-sized words in the shared scratch block.
//! Commands and word encodings:
//!
//! * [`GAME_CLIENT_THINK`]: words are `[slot, client_block, 0]`. The client
//!   block is 48 bytes: `+0 i32` family ([`ClientFamily`] declaration order),
//!   `+4 i32` buttons, `+8 i32` impulse, `+16/+24/+32/+40 f64`
//!   forward/side/right/up moves.
//! * [`GAME_ENTITY_FRAME`]: words are `[frame_block, 0, 0]`. The frame block
//!   is 24 bytes: `+0 i32` frame, `+4 i32` phase ([`FramePhase`] declaration
//!   order), `+8 f64` time seconds, `+16 f64` elapsed seconds.
//! * [`GAME_TOUCH`]: words are `[trigger, other, 0]`, each actor encoded as
//!   `slot | generation << 32`.
//! * [`GAME_MOVER_THINK`]: words are `[actor, phase, arrived]`, with the
//!   phase in [`MoverPhase`] declaration order and arrival as `0`/`1`.
//!
//! With no game loaded the hooks are no-ops. The first guest failure latches
//! into [`GuestServerLogic::failure`] and later hooks are skipped, so a
//! broken game can never silently desynchronize the simulation. Game imports
//! outside the C library and OS services resolve to explicit unsupported
//! traps; engine hostcalls surface through the same latch.

use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_world::client::{ClientCommand, ClientFamily};
use qa_world::movers::{MoverPhase, MoverTable};
use qa_world::server::ServerLogic;
use qa_world::session::Simulation;
use qa_world::triggers::TouchContact;

use crate::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use crate::abi::GuestCpu;
use crate::core::callbacks::HookState;
use crate::core::contracts::{
    GuestAddress, GuestAllocationOptions, GuestArchitecture, GuestCallContext, GuestCallResult,
    GuestCallSignature, GuestCallValue, GuestCallbackReference, GuestExport, GuestExportTarget,
    GuestIntegerWidth, GuestPermissions, GuestRegister, GuestStorage, GuestSymbolName,
    GuestValueLayout, ModuleIdentity, NativeAbi, NativeCallAbi,
};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use crate::elf::parse::inspect_elf;
use crate::error::GuestError;
use crate::pe::format::parse_pe;
use crate::pe::loader::{MapPeImageOptions, map_pe_image};
use crate::runtime::common::memory::write_unsigned;
use crate::runtime::system_v::contracts::{SystemVInitializeOptions, SystemVRuntimeOptions};
use crate::runtime::system_v::runtime::SystemVGuestRuntime;
use crate::runtime::windows::contracts::{WindowsInitializeOptions, WindowsRuntimeOptions};
use crate::runtime::windows::runtime::WindowsGuestRuntime;
use crate::x64::cpu::X64Cpu;
use crate::x86::cpu::I386Cpu;

/// `vmMain` command: run one queued client command.
pub const GAME_CLIENT_THINK: i32 = 0;
/// `vmMain` command: run per-frame game logic.
pub const GAME_ENTITY_FRAME: i32 = 1;
/// `vmMain` command: react to a trigger touch.
pub const GAME_TOUCH: i32 = 2;
/// `vmMain` command: react to a mover think.
pub const GAME_MOVER_THINK: i32 = 3;

/// Default instruction budget per `vmMain` call and lifecycle step.
pub const DEFAULT_INSTRUCTION_BUDGET: u64 = 1_000_000;
/// Scratch block size in bytes.
const SCRATCH_BYTES: usize = 4096;
/// Scratch offset of the client block.
const CLIENT_BLOCK: i64 = 64;
/// Scratch offset of the frame block.
const FRAME_BLOCK: i64 = 128;
/// Game stack size in bytes.
const STACK_BYTES: usize = 0x100000;
/// ELF load bias: relocatable game images map here, clear of the stack,
/// scratch, and allocation areas.
const ELF_LOAD_BIAS: u64 = 0x1000_0000;

/// Game CPU: width follows the loaded image.
enum ServerCpu {
    /// 64-bit game.
    X64(X64Cpu),
    /// 32-bit game.
    X86(I386Cpu),
}

impl ServerCpu {
    fn as_cpu(&mut self) -> &mut dyn GuestCpu {
        match self {
            ServerCpu::X64(cpu) => cpu,
            ServerCpu::X86(cpu) => cpu,
        }
    }

    const fn width(&self) -> usize {
        match self {
            ServerCpu::X64(_) => 8,
            ServerCpu::X86(_) => 4,
        }
    }
}

/// Game runtime: format follows the loaded image.
enum ServerRuntime {
    /// ELF game.
    SystemV(SystemVGuestRuntime),
    /// PE game.
    Windows(WindowsGuestRuntime),
}

/// Game logic executed inside the guest VM.
pub struct GuestServerLogic {
    module: ModuleIdentity,
    hooks: Rc<HookState>,
    cpu: ServerCpu,
    runtime: ServerRuntime,
    return_address: GuestAddress,
    scratch: GuestAddress,
    signature: GuestCallSignature,
    root: GuestCallContext,
    vmmain: Option<GuestAddress>,
    instruction_budget: u64,
    failure: Option<String>,
}

impl GuestServerLogic {
    /// Build an empty runner: a 64-bit guest with no game loaded. Hooks are
    /// no-ops until [`GuestServerLogic::load_game`] succeeds.
    pub fn new(module: ModuleIdentity) -> Result<Self, GuestError> {
        let hooks = Rc::new(HookState::new());
        let abi = NativeCallAbi::SystemVX86_64;
        let mut cpu = build_cpu(&module, 8)?;
        let runtime = build_runtime(&hooks, &mut cpu, true)?;
        let mut logic = Self {
            root: GuestCallContext {
                module: module.clone(),
                callback: GuestCallbackReference::NativeGuest {
                    module: module.clone(),
                    address: GuestAddress::new(0, 0),
                    abi,
                },
                parent: None,
                itself: None,
                other: None,
            },
            module,
            hooks,
            cpu,
            runtime,
            return_address: GuestAddress::new(0, 0),
            scratch: GuestAddress::new(0, 0),
            signature: GuestCallSignature {
                abi,
                parameters: vec![
                    GuestValueLayout::Scalar(GuestStorage::Int32),
                    GuestValueLayout::Scalar(GuestStorage::Pointer),
                ],
                result: Some(GuestValueLayout::Scalar(GuestStorage::Int32)),
                variadic: false,
            },
            vmmain: None,
            instruction_budget: DEFAULT_INSTRUCTION_BUDGET,
            failure: None,
        };
        logic.finish_reset()?;
        Ok(logic)
    }

    /// Whether a game module is loaded.
    #[must_use]
    pub const fn loaded(&self) -> bool {
        self.vmmain.is_some()
    }

    /// Loaded `vmMain` address, if any.
    #[must_use]
    pub const fn vmmain(&self) -> Option<GuestAddress> {
        self.vmmain
    }

    /// First latched guest failure, if any.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Instruction budget per call.
    #[must_use]
    pub const fn instruction_budget(&self) -> u64 {
        self.instruction_budget
    }

    /// Assign the instruction budget per call.
    pub fn set_instruction_budget(&mut self, budget: u64) {
        self.instruction_budget = budget.max(1);
    }

    /// Load an ELF or PE game module, resetting all guest state. The image
    /// must export `vmMain`.
    pub fn load_game(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        let abi = game_abi(bytes)?;
        let width = abi.pointer_bytes();
        let system_v = matches!(abi, NativeAbi::LinuxI386 | NativeAbi::LinuxX86_64);
        self.hooks = Rc::new(HookState::new());
        self.cpu = build_cpu(&self.module, width)?;
        self.runtime = build_runtime(&self.hooks, &mut self.cpu, system_v)?;
        self.signature.abi = match abi {
            NativeAbi::LinuxI386 => NativeCallAbi::SystemVI386,
            NativeAbi::LinuxX86_64 => NativeCallAbi::SystemVX86_64,
            NativeAbi::WindowsI386 => NativeCallAbi::Cdecl,
            NativeAbi::WindowsX86_64 => NativeCallAbi::MicrosoftX64,
        };
        self.finish_reset()?;
        if system_v {
            self.load_elf(bytes)?;
        } else {
            self.load_pe(bytes)?;
        }
        let Some(vmmain) = self.vmmain else {
            self.failure = Some("game module does not export vmMain".to_string());
            return Err(GuestError::invalid("game module does not export vmMain"));
        };
        self.root = self.root_context(vmmain);
        self.failure = None;
        Ok(())
    }

    fn load_elf(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        let image = {
            let Self { cpu, runtime, module, .. } = &mut *self;
            let ServerRuntime::SystemV(runtime) = runtime else {
                unreachable!("system-v guest rebuilds a system-v runtime");
            };
            let (_, memory) = cpu.as_cpu().parts();
            runtime.load(memory, bytes, module.clone(), ELF_LOAD_BIAS)?
        };
        self.vmmain = find_vmmain(&image.image.exports);
        let context = self.root_context(image.image.base);
        let budget = self.instruction_budget;
        let Self { cpu, runtime, hooks, return_address, module, .. } = &mut *self;
        let ServerRuntime::SystemV(runtime) = runtime else {
            unreachable!("system-v guest rebuilds a system-v runtime");
        };
        let mut runner =
            GuestCallRunner::new(cpu.as_cpu(), Rc::clone(hooks), *return_address, None)?;
        runtime.initialize(
            &mut runner,
            module,
            &SystemVInitializeOptions { context, instruction_budget: budget },
        )
    }

    fn load_pe(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        let module = self.module.clone();
        let image = {
            let (_, memory) = self.cpu.as_cpu().parts();
            map_pe_image(MapPeImageOptions {
                bytes,
                memory,
                module: Some(module),
                base: None,
                maximum_image_bytes: None,
            })?
        };
        {
            let Self { cpu, runtime, .. } = &mut *self;
            let ServerRuntime::Windows(runtime) = runtime else {
                unreachable!("windows guest rebuilds a windows runtime");
            };
            let (_, memory) = cpu.as_cpu().parts();
            runtime.prepare_image(memory, &image)?;
        }
        self.vmmain = find_vmmain(&image.image.exports);
        let context = self.root_context(image.image.base);
        let budget = self.instruction_budget;
        let Self { cpu, runtime, hooks, return_address, .. } = &mut *self;
        let ServerRuntime::Windows(runtime) = runtime else {
            unreachable!("windows guest rebuilds a windows runtime");
        };
        let mut runner =
            GuestCallRunner::new(cpu.as_cpu(), Rc::clone(hooks), *return_address, None)?;
        runtime.initialize(
            &mut runner,
            &image,
            &WindowsInitializeOptions { context, instruction_budget: budget },
        )
    }

    /// Invoke `vmMain(command, args)` with three pointer-sized words.
    pub fn call(&mut self, command: i32, a0: u64, a1: u64, a2: u64) -> Result<i32, GuestError> {
        if let Some(failure) = &self.failure {
            return Err(GuestError::callback(format!("guest game failed: {failure}")));
        }
        let Some(vmmain) = self.vmmain else {
            return Err(GuestError::invalid("no game module loaded"));
        };
        let Self {
            module,
            hooks,
            cpu,
            return_address,
            scratch,
            signature,
            root,
            instruction_budget,
            ..
        } = &mut *self;
        let width = cpu.width();
        {
            let (_, memory) = cpu.as_cpu().parts();
            for (index, word) in [a0, a1, a2].iter().enumerate() {
                write_unsigned(
                    memory,
                    memory.offset(*scratch, (index * width) as i64)?,
                    width,
                    i128::from(*word),
                )?;
            }
        }
        let request = GuestCallRequest {
            target: vmmain,
            signature: signature.clone(),
            arguments: vec![
                GuestCallValue::Int32(command),
                GuestCallValue::Pointer(Some(*scratch)),
            ],
            context: GuestCallContext {
                module: module.clone(),
                callback: GuestCallbackReference::NativeGuest {
                    module: module.clone(),
                    address: vmmain,
                    abi: signature.abi,
                },
                parent: Some(Box::new(root.clone())),
                itself: root.itself.clone(),
                other: root.other.clone(),
            },
            instruction_budget: *instruction_budget,
        };
        let mut runner = GuestCallRunner::new(cpu.as_cpu(), Rc::clone(hooks), *return_address, None)?;
        match runner.invoke(&request) {
            Ok(GuestCallResult::Value(GuestCallValue::Int32(value))) => Ok(value),
            Ok(result) => Err(GuestError::invalid(format!(
                "game vmMain returned non-integer result: {result:?}"
            ))),
            Err(failure) => Err(map_failure(failure)),
        }
    }

    fn root_context(&self, address: GuestAddress) -> GuestCallContext {
        GuestCallContext {
            module: self.module.clone(),
            callback: GuestCallbackReference::NativeGuest {
                module: self.module.clone(),
                address,
                abi: self.signature.abi,
            },
            parent: None,
            itself: None,
            other: None,
        }
    }

    /// Map the stack, return trap, and scratch block, then attach the
    /// runtime to the fresh CPU.
    fn finish_reset(&mut self) -> Result<(), GuestError> {
        let Self { cpu, runtime, hooks, return_address, scratch, .. } = &mut *self;
        let width = cpu.width();
        let stack_top = {
            let (_, memory) = cpu.as_cpu().parts();
            let stack = memory.allocate(&GuestAllocationOptions {
                byte_length: STACK_BYTES,
                alignment: 4096,
                permissions: GuestPermissions::ReadWrite,
                label: "game stack".to_string(),
            })?;
            *return_address = memory.allocate(&GuestAllocationOptions {
                byte_length: 16,
                alignment: 16,
                permissions: GuestPermissions::ReadExecute,
                label: "game return trap".to_string(),
            })?;
            *scratch = memory.allocate(&GuestAllocationOptions {
                byte_length: SCRATCH_BYTES,
                alignment: 16,
                permissions: GuestPermissions::ReadWrite,
                label: "game scratch block".to_string(),
            })?;
            stack.offset + STACK_BYTES as u64
        };
        {
            let (state, _) = cpu.as_cpu().parts();
            state.registers.write(
                GuestRegister::Rsp,
                if width == 8 { GuestIntegerWidth::B64 } else { GuestIntegerWidth::B32 },
                stack_top,
                false,
            )?;
        }
        let mut runner = GuestCallRunner::new(cpu.as_cpu(), Rc::clone(hooks), *return_address, None)?;
        match runtime {
            ServerRuntime::SystemV(runtime) => runtime.attach_runner(&mut runner)?,
            ServerRuntime::Windows(runtime) => runtime.attach_runner(&mut runner)?,
        }
        Ok(())
    }

    fn run_hook(&mut self, command: i32, a0: u64, a1: u64, a2: u64) {
        if self.vmmain.is_none() || self.failure.is_some() {
            return;
        }
        if let Err(error) = self.call(command, a0, a1, a2) {
            self.failure = Some(error.to_string());
        }
    }

    fn write_client_block(&mut self, command: &ClientCommand) -> Result<GuestAddress, GuestError> {
        let scratch = self.scratch;
        let (_, memory) = self.cpu.as_cpu().parts();
        let block = memory.offset(scratch, CLIENT_BLOCK)?;
        memory.write_i32(memory.offset(block, 0)?, family_id(command.family))?;
        memory.write_i32(memory.offset(block, 4)?, command.buttons)?;
        memory.write_i32(memory.offset(block, 8)?, command.impulse)?;
        memory.write_f64(memory.offset(block, 16)?, command.forward_move)?;
        memory.write_f64(memory.offset(block, 24)?, command.side_move)?;
        memory.write_f64(memory.offset(block, 32)?, command.right_move)?;
        memory.write_f64(memory.offset(block, 40)?, command.up_move)?;
        Ok(block)
    }

    fn write_frame_block(&mut self, frame: FrameContext) -> Result<GuestAddress, GuestError> {
        let scratch = self.scratch;
        let (_, memory) = self.cpu.as_cpu().parts();
        let block = memory.offset(scratch, FRAME_BLOCK)?;
        memory.write_i32(memory.offset(block, 0)?, frame.frame)?;
        memory.write_i32(memory.offset(block, 4)?, phase_id(frame.phase))?;
        memory.write_f64(memory.offset(block, 8)?, time_seconds(frame.time))?;
        memory.write_f64(memory.offset(block, 16)?, time_seconds(frame.elapsed))?;
        Ok(block)
    }
}

impl ServerLogic for GuestServerLogic {
    fn client_think(&mut self, _simulation: &mut Simulation, slot: u32, command: &ClientCommand) {
        let block = match self.write_client_block(command) {
            Ok(block) => block.offset,
            Err(error) => {
                self.failure = Some(error.to_string());
                return;
            }
        };
        self.run_hook(GAME_CLIENT_THINK, u64::from(slot), block, 0);
    }

    fn entity_frame(&mut self, _simulation: &mut Simulation, frame: FrameContext) {
        let block = match self.write_frame_block(frame) {
            Ok(block) => block.offset,
            Err(error) => {
                self.failure = Some(error.to_string());
                return;
            }
        };
        self.run_hook(GAME_ENTITY_FRAME, block, 0, 0);
    }

    fn touch(&mut self, _simulation: &mut Simulation, contact: &TouchContact) {
        self.run_hook(GAME_TOUCH, actor_id(&contact.trigger), actor_id(&contact.other), 0);
    }

    fn mover_think(
        &mut self,
        _simulation: &mut Simulation,
        _movers: &mut MoverTable,
        actor: &ActorId,
        phase: MoverPhase,
        arrived: bool,
    ) {
        self.run_hook(
            GAME_MOVER_THINK,
            actor_id(actor),
            u64::from(mover_id(phase)),
            u64::from(arrived),
        );
    }
}

fn game_abi(bytes: &[u8]) -> Result<NativeAbi, GuestError> {
    if bytes.len() >= 4 && bytes[0] == 0x7f && bytes[1] == b'E' && bytes[2] == b'L' && bytes[3] == b'F'
    {
        return Ok(inspect_elf(bytes)?.abi);
    }
    if bytes.len() >= 2 && bytes[0] == b'M' && bytes[1] == b'Z' {
        return Ok(parse_pe(bytes)?.abi);
    }
    Err(GuestError::invalid("unknown game image format"))
}

fn find_vmmain(exports: &[GuestExport]) -> Option<GuestAddress> {
    exports.iter().find_map(|export| match (&export.symbol, &export.target) {
        (
            GuestSymbolName::Name { name, .. },
            GuestExportTarget::Address(address),
        ) if name == "vmMain" => Some(*address),
        _ => None,
    })
}

fn build_cpu(module: &ModuleIdentity, width: usize) -> Result<ServerCpu, GuestError> {
    let memory = SparseGuestMemory::new(module.clone(), width, 0x10000)?;
    let architecture = if width == 8 {
        GuestArchitecture::X86_64
    } else {
        GuestArchitecture::I386
    };
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture,
        instruction_pointer: 0,
        stack_pointer: 0,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })?;
    if width == 8 {
        Ok(ServerCpu::X64(X64Cpu::new(state, memory)?))
    } else {
        Ok(ServerCpu::X86(I386Cpu::new(state, memory)?))
    }
}

fn build_runtime(
    hooks: &Rc<HookState>,
    cpu: &mut ServerCpu,
    system_v: bool,
) -> Result<ServerRuntime, GuestError> {
    let (_, memory) = cpu.as_cpu().parts();
    if system_v {
        Ok(ServerRuntime::SystemV(SystemVGuestRuntime::new(
            Rc::clone(hooks),
            memory,
            SystemVRuntimeOptions::default(),
        )?))
    } else {
        Ok(ServerRuntime::Windows(WindowsGuestRuntime::new(
            Rc::clone(hooks),
            memory,
            WindowsRuntimeOptions::default(),
        )?))
    }
}

fn actor_id(actor: &ActorId) -> u64 {
    u64::from(actor.slot()) | (u64::from(actor.generation()) << 32)
}

fn mover_id(phase: MoverPhase) -> u32 {
    match phase {
        MoverPhase::AtPos1 => 0,
        MoverPhase::AtPos2 => 1,
        MoverPhase::ToPos2 => 2,
        MoverPhase::ToPos1 => 3,
    }
}

fn family_id(family: ClientFamily) -> i32 {
    match family {
        ClientFamily::Q1Netquake => 0,
        ClientFamily::Q1Quakeworld => 1,
        ClientFamily::Q2Classic => 2,
        ClientFamily::Q2Rerelease => 3,
        ClientFamily::Q3 => 4,
    }
}

fn phase_id(phase: FramePhase) -> i32 {
    match phase {
        FramePhase::FrameEntry => 0,
        FramePhase::ClientCommand => 1,
        FramePhase::EntityPrethink => 2,
        FramePhase::EntityPhysics => 3,
        FramePhase::EntityThink => 4,
        FramePhase::ClientEndFrame => 5,
        FramePhase::FrameExit => 6,
    }
}

fn time_seconds(time: SourceTime) -> f64 {
    match time {
        SourceTime::Seconds(value) => f64::from(value),
        SourceTime::Milliseconds(value) => f64::from(value) / 1000.0,
    }
}

fn map_failure(failure: GuestCallFailure) -> GuestError {
    match failure {
        GuestCallFailure::Guest(error) => error,
        GuestCallFailure::Stopped { stop, .. } => {
            GuestError::callback(format!("game vmMain call stopped: {stop:?}"))
        }
    }
}
