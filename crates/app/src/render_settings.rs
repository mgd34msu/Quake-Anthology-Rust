//! Cold presentation settings from the one cvar table, selected per world.
use qa_console::{
    cvars::{Cvars, View},
    pending::PendingSettings,
    views::{Context, Role, RuleSetId},
};
use qa_content::vfs::Vfs;
use qa_core::presentation::RendererConfiguration;
use qa_render::{
    Assets,
    assets::{PaletteId, upload},
    material::{
        MaterialFinishPolicy,
        resources::{ImageSearchOrder, ImageSettings, PaletteSource, ResourceError, load_palette},
        world_load::WorldPresentationProfile,
    },
    world::{WorldLighting, WorldRenderSettings},
};

#[derive(Default)]
pub struct PaletteRegistrations {
    ids: [Option<PaletteId>; 5],
}
impl PaletteRegistrations {
    /// Adopt already registered resources at the startup owner boundary.
    pub fn from_registered(ids: [Option<PaletteId>; 5]) -> Self {
        Self { ids }
    }
    pub fn get(&self, rules: RuleSetId) -> Option<PaletteId> {
        self.ids[rules as usize]
    }
    pub(crate) fn checkpoint(&self) -> [Option<PaletteId>; 5] {
        self.ids
    }
    pub(crate) fn restore(&mut self, ids: [Option<PaletteId>; 5]) {
        self.ids = ids;
    }
    pub(crate) fn validate(&self, assets: &Assets) -> bool {
        self.ids
            .iter()
            .flatten()
            .all(|&id| assets.palette(id).is_some())
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.iter().all(Option::is_none)
    }
}

/// Palette resources are registered once for their exact native source, even
/// when multiple source/presentation roles request the same resource.
pub fn palette(
    vfs: &Vfs,
    assets: &mut Assets,
    rules: RuleSetId,
    registered: &mut PaletteRegistrations,
) -> Result<Option<PaletteId>, String> {
    palette_resource(vfs, assets, rules, registered).map_err(|error| format!("palette: {error:?}"))
}

pub(crate) fn palette_resource(
    vfs: &Vfs,
    assets: &mut Assets,
    rules: RuleSetId,
    registered: &mut PaletteRegistrations,
) -> Result<Option<PaletteId>, ResourceError> {
    let Some(source) = PaletteSource::native(rules) else {
        return Ok(None);
    };
    if let Some(id) = registered.get(rules) {
        return Ok(Some(id));
    }
    let id = load_palette(vfs, assets, source)?;
    for rule in RuleSetId::ALL {
        if PaletteSource::native(rule) == Some(source) {
            registered.ids[rule as usize] = Some(id);
        }
    }
    Ok(Some(id))
}

pub fn world_profile(
    vfs: &Vfs,
    assets: &mut Assets,
    vars: &Cvars,
    rules: RuleSetId,
    registered: &mut PaletteRegistrations,
) -> Result<WorldPresentationProfile, String> {
    let mut profile =
        WorldPresentationProfile::native(rules, palette(vfs, assets, rules, registered)?);
    profile.image_settings = image_settings(vars, rules)?;
    profile.finish = material_finish(vars)?;
    Ok(profile)
}

/// World geometry and lighting use one renderer policy, independently of the
/// source map or selected presentation.
pub fn world_load_options(
    vars: &Cvars,
    configuration: Option<&RendererConfiguration>,
) -> Result<qa_render::material::world_load::WorldLoadOptions, &'static str> {
    let settings = read_world_settings(vars, false, configuration)?;
    let mut options = qa_render::material::world_load::WorldLoadOptions::default();
    options.geometry.patch_subdivisions = settings.patch_subdivisions;
    options.lighting = settings.lighting;
    Ok(options)
}

