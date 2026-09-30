//! QVM mod actor frame: original entity-loop dispatch with owned filtering.
//!
//! Ports `src/compat/qvm/mod-actor-frame.ts` and absorbs
//! `src/contracts/qvm-mod-actor-frame.ts`. Source-call and record types come
//! from [`super::mod_actors`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use super::game_data::{
    QvmBranchBinding, QvmFunctionCall, QvmImage, QvmModule, QvmOpcode, QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use super::mod_actors::{QvmModActorRecord, QvmModReturn, QvmModScalar, QvmModSourceCall, QvmModValue, QvmTimeUnits};
use crate::error::GuestError;

/// Actor-frame clock declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmActorClock {
    /// Clock global address.
    pub address: usize,
    /// Argument-store instruction.
    pub store: usize,
    /// Time argument index.
    pub argument: usize,
}

/// Owned-entity loop filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmActorFilter {
    /// Predicate instruction.
    pub instruction: usize,
    /// Predicate local-setup instruction.
    pub local_instruction: usize,
}

/// Entity-loop end declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmActorEnd {
    /// End predicate instruction.
    pub instruction: usize,
    /// Taken direction meaning completion.
    pub completed_taken: bool,
}

/// Original actor-frame declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorFrameDeclaration {
    /// Frame call.
    pub call: QvmModSourceCall,
    /// Frame clock.
    pub clock: QvmActorClock,
    /// Owned-entity filters.
    pub owned: Vec<QvmActorFilter>,
    /// Loop end.
    pub end: QvmActorEnd,
}

/// Fresh-initialization constant store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmActorStore {
    /// Destination address.
    pub address: usize,
    /// Value.
    pub value: i32,
}

/// Collect fresh-initialization constant stores.
pub fn qvm_actor_bootstrap(
    stores: &[usize],
    image: &QvmImage,
    records: &[QvmModActorRecord],
) -> Result<Vec<QvmActorStore>, GuestError> {
    let mut destinations = HashSet::new();
    stores
        .iter()
        .map(|pc| {
            let store = image.instruction(*pc);
            let address = pc.checked_sub(2).and_then(|pc| image.instruction(pc));
            let value = pc.checked_sub(1).and_then(|pc| image.instruction(pc));
            let (Some(store), Some(address), Some(value)) = (store, address, value) else {
                return Err(GuestError::invalid(
                    "QVM source bootstrap is not an original constant store",
                ));
            };
            if store.opcode != QvmOpcode::OpStore4
                || address.opcode != QvmOpcode::OpConst
                || value.opcode != QvmOpcode::OpConst
            {
                return Err(GuestError::invalid(
                    "QVM source bootstrap is not an original constant store",
                ));
            }
            let offset = address.operand;
            let end = image.initialized_data.len() + image.bss_length;
            let overlaps = |record: &QvmModActorRecord| {
                offset < record.address as i32 + record.stride as i32 * record.capacity as i32
                    && offset + 4 > record.address as i32
            };
            if offset < 0
                || offset % 4 != 0
                || offset + 4 > end as i32
                || offset < image.initialized_data.len() as i32 && offset + 4 > image.data_length as i32
                || !destinations.insert(offset)
                || records.iter().any(overlaps)
            {
                return Err(GuestError::invalid(
                    "QVM source bootstrap overlaps projected or nonwritable storage",
                ));
            }
            Ok(QvmActorStore {
                address: offset as usize,
                value: value.operand,
            })
        })
        .collect()
}

