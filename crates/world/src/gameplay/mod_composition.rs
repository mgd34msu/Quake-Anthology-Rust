//! Mod operation composition: ordered transform/observe/replace contributions.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/gameplay/mod-composition.ts`.
//!
//! Rust adaptations: registrations release through an explicit token (the
//! donor returns a closure); continuation misuse panics instead of throwing;
//! a panicking canonical call unwinds through `replace` (the donor rethrows
//! a captured failure after `replace` returns, which has no equivalent
//! without `catch_unwind`); the continuation clones the request it
//! retargets, so `call` needs `Request: Clone`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use qa_core::identity::ProviderId;

use crate::WorldError;

/// Identity of one mod registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModRegistrationIdentity {
    /// Contributing provider.
    pub provider: ProviderId,
    /// Registration id (`namespace:name`).
    pub id: String,
    /// Dispatch order (lower runs first; ties break by sequence).
    /// The donor validates a safe integer; `i64` always qualifies.
    pub order: i64,
}

fn label(identity: &ModRegistrationIdentity) -> String {
    format!(
        "{}:{}/{}",
        identity.provider.namespace, identity.provider.name, identity.id
    )
}

type TransformFn<Request> = dyn Fn(Request) -> Request;
type ObserveFn<Request, Output> = dyn Fn(&Request, &Output);
type ReplaceFn<Request, Output> = dyn for<'x> Fn(Request, Continuation<'x, Request, Output>) -> Output;

/// One mod contribution to an operation.
pub enum ModOperationRegistration<Request, Output> {
    /// Rewrite the request before the canonical call.
    Transform {
        /// Registration identity.
        identity: ModRegistrationIdentity,
        /// Rewrite function.
        transform: Box<TransformFn<Request>>,
    },
    /// Observe the settled request and result.
    Observe {
        /// Registration identity.
        identity: ModRegistrationIdentity,
        /// Observer function.
        observe: Box<ObserveFn<Request, Output>>,
    },
    /// Wrap the canonical call with a single-use continuation.
    Replace {
        /// Registration identity.
        identity: ModRegistrationIdentity,
        /// Replacement function.
        replace: Box<ReplaceFn<Request, Output>>,
    },
}

impl<Request, Output> ModOperationRegistration<Request, Output> {
    fn identity(&self) -> &ModRegistrationIdentity {
        match self {
            ModOperationRegistration::Transform { identity, .. }
            | ModOperationRegistration::Observe { identity, .. }
            | ModOperationRegistration::Replace { identity, .. } => identity,
        }
    }

    fn is_replace(&self) -> bool {
        matches!(self, ModOperationRegistration::Replace { .. })
    }
}

struct ContinuationState<Request> {
    open: Cell<bool>,
    called: Cell<bool>,
    retarget: RefCell<Option<Request>>,
}

/// Single-use canonical continuation passed to a `replace` contribution.
pub struct Continuation<'a, Request, Output> {
    state: Rc<ContinuationState<Request>>,
    canonical: &'a dyn Fn(Request) -> Output,
    name: String,
    label: String,
}

impl<Request: Clone, Output> Continuation<'_, Request, Output> {
    /// Call the canonical operation once, retargeting the observed request.
    /// Panics when the continuation is closed or already called.
    pub fn call(&self, request: Request) -> Output {
        if !self.state.open.get() {
            panic!("Mod continuation for {} is closed: {}", self.name, self.label);
        }
        if self.state.called.get() {
            panic!("Mod continuation for {} was already called: {}", self.name, self.label);
        }
        self.state.called.set(true);
        self.state.retarget.replace(Some(request.clone()));
        (self.canonical)(request)
    }
}

struct Registration<Request, Output> {
    contribution: ModOperationRegistration<Request, Output>,
    sequence: u64,
    active: Cell<bool>,
}

/// Token releasing one registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModRegistrationToken(u64);

/// Composable operation with ordered mod contributions.
///
/// Registrations change between invocations; removing one takes effect even
/// during a nested call.
pub struct ModOperation<Request, Output> {
    name: String,
    entries: Vec<Rc<Registration<Request, Output>>>,
    sequence: u64,
}

