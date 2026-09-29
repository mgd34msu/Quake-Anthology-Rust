//! Port of `src/compat/q2/native-mod-invocations.ts`.
//! Bridges source invocations that a retired donor region can abort: retirement
//! ends its invocation and restores the synthetic processor snapshot instead of
//! fabricating a native return value.

use std::collections::HashMap;
use std::rc::Rc;

use thiserror::Error;

/// Outcome of one guarded source invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModInvocationResult<T> {
    /// The body ran to completion.
    Completed(T),
    /// A retired donor region aborted the invocation; state was restored.
    Retired,
}

/// Failures inside a guarded invocation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvocationError {
    /// A donor region retired its owning invocation.
    #[error("native donor region {0} retired its source invocation")]
    Retired(u64),
    /// Any other invocation failure.
    #[error("{0}")]
    Failed(String),
}

/// Liveness authority for one donor region: a token plus a staleness predicate.
#[derive(Clone)]
pub struct RegionAuthority {
    /// Stable region token.
    pub token: u64,
    current: Rc<dyn Fn() -> bool>,
}

impl RegionAuthority {
    /// Build an authority from a token and a liveness predicate.
    pub fn new(token: u64, current: impl Fn() -> bool + 'static) -> Self {
        Self {
            token,
            current: Rc::new(current),
        }
    }

    /// Whether the region still owns its invocation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        (self.current)()
    }
}

impl std::fmt::Debug for RegionAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegionAuthority")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

struct Scope {
    current: Rc<dyn Fn() -> bool>,
}

struct Retirement {
    authority_current: Rc<dyn Fn() -> bool>,
    scope: usize,
}

/// Tracks nested source invocations over a synthetic processor snapshot word.
#[derive(Debug, Default)]
pub struct NativeModInvocations {
    scopes: Vec<Scope>,
    retired: HashMap<u64, Retirement>,
    state: u64,
}

impl NativeModInvocations {
    /// Build the tracker with an initial synthetic processor snapshot.
    #[must_use]
    pub fn new(initial_state: u64) -> Self {
        Self {
            scopes: Vec::new(),
            retired: HashMap::new(),
            state: initial_state,
        }
    }

    /// Current synthetic processor snapshot word.
    #[must_use]
    pub fn state(&self) -> u64 {
        self.state
    }

    /// Overwrite the synthetic processor snapshot word.
    pub fn set_state(&mut self, state: u64) {
        self.state = state;
    }

    /// Run a source invocation, converting a matching retirement into `Retired`.
    pub fn run<T>(
        &mut self,
        current: impl Fn() -> bool + 'static,
        invoke: impl FnOnce(&mut Self) -> Result<T, InvocationError>,
    ) -> Result<NativeModInvocationResult<T>, InvocationError> {
        let before = self.state;
        self.scopes.push(Scope {
            current: Rc::new(current),
        });
        let depth = self.scopes.len();
        let outcome = invoke(self);
        let result = match outcome {
            Ok(value) => Ok(NativeModInvocationResult::Completed(value)),
            Err(InvocationError::Retired(token)) => {
                let matched = self.retired.get(&token).is_some_and(|retirement| {
                    if (retirement.authority_current)() {
                        return false;
                    }
                    let target = self
                        .scopes
                        .iter()
                        .position(|scope| !(scope.current)())
                        .unwrap_or(retirement.scope);
                    target == depth - 1
                });
                if matched {
                    self.retired.remove(&token);
                    self.state = before;
                    Ok(NativeModInvocationResult::Retired)
                } else {
                    Err(InvocationError::Retired(token))
                }
            }
            Err(other) => Err(other),
        };
        let popped = self.scopes.pop();
        debug_assert!(popped.is_some() && self.scopes.len() + 1 == depth);
        if self.scopes.is_empty() {
            self.retired.clear();
        }
        result
    }

    /// Guard donor-region work: entering a stale region retires the invocation.
    pub fn guard<T>(
        &mut self,
        authority: &RegionAuthority,
        invoke: impl FnOnce(&mut Self) -> Result<T, InvocationError>,
    ) -> Result<T, InvocationError> {
        let Some(scope) = self.scopes.len().checked_sub(1) else {
            return Err(InvocationError::Failed(
                "native donor has no owning source invocation".to_string(),
            ));
        };
        self.retired.insert(
            authority.token,
            Retirement {
                authority_current: authority.current.clone(),
                scope,
            },
        );
        if !authority.is_current() {
            return Err(InvocationError::Retired(authority.token));
        }
        match invoke(self) {
            Err(InvocationError::Retired(token))
                if token == authority.token && !authority.is_current() =>
            {
                Err(InvocationError::Retired(token))
            }
            outcome => {
                self.retired.remove(&authority.token);
                outcome
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn completed_run_preserves_state_and_retired_run_restores_it() {
        let mut invocations = NativeModInvocations::new(0xAAAA);
        let completed = invocations.run(|| true, |tracker| {
            tracker.set_state(0xBBBB);
            Ok::<_, InvocationError>(7)
        });
        assert_eq!(
            completed,
            Ok(NativeModInvocationResult::Completed(7))
        );
        assert_eq!(invocations.state(), 0xBBBB);

        let live = Rc::new(Cell::new(true));
        let probe = live.clone();
        let authority = RegionAuthority::new(3, move || probe.get());
        let retired = invocations.run(|| true, |tracker| {
            tracker.set_state(0xCCCC);
            tracker.guard(&authority, |_| {
                live.set(false);
                Err::<(), _>(InvocationError::Retired(3))
            })
        });
        assert_eq!(retired, Ok(NativeModInvocationResult::Retired));
        assert_eq!(invocations.state(), 0xBBBB);
    }

    #[test]
    fn retirement_routes_to_the_innermost_stale_scope() {
        let mut invocations = NativeModInvocations::new(1);
        let live = Rc::new(Cell::new(true));
        let probe = live.clone();
        let authority = RegionAuthority::new(9, move || probe.get());
        let outer = invocations.run(|| true, |tracker| {
            let inner = tracker.run(|| false, |tracker| {
                tracker.guard(&authority, |_| {
                    live.set(false);
                    Err::<u32, _>(InvocationError::Retired(9))
                })
            })?;
            assert_eq!(inner, NativeModInvocationResult::Retired);
            Ok::<_, InvocationError>(11u32)
        });
        assert_eq!(outer, Ok(NativeModInvocationResult::Completed(11)));
    }

    #[test]
    fn guard_without_scope_fails_and_foreign_errors_propagate() {
        let mut invocations = NativeModInvocations::new(0);
        let authority = RegionAuthority::new(1, || true);
        let missing = invocations.guard(&authority, |_| Ok::<_, InvocationError>(()));
        assert!(matches!(missing, Err(InvocationError::Failed(_))));
        let live = Rc::new(Cell::new(true));
        let probe = live.clone();
        let stale = RegionAuthority::new(2, move || probe.get());
        let outcome: Result<NativeModInvocationResult<()>, _> =
            invocations.run(|| true, |tracker| {
                tracker.guard(&stale, |_| {
                    Err(InvocationError::Failed("boom".to_string()))
                })?;
                Ok(())
            });
        assert!(matches!(outcome, Err(InvocationError::Failed(_))));
    }
}