/// Validate an actor-frame declaration against its image.
pub fn validate_qvm_mod_actor_frame(
    definition: &QvmModActorFrameDeclaration,
    image: &QvmImage,
    record: &QvmModActorRecord,
    inuse: usize,
) -> Result<(), GuestError> {
    let entry = definition.call.entry;
    let Some(enter) = image.instruction(entry) else {
        return Err(GuestError::invalid(
            "QVM actor frame requires its original void caller and entity loops",
        ));
    };
    if enter.opcode != QvmOpcode::OpEnter
        || definition.call.returns != QvmModReturn::Void
        || definition.owned.is_empty()
    {
        return Err(GuestError::invalid(
            "QVM actor frame requires its original void caller and entity loops",
        ));
    }
    let frame_size = enter.operand;
    let mut end = entry + 1;
    while image
        .instruction(end)
        .is_some_and(|instruction| instruction.opcode != QvmOpcode::OpEnter)
    {
        end += 1;
    }
    let mut decisions = HashSet::new();
    let mut branch = |pc: usize| -> Result<(), GuestError> {
        let instruction = image.instruction(pc);
        if pc <= entry
            || pc >= end
            || !decisions.insert(pc)
            || instruction.is_none_or(|instruction| !instruction.opcode.is_branch())
        {
            return Err(GuestError::invalid(
                "QVM actor frame predicate is not a distinct original conditional",
            ));
        }
        Ok(())
    };
    branch(definition.end.instruction)?;
    for filter in &definition.owned {
        branch(filter.instruction)?;
        let local = image.instruction(filter.local_instruction);
        let offset = image.instruction(filter.local_instruction + 2);
        let zero = image.instruction(filter.local_instruction + 5);
        let valid = filter.local_instruction + 6 == filter.instruction
            && local.is_some_and(|local| {
                local.opcode == QvmOpcode::OpLocal && local.operand >= 8 && local.operand + 4 <= frame_size
            })
            && image
                .instruction(filter.local_instruction + 1)
                .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpLoad4)
            && offset.is_some_and(|offset| offset.opcode == QvmOpcode::OpConst && offset.operand == inuse as i32)
            && image
                .instruction(filter.local_instruction + 3)
                .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpAdd)
            && image
                .instruction(filter.local_instruction + 4)
                .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpLoad4)
            && zero.is_some_and(|zero| zero.opcode == QvmOpcode::OpConst && zero.operand == 0)
            && image
                .instruction(filter.instruction)
                .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpNe)
            && inuse + 4 <= record.stride;
        if !valid {
            return Err(GuestError::invalid(
                "QVM actor frame filter differs from its original local entity/in-use predicate",
            ));
        }
    }
    let clock = &definition.clock;
    let argument = definition.call.arguments.get(clock.argument);
    let address = clock.store.checked_sub(3).and_then(|pc| image.instruction(pc));
    let local = clock.store.checked_sub(2).and_then(|pc| image.instruction(pc));
    let argument_valid = matches!(
        argument,
        Some(QvmModValue::Time {
            input,
            units: QvmTimeUnits::Milliseconds,
            encoding: QvmModScalar::Int32,
        }) if input.0 == "time"
    );
    let end_bytes = image.initialized_data.len() + image.bss_length;
    let valid = clock.argument < QVM_MAX_PRIVATE_ARGUMENT_WORDS
        && argument_valid
        && clock.address.is_multiple_of(4)
        && clock.address + 4 <= end_bytes
        && clock.store > entry
        && clock.store < end
        && address
            .is_some_and(|address| address.opcode == QvmOpcode::OpConst && address.operand == clock.address as i32)
        && local.is_some_and(|local| {
            local.opcode == QvmOpcode::OpLocal && local.operand == frame_size + 8 + clock.argument as i32 * 4
        })
        && clock
            .store
            .checked_sub(1)
            .and_then(|pc| image.instruction(pc))
            .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpLoad4)
        && image
            .instruction(clock.store)
            .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpStore4)
        && !definition
            .call
            .globals
            .iter()
            .any(|global| global.address == clock.address);
    if !valid {
        return Err(GuestError::invalid(
            "QVM actor frame clock differs from its original argument store",
        ));
    }
    Ok(())
}

/// Active frame invocation.
#[derive(Debug, Default)]
struct ActiveFrame {
    /// Whether the entity loop completed.
    completed: bool,
}

/// Original actor frame with owned-entity filtering.
pub struct QvmModActorFrame {
    /// Source module.
    module: QvmModule,
    /// Hook id.
    hook: u64,
    /// Frame locals: predicate instruction plus local offset.
    locals: Vec<(usize, usize)>,
    /// Frame size in bytes.
    frame_size: usize,
    /// Loop-end declaration.
    end: QvmActorEnd,
    /// Owned-entity predicate.
    owned: Rc<dyn Fn(usize) -> Result<bool, GuestError>>,
    /// Active invocation.
    active: Rc<RefCell<Option<ActiveFrame>>>,
    /// First stashed invocation error.
    error: Rc<RefCell<Option<GuestError>>>,
}

