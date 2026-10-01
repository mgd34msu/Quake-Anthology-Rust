//! Quake view-size commands, client cvars, chase camera, and view rectangles.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q1-client-settings.ts`
//! (`registerQ1ViewCommands`, `registerQ1ClientSettings`, `Q1ChaseSettings`,
//! `Q1ViewSettings`, `readQ1ViewSettings`, `q1ChaseCamera`, `q1ViewRectangle`,
//! `q1ViewCamera`). Three documented folds: routed `findCvar` lookups become the dispatch
//! registry (the merged [`CommandBuffer`](qa_core::cmd_buffer::CommandBuffer) owns one
//! registry, and its register-time cvar-conflict check subsumes the donor's
//! `findCvar(...) === undefined` guards); the `bindValue` validators and `document` help
//! text have no merged-registry API and are dropped while every declaration stays; and
//! the donor numeric profile folds to binary64 compute with binary32 storage, which is
//! exactly the Q1 donor profile (`q1:donor-binary64` compute, binary32 storage).

use std::f64::consts::PI;
use std::rc::Rc;

use qa_client::view::{Rect, SceneCamera};
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{BufferError, CommandBuffer, Invocation};
use qa_core::cvar::{flags, CvarError, CvarRegistry};
use qa_core::identity::ActorId;
use qa_core::math::{angles_to_axis, donor_angle_vectors, vec3, Vec3};
use qa_core::numeric::native_atoi;

/// Registered view-command names with their release (donor `registerQ1ViewCommands` guard).
#[derive(Debug, Default)]
pub struct Q1ViewCommandGuard {
    names: Vec<String>,
}

impl Q1ViewCommandGuard {
    /// Names this guard releases.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Unregister every command this guard owns.
    pub fn release(self, commands: &mut CommandBuffer) {
        for name in &self.names {
            commands.unregister(name);
        }
    }
}

/// Parse a Q1 color component the way the donor `color` commands do.
fn color_component(args: &[String], index: usize) -> i32 {
    let text = args.get(index).map_or("", String::as_str);
    let fallback = index == 1 && args.len() == 1;
    let text = if fallback {
        args.first().map_or("", String::as_str)
    } else {
        text
    };
    (native_atoi(text).unwrap_or(0) & 15).min(13)
}

/// Register the size/name/color view commands (donor `registerQ1ViewCommands`).
pub fn register_q1_view_commands(
    commands: &mut CommandBuffer,
    cvars: &CvarRegistry,
) -> Result<Q1ViewCommandGuard, BufferError> {
    let mut names = Vec::new();
    if commands.dialect() != Dialect::Q1Netquake && commands.dialect() != Dialect::Q1Quakeworld {
        return Ok(Q1ViewCommandGuard { names });
    }
    for (name, step) in [("sizeup", 10.0f32), ("sizedown", -10.0f32)] {
        if commands.exists(name) {
            continue;
        }
        let registered = commands.register(
            name,
            Some(Rc::new(move |command: &mut Invocation| {
                if let Some(size) = command.cvars().get("viewsize") {
                    let _ = command.insert(&format!("viewsize {}\n", size.numeric_value + step));
                }
            })),
            None,
            cvars,
        )?;
        if registered {
            names.push(name.to_string());
        }
    }
    if commands.dialect() == Dialect::Q1Netquake {
        if !commands.exists("name")
            && commands.register(
                "name",
                Some(Rc::new(|command: &mut Invocation| {
                    if command.args().is_empty() {
                        let _ = command.execute_now("_cl_name");
                        return;
                    }
                    let raw = if command.args().len() == 1 {
                        command.args()[0].clone()
                    } else {
                        command.args_text.clone()
                    };
                    let name: String = raw.chars().take(15).collect::<String>().replace(['"', '\n', '\r'], "");
                    let _ = command.execute_now(&format!("_cl_name \"{name}\""));
                })),
                None,
                cvars,
            )?
        {
            names.push("name".to_string());
        }
        if !commands.exists("color")
            && commands.register(
                "color",
                Some(Rc::new(|command: &mut Invocation| {
                    if command.args().is_empty() {
                        let _ = command.execute_now("_cl_color");
                        return;
                    }
                    let top = color_component(command.args(), 0);
                    let bottom = color_component(command.args(), 1);
                    let _ = command.execute_now(&format!("_cl_color {}", top * 16 + bottom));
                })),
                None,
                cvars,
            )?
        {
            names.push("color".to_string());
        }
    }
    if commands.dialect() == Dialect::Q1Quakeworld
        && !commands.exists("color")
        && commands.register(
            "color",
            Some(Rc::new(|command: &mut Invocation| {
                if command.args().is_empty() {
                    let _ = command.execute_now("topcolor");
                    let _ = command.execute_now("bottomcolor");
                    return;
                }
                let top = color_component(command.args(), 0);
                let bottom = color_component(command.args(), 1);
                let _ = command.execute_now(&format!("topcolor {top}"));
                let _ = command.execute_now(&format!("bottomcolor {bottom}"));
            })),
            None,
            cvars,
        )?
    {
        names.push("color".to_string());
    }
    Ok(Q1ViewCommandGuard { names })
}

