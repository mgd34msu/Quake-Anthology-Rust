//! Frame-time cvars and source frame-delta transforms.
//!
//! Behavior follows the originals: NetQuake `Host_FilterTime`
//! (`quake/WinQuake/host.c:501-522`), QuakeWorld client throttling
//! (`quake/QW/client/cl_main.c:1317-1328`), Quake 2 `Qcommon_Frame`
//! (`quake-2/qcommon/common.c:1491-1529`), Quake 3 `Com_ModifyMsec`
//! (`quake-iii-arena/code/qcommon/common.c:2584-2627`).
//!
//! `FrameTimeCvarMirror` is explicit-sync: `qa-core` has no cvar
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
    /// Timescale multiplier (Q2/Q3 only; Q1 has no timescale cvar).
    pub timescale: f64,
    /// Fixed frame time.
    pub fixedtime: f64,
    /// Host framerate override, in seconds (NetQuake only).
    pub host_framerate: f64,
    /// Camera mode.
    pub camera_mode: f64,
    /// QuakeWorld `cl_maxfps` (0 selects the rate fallback).
    pub maxfps: f64,
    /// QuakeWorld `rate` (bytes/sec) for the `cl_maxfps = 0` fallback.
    pub rate: f64,
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
///
/// Q1 has no `timescale` cvar (`host.c`, `cl_main.c`); NetQuake owns only
/// `host_framerate`, QuakeWorld only `cl_maxfps`.
#[must_use]
pub fn frame_time_cvar_names(dialect: Dialect) -> &'static [&'static str] {
    if dialect == Dialect::Q1Netquake {
        &["host_framerate"]
    } else if dialect == Dialect::Q1Quakeworld {
        &["cl_maxfps"]
    } else if dialect.is_q2() {
        &["timescale", "fixedtime"]
    } else {
        &["timescale", "fixedtime", "com_cameraMode"]
    }
}

/// NetQuake frame gate (`Host_FilterTime`, `host.c:501-522`): false when
/// the frame arrives too soon after the previous one (72 Hz throttle),
/// unless a timedemo is running.
#[must_use]
pub fn nq_frame_due(realtime_seconds: f64, old_realtime_seconds: f64, timedemo: bool) -> bool {
    timedemo || realtime_seconds - old_realtime_seconds >= 1.0 / 72.0
}

/// QuakeWorld simulation fps (`cl_main.c:1317-1320`): `cl_maxfps` clamped
/// to 30..72, or `rate / 80` clamped the same way when `cl_maxfps` is 0.
/// Callers must pass finite values.
#[must_use]
pub fn qw_fps(maxfps: f64, rate: f64) -> f64 {
    if maxfps != 0.0 {
        maxfps.clamp(30.0, 72.0)
    } else {
        (rate / 80.0).clamp(30.0, 72.0)
    }
}

