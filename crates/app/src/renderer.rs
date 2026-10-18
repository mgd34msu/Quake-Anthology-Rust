//! Client composition uses the same scene packets for either renderer.
pub mod console;
pub mod hud;
pub(crate) mod native;
pub mod text;
mod video;
mod view_policy;
use crate::model_assets::ModelProfile;
use crate::{
    client_scene::{ClientSceneInput, SceneCounts},
    host::ClientView,
};
use qa_core::{math::angle_vectors, sys_events::SeatId};
use qa_platform::{Stopwatch, Window, WorkerError, Workers};
use qa_render::shader::{
    reference::{NativeShaders, ShaderView},
    registration::ShaderOwner,
};
use qa_render::{
    Assets, BackendStats, BlendPhase, FrontEnd, Limits, Refdef, Viewport,
    cpu::{CpuBackend, CpuLimits, RasterBands, RasterConfig, run_cpu_job},
    gl::{GlBackend, GlContext},
    material::world_load::LoadedWorld,
    world::WorldView,
};
pub use video::PreparedVideo;
pub(crate) use video::{discard_live_video, prepare_live_video, publish_live_video};
pub(crate) use view_policy::ViewPolicyViews;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use qa_platform::allocations::Counts;
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use qa_render::cpu::MAX_BANDS;

pub fn parse_cpu_bands(value: &str) -> Result<RasterBands, &'static str> {
    value
        .parse()
        .ok()
        .and_then(RasterBands::from_count)
        .ok_or("CPU bands must be 1, 2, 4 or 8")
}

/// Resolve once at renderer load. A benchmark override takes precedence over
/// the shared cvar; None requests affinity-aware automatic selection.
pub fn cpu_band_setting(
    cvars: &qa_console::cvars::Cvars,
    benchmark: Option<RasterBands>,
) -> Result<(qa_core::primitives::CvarHandle, Option<RasterBands>), &'static str> {
    let handle = cvars.find("r_cpuBands").ok_or("missing CPU band setting")?;
    let selected = if benchmark.is_some() {
        benchmark
    } else {
        configured_cpu_bands(cvars.integer(handle))?
    };
    Ok((handle, selected))
}

fn configured_cpu_bands(value: i32) -> Result<Option<RasterBands>, &'static str> {
    if value == 0 {
        Ok(None)
    } else {
        usize::try_from(value)
            .ok()
            .and_then(RasterBands::from_count)
            .map(Some)
            .ok_or("r_cpuBands must be 0, 1, 2, 4 or 8")
    }
}

/// Preview the one capability latch without changing current values or revisions.
pub fn pending_cpu_band_setting(
    cvars: &qa_console::cvars::Cvars,
    view: qa_console::cvars::View,
    benchmark: Option<RasterBands>,
) -> Result<Option<RasterBands>, &'static str> {
    if benchmark.is_some() {
        return Ok(benchmark);
    }
    let value = match cvars
        .latched(view)
        .map_err(|_| "CPU band setting conversion")?
    {
        Some(text) => qa_core::text::atoi_prefix(text.as_str().as_bytes()),
        None => cvars
            .integer_view(view)
            .map_err(|_| "CPU band setting conversion")?,
    };
    configured_cpu_bands(value)
}

/// The app owns worker scheduling; render lends scoped preparation or raster jobs.
/// Developer retail draws use this same dispatcher and allocation accounting.
pub struct CpuDispatch {
    workers: Workers,
    failure: Option<WorkerError>,
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    scratch: [Counts; MAX_BANDS],
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pending: Counts,
}
impl CpuDispatch {
    pub fn load(bands: RasterBands) -> Result<Self, WorkerError> {
        let workers = Workers::load(bands.count() - 1)?;
        Ok(Self {
            workers,
            failure: None,
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            scratch: [Counts::default(); MAX_BANDS],
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            pending: Counts::default(),
        })
    }

    pub fn worker_count(&self) -> usize {
        self.workers.count()
    }

    pub fn failure(&self) -> Option<WorkerError> {
        self.failure
    }

    pub fn dispatch<J: Send>(
        &mut self,
        jobs: &mut [J],
        work: fn(&mut J),
    ) -> Result<(), WorkerError> {
        let result = self.workers.dispatch_scoped(jobs, work);
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        let result = {
            // Capture each completed or rejected batch before another dispatch.
            // The caller's participating jobs remain in its outer frame count.
            let counts = &mut self.scratch[..self.workers.count()];
            let count_result = self.workers.allocation_counts(counts);
            if count_result.is_ok() {
                for count in counts {
                    self.pending += *count;
                }
            }
            result.and(count_result)
        };
        if let Err(error) = result {
            self.failure.get_or_insert(error);
        }
        result
    }

    /// Consume every dispatch since the preceding merge, exactly once.
    /// Inline jobs already belong to the caller's thread-local counts.
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pub fn merge_counts(&mut self, mut caller: Counts) -> Counts {
        caller += std::mem::take(&mut self.pending);
        caller
    }
}

