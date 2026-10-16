//! Cold presentation settings from the one cvar table, selected per world.
use qa_console::{
    cvars::{Cvars, View},
    pending::PendingSettings,
    views::{Context, Role, RuleSetId},
};
use qa_content::vfs::Vfs;
use qa_render::{
    Assets,
    assets::{PaletteId, upload},
    material::{
        resources::{ImageSettings, PaletteSource, ResourceError, load_palette},
        world_load::WorldPresentationProfile,
    },
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
    Ok(profile)
}

/// World geometry reads the renderer's one native subdivision policy,
/// independently of its source map or selected presentation.
pub fn world_load_options(
    vars: &Cvars,
) -> Result<qa_render::material::world_load::WorldLoadOptions, &'static str> {
    let mut options = qa_render::material::world_load::WorldLoadOptions::default();
    options.geometry.patch_subdivisions = read_subdivisions(vars, false)?;
    Ok(options)
}

pub(crate) struct SubdivisionSettingsPlan {
    value: f32,
    pending: PendingSettings,
}
impl SubdivisionSettingsPlan {
    pub(crate) fn prepare(vars: &Cvars) -> Result<Self, &'static str> {
        Ok(Self {
            value: read_subdivisions(vars, true)?,
            pending: PendingSettings::capture(
                vars,
                [setting_view(vars, RuleSetId::Quake3, "r_subdivisions")?],
            ),
        })
    }
    pub(crate) fn value(&self) -> f32 {
        self.value
    }
    pub(crate) fn validate(&self, vars: &Cvars) -> Result<(), &'static str> {
        if self.pending.unchanged(vars) {
            Ok(())
        } else {
            Err("subdivision setting changed during restart")
        }
    }
    pub(crate) fn apply(&self, vars: &mut Cvars) -> Result<(), qa_console::cvars::WriteError> {
        self.pending.apply(vars)
    }
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

const IMAGE_CVARS: [&str; 7] = [
    "r_gamma",
    "r_intensity",
    "r_picmip",
    "r_roundImagesDown",
    "r_simpleMipMaps",
    "gl_max_size",
    "gl_skymip",
];

/// A cold preview keeps native source defaults and pending alias conversions in
/// the one cvar table until the replacement resources are ready to publish.
pub(crate) struct ImageSettingsPlan {
    settings: [Option<ImageSettings>; 5],
    pending: PendingSettings,
}
impl ImageSettingsPlan {
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
        Ok(Self {
            settings,
            pending: PendingSettings::capture(vars, views),
        })
    }
    pub(crate) fn selections(&self) -> [Option<ImageSettings>; 5] {
        self.settings
    }
    pub(crate) fn validate(&self, vars: &Cvars) -> Result<(), &'static str> {
        if !self.pending.unchanged(vars) {
            return Err("image settings changed during restart");
        }
        Ok(())
    }
    pub(crate) fn apply(&self, vars: &mut Cvars) -> Result<(), qa_console::cvars::WriteError> {
        self.pending.apply(vars)
    }
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
    .ok_or("missing image cvar")
}
fn setting_numbers(
    vars: &Cvars,
    source: RuleSetId,
    name: &str,
    pending: bool,
) -> Result<Option<(f32, i32)>, &'static str> {
    let view = setting_view(vars, source, name)?;
    if pending && let Some(text) = vars.latched(view).map_err(|_| "image setting conversion")? {
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
    Ok(settings)
}
