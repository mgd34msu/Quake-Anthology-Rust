use qa_console::{
    cvars::{Cvars, View},
    views::{Context, Role},
};
use qa_core::primitives::RuleSetId;
use qa_render::scene::ViewPolicy;

/// Bind once for the presentation role; copy current values at view submission.
#[derive(Clone, Copy)]
pub(crate) struct ViewPolicyViews {
    source: RuleSetId,
    views: [View; 5],
}
impl ViewPolicyViews {
    pub(crate) fn bind(vars: &Cvars, source: RuleSetId) -> Result<Self, &'static str> {
        let context = Context {
            source,
            role: Role::Engine,
            ..Context::default()
        };
        let [no_vis, no_cull, world, entities, near] = [
            "r_novis",
            "r_nocull",
            "r_drawworld",
            "r_drawentities",
            "r_znear",
        ]
        .map(|name| vars.bind(name, context));
        Ok(Self {
            source,
            views: [
                no_vis.ok_or("missing visibility setting")?,
                no_cull.ok_or("missing bounds culling setting")?,
                world.ok_or("missing world draw setting")?,
                entities.ok_or("missing entity draw setting")?,
                near.ok_or("missing near plane setting")?,
            ],
        })
    }

    pub(crate) fn read(
        self,
        vars: &Cvars,
    ) -> Result<(ViewPolicy, f32), qa_console::conversion::Error> {
        let [no_vis, no_cull, world, entities, near] = self.views;
        let enabled = |view| {
            if matches!(self.source, RuleSetId::Quake3 | RuleSetId::Quake2Rerelease) {
                vars.integer_view(view).map(|value| value != 0)
            } else {
                vars.numeric(view).map(|value| value != 0.0)
            }
        };
        let ignore_pvs = enabled(no_vis)?;
        Ok((
            ViewPolicy {
                ignore_pvs,
                ignore_areas: ignore_pvs && self.source == RuleSetId::Quake3,
                cull_bounds: !enabled(no_cull)?,
                draw_world: enabled(world)?,
                draw_entities: enabled(entities)?,
            },
            vars.numeric(near)?,
        ))
    }
}