/// Which profile's declarations to register (donor `registerQ1ClientSettings` profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ClientSettingsProfile {
    /// Every profile (donor `"all"`).
    All,
    /// One command dialect (the donor default is the registry dialect).
    Dialect(Dialect),
}

/// Register the native Q1 view declarations (donor `registerQ1ClientSettings`).
pub fn register_q1_client_settings(
    cvars: &mut CvarRegistry,
    profile: Q1ClientSettingsProfile,
) -> Result<(), CvarError> {
    if let Q1ClientSettingsProfile::Dialect(dialect) = profile {
        if dialect != Dialect::Q1Netquake && dialect != Dialect::Q1Quakeworld {
            return Ok(());
        }
    }
    cvars.register("viewsize", "100", flags::ARCHIVE)?;
    if matches!(
        profile,
        Q1ClientSettingsProfile::All | Q1ClientSettingsProfile::Dialect(Dialect::Q1Quakeworld)
    ) {
        cvars.register("cl_sbar", "0", flags::ARCHIVE)?;
    }
    if matches!(
        profile,
        Q1ClientSettingsProfile::All | Q1ClientSettingsProfile::Dialect(Dialect::Q1Netquake)
    ) {
        for (name, value) in [
            ("chase_active", "0"),
            ("chase_back", "100"),
            ("chase_up", "16"),
            ("chase_right", "0"),
        ] {
            cvars.register(name, value, flags::NONE)?;
        }
    }
    Ok(())
}

/// Chase-camera offsets (donor `Q1ChaseSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1ChaseSettings {
    /// Distance behind the player, in world units.
    pub back: f32,
    /// Height above the eye, in world units.
    pub up: f32,
    /// Lateral offset, positive opposite the view's right vector.
    pub right: f32,
}

/// Resolved view state (donor `Q1ViewSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1ViewSettings {
    /// Clamped view size.
    pub size: f32,
    /// Whether the status bar overlays the scene.
    pub overlay_status: bool,
    /// Chase offsets, when the chase camera is active.
    pub chase: Option<Q1ChaseSettings>,
}

/// Read and clamp the view settings (donor `readQ1ViewSettings`).
///
/// `profile` overrides the registry dialect; [`None`] reads it from the registry.
pub fn read_q1_view_settings(
    cvars: Option<&mut CvarRegistry>,
    profile: Option<Dialect>,
) -> Result<Option<Q1ViewSettings>, CvarError> {
    let resolved = profile.or_else(|| cvars.as_ref().map(|cvars| cvars.dialect()));
    let Some(profile) = resolved else {
        return Ok(None);
    };
    if profile != Dialect::Q1Netquake && profile != Dialect::Q1Quakeworld {
        return Ok(None);
    }
    let Some(cvars) = cvars else {
        return Ok(None);
    };
    let Some(current) = cvars.get("viewsize") else {
        return Ok(None);
    };
    let size = current.numeric_value.clamp(30.0, 120.0);
    if size != current.numeric_value {
        cvars.set("viewsize", &size.to_string(), false)?;
    }
    Ok(Some(Q1ViewSettings {
        size,
        overlay_status: profile == Dialect::Q1Quakeworld && cvars.variable_value("cl_sbar") == 0.0,
        chase: if profile == Dialect::Q1Netquake && cvars.variable_value("chase_active") != 0.0 {
            Some(Q1ChaseSettings {
                back: cvars.variable_value("chase_back"),
                up: cvars.variable_value("chase_up"),
                right: cvars.variable_value("chase_right"),
            })
        } else {
            None
        },
    }))
}

/// One chase-camera obstruction query result (donor `SceneQueries["trace"]` subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1ChaseTrace {
    /// Whether the trace started inside solid matter.
    pub start_solid: bool,
    /// Whether the trace was entirely inside solid matter.
    pub all_solid: bool,
    /// Fraction of the segment traveled.
    pub fraction: f32,
    /// Trace end point.
    pub end: Vec3,
}

