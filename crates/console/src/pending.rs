//! One selected settings group stays pending until its cold owner publishes.
use crate::cvars::{Cvars, View, WriteError};

pub struct PendingSettings {
    views: Vec<(View, u64)>,
}
impl PendingSettings {
    pub fn capture(vars: &Cvars, views: impl IntoIterator<Item = View>) -> Self {
        Self {
            views: views
                .into_iter()
                .map(|view| (view, vars.view_update_generation(view)))
                .collect(),
        }
    }
    pub fn unchanged(&self, vars: &Cvars) -> bool {
        self.views
            .iter()
            .all(|&(view, generation)| vars.view_update_generation(view) == generation)
    }
    pub fn apply(&self, vars: &mut Cvars) -> Result<(), WriteError> {
        for (index, &(view, _)) in self.views.iter().enumerate() {
            if self.views[..index]
                .iter()
                .any(|&(previous, _)| previous.canonical() == view.canonical())
            {
                continue;
            }
            vars.apply_latch(view)?;
            vars.set_latch_active(view.canonical(), true);
        }
        Ok(())
    }
}
