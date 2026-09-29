//! Selected bot navigation from `src/bots/behavior/q3/navigation-services.ts`.
//!
//! Wires the selected shared navigation runtime into
//! [`SourceBotNavigation`](super::navigation::SourceBotNavigation).

use crate::behavior::q3::navigation::SourceBotNavigation;
use crate::runtime::NavigationRuntime;

/// Build source bot navigation over a shared runtime.
pub fn q3_bot_navigation(runtime: NavigationRuntime<'_>) -> SourceBotNavigation<'_> {
    SourceBotNavigation::new(runtime)
}
