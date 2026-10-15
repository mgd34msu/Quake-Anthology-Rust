//! QuakeC parameter admission and return writes over the one call table.
use super::*;

#[derive(Debug)]
pub enum QcImportStep {
    Returned,
    Host {
        request: HostRequest,
        continuation: QcImportContinuation,
    },
}

#[derive(Debug)]
pub struct QcImportContinuation {
    kind: QcContinuation,
    returns: bool,
}
#[derive(Debug)]
pub(super) enum QcContinuation {
    Shared(ImportContinuation),
    Walk(quakec::monster::WalkAfterTouch),
    Goal(qa_session::physics::GoalCursor),
}

impl quakec::Builtins for QuakeCCalls<'_, '_> {
    fn entity_access(&self) -> quakec::EntityAccess<'_> {
        quakec::EntityAccess::engine(self.services)
    }
    fn call(&mut self, vm: &mut quakec::Vm, number: u32, argc: usize) -> Result<(), quakec::Trap> {
        let call = quakec::BuiltinCall::capture(vm, number, argc)?;
        self.stopped(vm, &call)
    }

    fn stopped(
        &mut self,
        vm: &mut quakec::Vm,
        call: &quakec::BuiltinCall,
    ) -> Result<(), quakec::Trap> {
        match self.import(vm, call)? {
            QcImportStep::Returned => Ok(()),
            // Engine composition drives owned host work. A direct builtin
            // caller cannot execute another module behind its service borrow.
            QcImportStep::Host { continuation, .. } => {
                self.abort_import(vm, continuation)?;
                Err(quakec::Trap::Builtin)
            }
        }
    }
}

impl QuakeCCalls<'_, '_> {
    pub fn import(
        &mut self,
        vm: &mut quakec::Vm,
        call: &quakec::BuiltinCall,
    ) -> Result<QcImportStep, quakec::Trap> {
        let number = call.number;
        // Unsupported native builtins trap only when executed; they never
        // inherit the Q3 convention of reporting an unknown import as zero.
        let Some(entry) = self.table.entries.get(number as usize).copied().flatten() else {
            self.unknown
                .report(self.services, self.context.module, number, "QuakeC builtin");
            return Err(quakec::Trap::Builtin);
        };
        let signature = entry.quakec.ok_or(quakec::Trap::Builtin)?;
        if matches!(signature.argc, QcArgc::Fixed(expected) if call.argc() != expected) {
            return Err(quakec::Trap::Builtin);
        }
        let mut arguments = [0u64; 24];
        let mut words = 0;
        let quakeworld = self.context.native_rules == qa_core::primitives::RuleSetId::QuakeWorld;
        let string_start = signature.args.string_start(quakeworld);
        for (index, raw) in call.arguments().iter().enumerate() {
            for (component, &raw) in raw.iter().enumerate().take(signature.args.width(index)) {
                let mut word = raw;
                if component == 0 && signature.args.string(index, string_start) {
                    vm.string(word as i32)?;
                }
                if index == 0 && matches!(signature.args, QcArgs::ClientStrings { .. }) {
                    word = vm.entity_number(word)?;
                }
                arguments[words] = u64::from(word);
                words += 1;
            }
        }
        if signature.developer_nq_only
            && !quakeworld
            && self.services.cvars.value(self.developer) == 0.0
        {
            return Ok(QcImportStep::Returned);
        }
        let value = if let Handler::QuakeC(entry) = entry.handler {
            entry(self, vm, &arguments[..words]).map_err(|_| quakec::Trap::Builtin)?
        } else {
            self.with_invocation(vm, &arguments[..words], |invocation| {
                entry.invoke(invocation, string_start)
            })
            .map_err(|_| quakec::Trap::Builtin)?
        };
        Self::apply_value(vm, value, signature.returns)
    }