pub(crate) struct WorldSettingsPlan {
    settings: WorldRenderSettings,
    pending: PendingSettings,
}
impl WorldSettingsPlan {
    pub(crate) fn prepare(
        vars: &Cvars,
        configuration: Option<&RendererConfiguration>,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            settings: read_world_settings(vars, true, configuration)?,
            pending: PendingSettings::capture(
                vars,
                ["r_subdivisions", "r_mapOverBrightBits", "r_overBrightBits"]
                    .map(|name| setting_view(vars, RuleSetId::Quake3, name))
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        })
    }
    pub(crate) fn settings(&self) -> WorldRenderSettings {
        self.settings
    }
    pub(crate) fn validate(&self, vars: &Cvars) -> Result<(), &'static str> {
        if self.pending.unchanged(vars) {
            Ok(())
        } else {
            Err("world render settings changed during restart")
        }
    }
    pub(crate) fn apply(&self, vars: &mut Cvars) -> Result<(), qa_console::cvars::WriteError> {
        self.pending.apply(vars)
    }
}
fn read_world_settings(
    vars: &Cvars,
    pending: bool,
    configuration: Option<&RendererConfiguration>,
) -> Result<WorldRenderSettings, &'static str> {
    let integer = |name| {
        setting_numbers(vars, RuleSetId::Quake3, name, pending)?
            .map(|value| value.1)
            .ok_or("missing native world setting default")
    };
    Ok(WorldRenderSettings {
        patch_subdivisions: read_subdivisions(vars, pending)?,
        lighting: WorldLighting::from_requests(
            integer("r_mapOverBrightBits")?,
            integer("r_overBrightBits")?,
            configuration,
        )?,
    })
}
fn read_subdivisions(vars: &Cvars, pending: bool) -> Result<f32, &'static str> {
    let value = setting_numbers(vars, RuleSetId::Quake3, "r_subdivisions", pending)?.map_or_else(
        || qa_render::world::geometry::GeometryOptions::default().patch_subdivisions,
        |value| value.0,
    );
    if value.is_finite() {
        Ok(value)
    } else {
        Err("non-finite subdivision tolerance")
    }
}

const IMAGE_CVARS: [&str; 9] = [
    "r_gamma",
    "r_intensity",
    "r_picmip",
    "r_roundImagesDown",
    "r_simpleMipMaps",
    "gl_max_size",
    "gl_skymip",
    "r_override_textures",
    "r_texture_overrides",
];

/// A cold preview keeps native source defaults and pending alias conversions in
/// the one cvar table until the replacement resources are ready to publish.
pub(crate) struct ResourceSettingsPlan {
    settings: [Option<ImageSettings>; 5],
    finish: MaterialFinishPolicy,
    pending: PendingSettings,
}
impl ResourceSettingsPlan {
    pub(crate) fn prepare(
        vars: &Cvars,
        sources: impl IntoIterator<Item = RuleSetId>,
    ) -> Result<Self, &'static str> {
        let mut settings = [None; 5];
        let mut views = Vec::new();
        for source in sources {
            if settings[source as usize].is_some() {
                continue;
            }
            settings[source as usize] = Some(read_image_settings(vars, source, true)?);
            for name in IMAGE_CVARS {
                let view = setting_view(vars, source, name)?;
                views.push(view);
            }
        }
        for name in MATERIAL_CVARS {
            views.push(setting_view(vars, RuleSetId::Quake3, name)?);
        }
        Ok(Self {
            settings,
            finish: read_material_finish(vars, true)?,
            pending: PendingSettings::capture(vars, views),
        })
    }
    pub(crate) fn selections(&self) -> [Option<ImageSettings>; 5] {
        self.settings
    }
    pub(crate) fn finish(&self) -> MaterialFinishPolicy {
        self.finish
    }
    pub(crate) fn validate(&self, vars: &Cvars) -> Result<(), &'static str> {
        if !self.pending.unchanged(vars) {
            return Err("render resource settings changed during restart");
        }
        Ok(())
    }
    pub(crate) fn apply(&self, vars: &mut Cvars) -> Result<(), qa_console::cvars::WriteError> {
        self.pending.apply(vars)
    }
}

const MATERIAL_CVARS: [&str; 3] = ["r_detailtextures", "r_vertexLight", "r_fullbright"];

pub fn material_finish(vars: &Cvars) -> Result<MaterialFinishPolicy, &'static str> {
    read_material_finish(vars, false)
}