/// Obstruction queries the chase camera needs (donor `Pick<SceneQueries, "trace">`).
pub trait Q1ChaseScene {
    /// Trace the ±4-unit chase hull from `start` to `end`.
    fn trace_box(&self, start: Vec3, end: Vec3, actor: &ActorId) -> Q1ChaseTrace;
    /// Trace the aim ray from `start` to `end`.
    fn trace_point(&self, start: Vec3, end: Vec3, actor: &ActorId) -> Q1ChaseTrace;
}

/// Q1 chase camera with shared obstruction queries (donor `q1ChaseCamera`).
pub fn q1_chase_camera(
    camera: &SceneCamera,
    angles: Vec3,
    settings: &Q1ChaseSettings,
    scene: &impl Q1ChaseScene,
    actor: &ActorId,
) -> SceneCamera {
    let mut forward = Vec3::default();
    let mut right = Vec3::default();
    donor_angle_vectors(angles, Some(&mut forward), Some(&mut right), None);
    let offset = |eye: f32, ahead: f32, side: f32| {
        (f64::from(eye) - f64::from(ahead) * f64::from(settings.back) - f64::from(side) * f64::from(settings.right))
            as f32
    };
    let desired = vec3(
        offset(camera.origin.x, forward.x, right.x),
        offset(camera.origin.y, forward.y, right.y),
        (f64::from(camera.origin.z) + f64::from(settings.up)) as f32,
    );
    let rear = scene.trace_box(camera.origin, desired, actor);
    let origin = if rear.start_solid || rear.all_solid {
        camera.origin
    } else {
        rear.end
    };
    let far = vec3(
        (f64::from(camera.origin.x) + f64::from(forward.x) * 4096.0) as f32,
        (f64::from(camera.origin.y) + f64::from(forward.y) * 4096.0) as f32,
        (f64::from(camera.origin.z) + f64::from(forward.z) * 4096.0) as f32,
    );
    let aim = scene.trace_point(camera.origin, far, actor);
    let target = if aim.fraction == 1.0 || aim.start_solid || aim.all_solid {
        far
    } else {
        aim.end
    };
    let dx = f64::from(target.x) - f64::from(origin.x);
    let dy = f64::from(target.y) - f64::from(origin.y);
    let dz = f64::from(target.z) - f64::from(origin.z);
    let horizontal = dx.hypot(dy);
    let view_angles = vec3(
        (-dz.atan2(horizontal) * 180.0 / PI) as f32,
        if horizontal == 0.0 {
            angles.y
        } else {
            (dy.atan2(dx) * 180.0 / PI) as f32
        },
        angles.z,
    );
    SceneCamera {
        origin,
        axis: angles_to_axis(view_angles),
        ..*camera
    }
}

/// `SCR_CalcRefdef` rectangle (donor `q1ViewRectangle`).
#[must_use]
pub fn q1_view_rectangle(area: Rect, viewsize: f32, intermission: bool, overlay_status: bool) -> Rect {
    let size = if intermission {
        120.0
    } else {
        f64::from(viewsize).clamp(30.0, 120.0)
    };
    let lines = if size >= 120.0 {
        0
    } else if size >= 110.0 {
        24
    } else {
        48
    };
    let available = (area.height - (if overlay_status && size >= 100.0 { 0 } else { lines })).max(1);
    let fraction = size.min(100.0) / 100.0;
    let width = area
        .width
        .min(96.max((f64::from(area.width) * fraction).trunc() as i32));
    let height = available.min((f64::from(area.height) * fraction).trunc() as i32).max(1);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (if size >= 100.0 { 0 } else { (available - height) / 2 }),
        width,
        height,
    }
}