/// Cache sizes describe the backend's load-time reservation, across all bands.
pub fn report_cpu_config(config: RasterConfig, workers: usize) {
    qa_console::logger::console(format_args!(
        "{{\"event\":\"cpu_raster_config\",\"bands\":{},\"workers\":{},\"total_cache_budget_bytes\":{},\"allocated_cache_bytes\":{},\"per_band_cache_bytes\":{},\"mandatory_cache_bytes\":{},\"loaded_mip_working_set_bytes\":{},\"bin_index_capacity_bytes\":{},\"mip_layout_metadata_bytes\":{},\"span_group_capacity_bytes\":{},\"preparation_capacity_bytes\":{},\"material_capacity_bytes\":{},\"view_effect_capacity_bytes\":{},\"prepare_minimum_primitives_per_job\":{},\"worker_affinity\":\"inherited_process_cpu_mask\",\"cpu_affinity_source\":\"private_harness_metadata\",\"individual_worker_pinning\":false}}\n",
        config.bands.count(),
        workers,
        config.total_cache_budget_bytes,
        config.allocated_cache_bytes,
        config.per_band_cache_bytes,
        config.mandatory_cache_bytes,
        config.loaded_mip_working_set_bytes,
        config.bin_index_capacity_bytes,
        config.mip_layout_metadata_bytes,
        config.span_group_capacity_bytes,
        config.preparation_capacity_bytes,
        config.material_capacity_bytes,
        config.view_effect_capacity_bytes,
        config.prepare_minimum_primitives_per_job,
    ));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Cpu,
    Gl,
}
impl Kind {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "cpu" => Ok(Self::Cpu),
            "gl" => Ok(Self::Gl),
            _ => Err("renderer must be cpu or gl"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gl => "gl",
        }
    }
    pub fn open(self, request: qa_core::presentation::WindowRequest) -> Result<Window, String> {
        Window::open_request(request, matches!(self, Self::Gl))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VideoPolicy {
    pub kind: Option<Kind>,
    pub cpu_bands: Option<RasterBands>,
    pub window: qa_core::presentation::WindowRequest,
}

/// Cold context and capability owner; it becomes the only live backend after
/// CLIENT registration finishes, without resolving the GL table a second time.
pub enum Context {
    Cpu(std::sync::Arc<qa_core::presentation::RendererConfiguration>),
    Gl(GlContext),
}
impl Context {
    pub fn load(kind: Kind, window: &Window) -> Result<Self, String> {
        let display = window.display_configuration()?;
        match kind {
            Kind::Cpu => {
                let mut configuration = qa_core::presentation::RendererConfiguration {
                    display,
                    max_texture_size: qa_render::assets::upload::MAX_DIMENSION,
                    max_active_textures: 1,
                    texture_env_add: true,
                    ..Default::default()
                };
                configuration
                    .renderer
                    .set("Quake Anthology CPU edge/span")
                    .map_err(|_| "CPU identity capacity")?;
                configuration
                    .vendor
                    .set("Quake Anthology")
                    .map_err(|_| "CPU identity capacity")?;
                configuration
                    .version
                    .set("CPU")
                    .map_err(|_| "CPU identity capacity")?;
                Ok(Self::Cpu(std::sync::Arc::new(configuration)))
            }
            Kind::Gl => {
                // SAFETY: Window owns this thread's current SDL GL context and
                // outlives Context and the consuming backend.
                Ok(Self::Gl(unsafe {
                    GlContext::load(|name| window.gl_proc(name), display)?
                }))
            }
        }
    }
    pub fn configuration(&self) -> &std::sync::Arc<qa_core::presentation::RendererConfiguration> {
        match self {
            Self::Cpu(configuration) => configuration,
            Self::Gl(context) => context.configuration(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub frontend_ns: u64,
    pub backend_ns: u64,
    pub present_ns: u64,
    pub stats: BackendStats,
    pub presented: bool,
    pub visible_surfaces: u32,
    pub client_scene: SceneCounts,
    pub console: console::ConsoleStats,
    pub hud: hud::HudStats,
}
#[expect(
    clippy::large_enum_variant,
    reason = "Keep load-owned backend layout inline; changing indirection requires separate measurement"
)]
enum Backend {
    Cpu {
        backend: CpuBackend,
        dispatch: CpuDispatch,
    },
    Gl(GlBackend),
}

impl Backend {
    fn load(
        context: Context,
        width: u32,
        height: u32,
        assets: &Assets,
        limits: Limits,
        bands: Option<RasterBands>,
    ) -> Result<Self, String> {
        let backend = match context {
            Context::Cpu(_) => {
                let physical_cores = qa_platform::physical_core_count();
                let backend = CpuBackend::load_with_limits(
                    width,
                    height,
                    assets,
                    CpuLimits {
                        scene: limits,
                        bands: bands.unwrap_or_else(|| RasterBands::at_most(physical_cores)),
                        auto_bands: bands.is_none(),
                        ..CpuLimits::default()
                    },
                )?;
                let dispatch = CpuDispatch::load(backend.raster_config().bands)
                    .map_err(|error| format!("CPU workers: {error}"))?;
                report_cpu_config(backend.raster_config(), dispatch.worker_count());
                qa_console::logger::console(format_args!(
                    "{{\"event\":\"cpu_band_selection\",\"automatic\":{},\"physical_cores\":{},\"bands\":{}}}\n",
                    bands.is_none(),
                    physical_cores,
                    backend.raster_config().bands.count()
                ));
                Backend::Cpu { backend, dispatch }
            }
            Context::Gl(context) => Backend::Gl(unsafe {
                GlBackend::load(context, assets, width, height, limits.vertices)?
            }),
        };
        if let Backend::Gl(gl) = &backend {
            let (renderer, version) = gl.renderer_info();
            qa_console::logger::console(format_args!(
                "{{\"event\":\"gl_context\",\"renderer\":{},\"version\":{}}}\n",
                serde_json::to_string(renderer).map_err(|e| format!("GL renderer report: {e}"))?,
                serde_json::to_string(version).map_err(|e| format!("GL version report: {e}"))?
            ));
        }
        Ok(backend)
    }
    fn prepare_assets(&mut self, assets: &Assets, scene: Option<Limits>) -> Result<(), String> {
        match self {
            Self::Cpu { backend, .. } => {
                backend.prepare_assets(assets, scene).map_err(str::to_owned)
            }
            Self::Gl(backend) => backend.prepare_assets(assets, scene),
        }
    }
    fn admitted_assets(&self) -> Option<qa_render::assets::AssetCounts> {
        match self {
            Self::Cpu { backend, .. } => backend.admitted_assets(),
            Self::Gl(backend) => backend.admitted_assets(),
        }
    }
    fn context_lost(self) {
        if let Self::Gl(backend) = self {
            backend.context_lost();
        }
    }
}

struct WorldScene {
    views: [WorldView; SeatId::COUNT],
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn light_view_entities(
    frame: &mut qa_render::scene::Frame,
    assets: &Assets,
    world: Option<qa_render::world::WorldId>,
    view: &Refdef,
    use_world_lighting: bool,
    scales: [f32; 2],
    mut point_view: Option<&mut WorldView>,
    modulate: f32,
    sun: qa_core::primitives::Vec3,
) {
    let world = world.and_then(|world| assets.world(world));
    let world = world.filter(|_| use_world_lighting);
    let grid = world.and_then(|world| world.light_grid());
    let unlit = qa_render::lighting::unlit_sample(view.identity_light, sun);
    frame.light_entities(view.identity_light, |entity| {
        if let Some(grid) = grid {
            return grid.sample(entity.light_origin(), scales[0], scales[1]);
        }
        world
            .zip(point_view.as_deref_mut())
            .and_then(|(world, scratch)| {
                scratch.point_lighting(world, entity.light_origin(), &view.lightstyles, modulate)
            })
            .unwrap_or(unlit)
    });
}
/// Each CLIENT supplies its selected presentation independently of the world.
pub fn view_refdef(client: ClientView, viewport: Viewport, kind: Kind) -> Refdef {
    let basis = angle_vectors(client.angles);
    let fov = qa_render::scene::fov_from_x(client.fov_x, viewport.width, viewport.height);
    let (frame_timing, skin_timing, sprite_placement) =
        model_presentation(kind, client.presentation_rules);
    let mut refdef = Refdef {
        viewport,
        origin: client.origin,
        axes: basis.axes(),
        fov,
        curve_error: client.curve_error,
        model_lod: client.model_lod,
        model_replacements: client.model_replacements,
        model_time: client.model_time,
        frame_timing,
        skin_timing,
        sprite_placement,
        policy: client.policy,
        near: client.near,
        blend: client.screen_blend,
        damage_blend: client.damage_blend,
        flags: client.flags,
        time_ms: client.time_ms as i64,
        ..Refdef::default()
    };
    apply_presentation(&mut refdef, &client.presentation, kind);
    refdef
}

fn model_presentation(
    kind: Kind,
    rules: qa_core::primitives::RuleSetId,
) -> (
    qa_render::scene::FrameTiming,
    qa_render::scene::SkinTiming,
    qa_render::scene::SpritePlacement,
) {
    if matches!(kind, Kind::Gl)
        && matches!(
            rules,
            qa_core::primitives::RuleSetId::Quake | qa_core::primitives::RuleSetId::QuakeWorld
        )
    {
        (
            qa_render::scene::FrameTiming::UniformFirst,
            qa_render::scene::SkinTiming::LegacyFour,
            qa_render::scene::SpritePlacement::OrientedOnly,
        )
    } else {
        (
            qa_render::scene::FrameTiming::Cumulative,
            qa_render::scene::SkinTiming::Cumulative,
            qa_render::scene::SpritePlacement::Full,
        )
    }
}

fn apply_presentation(
    refdef: &mut Refdef,
    presentation: &qa_render::material::world_load::PresentationDefaults,
    kind: Kind,
) {
    presentation.apply(refdef);
    if matches!(kind, Kind::Gl) {
        refdef.software_waterwarp = false;
        refdef.blend_phase = BlendPhase::AfterView;
        refdef.palette_transform = None;
    }
}
#[derive(Clone, Copy)]
pub(crate) struct GraphicRegistration<'a> {
    pub profile: ModelProfile,
    pub files: &'a qa_content::vfs::FileSearch,
    pub registration: &'a qa_render::ShaderRegistration,
}
impl<'a> GraphicRegistration<'a> {
    pub fn matches(self, profile: ModelProfile, files: &qa_content::vfs::FileSearch) -> bool {
        self.profile == profile && self.files.view().same_selection(files.view())
    }
    pub fn entry(
        self,
    ) -> (
        Option<crate::model_assets::ShaderResource>,
        &'a [qa_render::ShaderRegistration],
    ) {
        (
            Some(crate::model_assets::ShaderResource::Graphic(
                self.registration.key,
            )),
            std::slice::from_ref(self.registration),
        )
    }
}

const HUD_DEFAULTS: usize = qa_core::primitives::RuleSetId::ALL.len();
const HUD_SLOTS: usize = HUD_DEFAULTS + SeatId::COUNT;
type HudTable = [Option<hud::HudAssets>; HUD_SLOTS];

fn hud_slot(rules: qa_core::primitives::RuleSetId, seat: Option<SeatId>) -> usize {
    seat.map_or(rules as usize, |seat| HUD_DEFAULTS + seat.index())
}

/// The session owns one front end and its view scratch across module dispatch
/// and built-in client composition.
pub struct SceneComposer {
    frontend: FrontEnd,
    scenes: Vec<Option<WorldScene>>,
    frame: Option<qa_render::scene::Frame>,
    paused: Option<PausedScene>,
    publisher: Option<qa_core::primitives::ModuleId>,
    native_owned: [bool; SeatId::COUNT],
    graphics_owners: [Option<ShaderOwner>; SeatId::COUNT],
    native_axes: [Option<[qa_core::primitives::Vec3; 3]>; SeatId::COUNT],
    kind: Kind,
    beam: qa_render::ShaderBindingId,
    beam_alpha: qa_render::ShaderBindingId,
    console: Option<console::ConsoleAssets>,
    hud: HudTable,
    ui_commands: usize,
}

struct PausedScene {
    frame: Option<qa_render::scene::Frame>,
    publisher: Option<qa_core::primitives::ModuleId>,
    native_owned: [bool; SeatId::COUNT],
    graphics_owners: [Option<ShaderOwner>; SeatId::COUNT],
    native_axes: [Option<[qa_core::primitives::Vec3; 3]>; SeatId::COUNT],
}

fn stage_scene_capacity<'a>(
    frontend: &'a mut FrontEnd,
    frame: &'a mut Option<qa_render::scene::Frame>,
    paused: &'a mut Option<PausedScene>,
    limits: Limits,
) -> Result<qa_render::scene::FrontEndCapacity<'a>, &'static str> {
    frontend.stage_capacity(
        limits,
        [
            frame.as_mut(),
            paused.as_mut().and_then(|scene| scene.frame.as_mut()),
        ],
    )
}

/// Cold staging retains the existing scene owner until backend preparation
/// succeeds. Dropping this value publishes no world or arena replacement.
pub struct SceneWorldAppend<'a> {
    frontend: qa_render::scene::FrontEndCapacity<'a>,
    append: Option<(&'a mut Vec<Option<WorldScene>>, WorldScene)>,
}
impl SceneWorldAppend<'_> {
    pub fn limits(&self) -> Limits {
        self.frontend.limits()
    }

    pub fn commit(self) {
        self.frontend.commit();
        if let Some((scenes, scene)) = self.append {
            scenes.push(Some(scene));
        }
    }
}