fn read_material_finish(vars: &Cvars, pending: bool) -> Result<MaterialFinishPolicy, &'static str> {
    let defaults = MaterialFinishPolicy::default();
    let enabled = |name, fallback| -> Result<bool, &'static str> {
        Ok(setting_numbers(vars, RuleSetId::Quake3, name, pending)?
            .map_or(fallback, |value| value.1 != 0))
    };
    Ok(MaterialFinishPolicy {
        detail_textures: enabled(MATERIAL_CVARS[0], defaults.detail_textures)?,
        vertex_light: enabled(MATERIAL_CVARS[1], defaults.vertex_light)?,
        fullbright: enabled(MATERIAL_CVARS[2], defaults.fullbright)?,
    })
}
fn setting_view(vars: &Cvars, source: RuleSetId, name: &str) -> Result<View, &'static str> {
    vars.bind(
        name,
        Context {
            source,
            role: Role::Engine,
            ..Context::default()
        },
    )
    .ok_or("missing render cvar")
}

/// Cache the global image finder control once; source-specific fallback stays
/// with the resource's existing source policy.
pub fn image_search_view(vars: &Cvars) -> Result<View, &'static str> {
    setting_view(vars, RuleSetId::Quake2Rerelease, "r_texture_formats")
}

pub fn image_search(vars: &Cvars, view: View) -> Result<ImageSearchOrder, &'static str> {
    let text = vars
        .read(view)
        .map_err(|_| "image format setting conversion")?;
    ImageSearchOrder::parse(text.as_str().as_bytes()).map_err(|_| "invalid image format list")
}

pub(crate) fn model_lod_views(vars: &Cvars) -> Result<[View; 2], &'static str> {
    Ok([
        setting_view(vars, RuleSetId::Quake3, "r_lodscale")?,
        setting_view(vars, RuleSetId::Quake3, "r_lodbias")?,
    ])
}

pub(crate) fn model_replacements(
    vars: &Cvars,
    handles: qa_console::model::ModelHandles,
) -> Result<[qa_render::scene::ModelReplacementSettings; 5], &'static str> {
    use qa_render::scene::ModelReplacementSettings;
    let (q1, load) = handles
        .replacement_load(vars)
        .map_err(|_| "model load setting conversion")?;
    let q2 = load
        && vars
            .integer_view(handles.usage)
            .map_err(|_| "model use setting conversion")?
            != 0;
    let source_distance = vars
        .numeric(handles.distance)
        .map_err(|_| "model distance setting conversion")?;
    let text = vars
        .read(handles.limit)
        .map_err(|_| "model distance setting conversion")?;
    let text = text.as_str().trim();
    let distance = if text.eq_ignore_ascii_case("source") {
        None
    } else {
        let value = if text.is_empty() {
            0.0
        } else {
            text.parse::<f32>().map_err(|_| "invalid model distance")?
        };
        if !value.is_finite() {
            return Err("invalid model distance");
        }
        Some(value)
    };
    if !source_distance.is_finite() {
        return Err("invalid source model distance");
    }
    let q1 = ModelReplacementSettings {
        enabled: q1,
        distance: distance.unwrap_or(0.0),
    };
    let q2 = ModelReplacementSettings {
        enabled: q2,
        distance: distance.unwrap_or(source_distance),
    };
    Ok([q1, q1, q2, q2, ModelReplacementSettings::default()])
}

pub(crate) fn model_lod(vars: &Cvars, views: [View; 2]) -> qa_render::scene::ModelLodSettings {
    let defaults = qa_render::scene::ModelLodSettings::default();
    qa_render::scene::ModelLodSettings {
        scale: vars.numeric(views[0]).unwrap_or(defaults.scale),
        bias: vars.integer_view(views[1]).unwrap_or(defaults.bias),
    }
}

pub(crate) fn curve_error_view(vars: &Cvars) -> Result<View, &'static str> {
    setting_view(vars, RuleSetId::Quake3, "r_lodCurveError")
}