    pub fn resume_import(
        &mut self,
        vm: &mut quakec::Vm,
        continuation: QcImportContinuation,
        returned: u64,
    ) -> Result<QcImportStep, quakec::Trap> {
        let shared = match continuation.kind {
            QcContinuation::Shared(shared) => shared,
            QcContinuation::Walk(walk) => {
                let result = vm
                    .finish_walk_move(self.services, self.context, &walk)
                    .map_err(|_| quakec::Trap::Builtin)
                    .and_then(|()| {
                        Self::apply_value(
                            vm,
                            CallValue::Word(u64::from(1.0f32.to_bits())),
                            continuation.returns,
                        )
                    });
                // Native PF_walkmove writes OFS_RETURN before restoring self. A
                // legal self/return alias must observe that exact order.
                let restored = vm
                    .restore_walk_move(walk)
                    .map_err(|_| quakec::Trap::Builtin);
                return result.and_then(|step| restored.map(|()| step));
            }
            QcContinuation::Goal(cursor) => {
                let step = vm
                    .resume_goal(self.services, self.context, cursor)
                    .and_then(|step| qc_goal_value(vm, self.context, step))
                    .map_err(|_| quakec::Trap::Builtin)?;
                return Self::apply_value(vm, step, false);
            }
        };
        let step = self
            .with_invocation(vm, &[], |invocation| shared.resume(invocation, returned))
            .map_err(|_| quakec::Trap::Builtin)?;
        let value = match step {
            ImportStep::Return(value) => CallValue::Word(value),
            ImportStep::Host {
                request,
                continuation,
            } => CallValue::Host(request, continuation),
        };
        Self::apply_value(vm, value, continuation.returns)
    }

    pub fn abort_import(
        &mut self,
        vm: &mut quakec::Vm,
        continuation: QcImportContinuation,
    ) -> Result<(), quakec::Trap> {
        match continuation.kind {
            QcContinuation::Shared(_) | QcContinuation::Goal(_) => Ok(()),
            QcContinuation::Walk(walk) => vm
                .restore_walk_move(walk)
                .map_err(|_| quakec::Trap::Builtin),
        }
    }

    fn apply_value(
        vm: &mut quakec::Vm,
        value: CallValue,
        returns: bool,
    ) -> Result<QcImportStep, quakec::Trap> {
        match value {
            CallValue::Word(value) => {
                if returns {
                    *vm.image.globals.get_mut(1).ok_or(quakec::Trap::Memory)? = value as u32;
                }
            }
            CallValue::TemporaryString(offset, length) => {
                vm.register_temporary_string(length);
                if returns {
                    *vm.image.globals.get_mut(1).ok_or(quakec::Trap::Memory)? = offset;
                }
            }
            CallValue::Vector(vector) => {
                if returns {
                    vm.write_global_vector(1, vector)?;
                }
            }
            CallValue::Basis(basis) => vm.write_basis(basis)?,
            CallValue::Host(request, shared) => {
                return Ok(QcImportStep::Host {
                    request,
                    continuation: QcImportContinuation {
                        kind: QcContinuation::Shared(shared),
                        returns,
                    },
                });
            }
            CallValue::QuakeCHost(request, kind) => {
                return Ok(QcImportStep::Host {
                    request,
                    continuation: QcImportContinuation { kind, returns },
                });
            }
            CallValue::QuakeCReturn(value, restore) => {
                let result = Self::apply_value(vm, CallValue::Word(value), returns);
                let restored = vm
                    .finish_client_call(restore)
                    .map_err(|_| quakec::Trap::Builtin);
                return result.and_then(|step| restored.map(|()| step));
            }
        }
        Ok(QcImportStep::Returned)
    }

    fn with_invocation<R>(
        &mut self,
        vm: &mut quakec::Vm,
        arguments: &[u64],
        action: impl FnOnce(&mut Invocation<'_, '_, '_>) -> Result<R, CallError>,
    ) -> Result<R, CallError> {
        action(&mut Invocation {
            services: self.services,
            memory: &mut vm.strings,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: self.context,
            platform_time: self.platform_time,
            command: &[],
            addresses: Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 8,
            },
            arguments,
        })
    }
}