impl QvmModActorFrame {
    /// Bind an actor frame over a module.
    pub fn new(
        definition: &QvmModActorFrameDeclaration,
        module: QvmModule,
        image: &QvmImage,
        owned: Rc<dyn Fn(usize) -> Result<bool, GuestError>>,
    ) -> Result<Self, GuestError> {
        let Some(enter) = image.instruction(definition.call.entry) else {
            return Err(GuestError::invalid("Missing original actor frame"));
        };
        if enter.opcode != QvmOpcode::OpEnter {
            return Err(GuestError::invalid("Missing original actor frame"));
        }
        let mut locals = Vec::with_capacity(definition.owned.len());
        for filter in &definition.owned {
            let Some(local) = image.instruction(filter.local_instruction) else {
                return Err(GuestError::invalid("Missing original actor loop local"));
            };
            if local.opcode != QvmOpcode::OpLocal {
                return Err(GuestError::invalid("Missing original actor loop local"));
            }
            locals.push((filter.instruction, local.operand as usize));
        }
        let frame = Self {
            module: module.clone(),
            hook: 0,
            locals,
            frame_size: enter.operand as usize,
            end: definition.end,
            owned,
            active: Rc::new(RefCell::new(None)),
            error: Rc::new(RefCell::new(None)),
        };
        let hook = {
            let frame = frame.bind_state();
            module.bind_invocation(
                definition.call.entry,
                Rc::new(move |call: &mut QvmFunctionCall| frame.on_call(call)),
            )
        };
        Ok(Self { hook, ..frame })
    }

    /// Shared invocation state for the hook closure.
    fn bind_state(&self) -> BoundFrame {
        BoundFrame {
            locals: self.locals.clone(),
            frame_size: self.frame_size,
            end: self.end,
            owned: Rc::clone(&self.owned),
            active: Rc::clone(&self.active),
            error: Rc::clone(&self.error),
        }
    }

    /// Run the frame around an invocation.
    pub fn run(&self, invoke: &dyn Fn()) -> Result<bool, GuestError> {
        if self.active.borrow().is_some() {
            return Err(GuestError::invalid("QVM actor frame is already executing"));
        }
        *self.active.borrow_mut() = Some(ActiveFrame::default());
        invoke();
        let completed = self
            .active
            .borrow_mut()
            .take()
            .map(|active| active.completed)
            .unwrap_or(false);
        if let Some(error) = self.error.borrow_mut().take() {
            return Err(error);
        }
        Ok(completed)
    }

    /// Remove the frame hook.
    pub fn close(&self) {
        self.module.remove_hook(self.hook);
    }
}

/// Hook-shared frame state.
#[derive(Clone)]
struct BoundFrame {
    /// Predicate instruction plus local offset.
    locals: Vec<(usize, usize)>,
    /// Frame size in bytes.
    frame_size: usize,
    /// Loop end.
    end: QvmActorEnd,
    /// Owned-entity predicate.
    owned: Rc<dyn Fn(usize) -> Result<bool, GuestError>>,
    /// Active invocation.
    active: Rc<RefCell<Option<ActiveFrame>>>,
    /// First stashed invocation error.
    error: Rc<RefCell<Option<GuestError>>>,
}

impl BoundFrame {
    /// Handle one frame invocation.
    fn on_call(&self, call: &mut QvmFunctionCall) -> i32 {
        if self.active.borrow().is_none() {
            return call.proceed();
        }
        let stack = call.stack_address.checked_sub(8 + self.frame_size);
        let Some(stack) = stack else {
            self.stash(GuestError::invalid("QVM actor frame stack is outside its caller"));
            return 0;
        };
        let mut bindings = Vec::with_capacity(self.locals.len() + 1);
        for (instruction, offset) in &self.locals {
            let frame = self.clone();
            let offset = *offset;
            let guest = call.guest.clone();
            bindings.push(QvmBranchBinding {
                instruction_index: *instruction,
                decide: Box::new(move |taken, _| {
                    if !taken {
                        return false;
                    }
                    let pointer = stack
                        .checked_add(offset)
                        .and_then(|address| guest.read_i32(address).ok());
                    let Some(pointer) = pointer else {
                        frame.stash(GuestError::invalid("QVM actor loop local is outside its caller frame"));
                        return false;
                    };
                    match (frame.owned)(pointer as usize) {
                        Ok(owned) => owned,
                        Err(error) => {
                            frame.stash(error);
                            false
                        }
                    }
                }),
            });
        }
        let end = self.end;
        let active = Rc::clone(&self.active);
        bindings.push(QvmBranchBinding {
            instruction_index: end.instruction,
            decide: Box::new(move |taken, control| {
                if taken == end.completed_taken {
                    if let Some(active) = active.borrow_mut().as_mut() {
                        active.completed = true;
                    }
                    control.cancel_function();
                }
                taken
            }),
        });
        call.branches(bindings);
        call.proceed()
    }

