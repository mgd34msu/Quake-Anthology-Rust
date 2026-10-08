//! Cold presentation settings from the one cvar table, selected per world.
use qa_console::{cvars::Cvars, views::Source};
use qa_render::{assets::upload, material::resources::ImageSettings};

pub fn image_settings(vars: &Cvars, source: Source) -> Result<ImageSettings, &'static str> {
    let family = match source {
        Source::Quake | Source::QuakeWorld => 1,
        Source::Quake2 | Source::Quake2Rerelease => 2,
        Source::Quake3 => 3,
    };
    let mut settings = ImageSettings::native(family);
    if source == Source::Quake2Rerelease {
        settings.max_dimension = upload::MAX_DIMENSION;
    }
    let number = |name, fallback| -> Result<f32, &'static str> {
        let handle = vars.find(name).ok_or("missing image cvar")?;
        let value = if vars.is_explicit(handle) || vars.native_default_available(handle, source) {
            vars.value_in(handle, source)
        } else {
            fallback
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err("non-finite image cvar")
        }
    };
    let integer = |name, fallback| -> Result<i32, &'static str> {
        let handle = vars.find(name).ok_or("missing image cvar")?;
        Ok(
            if vars.is_explicit(handle) || vars.native_default_available(handle, source) {
                vars.integer_in(handle, source)
            } else {
                fallback
            },
        )
    };
    let gamma = number("r_gamma", 1.0)?;
    // tr_image.c:R_SetColorMappings clamps the native Q3 gamma value before
    // its reciprocal. Legacy gamma aliases already convert at the cvar edge.
    let gamma = if source == Source::Quake3 {
        gamma.clamp(0.5, 3.0)
    } else {
        gamma
    };
    if gamma <= 0.0 {
        return Err("non-positive image gamma");
    }
    settings.gamma_exponent = 1.0 / gamma;
    settings.intensity = number("r_intensity", settings.intensity)?;
    if source == Source::Quake2Rerelease {
        settings.intensity = settings.intensity.clamp(1.0, 5.0);
    }
    let picmip = match source {
        Source::Quake3 => integer("r_picmip", i32::from(settings.picmip))?.clamp(0, 16) as f32,
        Source::Quake2Rerelease => {
            integer("r_picmip", i32::from(settings.picmip))?.clamp(0, 31) as f32
        }
        _ => number("r_picmip", f32::from(settings.picmip))?.trunc(),
    };
    if !(0.0..32.0).contains(&picmip) {
        return Err("image picmip exceeds shift range");
    }
    settings.picmip = picmip as u8;
    // Classic Q2 uses the float value; Q3/Q2 rerelease use atoi's integer.
    settings.round_images_down = if matches!(source, Source::Quake3 | Source::Quake2Rerelease) {
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
