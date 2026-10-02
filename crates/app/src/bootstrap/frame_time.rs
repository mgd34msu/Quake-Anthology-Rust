//! Frame-time cvars and source frame-delta transforms.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/frame-time.ts`
//! (`FrameTimeControls`, `q3ServerPaused`, `frameTimeCvarNames`,
//! `refreshFrameTimeCvars`, `FrameTimeCvarMirror`, `registerFrameTimeCvars`,
//! `readFrameTimeControls`, `sourceFrameMilliseconds`).
//!
//! `FrameTimeCvarMirror` is an explicit-sync port: `qa-core` has no cvar
//! value-subscription API, so owner-to-mirror copies happen through
//! [`FrameTimeCvarMirror::refresh`] and mirror-to-owner writes through
//! [`FrameTimeCvarMirror::push`] instead of live bindings.

use qa_core::cmd::Dialect;
use qa_core::cvar::flags;
use qa_core::cvar::q2_flags;
use qa_core::cvar::{CvarError, CvarRegistry};
use thiserror::Error;

/// Failure of a frame-time operation.
#[derive(Debug, Error)]
pub enum FrameTimeError {
    /// Non-finite or negative frame input.
    #[error("{0}")]
    Invalid(String),
    /// Underlying cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Timescale, fixed-step, host-framerate, and camera-mode controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTimeControls {
    /// Timescale multiplier.
    pub timescale: f64,
    /// Fixed frame time.
    pub fixedtime: f64,
    /// Host framerate override (frames per second).
    pub host_framerate: f64,
    /// Camera mode.
    pub camera_mode: f64,
}

/// Host dedication for the Q3 frame clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTimeHost {
    /// Dedicated server.
    pub dedicated: bool,
    /// Local (non-remote) server.
    pub local_server: bool,
}

/// `SV_CheckPaused` counts connected humans, including clients not begun.
pub fn q3_server_paused(
    cvars: &mut CvarRegistry,
    requested: bool,
    connected_humans: u32,
) -> Result<bool, FrameTimeError> {
    if cvars.get("sv_paused").is_none() {
        cvars.register("sv_paused", "0", flags::READ_ONLY)?;
    }
    if !requested {
        return Ok(false);
    }
    let paused = connected_humans <= 1;
    cvars.set("sv_paused", if paused { "1" } else { "0" }, true)?;
    Ok(paused)
}

/// Frame-time cvar names owned by a dialect.
#[must_use]
pub fn frame_time_cvar_names(dialect: Dialect) -> &'static [&'static str] {
    if dialect.is_q1() {
        &["timescale", "host_framerate"]
    } else if dialect.is_q2() {
        &["timescale", "fixedtime"]
    } else {
        &["timescale", "fixedtime", "com_cameraMode"]
    }
}

/// Copy declared frame-time values from `owner` into `mirror`.
pub fn refresh_frame_time_cvars(owner: &CvarRegistry, mirror: &mut CvarRegistry) -> Result<(), FrameTimeError> {
    for name in frame_time_cvar_names(owner.dialect()) {
        if let Some(value) = owner.get(name) {
            mirror.set(name, &value.value, true)?;
        }
    }
    Ok(())
}

/// Explicit-sync mirror of one seat registry's frame-time cvars.
///
/// The donor binds live value subscriptions; without that API the owner
/// pushes values through [`FrameTimeCvarMirror::refresh`] and the seat
/// writes back through [`FrameTimeCvarMirror::push`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameTimeCvarMirror {
    names: Vec<String>,
}

impl FrameTimeCvarMirror {
    /// Declare the mirrored names present in `owner` and refresh `mirror`.
    pub fn new(owner: &CvarRegistry, mirror: &mut CvarRegistry) -> Result<Self, FrameTimeError> {
        if std::ptr::eq(owner, mirror as &CvarRegistry) || owner.dialect() != mirror.dialect() {
            return Err(FrameTimeError::Invalid(
                "Shared cvar mirror requires distinct registries in the same dialect".to_owned(),
            ));
        }
        let names: Vec<String> = frame_time_cvar_names(owner.dialect())
            .iter()
            .filter(|name| owner.get(name).is_some())
            .map(ToString::to_string)
            .collect();
        for name in &names {
            let value = owner
                .get(name)
                .ok_or_else(|| FrameTimeError::Invalid(format!("Shared engine cvar {name} is not declared")))?;
            mirror.register(name, &value.reset_value, value.flags)?;
        }
        let this = Self { names };
        this.refresh(owner, mirror)?;
        Ok(this)
    }

