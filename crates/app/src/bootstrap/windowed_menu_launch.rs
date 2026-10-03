//! Windowed menu launch queue and controller pump.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts` (`StartupApplication`
//! menu wiring and `step`). The donor wires the menu's `play`, `playPreset`,
//! and `load` callbacks to queue a pending source change (`this.pending =
//! { kind: "play" }`, and friends), which the next step consumes by
//! resolving the selection model and opening the game client borrowed on the
//! same renderer. [`MenuLaunchQueue`] is that pending slot: the windowed
//! menu pushes [`StartupAction`] values from its button callbacks, and the
//! windowed backend drains the queue at the top of its frame, resolving
//! launch options through [`launch_options`] and swapping the menu overlay
//! for the scene path. The controller half mirrors `./input.ts`
//! (`ApplicationInput.pump`): window events flow through
//! [`InputRouter::handle_platform`](qa_client::input::router::InputRouter::handle_platform)
//! while [`pump_windowed_controllers`] polls [`SdlControllers`] and feeds
//! each [`ControllerEvent`] to
//! [`InputRouter::handle_controller`](qa_client::input::router::InputRouter::handle_controller),
//! so gamepad navigation reaches the same seat input queue as the keyboard.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use qa_client::input::router::InputRouter;
use qa_platform::controller::ControllerEvent;
use qa_platform::controller::SdlControllers;

use super::startup::StartupAction;
use super::startup_selection::StartupSelectionModel;
use crate::options::ApplicationOptions;

/// Pending menu launch requests (donor `this.pending`, queued).
#[derive(Clone, Default)]
pub struct MenuLaunchQueue {
    actions: Rc<RefCell<VecDeque<StartupAction>>>,
}

impl MenuLaunchQueue {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue one launch action (donor `play`/`playPreset`/`load` tails).
    pub fn push(&self, action: StartupAction) {
        self.actions.borrow_mut().push_back(action);
    }

    /// Queued action count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.borrow().len()
    }

    /// Whether the queue holds no actions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.borrow().is_empty()
    }

    /// Drain every queued action in order (donor `step` consumption).
    pub fn drain(&self) -> Vec<StartupAction> {
        self.actions.borrow_mut().drain(..).collect()
    }
}

/// Resolve one launch action to windowed launch options (donor `resolve`
/// tail in `step`, through the windowed composition's own launch semantic:
/// the Run entry loads its world straight from options, so `play` and
/// `load` read the draft options and let the world loader validate
/// content; `load` relaunches the draft world because this build has no
/// save subsystem yet. `preset` keeps the full donor path and fails
/// honestly where the smoke collaborators cannot resolve presets).
pub fn launch_options(model: &mut StartupSelectionModel, action: &StartupAction) -> Result<ApplicationOptions, String> {
    match action {
        StartupAction::Play | StartupAction::Load { .. } => model.options().map_err(|error| error.to_string()),
        StartupAction::Preset { id, skill, arena_map } => model
            .resolve_preset(id, Some(*skill), arena_map.as_deref())
            .map(|launch| launch.options)
            .map_err(|error| error.to_string()),
        other => Err(format!("launching from the menu cannot start {other:?} in this build")),
    }
}

/// Poll SDL controllers once and route every event to the seat router
/// (donor `ApplicationInput.pump` controller half).
pub fn pump_windowed_controllers(controllers: &mut SdlControllers, router: &mut InputRouter) -> Result<(), String> {
    let events: Vec<ControllerEvent> = controllers.poll_events().map_err(|error| error.to_string())?;
    for event in events {
        router.handle_controller(event).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_content::catalog::CatalogProduct;
    use qa_content::catalog::InstalledCatalog;
    use qa_content::catalog::ProductAvailability;
    use qa_content::catalog::ProductExpectation;
    use qa_content::contract::ContentId;
    use qa_content::contract::GameFamily;

    use super::super::windowed::WindowedCollaborators;
    use super::*;

    fn model() -> StartupSelectionModel {
        let catalog = InstalledCatalog::new(
            "menu-launch-test".to_string(),
            vec![CatalogProduct {
                id: ContentId("q2-classic-baseq2".to_string()),
                expectation: ProductExpectation {
                    id: "q2-classic-baseq2".to_string(),
                    family: GameFamily::Q2,
                    edition: "classic".to_string(),
                    campaign: "baseq2".to_string(),
                    title: "Quake II".to_string(),
                    content_directory: "baseq2".to_string(),
                    base_product: None,
                    required_content_archives: Vec::new(),
                    required_programs: Vec::new(),
                    map_witness: None,
                    unresolved_reason: None,
                },
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            }],
            Vec::new(),
            0,
            None,
        )
        .unwrap();
        StartupSelectionModel::new(catalog, ApplicationOptions::default(), Box::new(WindowedCollaborators)).unwrap()
    }

    #[test]
    fn queue_pushes_and_drains_in_order() {
        let queue = MenuLaunchQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        queue.push(StartupAction::Play);
        queue.push(StartupAction::Preset {
            id: "q2-baseq2".to_string(),
            skill: 1,
            arena_map: None,
        });
        assert_eq!(queue.len(), 2);
        let drained = queue.drain();
        assert_eq!(
            drained,
            vec![
                StartupAction::Play,
                StartupAction::Preset {
                    id: "q2-baseq2".to_string(),
                    skill: 1,
                    arena_map: None,
                },
            ]
        );
        assert!(queue.is_empty());
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn queue_is_shared_across_clones() {
        let queue = MenuLaunchQueue::new();
        let shared = queue.clone();
        shared.push(StartupAction::Play);
        assert_eq!(queue.drain(), vec![StartupAction::Play]);
    }

    #[test]
    fn play_resolves_the_draft_options() {
        let mut owned = model();
        let options = launch_options(&mut owned, &StartupAction::Play).unwrap();
        assert_eq!(options.product, "q2-classic-baseq2");
        assert_eq!(options.map, "maps/base1.bsp");
        let load = launch_options(
            &mut owned,
            &StartupAction::Load {
                path: "save0".to_string(),
                source_product: None,
            },
        )
        .unwrap();
        assert_eq!(load.product, "q2-classic-baseq2");
    }

    #[test]
    fn unknown_preset_is_a_launch_error() {
        let mut owned = model();
        let error = launch_options(
            &mut owned,
            &StartupAction::Preset {
                id: "no-such-preset".to_string(),
                skill: 1,
                arena_map: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("no-such-preset"), "unexpected error: {error}");
    }

    #[test]
    fn non_launch_actions_are_rejected() {
        let mut owned = model();
        let error = launch_options(&mut owned, &StartupAction::Frontend).unwrap_err();
        assert!(error.contains("Frontend"), "unexpected error: {error}");
    }
}