impl<Request, Output> ModOperation<Request, Output> {
    /// Create an operation with `name`.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            entries: Vec::new(),
            sequence: 0,
        }
    }

    /// Operation name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether any contribution is registered.
    #[must_use]
    pub fn active(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Register a contribution; the token releases exactly this entry.
    pub fn register(
        &mut self,
        contribution: ModOperationRegistration<Request, Output>,
    ) -> Result<ModRegistrationToken, WorldError> {
        for entry in &self.entries {
            let previous = &entry.contribution;
            if previous.identity().provider == contribution.identity().provider
                && previous.identity().id == contribution.identity().id
            {
                return Err(WorldError::BadModOperation(format!(
                    "Duplicate mod registration for {}: {}",
                    self.name,
                    label(contribution.identity())
                )));
            }
            if previous.is_replace() && contribution.is_replace() {
                return Err(WorldError::BadModOperation(format!(
                    "Conflicting replacements for {}: {} and {}",
                    self.name,
                    label(previous.identity()),
                    label(contribution.identity())
                )));
            }
        }
        let token = ModRegistrationToken(self.sequence);
        let entry = Rc::new(Registration {
            contribution,
            sequence: self.sequence,
            active: Cell::new(true),
        });
        self.sequence += 1;
        self.entries.push(entry);
        self.entries.sort_by(|a, b| {
            a.contribution
                .identity()
                .order
                .cmp(&b.contribution.identity().order)
                .then(a.sequence.cmp(&b.sequence))
        });
        Ok(token)
    }

    /// Release the registration installed under `token`.
    pub fn unregister(&mut self, token: ModRegistrationToken) -> bool {
        let mut released = false;
        self.entries.retain(|entry| {
            if entry.sequence == token.0 && entry.active.get() {
                entry.active.set(false);
                released = true;
                false
            } else {
                true
            }
        });
        released
    }

    /// Dispatch through transforms, an optional replacement, and observers.
    /// `before_observers` runs after the canonical call, before observers.
    pub fn dispatch(
        &self,
        input: Request,
        canonical: &dyn Fn(Request) -> Output,
        before_observers: Option<&dyn Fn()>,
    ) -> Output
    where
        Request: Clone,
    {
        let entries = self.entries.clone();
        if entries.is_empty() {
            let result = canonical(input);
            if let Some(before) = before_observers {
                before();
            }
            return result;
        }
        let mut request = input;
        for entry in &entries {
            if !entry.active.get() {
                continue;
            }
            if let ModOperationRegistration::Transform { transform, .. } = &entry.contribution {
                request = transform(request);
            }
        }
        let replacement = entries
            .iter()
            .find(|entry| entry.active.get() && entry.contribution.is_replace());
        if let Some(entry) = replacement {
            let ModOperationRegistration::Replace { replace, .. } = &entry.contribution else {
                unreachable!("replacement entry");
            };
            let state = Rc::new(ContinuationState {
                open: Cell::new(true),
                called: Cell::new(false),
                retarget: RefCell::new(None),
            });
            struct Closer<'a> {
                open: &'a Cell<bool>,
            }
            impl Drop for Closer<'_> {
                fn drop(&mut self) {
                    self.open.set(false);
                }
            }
            let _closer = Closer { open: &state.open };
            let continuation = Continuation {
                state: Rc::clone(&state),
                canonical,
                name: self.name.clone(),
                label: label(entry.contribution.identity()),
            };
            let result = replace(request.clone(), continuation);
            let observed = state.retarget.borrow_mut().take().unwrap_or(request);
            return self.finish(observed, result, &entries, before_observers);
        }
        let result = canonical(request.clone());
        self.finish(request, result, &entries, before_observers)
    }

    fn finish(
        &self,
        request: Request,
        result: Output,
        entries: &[Rc<Registration<Request, Output>>],
        before_observers: Option<&dyn Fn()>,
    ) -> Output {
        if let Some(before) = before_observers {
            before();
        }
        for entry in entries {
            if !entry.active.get() {
                continue;
            }
            if let ModOperationRegistration::Observe { observe, .. } = &entry.contribution {
                observe(&request, &result);
            }
        }
        result
    }

    /// Deactivate and drop every registration.
    pub fn clear(&mut self) {
        for entry in &self.entries {
            entry.active.set(false);
        }
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: &str, order: i64) -> ModRegistrationIdentity {
        ModRegistrationIdentity {
            provider: ProviderId::new("test", "mod"),
            id: id.to_string(),
            order,
        }
    }

    #[test]
    fn empty_dispatch_runs_canonical() {
        let operation = ModOperation::<i32, i32>::new("op");
        assert!(!operation.active());
        let before = Cell::new(0);
        let result = operation.dispatch(40, &|value| value + 2, Some(&|| before.set(before.get() + 1)));
        assert_eq!(result, 42);
        assert_eq!(before.get(), 1);
    }

    #[test]
    fn transforms_observers_and_replacement_compose() {
        let mut operation = ModOperation::<i32, i32>::new("op");
        operation
            .register(ModOperationRegistration::Transform {
                identity: identity("double", 1),
                transform: Box::new(|value| value * 2),
            })
            .unwrap();
        operation
            .register(ModOperationRegistration::Replace {
                identity: identity("wrap", 2),
                replace: Box::new(|value, next| next.call(value) + 100),
            })
            .unwrap();
        let observed = Rc::new(Cell::new((0, 0)));
        let capture = Rc::clone(&observed);
        operation
            .register(ModOperationRegistration::Observe {
                identity: identity("watch", 3),
                observe: Box::new(move |request, result| capture.set((*request, *result))),
            })
            .unwrap();
        assert!(operation.active());
        let result = operation.dispatch(5, &|value| value + 1, None);
        assert_eq!(result, 111);
        assert_eq!(observed.get(), (10, 111));
    }

    #[test]
    fn registrations_reject_duplicates_and_conflicts() {
        let mut operation = ModOperation::<i32, i32>::new("op");
        operation
            .register(ModOperationRegistration::Transform {
                identity: identity("a", 0),
                transform: Box::new(|value| value),
            })
            .unwrap();
        assert!(operation
            .register(ModOperationRegistration::Observe {
                identity: identity("a", 0),
                observe: Box::new(|_, _| {}),
            })
            .is_err());
        operation
            .register(ModOperationRegistration::Replace {
                identity: identity("r1", 1),
                replace: Box::new(|value, next| next.call(value)),
            })
            .unwrap();
        assert!(operation
            .register(ModOperationRegistration::Replace {
                identity: identity("r2", 2),
                replace: Box::new(|value, next| next.call(value)),
            })
            .is_err());
    }

    #[test]
    fn unregister_and_clear_take_effect() {
        let mut operation = ModOperation::<i32, i32>::new("op");
        let token = operation
            .register(ModOperationRegistration::Transform {
                identity: identity("a", 0),
                transform: Box::new(|value| value + 1),
            })
            .unwrap();
        assert!(operation.unregister(token));
        assert!(!operation.unregister(token));
        assert!(!operation.active());
        operation
            .register(ModOperationRegistration::Transform {
                identity: identity("b", 0),
                transform: Box::new(|value| value + 1),
            })
            .unwrap();
        operation.clear();
        assert!(!operation.active());
        assert_eq!(operation.dispatch(1, &|value| value, None), 1);
    }
}