    /// Mirrored cvar names.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Copy owner values into the mirror registry.
    pub fn refresh(&self, owner: &CvarRegistry, mirror: &mut CvarRegistry) -> Result<(), FrameTimeError> {
        for name in &self.names {
            mirror.set(name, &owner.variable_string(name), true)?;
        }
        Ok(())
    }

    /// Write one mirror value back to the owner after asserting currency.
    pub fn push(
        &self,
        owner: &mut CvarRegistry,
        name: &str,
        value: &str,
        assert_current: &dyn Fn(),
    ) -> Result<(), FrameTimeError> {
        if !self.names.iter().any(|mirrored| mirrored == name) {
            return Err(FrameTimeError::Invalid(format!(
                "Shared engine cvar {name} is not mirrored"
            )));
        }
        assert_current();
        owner.set(name, value, true)?;
        Ok(())
    }
}

/// Register the frame-time cvars for the registry dialect.
pub fn register_frame_time_cvars(cvars: &mut CvarRegistry) -> Result<(), FrameTimeError> {
    let q1 = cvars.dialect().is_q1();
    let q2 = cvars.dialect().is_q2();
    let mut register = |name: &str, value: &str, flag_word: u32| -> Result<(), FrameTimeError> {
        if !q1 || cvars.get(name).is_none() {
            cvars.register(name, value, flag_word)?;
        }
        Ok(())
    };
    register(
        "timescale",
        "1",
        if q1 {
            0
        } else if q2 {
            q2_flags::CHEAT
        } else {
            flags::CHEAT | flags::SYSTEM_INFO
        },
    )?;
    if q1 {
        register("host_framerate", "0", 0)?;
    } else {
        register("fixedtime", "0", if q2 { q2_flags::CHEAT } else { flags::CHEAT })?;
        if !q2 {
            register("com_cameraMode", "0", flags::CHEAT)?;
        }
    }
    Ok(())
}

/// Read the timescale controls from a registry.
#[must_use]
pub fn read_frame_time_controls(cvars: &CvarRegistry) -> FrameTimeControls {
    let fixedtime = if cvars.dialect() == Dialect::Q3 {
        cvars
            .get("fixedtime")
            .map_or(0.0, |value| f64::from(value.integer_value))
    } else {
        f64::from(cvars.variable_value("fixedtime"))
    };
    FrameTimeControls {
        timescale: cvars
            .get("timescale")
            .map_or(1.0, |value| f64::from(value.numeric_value)),
        fixedtime,
        host_framerate: f64::from(cvars.variable_value("host_framerate")),
        camera_mode: cvars
            .get("com_cameraMode")
            .map_or(0.0, |value| f64::from(value.integer_value)),
    }
}

