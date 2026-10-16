use super::{Backend, Context, Kind, VideoPolicy};
use crate::host::LiveFrame;
use qa_core::presentation::RendererConfiguration;
use qa_platform::Window;
use qa_render::{
    Assets, Limits,
    assets::AssetCounts,
    cpu::{CpuBackend, RasterBands},
};
use std::sync::Arc;

/// A cold replacement owns its resources until publication or rejection. The
/// active renderer retains the scene, effects and every registered asset.
pub struct PreparedVideo {
    window: Option<Window>,
    backend: Option<Backend>,
    configuration: Arc<RendererConfiguration>,
    kind: Kind,
}
impl PreparedVideo {
    fn load(
        window: Option<Window>,
        context: Context,
        assets: &Assets,
        limits: Limits,
        bands: Option<RasterBands>,
    ) -> Result<Self, String> {
        let configuration = Arc::clone(context.configuration());
        let kind = match &context {
            Context::Cpu(_) => Kind::Cpu,
            Context::Gl(_) => Kind::Gl,
        };
        let backend = Backend::load(
            context,
            configuration.display.width,
            configuration.display.height,
            assets,
            limits,
            bands,
        )?;
        Ok(Self {
            window,
            backend: Some(backend),
            configuration,
            kind,
        })
    }
    /// Offscreen clients use the same CPU backend loader without an SDL window.
    pub fn cpu(
        configuration: Arc<RendererConfiguration>,
        assets: &Assets,
        limits: Limits,
        bands: Option<RasterBands>,
    ) -> Result<Self, String> {
        Self::load(None, Context::Cpu(configuration), assets, limits, bands)
    }
    pub fn configuration(&self) -> &Arc<RendererConfiguration> {
        &self.configuration
    }
    pub fn admitted_assets(&self) -> Option<AssetCounts> {
        self.backend.as_ref()?.admitted_assets()
    }
    pub fn prepare_assets(&mut self, assets: &Assets, limits: Limits) -> Result<(), String> {
        if self
            .window
            .as_ref()
            .is_some_and(|window| !window.ensure_gl_current())
        {
            return Err("replacement renderer context is unavailable".into());
        }
        self.backend
            .as_mut()
            .ok_or("replacement renderer was consumed")?
            .prepare_assets(assets, Some(limits))
    }
    /// Transfer an offscreen target to its embedding client's existing owner.
    pub fn into_cpu(mut self) -> Result<(CpuBackend, Arc<RendererConfiguration>), String> {
        if self.window.is_some() {
            return Err("windowed replacement requires window publication".into());
        }
        match self
            .backend
            .take()
            .ok_or("replacement renderer was consumed")?
        {
            Backend::Cpu { backend, dispatch } => {
                drop(dispatch);
                Ok((backend, Arc::clone(&self.configuration)))
            }
            Backend::Gl(backend) => {
                self.backend = Some(Backend::Gl(backend));
                Err("offscreen target is not a CPU renderer".into())
            }
        }
    }
}
impl Drop for PreparedVideo {
    fn drop(&mut self) {
        let current = self.window.as_ref().is_none_or(Window::ensure_gl_current);
        if let Some(backend) = self.backend.take() {
            if current {
                drop(backend);
            } else {
                backend.context_lost();
            }
        }
        // Window/context destruction follows backend retirement. Even after a
        // failed context switch it reclaims the context's GPU objects.
    }
}

pub(crate) fn prepare_live_video(
    source: &mut LiveFrame<'_>,
    policy: VideoPolicy,
    assets: &Assets,
    limits: Limits,
) -> Result<PreparedVideo, String> {
    let result = (|| {
        let renderer = source
            .renderer
            .as_deref()
            .ok_or("missing active renderer")?;
        let kind = policy.kind.unwrap_or(renderer.kind);
        let window = kind.open(policy.window)?;
        let context = Context::load(kind, &window)?;
        PreparedVideo::load(Some(window), context, assets, limits, policy.cpu_bands)
    })();
    if let Err(restore) = source.window.restore_video_state() {
        let preparation = match result {
            Ok(candidate) => {
                drop(candidate);
                None
            }
            Err(error) => Some(error),
        };
        let mut error = preparation.map_or_else(
            || format!("active renderer state could not be restored: {restore}"),
            |error| format!("{error}; active renderer state could not be restored: {restore}"),
        );
        if let Err(retry) = source.window.restore_video_state() {
            error.push_str(&format!("; restoration after candidate cleanup: {retry}"));
        }
        return Err(error);
    }
    result
}

pub(crate) fn discard_live_video(
    source: &mut LiveFrame<'_>,
    candidate: PreparedVideo,
) -> Result<(), String> {
    drop(candidate);
    source.window.restore_video_state()
}

pub(crate) fn publish_live_video(
    source: &mut LiveFrame<'_>,
    mut candidate: PreparedVideo,
) -> Result<Arc<RendererConfiguration>, String> {
    let result = (|| {
        let renderer = source
            .renderer
            .as_deref_mut()
            .ok_or("missing active renderer")?;
        let window = candidate
            .window
            .as_ref()
            .ok_or("replacement has no window")?;
        if candidate.backend.is_none() {
            return Err("replacement renderer was consumed".into());
        }
        window.restore_video_state()?;
        let backend = candidate
            .backend
            .take()
            .ok_or("replacement renderer was consumed")?;
        let mut window = candidate.window.take().ok_or("replacement has no window")?;
        window.handoff_input();
        let old_backend = std::mem::replace(&mut renderer.backend, backend);
        let old_window = std::mem::replace(source.window, window);
        renderer.kind = candidate.kind;
        renderer.width = candidate.configuration.display.width;
        renderer.height = candidate.configuration.display.height;
        renderer.particle_presentation = renderer
            .particle_presentation
            .for_target(renderer.width, renderer.height);
        renderer
            .effects
            .resize_target(renderer.width, renderer.height);
        renderer.sample = Default::default();
        renderer.draw_failed = false;
        // The old context is retired immediately, so it reclaims GPU objects.
        // Drop Rust storage without calling old GL names in the new context.
        old_backend.context_lost();
        drop(old_window);
        Ok(Arc::clone(&candidate.configuration))
    })();
    drop(candidate);
    match result {
        Ok(configuration) => Ok(configuration),
        Err(error) => match source.window.restore_video_state() {
            Ok(()) => Err(error),
            Err(restore) => Err(format!("{error}; active renderer restoration: {restore}")),
        },
    }
}
