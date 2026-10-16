//! Cold window settings use the console's shared native mode conversion policy.
use crate::renderer::Kind;
use qa_console::{
    cvars::{Cvars, WriteError},
    pending::PendingSettings,
    views::Context,
};
use qa_core::presentation::WindowRequest;

pub struct WindowSettingsPlan {
    request: WindowRequest,
    pending: PendingSettings,
}
impl WindowSettingsPlan {
    pub fn prepare(vars: &Cvars, kind: Kind) -> Result<Self, String> {
        let gl = matches!(kind, Kind::Gl);
        let request = vars
            .window_request(true, gl)
            .map_err(|error| format!("window settings: {error:?}"))?;
        let views = vars
            .window_views(gl)
            .map_err(|error| format!("window settings: {error:?}"))?;
        Ok(Self {
            request,
            pending: PendingSettings::capture(vars, views),
        })
    }
    pub fn request(&self) -> WindowRequest {
        self.request
    }
    pub fn validate(&self, vars: &Cvars) -> Result<(), String> {
        if !self.pending.unchanged(vars) {
            return Err("window settings changed during restart".into());
        }
        Ok(())
    }
    pub fn apply(&self, vars: &mut Cvars) -> Result<(), WriteError> {
        self.pending.apply(vars)
    }
}

/// Command-line dimensions override only the requested axes after saved
/// settings have used the same native conversion policy as the running console.
pub fn startup(
    vars: &mut Cvars,
    kind: Kind,
    width: Option<i32>,
    height: Option<i32>,
) -> Result<WindowSettingsPlan, String> {
    if width.is_some() || height.is_some() {
        let current = vars
            .window_request(true, matches!(kind, Kind::Gl))
            .map_err(|error| format!("window settings: {error:?}"))?;
        let width = width
            .map(|value| u32::try_from(value).map_err(|_| "invalid window width"))
            .transpose()?
            .unwrap_or(current.width);
        let height = height
            .map(|value| u32::try_from(value).map_err(|_| "invalid window height"))
            .transpose()?
            .unwrap_or(current.height);
        for (name, value) in [
            ("r_mode", "-1".to_owned()),
            ("r_customwidth", width.to_string()),
            ("r_customheight", height.to_string()),
        ] {
            let view = vars
                .bind(name, Context::default())
                .ok_or("missing window cvar")?;
            vars.force_write(view, &value)
                .map_err(|error| format!("window override: {error:?}"))?;
        }
    }
    WindowSettingsPlan::prepare(vars, kind)
}
