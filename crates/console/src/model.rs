use crate::{
    conversion,
    cvars::{Cvars, View},
    views::{Context, Role},
};
use qa_core::primitives::RuleSetId;

/// Cached controls shared by cold module registration and copied scene views.
#[derive(Clone, Copy, Debug)]
pub struct ModelHandles {
    pub enhanced: View,
    pub load: View,
    pub usage: View,
    pub distance: View,
    pub limit: View,
}
impl ModelHandles {
    pub fn for_context(self, vars: &Cvars, context: Context) -> Option<Self> {
        let [enhanced, load, usage, distance, limit] = vars.value_views(
            [
                self.enhanced,
                self.load,
                self.usage,
                self.distance,
                self.limit,
            ],
            context,
        )?;
        Some(Self {
            enhanced,
            load,
            usage,
            distance,
            limit,
        })
    }

    pub fn bind(cvars: &Cvars) -> Result<Self, &'static str> {
        let bind = |name, source| {
            cvars
                .bind(
                    name,
                    Context {
                        source,
                        role: Role::Engine,
                        ..Context::default()
                    },
                )
                .ok_or("missing model control")
        };
        Ok(Self {
            enhanced: bind("r_enhancedmodels", RuleSetId::Quake)?,
            load: bind("gl_md5_load", RuleSetId::Quake2Rerelease)?,
            usage: bind("gl_md5_use", RuleSetId::Quake2Rerelease)?,
            distance: bind("gl_md5_distance", RuleSetId::Quake2Rerelease)?,
            limit: bind("r_model_distance", RuleSetId::Quake)?,
        })
    }

    /// MDL enhancement follows numeric truth; MD2 loading uses native integer truth.
    pub fn replacement_load(self, cvars: &Cvars) -> Result<(bool, bool), conversion::Error> {
        Ok((
            cvars.numeric(self.enhanced)? != 0.0,
            cvars.integer_view(self.load)? != 0,
        ))
    }
}
