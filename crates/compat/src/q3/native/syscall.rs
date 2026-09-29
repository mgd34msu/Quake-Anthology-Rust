//! Q3 1.32 native variadic syscall dispatcher.
//!
//! Donor: `src/compat/q3/native/syscall.ts` — bridges the single variadic
//! `syscall(code, ...)` guest entry onto per-code headless service bindings.
//! Only arguments declared by the owning binding are decoded; unbound codes
//! fail before any guessed argument is read.

use std::collections::HashMap;

use qa_guest::core::contracts::{
    GuestAddress, GuestCallContext, GuestCallResult, GuestCallSignature, GuestCallValue, GuestStorage,
    GuestValueLayout, NativeAbi,
};
use qa_guest::error::GuestError;
use thiserror::Error;

use super::module::{call_integer, native_q3_signature};

/// First synthetic trap offset minted for native Q3 entries.
pub const Q3_TRAP_BASE: u64 = 0x7e00_0000;

/// Stride between consecutive synthetic trap addresses.
pub const Q3_TRAP_STRIDE: u64 = 16;

/// Failure binding or dispatching a Q3 native syscall.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3SyscallError {
    /// Syscall profile requires the i386 source ABI.
    #[error("Q3 1.32 syscalls require the i386 source ABI")]
    UnsupportedAbi,
    /// Two bindings claimed the same syscall code.
    #[error("Duplicate native Q3 syscall {0}")]
    DuplicateSyscall(i32),
    /// No binding serves this syscall code.
    #[error("Unbound Q3 native syscall {0}")]
    UnboundSyscall(i32),
    /// Variadic call carried no syscall code.
    #[error("Q3 native syscall without a code argument")]
    MissingCode,
    /// Syscall argument has the wrong shape.
    #[error("{0}")]
    BadArgument(String),
    /// Underlying guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Headless service behind one syscall code. Pointer arguments stay guest
/// pointers; float-carrying slots arrive as int32 bits.
pub type SyscallHandler = Box<dyn Fn(&GuestCallContext, &[GuestCallValue]) -> Result<i32, Q3SyscallError>>;

/// One bound Q3 native syscall.
pub struct NativeQ3SyscallBinding {
    /// Syscall code (first variadic argument).
    pub code: i32,
    /// Service name for diagnostics.
    pub name: String,
    /// Declared parameter storages used for variadic decoding.
    pub parameters: Vec<GuestStorage>,
    /// Service implementation.
    pub invoke: SyscallHandler,
}

impl NativeQ3SyscallBinding {
    /// Bind `code` to `invoke` with the given declared parameters.
    pub fn new(
        code: i32,
        name: &str,
        parameters: Vec<GuestStorage>,
        invoke: impl Fn(&GuestCallContext, &[GuestCallValue]) -> Result<i32, Q3SyscallError> + 'static,
    ) -> Self {
        Self {
            code,
            name: name.to_string(),
            parameters,
            invoke: Box::new(invoke),
        }
    }
}

impl std::fmt::Debug for NativeQ3SyscallBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeQ3SyscallBinding")
            .field("code", &self.code)
            .field("name", &self.name)
            .field("parameters", &self.parameters)
            .finish_non_exhaustive()
    }
}

/// Mints synthetic trap addresses in one guest address space, standing in
/// for the host callback table the donor binds through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapAddressAllocator {
    space: u64,
    owner: String,
    next: u64,
}

impl TrapAddressAllocator {
    /// Fresh allocator for `space`; `owner` names callback ids.
    #[must_use]
    pub fn new(space: u64, owner: &str) -> Self {
        Self {
            space,
            owner: owner.to_string(),
            next: Q3_TRAP_BASE,
        }
    }

    /// Owning address space.
    #[must_use]
    pub fn space(&self) -> u64 {
        self.space
    }

    /// Owner label embedded in callback ids.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Mint the next trap address.
    #[must_use]
    pub fn bind(&mut self) -> GuestAddress {
        let address = GuestAddress::new(self.space, self.next);
        self.next += Q3_TRAP_STRIDE;
        address
    }
}

/// Shared variadic `syscall(code, ...)` entry over headless bindings.
pub struct NativeQ3Syscalls {
    address: GuestAddress,
    callback_id: String,
    abi: NativeAbi,
    services: HashMap<i32, NativeQ3SyscallBinding>,
}

impl NativeQ3Syscalls {
    /// Bind the shared entry, rejecting duplicate codes and non-i386 ABIs.
    pub fn new(
        allocator: &mut TrapAddressAllocator,
        abi: NativeAbi,
        bindings: Vec<NativeQ3SyscallBinding>,
    ) -> Result<Self, Q3SyscallError> {
        if abi.pointer_bytes() != 4 {
            return Err(Q3SyscallError::UnsupportedAbi);
        }
        let mut services = HashMap::with_capacity(bindings.len());
        for binding in bindings {
            if services.contains_key(&binding.code) {
                return Err(Q3SyscallError::DuplicateSyscall(binding.code));
            }
            services.insert(binding.code, binding);
        }
        let address = allocator.bind();
        let callback_id = format!("q3-native-syscall:{}", allocator.owner());
        Ok(Self {
            address,
            callback_id,
            abi,
            services,
        })
    }

    /// Shared variadic entry address handed to the guest module.
    #[must_use]
    pub fn address(&self) -> GuestAddress {
        self.address
    }

    /// Callback id of the shared entry.
    #[must_use]
    pub fn callback_id(&self) -> &str {
        &self.callback_id
    }

    /// Signature of the shared entry: fixed int32 code plus variadic tail.
    #[must_use]
    pub fn entry_signature(&self) -> GuestCallSignature {
        native_q3_signature(self.abi, &[GuestStorage::Int32], Some(GuestStorage::Int32), true)
    }

