//! Window servicing during loading operations.
//!
//! Donor provenance: `src/app/bootstrap/loading.ts` (`serviceLoading`).
//!
//! Sync adaptation: the donor runs an async operation concurrently with a
//! window-service pump, rendezvousing through promise resolvers and an idle
//! heartbeat timer so native events keep flowing while a guest slice owns
//! the window. The sync port runs the operation inline on the calling thread:
//! every `next_frame()` call pumps `service()` once, and `service()` is also
//! pumped once up front so the window is serviced at least once even when the
//! operation never yields (the donor's loop likewise always services at
//! least once). Operation results and errors pass through unchanged.

use thiserror::Error;

/// Loading-pump error (reserved for API symmetry).
///
/// The synchronous pump performs no fallible work itself: the operation's own
/// `Result` carries success and failure, and panics propagate naturally.
#[derive(Debug, Error)]
pub enum LoadingError {
    /// Reserved: the synchronous pump is infallible.
    #[error("loading pump is infallible")]
    Infallible,
}

/// Keep the native window serviced while a loading operation owns it.
///
/// The operation may call `next_frame()` whenever it would yield; each call
/// pumps `service()` once.
pub fn service_loading<T, E>(
    operation: impl FnOnce(&mut dyn FnMut()) -> Result<T, E>,
    service: &mut dyn FnMut(),
) -> Result<T, E> {
    service();
    let mut next_frame = || service();
    operation(&mut next_frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_pump_service_and_result_passes_through() {
        let mut pumps = 0;
        let mut service = || pumps += 1;
        let value: Result<i32, String> = service_loading(
            |next| {
                next();
                next();
                next();
                Ok(7)
            },
            &mut service,
        );
        assert_eq!(value, Ok(7));
        assert_eq!(pumps, 4);
    }

    #[test]
    fn errors_pass_through_and_idle_operation_pumps_once() {
        let mut pumps = 0;
        let mut service = || pumps += 1;
        let value: Result<i32, String> = service_loading(|_| Err("failed".to_string()), &mut service);
        assert_eq!(value, Err("failed".to_string()));
        assert_eq!(pumps, 1);
    }
}