/// A cold HUD candidate occupies the existing role slot only on commit.
pub struct SceneHudAppend<'a> {
    frontend: qa_render::scene::FrontEndCapacity<'a>,
    target: &'a mut HudTable,
    candidates: Vec<(usize, hud::HudUpdate)>,
    console: Option<(&'a mut console::ConsoleAssets, console::ConsoleUpdate)>,
    ui_commands: &'a mut usize,
    reserved_ui_commands: usize,
}
impl SceneHudAppend<'_> {
    pub fn limits(&self) -> Limits {
        self.frontend.limits()
    }
    pub(crate) fn registrations(&self) -> impl Iterator<Item = GraphicRegistration<'_>> + Clone {
        self.candidates
            .iter()
            .flat_map(|(_, update)| update.registrations())
            .chain(
                self.console
                    .iter()
                    .flat_map(|(_, update)| update.registrations()),
            )
    }
    pub fn commit(self) {
        self.frontend.commit();
        if let Some((console, update)) = self.console {
            update.commit(console);
        }
        for (slot, candidate) in self.candidates {
            candidate.commit(&mut self.target[slot]);
        }
        *self.ui_commands = self.reserved_ui_commands;
    }
}

pub(crate) struct VideoGraphics {
    console: Option<console::ConsoleAssets>,
    hud: HudTable,
}

/// Replacement UI and capacity remain private until the video candidate commits.
pub(crate) struct SceneVideoAppend<'a> {
    frontend: qa_render::scene::FrontEndCapacity<'a>,
    kind: &'a mut Kind,
    console: &'a mut Option<console::ConsoleAssets>,
    hud: &'a mut HudTable,
    graphics: VideoGraphics,
    ui_commands: &'a mut usize,
    reserved_ui_commands: usize,
}
impl SceneVideoAppend<'_> {
    pub(crate) fn limits(&self) -> Limits {
        self.frontend.limits()
    }
    pub(crate) fn graphic_registrations(
        &self,
    ) -> impl Iterator<Item = GraphicRegistration<'_>> + Clone {
        graphic_registrations(&self.graphics.console, &self.graphics.hud)
    }
    pub(crate) fn commit(self, kind: Kind) {
        self.frontend.commit();
        *self.kind = kind;
        *self.console = self.graphics.console;
        *self.hud = self.graphics.hud;
        *self.ui_commands = self.reserved_ui_commands;
    }
}