    /// Bound service names by code.
    #[must_use]
    pub fn service_names(&self) -> HashMap<i32, &str> {
        self.services
            .iter()
            .map(|(code, binding)| (*code, binding.name.as_str()))
            .collect()
    }

    fn service(&self, fixed: &[GuestCallValue]) -> Result<&NativeQ3SyscallBinding, Q3SyscallError> {
        if fixed.is_empty() {
            return Err(Q3SyscallError::MissingCode);
        }
        let code = i32::try_from(
            call_integer(fixed, 0)
                .map_err(|_| Q3SyscallError::BadArgument("Q3 syscall code must be an integer".to_string()))?,
        )
        .map_err(|_| Q3SyscallError::BadArgument("Q3 syscall code exceeds int32 range".to_string()))?;
        self.services.get(&code).ok_or(Q3SyscallError::UnboundSyscall(code))
    }

    /// Run the shared entry: decode the code, then hand the tail to the
    /// owning binding. Unbound codes fail before any tail argument is read.
    pub fn dispatch(
        &self,
        context: &GuestCallContext,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, Q3SyscallError> {
        let service = self.service(args)?;
        let value = (service.invoke)(context, &args[1..])?;
        Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
    }

    /// Variadic layouts for the shared entry's tail, or `None` when
    /// `callback_id` names a different entry. Unknown codes fail rather
    /// than decoding guessed arguments.
    pub fn variadic_layouts(
        &self,
        callback_id: &str,
        fixed: &[GuestCallValue],
    ) -> Result<Option<Vec<GuestValueLayout>>, Q3SyscallError> {
        if callback_id != self.callback_id {
            return Ok(None);
        }
        let service = self.service(fixed)?;
        Ok(Some(
            service
                .parameters
                .iter()
                .map(|storage| GuestValueLayout::Scalar(*storage))
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{CallbackId, ContentDigest, GuestCallbackReference, ModuleIdentity};

    use super::*;

    fn test_context() -> GuestCallContext {
        GuestCallContext {
            module: ModuleIdentity::new(
                ProviderId::new("test", "q3-syscall"),
                "test-syscall.so",
                ContentDigest::new("sha256", "abc"),
                "rev",
            ),
            callback: GuestCallbackReference::TypeScript {
                provider: ProviderId::new("test", "root"),
                callback: CallbackId::new("test", "root"),
            },
            parent: None,
            itself: None,
            other: None,
        }
    }

    fn print_binding() -> NativeQ3SyscallBinding {
        NativeQ3SyscallBinding::new(1, "G_Printf", vec![GuestStorage::Pointer], |_context, args| {
            if args.len() != 1 {
                return Err(Q3SyscallError::BadArgument("G_Printf expects one argument".to_string()));
            }
            Ok(7)
        })
    }

    #[test]
    fn dispatch_routes_by_code_and_layouts_decode_tail() {
        let mut allocator = TrapAddressAllocator::new(11, "game");
        let syscalls = NativeQ3Syscalls::new(
            &mut allocator,
            NativeAbi::LinuxI386,
            vec![
                print_binding(),
                NativeQ3SyscallBinding::new(2, "G_Error", vec![GuestStorage::Pointer], |_, _| Ok(-1)),
            ],
        )
        .expect("bind syscalls");
        assert_eq!(syscalls.address(), GuestAddress::new(11, Q3_TRAP_BASE));
        assert_eq!(syscalls.callback_id(), "q3-native-syscall:game");
        let signature = syscalls.entry_signature();
        assert!(signature.variadic);
        assert_eq!(
            signature.parameters,
            vec![GuestValueLayout::Scalar(GuestStorage::Int32)]
        );

        let context = test_context();
        let target = GuestAddress::new(11, 0x5000);
        let result = syscalls
            .dispatch(
                &context,
                &[GuestCallValue::Int32(1), GuestCallValue::Pointer(Some(target))],
            )
            .expect("dispatch print");
        assert_eq!(result, GuestCallResult::Value(GuestCallValue::Int32(7)));

        let layouts = syscalls
            .variadic_layouts(syscalls.callback_id(), &[GuestCallValue::Int32(1)])
            .expect("layouts")
            .expect("some layouts");
        assert_eq!(layouts, vec![GuestValueLayout::Scalar(GuestStorage::Pointer)]);
        assert_eq!(
            syscalls
                .variadic_layouts("other-entry", &[GuestCallValue::Int32(1)])
                .expect("foreign entry"),
            None
        );
        assert!(matches!(
            syscalls.dispatch(&context, &[GuestCallValue::Int32(99)]),
            Err(Q3SyscallError::UnboundSyscall(99))
        ));
        assert!(matches!(
            syscalls.dispatch(&context, &[]),
            Err(Q3SyscallError::MissingCode)
        ));
    }

    #[test]
    fn bind_rejects_duplicates_and_non_i386() {
        let mut allocator = TrapAddressAllocator::new(12, "game");
        assert!(matches!(
            NativeQ3Syscalls::new(
                &mut allocator,
                NativeAbi::LinuxI386,
                vec![print_binding(), print_binding()],
            ),
            Err(Q3SyscallError::DuplicateSyscall(1))
        ));
        assert!(matches!(
            NativeQ3Syscalls::new(&mut allocator, NativeAbi::LinuxX86_64, vec![print_binding()],),
            Err(Q3SyscallError::UnsupportedAbi)
        ));
        let bound = NativeQ3Syscalls::new(&mut allocator, NativeAbi::LinuxI386, vec![print_binding()])
            .expect("bind");
        let names = bound.service_names();
        assert_eq!(names.get(&1), Some(&"G_Printf"));
    }
}
