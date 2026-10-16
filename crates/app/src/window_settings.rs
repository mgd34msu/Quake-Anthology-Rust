//! Cold window settings use the console's shared native mode conversion policy.
use crate::renderer::Kind;
use qa_console::{
    cvars::{Cvars, View, WriteError},
    pending::PendingSettings,
    views::Context,
};
use qa_core::presentation::{DisplayConfiguration, ExclusivePolicy, WindowMode, WindowRequest};

pub struct WindowSettingsPlan {
    request: WindowRequest,
    pending: PendingSettings,
    fullscreen: View,
}
impl WindowSettingsPlan {
    pub fn prepare(
        vars: &Cvars,
        kind: Kind,
        baseline: Option<WindowRequest>,
    ) -> Result<Self, String> {
        let gl = matches!(kind, Kind::Gl);
        let request = vars
            .window_request(true, gl, baseline)
            .map_err(|error| format!("window settings: {error:?}"))?;
        let views = vars
            .window_views(gl)
            .map_err(|error| format!("window settings: {error:?}"))?;
        Ok(Self {
            request,
            pending: PendingSettings::capture(vars, views),
            fullscreen: vars
                .bind("r_fullscreen", Context::default())
                .ok_or("missing fullscreen cvar")?,
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
    pub fn apply(
        &self,
        vars: &mut Cvars,
        display: &DisplayConfiguration,
    ) -> Result<(), WriteError> {
        self.pending.apply(vars)?;
        if self.request.exclusive_policy == ExclusivePolicy::ExactOrWindowed
            && matches!(self.request.mode, WindowMode::Exclusive { .. })
            && !display.fullscreen
        {
            vars.set_text(self.fullscreen.canonical(), "0")?;
        }
        Ok(())
    }
    pub fn report_outcome(
        &self,
        actual: Option<WindowRequest>,
        events: &mut qa_core::events::EventRing,
    ) {
        let Some(actual) = actual else {
            return;
        };
        if matches!(self.request.mode, WindowMode::Exclusive { .. })
            && actual.mode == WindowMode::Windowed
        {
            let _ = events.console_formatted(format_args!(
                "Fullscreen unavailable; using a {}x{} window.\n",
                actual.width, actual.height
            ));
        }
        if self.request.gl.swap_interval != actual.gl.swap_interval {
            let _ = events.console_formatted(format_args!(
                "Swap interval {} unavailable; using {}.\n",
                self.request.gl.swap_interval, actual.gl.swap_interval
            ));
        }
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
            .window_request(true, matches!(kind, Kind::Gl), None)
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
            let result = if name == "r_mode" && current.window_aspect.is_some() {
                vars.force_write(view, &value)
            } else {
                vars.set_text(view.canonical(), &value)
            };
            result.map_err(|error| format!("window override: {error:?}"))?;
        }
    }
    WindowSettingsPlan::prepare(vars, kind, None)
}