/// Camera narrowed to the view rectangle (donor `q1ViewCamera`).
#[must_use]
pub fn q1_view_camera(camera: &SceneCamera, area: Rect, settings: &Q1ViewSettings, intermission: bool) -> SceneCamera {
    let viewport = q1_view_rectangle(area, settings.size, intermission, settings.overlay_status);
    let ratio = (f64::from(viewport.width) / f64::from(viewport.height))
        / (f64::from(camera.viewport.width) / f64::from(camera.viewport.height));
    let mut projection = camera.projection;
    for index in [1, 5, 9, 13] {
        projection[index] = (f64::from(projection[index]) * ratio) as f32;
    }
    SceneCamera {
        viewport,
        projection,
        ..*camera
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd_buffer::{BufferOptions, CommandContext};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    fn buffer(dialect: Dialect) -> (CommandBuffer, CvarRegistry) {
        let owner = IdentityOwner::create("q1-view").unwrap();
        let context = CommandContext::new(
            owner.session().clone(),
            qa_core::cmd_buffer::CommandOrigin::LocalConsole,
        );
        let buffer = CommandBuffer::new(dialect, context, BufferOptions::default()).unwrap();
        (buffer, CvarRegistry::new(dialect))
    }

    #[test]
    fn view_commands_register_and_release() {
        let (mut commands, cvars) = buffer(Dialect::Q1Netquake);
        let guard = register_q1_view_commands(&mut commands, &cvars).unwrap();
        assert!(commands.exists("sizeup"));
        assert!(commands.exists("sizedown"));
        assert!(commands.exists("name"));
        assert!(commands.exists("color"));
        assert_eq!(guard.names().len(), 4);
        guard.release(&mut commands);
        assert!(!commands.exists("sizeup"));
        assert!(!commands.exists("color"));
    }

    #[test]
    fn non_q1_registers_nothing() {
        let (mut commands, cvars) = buffer(Dialect::Q3);
        let guard = register_q1_view_commands(&mut commands, &cvars).unwrap();
        assert!(guard.names().is_empty());
    }

    #[test]
    fn client_settings_follow_profile() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_q1_client_settings(&mut cvars, Q1ClientSettingsProfile::Dialect(Dialect::Q1Netquake)).unwrap();
        assert_eq!(cvars.variable_string("viewsize"), "100");
        assert!(cvars.get("cl_sbar").is_none());
        assert_eq!(cvars.variable_string("chase_back"), "100");
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_q1_client_settings(&mut cvars, Q1ClientSettingsProfile::Dialect(Dialect::Q2Classic)).unwrap();
        assert!(cvars.get("viewsize").is_none());
    }

    #[test]
    fn read_clamps_and_reports_chase() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_q1_client_settings(&mut cvars, Q1ClientSettingsProfile::Dialect(Dialect::Q1Netquake)).unwrap();
        cvars.set("viewsize", "200", true).unwrap();
        cvars.set("chase_active", "1", true).unwrap();
        let settings = read_q1_view_settings(Some(&mut cvars), None).unwrap().unwrap();
        assert_eq!(settings.size, 120.0);
        assert_eq!(cvars.variable_string("viewsize"), "120");
        assert!(settings.chase.is_some());
        assert!(!settings.overlay_status);
    }

    #[test]
    fn quakeworld_sbar_overlays() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Quakeworld);
        register_q1_client_settings(&mut cvars, Q1ClientSettingsProfile::Dialect(Dialect::Q1Quakeworld)).unwrap();
        let settings = read_q1_view_settings(Some(&mut cvars), None).unwrap().unwrap();
        assert!(settings.overlay_status);
        assert!(settings.chase.is_none());
    }

    #[test]
    fn view_rectangle_matches_donor_bands() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        };
        let full = q1_view_rectangle(area, 100.0, false, false);
        assert_eq!((full.width, full.height), (640, 432));
        let hidden = q1_view_rectangle(area, 120.0, false, false);
        assert_eq!((hidden.width, hidden.height), (640, 480));
        let small = q1_view_rectangle(area, 30.0, false, false);
        assert!(small.width < full.width && small.height < full.height);
        let intermission = q1_view_rectangle(area, 30.0, true, false);
        assert_eq!((intermission.width, intermission.height), (640, 480));
    }

    #[test]
    fn chase_camera_aims_at_trace_target() {
        struct Open;
        impl Q1ChaseScene for Open {
            fn trace_box(&self, _start: Vec3, end: Vec3, _actor: &ActorId) -> Q1ChaseTrace {
                Q1ChaseTrace {
                    start_solid: false,
                    all_solid: false,
                    fraction: 1.0,
                    end,
                }
            }
            fn trace_point(&self, _start: Vec3, end: Vec3, _actor: &ActorId) -> Q1ChaseTrace {
                Q1ChaseTrace {
                    start_solid: false,
                    all_solid: false,
                    fraction: 1.0,
                    end,
                }
            }
        }
        let owner = IdentityOwner::create("q1-chase").unwrap();
        let actor = owner.actor(0, 1);
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: angles_to_axis(vec3(0.0, 0.0, 0.0)),
            projection: [1.0; 16],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: qa_client::view::CameraClip::None,
        };
        let moved = q1_chase_camera(
            &camera,
            vec3(0.0, 0.0, 0.0),
            &Q1ChaseSettings {
                back: 100.0,
                up: 16.0,
                right: 0.0,
            },
            &Open,
            &actor,
        );
        assert!(moved.origin.x < 0.0);
        assert_eq!(moved.origin.z, 16.0);
    }
}