/// Transform simulation/client frame deltas; socket timestamps stay wall time.
pub fn source_frame_milliseconds(
    dialect: Dialect,
    raw_milliseconds: f64,
    controls: &FrameTimeControls,
    host: &FrameTimeHost,
) -> Result<f64, FrameTimeError> {
    if !raw_milliseconds.is_finite() || raw_milliseconds < 0.0 {
        return Err(FrameTimeError::Invalid(
            "Frame milliseconds must be finite and nonnegative".to_owned(),
        ));
    }
    if ![
        controls.timescale,
        controls.fixedtime,
        controls.host_framerate,
        controls.camera_mode,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return Err(FrameTimeError::Invalid("Frame time controls must be finite".to_owned()));
    }
    if dialect == Dialect::Q1Netquake || dialect == Dialect::Q1Quakeworld {
        if controls.host_framerate > 0.0 {
            return Ok(controls.host_framerate * 1000.0);
        }
        let scaled = if controls.timescale == 0.0 {
            raw_milliseconds
        } else {
            raw_milliseconds * controls.timescale
        };
        return Ok(scaled.clamp(1.0, 100.0));
    }
    if dialect.is_q2() {
        if controls.fixedtime != 0.0 {
            return Ok(controls.fixedtime);
        }
        return Ok(if controls.timescale == 0.0 {
            raw_milliseconds
        } else {
            (raw_milliseconds * controls.timescale).max(1.0)
        });
    }
    let mut milliseconds = raw_milliseconds.trunc() as i64;
    let scale = controls.timescale as f32;
    let fixedtime = controls.fixedtime.trunc() as i64;
    let camera_mode = controls.camera_mode.trunc() as i64;
    if fixedtime != 0 {
        milliseconds = fixedtime;
    } else if scale != 0.0 || camera_mode != 0 {
        let product = (milliseconds as f32) * scale;
        if !product.is_finite() || product < -2_147_483_648.0 || product >= 2_147_483_648.0 {
            return Err(FrameTimeError::Invalid(
                "Undefined native common float-to-int time conversion".to_owned(),
            ));
        }
        milliseconds = product.trunc() as i64;
    }
    if milliseconds < 1 && scale != 0.0 {
        milliseconds = 1;
    }
    Ok((milliseconds.min(if host.dedicated || !host.local_server {
        5000
    } else {
        200
    })) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> FrameTimeControls {
        FrameTimeControls {
            timescale: 1.0,
            fixedtime: 0.0,
            host_framerate: 0.0,
            camera_mode: 0.0,
        }
    }

    fn host() -> FrameTimeHost {
        FrameTimeHost {
            dedicated: false,
            local_server: true,
        }
    }

    #[test]
    fn q1_clamps_scaled_frame() {
        let mut scaled = controls();
        scaled.timescale = 2.0;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Netquake, 30.0, &scaled, &host()).expect("scaled"),
            60.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Quakeworld, 500.0, &controls(), &host()).expect("clamped"),
            100.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Netquake, 0.0, &controls(), &host()).expect("floored"),
            1.0
        );
        let mut framerate = controls();
        framerate.host_framerate = 0.05;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Netquake, 30.0, &framerate, &host()).expect("framerate"),
            50.0
        );
    }

    #[test]
    fn q2_prefers_fixedtime() {
        let mut fixed = controls();
        fixed.fixedtime = 8.0;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Classic, 30.0, &fixed, &host()).expect("fixed"),
            8.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Rerelease, 0.2, &controls(), &host()).expect("floored"),
            1.0
        );
    }

    #[test]
    fn q3_applies_float_scale_and_local_cap() {
        let mut scaled = controls();
        scaled.timescale = 0.5;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q3, 33.0, &scaled, &host()).expect("scaled"),
            16.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q3, 1000.0, &controls(), &host()).expect("capped"),
            200.0
        );
        let dedicated = FrameTimeHost {
            dedicated: true,
            local_server: true,
        };
        assert_eq!(
            source_frame_milliseconds(Dialect::Q3, 9000.0, &controls(), &dedicated).expect("dedicated cap"),
            5000.0
        );
    }

    #[test]
    fn rejects_non_finite_inputs() {
        assert!(source_frame_milliseconds(Dialect::Q3, f64::NAN, &controls(), &host()).is_err());
        assert!(source_frame_milliseconds(Dialect::Q3, -1.0, &controls(), &host()).is_err());
        let mut bad = controls();
        bad.timescale = f64::INFINITY;
        assert!(source_frame_milliseconds(Dialect::Q3, 10.0, &bad, &host()).is_err());
        let mut overflow = controls();
        overflow.timescale = 1.0e30;
        assert!(source_frame_milliseconds(Dialect::Q3, 100.0, &overflow, &host()).is_err());
    }

    #[test]
    fn registers_and_reads_dialect_cvars() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_frame_time_cvars(&mut cvars).expect("register");
        assert!(cvars.get("timescale").is_some());
        assert!(cvars.get("host_framerate").is_some());
        let read = read_frame_time_controls(&cvars);
        assert_eq!(read.timescale, 1.0);
        let mut q3 = CvarRegistry::new(Dialect::Q3);
        register_frame_time_cvars(&mut q3).expect("register");
        assert!(q3.get("com_cameraMode").is_some());
    }

    #[test]
    fn pauses_q3_server_with_at_most_one_human() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        assert!(!q3_server_paused(&mut cvars, false, 0).expect("unrequested"));
        assert!(q3_server_paused(&mut cvars, true, 1).expect("paused"));
        assert_eq!(cvars.variable_string("sv_paused"), "1");
        assert!(!q3_server_paused(&mut cvars, true, 2).expect("unpaused"));
    }

    #[test]
    fn mirror_refreshes_and_pushes() {
        let mut owner = CvarRegistry::new(Dialect::Q3);
        register_frame_time_cvars(&mut owner).expect("register");
        owner.set("timescale", "2", true).expect("set");
        let mut mirror = CvarRegistry::new(Dialect::Q3);
        let synced = FrameTimeCvarMirror::new(&owner, &mut mirror).expect("mirror");
        assert_eq!(mirror.variable_string("timescale"), "2");
        owner.set("timescale", "3", true).expect("set");
        synced.refresh(&owner, &mut mirror).expect("refresh");
        assert_eq!(mirror.variable_string("timescale"), "3");
        synced.push(&mut owner, "timescale", "4", &|| {}).expect("push");
        assert_eq!(owner.variable_string("timescale"), "4");
    }
}