pub(crate) fn curve_error(vars: &Cvars, view: View) -> f32 {
    vars.numeric(view)
        .unwrap_or(qa_render::scene::DEFAULT_CURVE_ERROR)
}

fn setting_numbers(
    vars: &Cvars,
    source: RuleSetId,
    name: &str,
    pending: bool,
) -> Result<Option<(f32, i32)>, &'static str> {
    let view = setting_view(vars, source, name)?;
    if pending
        && let Some(text) = vars
            .latched(view)
            .map_err(|_| "render setting conversion")?
    {
        return Ok(Some((
            qa_console::numbers::number(text.as_str(), source),
            qa_console::numbers::integer(text.as_str()),
        )));
    }
    Ok(vars
        .native_number(view.canonical(), source)
        .zip(vars.native_integer(view.canonical(), source)))
}

pub fn image_settings(vars: &Cvars, source: RuleSetId) -> Result<ImageSettings, &'static str> {
    read_image_settings(vars, source, false)
}
fn read_image_settings(
    vars: &Cvars,
    source: RuleSetId,
    pending: bool,
) -> Result<ImageSettings, &'static str> {
    let mut settings = ImageSettings::native(source);
    if source == RuleSetId::Quake2Rerelease {
        settings.max_dimension = upload::MAX_DIMENSION;
    }
    let number = |name, fallback| -> Result<f32, &'static str> {
        let value = setting_numbers(vars, source, name, pending)?.map_or(fallback, |v| v.0);
        if value.is_finite() {
            Ok(value)
        } else {
            Err("non-finite image cvar")
        }
    };
    let integer = |name, fallback| -> Result<i32, &'static str> {
        Ok(setting_numbers(vars, source, name, pending)?.map_or(fallback, |v| v.1))
    };
    let gamma = number("r_gamma", 1.0)?;
    // tr_image.c:R_SetColorMappings clamps the native Q3 gamma value before
    // its reciprocal. Legacy gamma aliases already convert at the cvar edge.
    let gamma = if source == RuleSetId::Quake3 {
        gamma.clamp(0.5, 3.0)
    } else {
        gamma
    };
    if gamma <= 0.0 {
        return Err("non-positive image gamma");
    }
    settings.gamma_exponent = 1.0 / gamma;
    settings.intensity = number("r_intensity", settings.intensity)?;
    if source == RuleSetId::Quake2Rerelease {
        settings.intensity = settings.intensity.clamp(1.0, 5.0);
    }
    let picmip = match source {
        RuleSetId::Quake3 => integer("r_picmip", i32::from(settings.picmip))?.clamp(0, 16) as f32,
        RuleSetId::Quake2Rerelease => {
            integer("r_picmip", i32::from(settings.picmip))?.clamp(0, 31) as f32
        }
        _ => number("r_picmip", f32::from(settings.picmip))?.trunc(),
    };
    if !(0.0..32.0).contains(&picmip) {
        return Err("image picmip exceeds shift range");
    }
    settings.picmip = picmip as u8;
    // Classic Q2 uses the float value; Q3/Q2 rerelease use atoi's integer.
    settings.round_images_down = if matches!(source, RuleSetId::Quake3 | RuleSetId::Quake2Rerelease)
    {
        integer("r_roundImagesDown", i32::from(settings.round_images_down))? != 0
    } else {
        number(
            "r_roundImagesDown",
            u8::from(settings.round_images_down) as f32,
        )? != 0.0
    };
    settings.simple_mipmaps = integer("r_simpleMipMaps", i32::from(settings.simple_mipmaps))? != 0;
    let maximum = number("gl_max_size", settings.max_dimension as f32)?.trunc();
    if !(1.0..=upload::MAX_DIMENSION as f32).contains(&maximum) {
        return Err("image maximum exceeds load capacity");
    }
    settings.max_dimension = maximum as u32;
    settings.sky_mip = number("gl_skymip", u8::from(settings.sky_mip) as f32)? != 0.0;
    settings.override_textures = integer("r_override_textures", settings.override_textures)?;
    settings.texture_overrides =
        integer("r_texture_overrides", settings.texture_overrides as i32)? as u32;
    Ok(settings)
}