fn graphic_registrations<'a>(
    console: &'a Option<console::ConsoleAssets>,
    hud: &'a HudTable,
) -> impl Iterator<Item = GraphicRegistration<'a>> + Clone {
    console
        .iter()
        .flat_map(|console| {
            console
                .registrations()
                .map(move |registration| console.font().registered(registration))
        })
        .chain(hud.iter().flatten().flat_map(|hud| {
            hud.registrations()
                .map(move |registration| hud.font().registered(registration))
        }))
}

fn validate_world_presentation(
    assets: &Assets,
    world: qa_render::world::WorldId,
    entry: qa_render::material::world_load::LoadedWorldPresentation,
) -> Result<(), String> {
    if assets
        .world_presentation_registration(entry.id)
        .is_none_or(|registered| registered != entry)
        || assets
            .world(world)
            .is_none_or(|world| !world.presentations().contains(&entry.id))
    {
        return Err("invalid appended world presentation".to_owned());
    }
    Ok(())
}

fn hud_command_capacity() -> Result<usize, String> {
    hud::HudAssets::command_capacity()
        .checked_mul(SeatId::COUNT)
        .ok_or_else(|| "HUD scene capacity".to_owned())
}
fn ui_command_capacity(
    console: Option<&console::ConsoleAssets>,
    hud: bool,
) -> Result<usize, String> {
    if console.is_none() && !hud {
        return Ok(0);
    }
    console
        .map_or(0, console::ConsoleAssets::command_capacity)
        .checked_add(hud_command_capacity()?)
        .ok_or_else(|| "UI scene capacity".to_owned())
}

fn scene_ui_limits(
    mut limits: Limits,
    old: usize,
    required: usize,
) -> Result<(Limits, usize), String> {
    // Frontend arenas keep their high-water capacity across smaller windows.
    // Replace the recorded reservation so repeated resizes cannot add it twice.
    let commands = old.max(required);
    limits.commands = limits
        .commands
        .checked_sub(old)
        .and_then(|base| base.checked_add(commands))
        .ok_or("UI command capacity")?;
    limits.vertices = limits
        .vertices
        .checked_sub(old.checked_mul(6).ok_or("UI vertex capacity")?)
        .and_then(|base| base.checked_add(commands.checked_mul(6)?))
        .ok_or("UI vertex capacity")?;
    Ok((limits, commands))
}

fn scene_surface_limits(mut limits: Limits, surfaces: usize) -> Result<Limits, String> {
    limits.surfaces = limits.surfaces.max(
        surfaces
            .checked_mul(SeatId::COUNT)
            .ok_or("scene surface capacity")?,
    );
    Ok(limits)
}