/// QuakeWorld frame gate (`cl_main.c:1314-1328`): false when the frame
/// arrives too soon at the current `fps` throttle, unless a timedemo is
/// running. A clock that runs backward resets the previous timestamp.
#[must_use]
pub fn qw_frame_due(realtime_seconds: f64, old_realtime_seconds: f64, fps: f64, timedemo: bool) -> bool {
    let old = if old_realtime_seconds > realtime_seconds {
        0.0
    } else {
        old_realtime_seconds
    };
    timedemo || realtime_seconds - old >= 1.0 / fps
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
///
/// Q1 dialects register no `timescale`: the originals have no such cvar.
pub fn register_frame_time_cvars(cvars: &mut CvarRegistry) -> Result<(), FrameTimeError> {
    let dialect = cvars.dialect();
    let q2 = dialect.is_q2();
    if dialect == Dialect::Q1Netquake {
        if cvars.get("host_framerate").is_none() {
            cvars.register("host_framerate", "0", 0)?;
        }
        return Ok(());
    }
    if dialect == Dialect::Q1Quakeworld {
        if cvars.get("cl_maxfps").is_none() {
            cvars.register("cl_maxfps", "0", 0)?;
        }
        return Ok(());
    }
    let cheat = if dialect == Dialect::Q2Classic {
        0
    } else if q2 {
        q2_flags::CHEAT
    } else {
        flags::CHEAT
    };
    cvars.register(
        "timescale",
        "1",
        if q2 { cheat } else { flags::CHEAT | flags::SYSTEM_INFO },
    )?;
    cvars.register("fixedtime", "0", cheat)?;
    if !q2 {
        cvars.register("com_cameraMode", "0", flags::CHEAT)?;
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
        maxfps: f64::from(cvars.variable_value("cl_maxfps")),
        rate: cvars.get("rate").map_or(2500.0, |value| f64::from(value.numeric_value)),
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
        controls.maxfps,
        controls.rate,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return Err(FrameTimeError::Invalid("Frame time controls must be finite".to_owned()));
    }
    if dialect == Dialect::Q1Netquake {
        if controls.host_framerate > 0.0 {
            return Ok(controls.host_framerate * 1000.0);
        }
        return Ok(raw_milliseconds.clamp(1.0, 100.0));
    }
    if dialect == Dialect::Q1Quakeworld {
        return Ok(raw_milliseconds.min(200.0));
    }
    if dialect.is_q2() {
        if controls.fixedtime != 0.0 {
            return Ok(controls.fixedtime.trunc());
        }
        if controls.timescale == 0.0 {
            return Ok(raw_milliseconds.trunc());
        }
        return Ok((raw_milliseconds * controls.timescale).trunc().max(1.0));
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
            maxfps: 0.0,
            rate: 2500.0,
        }
    }

    fn host() -> FrameTimeHost {
        FrameTimeHost {
            dedicated: false,
            local_server: true,
        }
    }

    #[test]
    fn netquake_ignores_timescale_and_clamps() {
        let mut scaled = controls();
        scaled.timescale = 2.0;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Netquake, 30.0, &scaled, &host()).expect("unscaled"),
            30.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Netquake, 500.0, &controls(), &host()).expect("clamped"),
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
    fn netquake_gate_throttles_to_72hz() {
        assert!(!nq_frame_due(1.0, 1.0, false));
        assert!(!nq_frame_due(1.0 + 1.0 / 72.0 - 0.0001, 1.0, false));
        assert!(nq_frame_due(1.0 + 1.0 / 72.0 + 0.0001, 1.0, false));
        assert!(nq_frame_due(2.0, 1.0, false));
        assert!(nq_frame_due(1.0, 1.0, true));
    }

    #[test]
    fn quakeworld_caps_at_200ms_without_floor_or_framerate() {
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Quakeworld, 500.0, &controls(), &host()).expect("capped"),
            200.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Quakeworld, 0.0, &controls(), &host()).expect("no floor"),
            0.0
        );
        let mut framerate = controls();
        framerate.host_framerate = 0.05;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q1Quakeworld, 30.0, &framerate, &host()).expect("framerate ignored"),
            30.0
        );
    }

    #[test]
    fn quakeworld_fps_follows_maxfps_or_rate() {
        assert_eq!(qw_fps(60.0, 2500.0), 60.0);
        assert_eq!(qw_fps(100.0, 2500.0), 72.0);
        assert_eq!(qw_fps(10.0, 2500.0), 30.0);
        assert_eq!(qw_fps(0.0, 2500.0), 31.25);
        assert_eq!(qw_fps(0.0, 100_000.0), 72.0);
        assert!(!qw_frame_due(1.0, 1.0, 31.25, false));
        assert!(qw_frame_due(1.0 + 1.0 / 31.25 + 0.0001, 1.0, 31.25, false));
        assert!(qw_frame_due(1.0, 1.0, 31.25, true));
        assert!(qw_frame_due(0.5, 1.0, 31.25, false));
    }

    #[test]
    fn q2_truncates_to_integer_msec() {
        let mut fixed = controls();
        fixed.fixedtime = 8.0;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Classic, 30.0, &fixed, &host()).expect("fixed"),
            8.0
        );
        fixed.fixedtime = 8.9;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Classic, 30.0, &fixed, &host()).expect("fixed truncates"),
            8.0
        );
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Rerelease, 0.2, &controls(), &host()).expect("floored"),
            1.0
        );
        let mut unscaled = controls();
        unscaled.timescale = 0.0;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Classic, 30.7, &unscaled, &host()).expect("raw truncates"),
            30.0
        );
        let mut half = controls();
        half.timescale = 0.5;
        assert_eq!(
            source_frame_milliseconds(Dialect::Q2Classic, 33.0, &half, &host()).expect("product truncates"),
            16.0
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
    fn q2_timescale_flags_follow_edition() {
        let mut classic = CvarRegistry::new(Dialect::Q2Classic);
        register_frame_time_cvars(&mut classic).expect("register");
        assert_eq!(classic.get("timescale").expect("timescale").flags, 0);
        assert_eq!(classic.get("fixedtime").expect("fixedtime").flags, 0);
        let mut rerelease = CvarRegistry::new(Dialect::Q2Rerelease);
        register_frame_time_cvars(&mut rerelease).expect("register");
        assert_eq!(rerelease.get("timescale").expect("timescale").flags, q2_flags::CHEAT);
        assert_eq!(rerelease.get("fixedtime").expect("fixedtime").flags, q2_flags::CHEAT);
    }

    #[test]
    fn registers_and_reads_dialect_cvars() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_frame_time_cvars(&mut cvars).expect("register");
        assert!(cvars.get("timescale").is_none());
        assert!(cvars.get("host_framerate").is_some());
        let mut qw = CvarRegistry::new(Dialect::Q1Quakeworld);
        register_frame_time_cvars(&mut qw).expect("register");
        assert!(qw.get("timescale").is_none());
        assert!(qw.get("host_framerate").is_none());
        assert!(qw.get("cl_maxfps").is_some());
        let read = read_frame_time_controls(&cvars);
        assert_eq!(read.timescale, 1.0);
        assert_eq!(read.maxfps, 0.0);
        assert_eq!(read.rate, 2500.0);
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