    /// Stash the first invocation error.
    fn stash(&self, error: GuestError) {
        if self.error.borrow().is_none() {
            *self.error.borrow_mut() = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::super::game_data::{
        AbiProfile, ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole,
    };
    use super::super::mod_actors::{
        QvmCallbackInput, QvmModActorRecord, QvmModReturn, QvmModScalar, QvmModValue, QvmTimeUnits,
    };
    use super::*;

    fn image() -> QvmImage {
        let mut image = QvmImage {
            instructions: vec![QvmInstruction::word(QvmOpcode::OpEnter, 64, 0)],
            ..Default::default()
        };
        for index in 1..6 {
            image
                .instructions
                .push(QvmInstruction::word(QvmOpcode::OpConst, 0, index * 8));
        }
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpLocal, 16, 48));
        image.instructions.push(QvmInstruction::word(QvmOpcode::OpLoad4, 0, 56));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpConst, 208, 64));
        image.instructions.push(QvmInstruction::word(QvmOpcode::OpAdd, 0, 72));
        image.instructions.push(QvmInstruction::word(QvmOpcode::OpLoad4, 0, 80));
        image.instructions.push(QvmInstruction::word(QvmOpcode::OpConst, 0, 88));
        image.instructions.push(QvmInstruction::word(QvmOpcode::OpNe, 0, 96));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpConst, 1, 104));
        image.instructions.push(QvmInstruction::word(QvmOpcode::OpEq, 0, 112));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpConst, 2, 120));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpConst, 3, 128));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpConst, 64, 136));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpLocal, 72, 144));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpLoad4, 0, 152));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpStore4, 0, 160));
        image
            .instructions
            .push(QvmInstruction::word(QvmOpcode::OpEnter, 0, 168));
        image.data_length = 8192;
        image.initialized_data = vec![0u8; 8192];
        image.allocated_data_length = 65536;
        image
    }

    fn definition(image: &QvmImage) -> QvmModActorFrameDeclaration {
        let _ = image;
        QvmModActorFrameDeclaration {
            call: QvmModSourceCall {
                entry: 0,
                arguments: vec![QvmModValue::Time {
                    input: QvmCallbackInput::named("time"),
                    units: QvmTimeUnits::Milliseconds,
                    encoding: QvmModScalar::Int32,
                }],
                globals: Vec::new(),
                returns: QvmModReturn::Void,
            },
            clock: QvmActorClock {
                address: 64,
                store: 20,
                argument: 0,
            },
            owned: vec![QvmActorFilter {
                instruction: 12,
                local_instruction: 6,
            }],
            end: QvmActorEnd {
                instruction: 14,
                completed_taken: true,
            },
        }
    }

    fn record() -> QvmModActorRecord {
        QvmModActorRecord {
            id: "entity".to_string(),
            address: 4096,
            stride: 512,
            capacity: 4,
            fields: Vec::new(),
        }
    }

    fn module(image: &QvmImage) -> QvmModule {
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: Some(AbiProfile::Modern),
            image: image.clone(),
        };
        QvmModule::new(artifact, None, None).unwrap()
    }

    #[test]
    fn validates_frames() {
        let image = image();
        assert!(validate_qvm_mod_actor_frame(&definition(&image), &image, &record(), 208).is_ok());

        let mut bad = definition(&image);
        bad.call.returns = QvmModReturn::Int32;
        assert!(validate_qvm_mod_actor_frame(&bad, &image, &record(), 208).is_err());

        let mut bad = definition(&image);
        bad.owned.clear();
        assert!(validate_qvm_mod_actor_frame(&bad, &image, &record(), 208).is_err());

        let mut bad = definition(&image);
        bad.owned[0].local_instruction = 7;
        assert!(validate_qvm_mod_actor_frame(&bad, &image, &record(), 208).is_err());

        let mut bad = definition(&image);
        bad.clock.argument = 3;
        assert!(validate_qvm_mod_actor_frame(&bad, &image, &record(), 208).is_err());

        let mut bad = definition(&image);
        bad.end.instruction = 13;
        assert!(validate_qvm_mod_actor_frame(&bad, &image, &record(), 208).is_err());

        let mut bad = definition(&image);
        bad.call.globals.push(super::super::mod_actors::QvmModGlobal {
            address: 64,
            value: QvmModValue::Address(0),
        });
        assert!(validate_qvm_mod_actor_frame(&bad, &image, &record(), 208).is_err());
    }

    #[test]
    fn collects_bootstrap_stores() {
        let mut image = image();
        image.instructions[1] = QvmInstruction::word(QvmOpcode::OpConst, 100, 8);
        image.instructions[2] = QvmInstruction::word(QvmOpcode::OpConst, 7, 16);
        image.instructions[3] = QvmInstruction::word(QvmOpcode::OpStore4, 0, 24);
        let stores = qvm_actor_bootstrap(&[3], &image, &[record()]).unwrap();
        assert_eq!(stores, vec![QvmActorStore { address: 100, value: 7 }]);
        assert!(qvm_actor_bootstrap(&[4], &image, &[record()]).is_err());
        assert!(qvm_actor_bootstrap(&[3, 3], &image, &[record()]).is_err());

        image.instructions[1] = QvmInstruction::word(QvmOpcode::OpConst, 4096, 8);
        assert!(qvm_actor_bootstrap(&[3], &image, &[record()]).is_err());
    }

    #[test]
    fn run_filters_and_completes() {
        let image = image();
        let module = module(&image);
        module.memory().write_i32(944, 4096).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_hook = Rc::clone(&seen);
        let frame = QvmModActorFrame::new(
            &definition(&image),
            module.clone(),
            &image,
            Rc::new(move |pointer| {
                seen_hook.borrow_mut().push(pointer);
                Ok(pointer == 4096)
            }),
        )
        .unwrap();
        let result = frame.run(&|| {
            module.call(&[], 0).unwrap();
        });
        assert!(result.is_err());
        assert!(seen.borrow().is_empty());

        let frame_state = frame.bind_state();
        let result = frame
            .run(&|| {
                let memory = module.memory();
                let mut call = super::super::game_data::QvmFunctionCall::entered(0, Vec::new(), memory);
                call.stack_address = 1000;
                assert_eq!(frame_state.on_call(&mut call), 0);
                assert_eq!(call.branch_bindings.len(), 2);
                assert!(call.decide_branch(0, true));
                assert!(!call.decide_branch(0, false));
            })
            .unwrap();
        assert!(!result);
        assert_eq!(seen.borrow().as_slice(), &[4096]);

        let completed = Rc::new(Cell::new(false));
        let done = Rc::clone(&completed);
        let result = frame
            .run(&|| {
                let memory = module.memory();
                let mut call = super::super::game_data::QvmFunctionCall::entered(0, Vec::new(), memory);
                call.stack_address = 1000;
                frame_state.on_call(&mut call);
                done.set(call.decide_branch(1, true));
            })
            .unwrap();
        assert!(result);
        assert!(completed.get());
    }

    #[test]
    fn run_rejects_reentry_and_surfaces_errors() {
        let image = image();
        let module = module(&image);
        let frame = QvmModActorFrame::new(&definition(&image), module.clone(), &image, Rc::new(|_| Ok(true))).unwrap();
        let nested = Rc::new(Cell::new(false));
        let flag = Rc::clone(&nested);
        frame
            .run(&|| {
                if frame.run(&|| {}).is_err() {
                    flag.set(true);
                }
            })
            .unwrap();
        assert!(nested.get());

        let failing = QvmModActorFrame::new(
            &definition(&image),
            module.clone(),
            &image,
            Rc::new(|_| Err(GuestError::invalid("foreign actor"))),
        )
        .unwrap();
        module.memory().write_i32(944, 8).unwrap();
        let state = failing.bind_state();
        let result = failing.run(&|| {
            let memory = module.memory();
            let mut call = super::super::game_data::QvmFunctionCall::entered(0, Vec::new(), memory);
            call.stack_address = 1000;
            state.on_call(&mut call);
            assert!(!call.decide_branch(0, true));
        });
        assert!(result.is_err());
    }

    #[test]
    fn close_removes_hook() {
        let image = image();
        let module = module(&image);
        let frame = QvmModActorFrame::new(&definition(&image), module.clone(), &image, Rc::new(|_| Ok(true))).unwrap();
        module.set_default_return(7);
        assert_eq!(module.call(&[], 0).unwrap(), 0);
        frame.close();
        assert_eq!(module.call(&[], 0).unwrap(), 7);
    }
}