impl SceneComposer {
    pub(crate) fn stage_model_choices(
        &mut self,
        assets: &Assets,
        required: Option<Limits>,
        choices: usize,
    ) -> Result<qa_render::scene::FrontEndCapacity<'_>, String> {
        let limits = required
            .unwrap_or(self.frontend.limits())
            .for_assets(assets)?
            .for_model_choices(choices)?;
        stage_scene_capacity(
            &mut self.frontend,
            &mut self.frame,
            &mut self.paused,
            limits,
        )
        .map_err(str::to_owned)
    }
    pub fn limits(&self) -> Limits {
        self.frontend.limits()
    }
    pub(crate) fn has_world(&self, world: qa_render::world::WorldId) -> bool {
        self.scenes
            .get(world.0 as usize)
            .is_some_and(Option::is_some)
    }

    pub fn asset_stage_ready(&mut self) -> bool {
        self.frame.is_none()
            && self
                .frontend
                .stage_capacity(self.frontend.limits(), [None, None])
                .is_ok()
    }

    fn hud_index(
        &self,
        rules: qa_core::primitives::RuleSetId,
        seat: Option<SeatId>,
        source: Option<qa_core::primitives::FileSearchId>,
    ) -> Option<usize> {
        [
            seat.map(|seat| hud_slot(rules, Some(seat))),
            Some(hud_slot(rules, None)),
        ]
        .into_iter()
        .flatten()
        .find(|&slot| {
            self.hud[slot]
                .as_ref()
                .is_some_and(|hud| hud.rules() == rules && hud.font_source() == source)
        })
    }
    pub fn has_hud(
        &self,
        rules: qa_core::primitives::RuleSetId,
        seat: Option<SeatId>,
        source: Option<qa_core::primitives::FileSearchId>,
    ) -> bool {
        self.hud_index(rules, seat, source).is_some()
    }
    pub fn hud_font(
        &self,
        rules: qa_core::primitives::RuleSetId,
        seat: Option<SeatId>,
        source: Option<qa_core::primitives::FileSearchId>,
    ) -> Option<std::sync::Arc<console::FontAssets>> {
        self.hud_index(rules, seat, source)
            .and_then(|slot| self.hud[slot].as_ref().map(|hud| hud.font_assets()))
            .or_else(|| {
                self.console
                    .as_ref()?
                    .font_for(rules)
                    .filter(|font| font.files.id() == source)
            })
    }
    pub(crate) fn graphics_owners(&mut self, owners: [Option<ShaderOwner>; SeatId::COUNT]) {
        self.graphics_owners = owners;
    }
    pub(crate) fn graphic_sources(
        &self,
    ) -> impl Iterator<Item = (ModelProfile, &qa_content::vfs::FileSearch)> + Clone {
        self.console
            .iter()
            .map(|console| (console.profile(), &console.font().files))
            .chain(
                self.hud
                    .iter()
                    .flatten()
                    .map(|hud| (hud.profile(), &hud.font().files)),
            )
    }
    pub(crate) fn graphic_profiles(&self) -> impl Iterator<Item = ModelProfile> + Clone {
        self.console
            .iter()
            .map(console::ConsoleAssets::profile)
            .chain(self.hud.iter().enumerate().filter_map(|(index, hud)| {
                let profile = hud.as_ref()?.profile();
                (!self
                    .console
                    .as_ref()
                    .is_some_and(|console| console.profile() == profile)
                    && !self.hud[..index]
                        .iter()
                        .flatten()
                        .any(|previous| previous.profile() == profile))
                .then_some(profile)
            }))
    }
    pub(crate) fn graphic_registrations(
        &self,
    ) -> impl Iterator<Item = GraphicRegistration<'_>> + Clone {
        graphic_registrations(&self.console, &self.hud)
    }
    pub(crate) fn replay_graphics(
        &self,
        contexts: &mut crate::modules::renderer_contexts::Contexts,
        handle: qa_render::shader::reference::ShaderContextId,
    ) -> Result<(), String> {
        contexts.replay_graphics(handle, self.graphic_registrations())
    }
    pub fn append_hud(
        &mut self,
        frame: &mut qa_render::scene::Frame,
        assets: &Assets,
        view: hud::HudView<'_>,
        refdef: Refdef,
        native: &dyn NativeShaders,
        owner: ShaderOwner,
    ) -> hud::HudStats {
        let slot = self.hud_index(view.rules, view.seat, view.file_source);
        match slot.and_then(|slot| self.hud[slot].as_mut()) {
            Some(hud) => {
                let profile = hud.profile();
                let context = native
                    .context(owner, profile, hud.font_source())
                    .or_else(|| native.context(ShaderOwner::Engine, profile, hud.font_source()));
                hud.append(frame, assets, view, refdef, ShaderView { native, context })
            }
            None => hud::HudStats {
                missing_font: u32::from(view.show_text),
                ..Default::default()
            },
        }
    }
    pub fn append_console(
        &mut self,
        frame: &mut qa_render::scene::Frame,
        assets: &Assets,
        view: qa_console::presentation::ConsoleView<'_>,
        width: u32,
        height: u32,
        software: bool,
        native: &dyn NativeShaders,
    ) -> console::ConsoleStats {
        let authority = view
            .editor
            .seat()
            .and_then(|seat| self.graphics_owners[seat.index()]);
        self.console
            .as_mut()
            .map_or_else(Default::default, |console| {
                let profile = console.profile();
                let owner = authority
                    .filter(|&owner| {
                        native
                            .context(owner, profile, console.font().files.id())
                            .is_some()
                    })
                    .unwrap_or(ShaderOwner::Engine);
                let context = native.context(owner, profile, console.font().files.id());
                console.append(
                    frame,
                    assets,
                    view,
                    width,
                    height,
                    software,
                    ShaderView { native, context },
                )
            })
    }

    pub fn stage_hud(
        &mut self,
        candidate: hud::HudAssets,
        seat: Option<SeatId>,
    ) -> Result<SceneHudAppend<'_>, String> {
        if self.frame.is_some() {
            return Err("scene frame is active".to_owned());
        }
        let (limits, reserved_ui_commands) = scene_ui_limits(
            self.frontend.limits(),
            self.ui_commands,
            ui_command_capacity(self.console.as_ref(), true)?,
        )?;
        let frontend = stage_scene_capacity(
            &mut self.frontend,
            &mut self.frame,
            &mut self.paused,
            limits,
        )?;
        Ok(SceneHudAppend {
            frontend,
            target: &mut self.hud,
            candidates: vec![(
                hud_slot(candidate.rules(), seat),
                hud::HudUpdate::Replace(candidate),
            )],
            console: None,
            ui_commands: &mut self.ui_commands,
            reserved_ui_commands,
        })
    }

    pub(crate) fn stage_downloaded_hud(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        owner: &mut crate::model_assets::ModelAssets,
        path: &[u8],
        models_changed: bool,
    ) -> Result<Option<SceneHudAppend<'_>>, String> {
        let console = self
            .console
            .as_ref()
            .map(|console| console.downloaded(vfs, owner, path))
            .transpose()?
            .flatten();
        let mut candidates = Vec::new();
        for (slot, hud) in self
            .hud
            .iter()
            .enumerate()
            .filter_map(|(slot, hud)| hud.as_ref().map(|hud| (slot, hud)))
        {
            let font = console
                .as_ref()
                .and_then(|update| update.font_for(hud.profile(), &hud.font().files));
            if let Some(candidate) = hud.downloaded(vfs, owner, path, font)? {
                candidates.push((slot, candidate));
            }
        }
        if candidates.is_empty() && console.is_none() && !models_changed {
            return Ok(None);
        }
        let limits = self
            .frontend
            .limits()
            .for_assets(owner.assets())?
            .for_model_choices(owner.catalog().max_choice_nodes())?;
        let frontend = stage_scene_capacity(
            &mut self.frontend,
            &mut self.frame,
            &mut self.paused,
            limits,
        )?;
        let reserved_ui_commands = self.ui_commands;
        Ok(Some(SceneHudAppend {
            frontend,
            target: &mut self.hud,
            candidates,
            console: self.console.as_mut().zip(console),
            ui_commands: &mut self.ui_commands,
            reserved_ui_commands,
        }))
    }

    pub(crate) fn load_video_graphics(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        owner: &mut crate::model_assets::ModelAssets,
        refresh: &crate::model_assets::ImageRefresh,
        ui_fullscreen: bool,
        width: u32,
        height: u32,
    ) -> Result<VideoGraphics, String> {
        if !self.asset_stage_ready() {
            return Err("video graphics require an idle scene".to_owned());
        }
        let console = self
            .console
            .as_ref()
            .map(|previous| {
                let profile = refresh.profile(previous.profile());
                let candidate = console::ConsoleAssets::load(
                    vfs,
                    owner,
                    profile.rules,
                    profile.image_settings,
                    profile.finish.compile(ui_fullscreen),
                    width,
                    height,
                )?;
                if candidate.profile() != profile {
                    return Err("video console profile changed source".to_owned());
                }
                Ok(candidate)
            })
            .transpose()?;
        let mut hud = std::array::from_fn(|_| None);
        for (slot, previous) in self.hud.iter().enumerate() {
            let Some(previous) = previous else { continue };
            let profile = refresh.profile(previous.profile());
            let font = console
                .as_ref()
                .and_then(|console| console.font_for(profile.rules))
                .filter(|font| {
                    font.profile == profile
                        && font.files.view().same_selection(previous.file_search())
                })
                .map_or_else(
                    || {
                        console::FontAssets::load(
                            vfs,
                            owner,
                            profile.rules,
                            profile.image_settings,
                            profile.finish.compile(ui_fullscreen),
                            Some(previous.file_search()),
                        )
                    },
                    Ok,
                )?;
            if font.profile != profile {
                return Err("video HUD profile changed source".to_owned());
            }
            hud[slot] = Some(hud::HudAssets::load(
                vfs,
                owner,
                previous.rules(),
                font,
                width,
            )?);
        }
        Ok(VideoGraphics { console, hud })
    }

    pub(crate) fn stage_video_graphics(
        &mut self,
        assets: &Assets,
        graphics: VideoGraphics,
        refresh: &crate::model_assets::ImageRefresh,
        model_choices: usize,
    ) -> Result<SceneVideoAppend<'_>, String> {
        if self.frame.is_some() {
            return Err("scene frame is active".to_owned());
        }
        if self.console.is_some() != graphics.console.is_some()
            || self
                .hud
                .iter()
                .zip(&graphics.hud)
                .any(|(old, new)| old.is_some() != new.is_some())
        {
            return Err("video graphics changed scene layout".to_owned());
        }
        let (mut limits, reserved_ui_commands) = scene_ui_limits(
            self.frontend.limits(),
            self.ui_commands,
            ui_command_capacity(
                graphics.console.as_ref(),
                graphics.hud.iter().any(Option::is_some),
            )?,
        )?;
        for &(world, previous, next) in &refresh.worlds {
            if !self.has_world(world) {
                return Err("video world has no scene scratch".to_owned());
            }
            let old = assets
                .world_presentation_registration(previous)
                .ok_or("video world presentation is unavailable")?;
            validate_world_presentation(assets, world, old)?;
            validate_world_presentation(assets, world, next)?;
            let world = assets.world(world).ok_or("missing video world")?;
            limits = scene_surface_limits(limits, world.visibility().surface_count())?;
        }
        limits = limits
            .for_assets(assets)?
            .for_model_choices(model_choices)?;
        let frontend = stage_scene_capacity(
            &mut self.frontend,
            &mut self.frame,
            &mut self.paused,
            limits,
        )?;
        Ok(SceneVideoAppend {
            frontend,
            kind: &mut self.kind,
            console: &mut self.console,
            hud: &mut self.hud,
            graphics,
            ui_commands: &mut self.ui_commands,
            reserved_ui_commands,
        })
    }

    pub fn load(
        mut limits: Limits,
        assets: &mut Assets,
        worlds: Vec<LoadedWorld>,
    ) -> Result<(Self, Limits), String> {
        let mut scenes: Vec<Option<WorldScene>> =
            (0..assets.worlds().len()).map(|_| None).collect();
        let mut max_surfaces = 0usize;
        for loaded in worlds {
            assets.register_world_presentation_metadata(
                loaded.world,
                &loaded.presentations,
                None,
            )?;
            let world = assets.world(loaded.world).ok_or("missing loaded world")?;
            let index = loaded.world.0 as usize;
            if scenes[index].is_some() {
                return Err("duplicate world geometry".into());
            }
            for diagnostic in &loaded.diagnostics {
                qa_console::logger::console(format_args!(
                    "world {}: {diagnostic}\n",
                    loaded.world.0
                ));
            }
            max_surfaces = max_surfaces.max(world.visibility().surface_count());
            scenes[index] = Some(WorldScene {
                views: std::array::from_fn(|_| WorldView::load(world)),
            });
        }
        if scenes.iter().any(Option::is_none) {
            return Err("missing loaded world geometry".into());
        }
        limits = scene_surface_limits(limits, max_surfaces)?;
        limits = limits.for_assets(assets)?;

        let frontend = FrontEnd::load(limits)?;
        let beam = qa_render::entities::register_beam_shader(
            assets,
            qa_render::shader::StageBlend {
                source: qa_render::shader::BlendFactor::One,
                destination: qa_render::shader::BlendFactor::One,
            },
        )
        .map_err(str::to_owned)?;
        let beam_alpha =
            qa_render::entities::register_beam_shader(assets, qa_render::shader::StageBlend::ALPHA)
                .map_err(str::to_owned)?;
        Ok((
            Self {
                frontend,
                scenes,
                frame: None,
                paused: None,
                publisher: None,
                native_owned: [false; SeatId::COUNT],
                native_axes: [None; SeatId::COUNT],
                graphics_owners: [None; SeatId::COUNT],
                kind: Kind::Cpu,
                beam,
                beam_alpha,
                console: None,
                hud: std::array::from_fn(|_| None),
                ui_commands: 0,
            },
            limits,
        ))
    }

    pub fn stage_world<'a>(
        &'a mut self,
        assets: &Assets,
        loaded: &LoadedWorld,
    ) -> Result<SceneWorldAppend<'a>, String> {
        if self.frame.is_some() {
            return Err("scene frame is active".to_owned());
        }
        let append_world = loaded.world.0 as usize == self.scenes.len();
        if if append_world {
            self.scenes.len().checked_add(1) != Some(assets.worlds().len())
        } else {
            self.scenes.len() != assets.worlds().len()
                || self
                    .scenes
                    .get(loaded.world.0 as usize)
                    .and_then(Option::as_ref)
                    .is_none()
        } {
            return Err("scene world prefix does not match appended world".to_owned());
        }
        let world = assets.world(loaded.world).ok_or("missing appended world")?;
        if loaded.presentations.is_empty() {
            return Err("invalid appended world presentation".to_owned());
        }
        for &entry in &loaded.presentations {
            validate_world_presentation(assets, loaded.world, entry)?;
        }
        let limits =
            scene_surface_limits(self.frontend.limits(), world.visibility().surface_count())?
                .for_assets(assets)?;
        let frontend = stage_scene_capacity(
            &mut self.frontend,
            &mut self.frame,
            &mut self.paused,
            limits,
        )?;
        let append = if append_world {
            self.scenes
                .try_reserve(1)
                .map_err(|_| "scene world capacity".to_owned())?;
            Some((
                &mut self.scenes,
                WorldScene {
                    views: std::array::from_fn(|_| WorldView::load(world)),
                },
            ))
        } else {
            None
        };
        Ok(SceneWorldAppend { frontend, append })
    }

    pub fn frame_mut(&mut self) -> Option<&mut qa_render::scene::Frame> {
        self.frame.as_mut()
    }

    pub fn finish_frame(&mut self) -> Option<qa_render::scene::CommandList> {
        self.frame.take().map(qa_render::scene::Frame::finish)
    }

    pub fn recycle(
        &mut self,
        packet: qa_render::scene::CommandList,
    ) -> Result<(), qa_render::scene::CommandList> {
        self.frontend.recycle(packet)
    }

    pub(crate) fn screen_active(&self) -> bool {
        self.paused.is_some()
    }

    pub(crate) fn begin_screen(&mut self) -> bool {
        if self.screen_active() {
            return false;
        }
        let Some(frame) = self.frontend.begin_frame(None) else {
            return false;
        };
        self.paused = Some(PausedScene {
            frame: self.frame.replace(frame),
            publisher: self.publisher.take(),
            native_owned: self.native_owned,
            graphics_owners: self.graphics_owners,
            native_axes: self.native_axes,
        });
        self.native_axes.fill(None);
        true
    }

    pub(crate) fn end_screen(&mut self) {
        let Some(paused) = self.paused.take() else {
            return;
        };
        self.discard_frame();
        self.frame = paused.frame;
        self.publisher = paused.publisher;
        self.native_owned = paused.native_owned;
        self.graphics_owners = paused.graphics_owners;
        self.native_axes = paused.native_axes;
    }

    pub fn begin_frame(&mut self) -> bool {
        self.discard_frame();
        self.publisher = None;
        self.native_axes.fill(None);
        self.frame = self.frontend.begin_frame([18, 26, 34, 255]);
        self.frame.is_some()
    }
    pub(crate) fn native_owners(&mut self, owners: [bool; SeatId::COUNT]) {
        self.native_owned = owners;
    }
    pub(crate) fn submitted_axes(&self, seat: usize) -> Option<[qa_core::primitives::Vec3; 3]> {
        self.native_axes[seat]
    }

    /// A headless client phase still retires its owned list before reuse.
    pub fn discard_frame(&mut self) {
        if let Some(frame) = self.frame.take() {
            let _ = self.frontend.recycle(frame.finish());
        }
    }
}

pub struct Renderer {
    effects: qa_render::effects::Effects,
    particle_shader: qa_render::ShaderBindingId,
    particle_presentation: qa_render::scene::ParticlePresentation,
    effect_clients: [Option<qa_render::effects::EffectClient>; SeatId::COUNT],
    effect_time: qa_core::sys_events::EventTime,
    effect_gravity: f32,
    backend: Backend,
    kind: Kind,
    pub sample: Sample,
    width: u32,
    height: u32,
    recorded_rendered_frames: u64,
    draw_failed: bool,
    lightstyles: [hud::LightStyles; SeatId::COUNT],
}
/// A short cold borrow; implementations prepare the complete resource prefix.
/// Success retains every existing resource and admits all supplied additions.
pub trait AssetPreparer {
    fn target_size(&self) -> Option<[u32; 2]> {
        None
    }
    fn prepare_assets(&mut self, assets: &Assets, scene: Option<Limits>) -> Result<(), String>;
    fn prepare_model_assets(&mut self, assets: &Assets, _per_entity: usize) -> Result<(), String> {
        self.prepare_assets(assets, None)
    }
}
impl AssetPreparer for Renderer {
    fn target_size(&self) -> Option<[u32; 2]> {
        Some([self.width, self.height])
    }
    fn prepare_assets(&mut self, assets: &Assets, scene: Option<Limits>) -> Result<(), String> {
        self.backend.prepare_assets(assets, scene)
    }
}
impl Renderer {
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn admitted_assets(&self) -> Option<qa_render::assets::AssetCounts> {
        self.backend.admitted_assets()
    }

    /// Backend resources borrow the one owner during registration and drawing.
    pub fn load(
        context: Context,
        width: u32,
        height: u32,
        assets: &mut Assets,
        worlds: Vec<LoadedWorld>,
        bands: Option<RasterBands>,
        console: console::ConsoleAssets,
        hud: [Option<hud::HudAssets>; HUD_DEFAULTS],
        mut effects: qa_render::effects::Effects,
        model_choices: usize,
    ) -> Result<(Self, SceneComposer), String> {
        let kind = match &context {
            Context::Cpu(_) => Kind::Cpu,
            Context::Gl(_) => Kind::Gl,
        };
        let display = context.configuration().display;
        if display.width != width || display.height != height {
            return Err("renderer target changed during registration".into());
        }
        let particle_shader =
            qa_render::particles::register_point_shader(assets, "engine/particles/point")?;
        let mut limits = Limits::default();
        limits.particles = effects
            .capacity()
            .checked_mul(SeatId::COUNT)
            .ok_or("particle scene capacity")?;
        let (limits, ui_commands) =
            scene_ui_limits(limits, 0, ui_command_capacity(Some(&console), true)?)?;
        let limits = limits.for_model_choices(model_choices)?;
        let (mut scene, limits) = SceneComposer::load(limits, assets, worlds)?;
        scene.kind = kind;
        scene.console = Some(console);
        for (target, source) in scene.hud.iter_mut().zip(hud) {
            *target = source;
        }
        scene.ui_commands = ui_commands;
        let backend = Backend::load(context, width, height, assets, limits, bands)?;
        effects.resize_target(width, height);
        Ok((
            Self {
                effects,
                particle_shader,
                particle_presentation: qa_render::scene::ParticlePresentation::default()
                    .for_target(width, height),
                effect_clients: [None; SeatId::COUNT],
                effect_time: qa_core::sys_events::EventTime::default(),
                effect_gravity: 800.0,
                backend,
                kind,
                sample: Sample::default(),
                width,
                height,
                recorded_rendered_frames: 0,
                draw_failed: false,
                lightstyles: std::array::from_fn(|_| Default::default()),
            },
            scene,
        ))
    }
    pub fn effect(
        &mut self,
        palette: Option<qa_render::PaletteId>,
        event: qa_core::primitives::EffectEvent,
    ) -> bool {
        if let Some(palette) = palette {
            self.effects.bind_missing(
                event.effect,
                qa_render::effects::ParticleBinding {
                    palette,
                    shader: self.particle_shader,
                    presentation: self.particle_presentation,
                },
            );
        }
        self.effects.accept(event)
    }
    pub fn set_effect_clients(
        &mut self,
        clients: [Option<qa_render::effects::EffectClient>; SeatId::COUNT],
        time: qa_core::sys_events::EventTime,
        gravity: f32,
    ) {
        self.effects.synchronize(&clients);
        self.effect_clients = clients;
        self.effect_time = time;
        self.effect_gravity = gravity;
    }
    pub fn reset_client_effects(
        &mut self,
        client: qa_core::primitives::ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) {
        self.effects.reset_client(client, lifetime);
    }

    fn effect_phase(
        &mut self,
        action: fn(
            &mut qa_render::effects::Effects,
            qa_core::primitives::ClientId,
            qa_core::primitives::EntityId,
            qa_core::sys_events::EventTime,
            f32,
        ) -> bool,
    ) {
        for (seat, client) in self.effect_clients.iter().enumerate() {
            let Some(client) = client else { continue };
            if self.effect_clients[..seat].contains(&Some(*client)) {
                continue;
            }
            action(
                &mut self.effects,
                client.client,
                client.lifetime,
                self.effect_time,
                self.effect_gravity,
            );
        }
    }
    /// Last backend-rendered frame workload, with lifetime cache counters.
    pub fn cpu_world_stats(&self) -> Option<qa_render::cpu::WorldStats> {
        match &self.backend {
            Backend::Cpu { backend, .. } => Some(backend.world_stats()),
            Backend::Gl(_) => None,
        }
    }
    /// Includes startup and warmup; failed front-end acquisitions do not count.
    pub fn recorded_rendered_frames(&self) -> u64 {
        self.recorded_rendered_frames
    }
    pub fn worker_count(&self) -> usize {
        match &self.backend {
            Backend::Cpu { dispatch, .. } => dispatch.worker_count(),
            Backend::Gl(_) => 0,
        }
    }
    pub fn worker_error(&self) -> Option<WorkerError> {
        match &self.backend {
            Backend::Cpu { dispatch, .. } => dispatch.failure(),
            Backend::Gl(_) => None,
        }
    }
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pub fn merge_worker_counts(&mut self, caller: Counts) -> Counts {
        match &mut self.backend {
            Backend::Cpu { dispatch, .. } => dispatch.merge_counts(caller),
            Backend::Gl(_) => caller,
        }
    }
    pub fn frame(
        &mut self,
        composition: &mut SceneComposer,
        resources: crate::model_assets::ModelView<'_>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        views: &[Option<ClientView>; SeatId::COUNT],
        scenes: &[Option<ClientSceneInput<'_>>; SeatId::COUNT],
        hud: &[Option<hud::HudView<'_>>; SeatId::COUNT],
        console: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.sample = Sample::default();
        self.draw_failed = false;
        self.effect_phase(qa_render::effects::Effects::begin_frame);
        let timer = Stopwatch::start();
        if composition.frame.is_none() {
            composition.begin_frame();
        }
        let Some(mut frame) = composition.frame.take() else {
            self.sample.stats.rejected += 1;
            self.draw_failed = true;
            self.effect_phase(qa_render::effects::Effects::finish_frame);
            return;
        };
        let count = views.iter().filter(|view| view.is_some()).count();
        let mut view_index = 0;
        let mut rejected = 0;
        frame.append_client_effects(&mut self.effects, resources.assets);
        frame.background(|frame| {
            for (seat, client) in views.iter().enumerate() {
                let Some(client) = client else {
                    continue;
                };
                let viewport = viewport(self.width, self.height, view_index, count);
                view_index += 1;
                if composition.native_owned[seat] {
                    rejected += u32::from(composition.native_axes[seat].is_none());
                    continue;
                }
                let Some(scene) = composition
                    .scenes
                    .get_mut(client.world.0 as usize)
                    .and_then(Option::as_mut)
                else {
                    rejected += 1;
                    continue;
                };
                let Some(world) = resources.assets.world(client.world) else {
                    rejected += 1;
                    continue;
                };
                let Some(presentation) = resources.world_presentation(
                    client.world,
                    None,
                    Some(client.world_presentation),
                    None,
                ) else {
                    rejected += 1;
                    continue;
                };
                let mut refdef = view_refdef(*client, viewport, self.kind);
                if let Some(hud) = hud[seat] {
                    self.sample.hud +=
                        hud.apply_lightstyles(&mut refdef, &mut self.lightstyles[seat]);
                }
                // Native snapshot areas use at most 255 bytes. This bounded stack
                // scratch is shared by visibility and the copied scene command.
                let mut area_mask = [0; 256];
                let hidden_areas = if let Some(client_scene) = &scenes[seat] {
                    let (mask, truncated) = client_scene.hidden_areas(&mut area_mask);
                    self.sample.client_scene.unsupported += u32::from(truncated);
                    mask
                } else {
                    &[]
                };
                let no_world = refdef
                    .flags
                    .contains(qa_core::primitives::PlayerViewFlags::NO_WORLD_MODEL);
                let surfaces = if no_world {
                    &[][..]
                } else {
                    let Ok(surfaces) = scene.views[seat].query(world, refdef, None, hidden_areas)
                    else {
                        rejected += 1;
                        continue;
                    };
                    surfaces
                };
                self.sample.visible_surfaces += surfaces.len() as u32;
                frame.clear_scene();
                let world_added =
                    no_world || frame.add_world(client.world, client.world_presentation, surfaces);
                if let Some(client_scene) = &scenes[seat] {
                    let mut random = || self.effects.random_word();
                    self.sample.client_scene += client_scene.append(
                        frame,
                        resources.assets,
                        &refdef,
                        matches!(self.kind, Kind::Cpu),
                        Some(crate::client_scene::BeamInput {
                            shader: composition.beam_alpha,
                            random: &mut random,
                        }),
                    );
                }
                if let Some(effect_client) = self.effect_clients[seat] {
                    self.effects
                        .append(frame, effect_client.client, effect_client.lifetime);
                }
                light_view_entities(
                    frame,
                    resources.assets,
                    Some(client.world),
                    &refdef,
                    !no_world,
                    client.lighting_scales,
                    Some(&mut scene.views[seat]),
                    presentation.profile.q2_modulate,
                    resources
                        .assets
                        .world_presentation(presentation.id)
                        .map_or_else(qa_render::lighting::default_sun_direction, |presentation| {
                            presentation.sun_direction()
                        }),
                );
                let rendered = if no_world {
                    frame.render_scene(
                        refdef,
                        hidden_areas,
                        resources.assets,
                        qa_render::shader::reference::ShaderView::direct(),
                    )
                } else {
                    frame.render_world_scene(
                        client.world,
                        client.world_presentation,
                        refdef,
                        hidden_areas,
                        &mut scene.views[seat],
                        resources.assets,
                        qa_render::shader::reference::ShaderView::direct(),
                    )
                };
                if !world_added || !rendered {
                    rejected += 1;
                }
                if let Some(hud) = hud[seat] {
                    self.sample.hud += composition.append_hud(
                        frame,
                        resources.assets,
                        hud,
                        refdef,
                        native,
                        ShaderOwner::Client(hud.client, hud.client_lifetime),
                    );
                }
            }
        });
        self.sample.console = composition.append_console(
            &mut frame,
            resources.assets,
            console,
            self.width,
            self.height,
            matches!(self.kind, Kind::Cpu),
            native,
        );
        rejected += self.sample.console.rejected;
        rejected += self.sample.hud.rejected;
        self.submit_scene(composition, resources, native, frame, &timer, rejected);
        self.effect_phase(qa_render::effects::Effects::finish_frame);
    }

    pub(crate) fn refresh_scene(
        &mut self,
        composition: &mut SceneComposer,
        resources: crate::model_assets::ModelView<'_>,
        native: &dyn qa_render::shader::reference::NativeShaders,
    ) -> bool {
        self.sample = Sample::default();
        self.draw_failed = false;
        let timer = Stopwatch::start();
        if let Some(frame) = composition.frame.take() {
            self.submit_scene(composition, resources, native, frame, &timer, 0);
        } else {
            self.sample.stats.rejected += 1;
            self.draw_failed = true;
        }
        !self.draw_failed
    }

    fn submit_scene(
        &mut self,
        composition: &mut SceneComposer,
        resources: crate::model_assets::ModelView<'_>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        frame: qa_render::scene::Frame,
        timer: &Stopwatch,
        rejected: u32,
    ) {
        let mut packet = frame.finish();
        packet.resolve_shaders(resources.assets, native);
        self.sample.frontend_ns = timer.elapsed().as_nanos() as u64;
        let timer = Stopwatch::start();
        let result = match &mut self.backend {
            Backend::Cpu { backend, dispatch } => {
                backend.render_with_dispatch(&packet, resources.assets, |jobs| {
                    dispatch.dispatch(jobs, run_cpu_job)
                })
            }
            Backend::Gl(gl) => Ok(gl.render(&packet, resources.assets)),
        };
        self.sample.backend_ns = timer.elapsed().as_nanos() as u64;
        match result {
            Ok(stats) => {
                self.sample.stats = stats;
                self.recorded_rendered_frames += 1;
            }
            Err(_) => {
                self.sample.stats.rejected += 1;
                self.draw_failed = true;
            }
        }
        self.sample.stats.rejected += rejected;
        if composition.recycle(packet).is_err() {
            self.sample.stats.rejected += 1;
        }
    }
    pub fn present(&mut self, window: &mut Window) {
        if self.draw_failed {
            self.sample.presented = false;
            self.sample.present_ns = 0;
            return;
        }
        let timer = Stopwatch::start();
        self.sample.presented = match &self.backend {
            Backend::Cpu { backend, .. } => {
                window.present_pixels(self.width, self.height, backend.pixels())
            }
            Backend::Gl(_) => window.present_gl(),
        };
        self.sample.present_ns = timer.elapsed().as_nanos() as u64;
    }
}

pub(crate) fn viewport(width: u32, height: u32, view: usize, count: usize) -> Viewport {
    let columns = if count > 1 { 2 } else { 1 };
    let rows = if count > 2 { 2 } else { 1 };
    let x = view as u32 % columns;
    let y = view as u32 / columns;
    let left = width * x / columns;
    let top = height * y / rows;
    Viewport {
        x: left,
        y: top,
        width: width * (x + 1) / columns - left,
        height: height * (y + 1) / rows - top,
    }
}
