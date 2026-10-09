//! Saved settings are a cold JSON boundary, never an input history or player.
//! Schema: C settings/codec.c and frontend/shared_storage.c/config_store.c.
use crate::Runtime;
use qa_console::{
    catalog::Scope,
    commands::Console,
    cvars::Cvars,
    views::Context,
};
use qa_content::vfs::{FileRef, Vfs};
use qa_core::{
    primitives::{CvarHandle, RuleSetId},
    sys_events::{EventTime, SeatId},
};
use qa_formats::archive::ArchiveReader;
use qa_input::{InputPolicy, Target, keys};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Import {
    pub files: Vec<String>,
    pub cvars: u32,
    pub bindings: u32,
    pub unsupported: u32,
}
impl Import {
    pub fn consumed(&self) -> bool {
        !self.files.is_empty() && self.cvars + self.bindings != 0
    }
}

fn document(vfs: &Vfs, file: FileRef) -> Result<Value, String> {
    let length = usize::try_from(
        vfs.length(file)
            .map_err(|e| format!("profile size: {e:?}"))?,
    )
    .map_err(|_| "profile size exceeds platform")?;
    if length > 4 * 1024 * 1024 {
        return Err("profile document exceeds 4 MiB".into());
    }
    let mut bytes = vec![0; length];
    if vfs
        .read_into_reusing(file, &mut bytes, &mut ArchiveReader::default())
        .map_err(|e| format!("profile read: {e:?}"))?
        != length
    {
        return Err("incomplete profile document".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("profile JSON: {e}"))?;
    if !value.is_object() || value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported profile schema".into());
    }
    Ok(value)
}

fn dialect(name: &str) -> Option<RuleSetId> {
    match name {
        "q1" | "q1-netquake" => Some(RuleSetId::Quake),
        "qw" | "q1-quakeworld" => Some(RuleSetId::QuakeWorld),
        "q2" | "q2-classic" => Some(RuleSetId::Quake2),
        "q2rr" | "q2-rerelease" => Some(RuleSetId::Quake2Rerelease),
        "q3" => Some(RuleSetId::Quake3),
        _ => None,
    }
}

fn put(
    console: &mut Console<Runtime>,
    source: RuleSetId,
    name: &str,
    value: &str,
    report: &mut Import,
) {
    let context = Context {
        source,
        ..console.cvars.context()
    };
    let view = console.cvars.bind(name, context).or_else(|| {
        console.cvars.bind(
            name,
            Context {
                side: Scope::Server,
                ..context
            },
        )
    });
    if let Some(view) = view
        && console.cvars.write(view, value).is_ok()
    {
        report.cvars += 1;
    } else {
        report.unsupported += 1;
    }
}
fn entries(
    console: &mut Console<Runtime>,
    value: &Value,
    source: RuleSetId,
    report: &mut Import,
) -> Result<(), String> {
    let rows = value.as_array().ok_or("profile entries must be a list")?;
    if rows.len() > 16384 {
        return Err("too many profile cvars".into());
    }
    let mut names = BTreeSet::new();
    for row in rows {
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .ok_or("profile cvar needs a name")?;
        let value = row
            .get("value")
            .and_then(Value::as_str)
            .ok_or("profile cvar needs a string value")?;
        if name.contains('\0') || value.contains('\0') || !names.insert(name) {
            return Err("invalid or duplicate profile cvar".into());
        }
        put(console, source, name, value, report);
    }
    Ok(())
}

/// The C seat JSON stores its canonical physical key numbers (Q3 namespace),
/// even when its cvar archive dialect is NetQuake. Native wire keys are separate.
fn control(input: &Value) -> Option<u16> {
    match input.get("kind")?.as_str()? {
        "key" => {
            let code = input.get("code")?.as_u64()?;
            if code <= 255 {
                keys::parse(&format!("0x{code:02x}"))
            } else if (256..272).contains(&code) {
                Some(656 + (code as u16 - 256))
            } else {
                None
            }
        }
        "mouse-button" => {
            let button = input.get("button")?.as_u64()?;
            (1..=32).contains(&button).then_some(if button == 32 {
                580
            } else {
                512 + button as u16
            })
        }
        // Device-specific pad/axis tuning needs the later device settings gate.
        _ => None,
    }
}
struct ColdTarget;
impl Target for ColdTarget {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, _: SeatId, _: EventTime, _: &str) {}
}
fn seat(
    console: &mut Console<Runtime>,
    runtime: &mut Runtime,
    value: &Value,
    source: RuleSetId,
    apply_preferences: bool,
    report: &mut Import,
) -> Result<(), String> {
    if let Some(rows) = value.get("bindings").and_then(Value::as_array) {
        if rows.len() > 4096 {
            return Err("too many saved bindings".into());
        }
        runtime
            .input
            .unbind_all(EventTime::default(), &mut ColdTarget);
        for row in rows {
            let Some(control) = row.get("input").and_then(control) else {
                report.unsupported += 1;
                continue;
            };
            let target = row.get("target").ok_or("binding target missing")?;
            let command = match target.get("kind").and_then(Value::as_str) {
                Some("command") => target
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                Some("action") => {
                    target
                        .get("action")
                        .and_then(Value::as_str)
                        .map(|action| match action {
                            "move-left" => "+moveleft".into(),
                            "move-right" => "+moveright".into(),
                            "move-up" => "+moveup".into(),
                            "move-down" => "+movedown".into(),
                            "turn-left" => "+left".into(),
                            "turn-right" => "+right".into(),
                            "look-up" => "+lookup".into(),
                            "look-down" => "+lookdown".into(),
                            "crouch" => "+duck".into(),
                            "walk" => "+speed".into(),
                            other => format!("+{other}"),
                        })
                }
                _ => None,
            };
            if let Some(command) = command {
                if runtime
                    .input
                    .bind_text(control, &command, EventTime::default(), &mut ColdTarget)
                    .is_ok()
                {
                    report.bindings += 1;
                } else {
                    report.unsupported += 1;
                }
            } else {
                report.unsupported += 1;
            }
        }
    }
    if !apply_preferences {
        return Ok(());
    }
    if let Some(always) = value.get("alwaysRun").and_then(Value::as_bool) {
        put(
            console,
            source,
            "cl_run",
            if always { "1" } else { "0" },
            report,
        );
    }
    if let Some(mouse) = value.get("mouse") {
        for (field, name) in [
            ("sensitivity", "sensitivity"),
            ("acceleration", "cl_mouseAccel"),
            ("yaw", "m_yaw"),
            ("side", "m_side"),
            ("forward", "m_forward"),
        ] {
            if let Some(number) = mouse.get(field).and_then(Value::as_f64) {
                put(console, source, name, &number.to_string(), report);
            }
        }
        if let Some(number) = mouse.get("pitch").and_then(Value::as_f64) {
            let pitch = if mouse.get("invertPitch").and_then(Value::as_bool) == Some(true) {
                -number.abs()
            } else {
                number
            };
            put(console, source, "m_pitch", &pitch.to_string(), report);
        }
        for (field, name) in [
            ("filter", "m_filter"),
            ("freeLook", "freelook"),
            ("lookStrafe", "lookstrafe"),
            ("lookSpring", "lookspring"),
        ] {
            if let Some(value) = mouse.get(field).and_then(Value::as_bool) {
                put(console, source, name, if value { "1" } else { "0" }, report);
            }
        }
    }
    Ok(())
}

/// Only explicit settings documents under the selected product are imported.
/// Authority-bound FileRefs prevent a mod/retail file from impersonating them.
pub fn load(
    console: &mut Console<Runtime>,
    runtime: &mut Runtime,
    product: &str,
    movement: RuleSetId,
) -> Result<Import, String> {
    let mut report = Import::default();
    let Some(root) = qa_platform::saved_profile_root() else {
        return Ok(report);
    };
    let mount = runtime
        .vfs
        .mount_settings_directory(&root, -1_000_000)
        .map_err(|e| format!("profile mount: {e:?}"))?;
    let files: Vec<_> = runtime
        .vfs
        .files_in_mount(mount)
        .map(|(file, name)| (file, name.to_vec()))
        .collect();
    let canonical = files
        .iter()
        .find(|(_, name)| name.as_slice() == b"cvars/shared/canonical.json");
    let default_source = console.cvars.context().source;
    let movement_source = movement;
    let prefix = format!("{product}/").to_ascii_lowercase();
    if let Some((file, name)) = canonical {
        let value = document(&runtime.vfs, *file)?;
        if value.get("dialect").and_then(Value::as_str) != Some("q3") {
            return Err("canonical profile must use q3 names".into());
        }
        entries(
            console,
            value.get("entries").ok_or("canonical entries missing")?,
            RuleSetId::Quake3,
            &mut report,
        )?;
        if let Some(players) = value.get("players").and_then(Value::as_array) {
            for player in players {
                if player.get("seat").and_then(Value::as_u64) == Some(0) {
                    entries(
                        console,
                        player.get("entries").ok_or("player entries missing")?,
                        RuleSetId::Quake3,
                        &mut report,
                    )?;
                }
            }
        }
        report
            .files
            .push(String::from_utf8_lossy(name).into_owned());
    }
    if canonical.is_none() {
        // RuleSetId/client archives precede movement and mouse-specific overrides.
        for stage in [
            "cvars/fallback/",
            "cvars/source/",
            "cvars/client/",
            "cvars/movement/",
            "cvars/input/",
        ] {
            for (file, name) in &files {
                let Ok(path) = std::str::from_utf8(name) else {
                    continue;
                };
                let Some(relative) = path.strip_prefix(&prefix) else {
                    continue;
                };
                if !relative.starts_with(stage) || !relative.ends_with(".json") {
                    continue;
                }
                let value = document(&runtime.vfs, *file)?;
                let file_source = value
                    .get("dialect")
                    .and_then(Value::as_str)
                    .and_then(dialect)
                    .ok_or("unknown profile cvar dialect")?;
                let expected = if stage == "cvars/movement/" {
                    movement_source
                } else {
                    default_source
                };
                if file_source != expected {
                    continue;
                }
                if (stage == "cvars/client/" || stage == "cvars/input/")
                    && !relative.ends_with("/0.json")
                {
                    continue;
                }
                entries(
                    console,
                    value.get("entries").ok_or("profile entries missing")?,
                    file_source,
                    &mut report,
                )?;
                report.files.push(path.to_owned());
            }
        }
    }
    let seat_name = format!("{prefix}input/seat-1.json");
    if let Some((file, _)) = files
        .iter()
        .find(|(_, name)| name.as_slice() == seat_name.as_bytes())
    {
        let value = document(&runtime.vfs, *file)?;
        seat(
            console,
            runtime,
            &value,
            default_source,
            canonical.is_none(),
            &mut report,
        )?;
        report.files.push(seat_name);
    }
    let view_name = format!("{prefix}view.json");
    if canonical.is_none()
        && let Some((file, _)) = files
            .iter()
            .find(|(_, name)| name.as_slice() == view_name.as_bytes())
    {
        let value = document(&runtime.vfs, *file)?;
        if let Some(fov) = value.get("fieldOfView").and_then(Value::as_f64) {
            put(
                console,
                default_source,
                "cg_fov",
                &fov.to_string(),
                &mut report,
            );
        }
        report.files.push(view_name);
    }
    // Imported numeric/text values remain in the one cvar table. Profile mounts
    // leave the live asset namespace after load, including non-imported files.
    if !runtime.vfs.unmount(mount) {
        return Err("profile mount lost during import".into());
    }
    Ok(report)
}

const INPUT_NAMES: [&str; 19] = [
    "cl_forwardspeed",
    "cl_backspeed",
    "cl_sidespeed",
    "cl_upspeed",
    "cl_yawspeed",
    "cl_pitchspeed",
    "cl_anglespeedkey",
    "cl_movespeedkey",
    "cl_run",
    "sensitivity",
    "cl_mouseAccel",
    "m_yaw",
    "m_pitch",
    "m_side",
    "m_forward",
    "m_filter",
    "freelook",
    "lookstrafe",
    "lookspring",
];
pub struct InputHandles([CvarHandle; INPUT_NAMES.len()]);
impl InputHandles {
    pub fn load(vars: &Cvars) -> Result<Self, String> {
        let mut handles = [CvarHandle(0); INPUT_NAMES.len()];
        for (index, name) in INPUT_NAMES.iter().enumerate() {
            handles[index] = vars
                .find(name)
                .ok_or_else(|| format!("missing input cvar {name}"))?;
        }
        Ok(Self(handles))
    }
    pub fn policy(&self, vars: &Cvars, rules: RuleSetId) -> InputPolicy {
        let source = rules;
        let mut policy = InputPolicy::native(rules);
        let value = |index: usize, fallback: f32| {
            let number = vars.value_in(self.0[index], source);
            if number.is_finite() { number } else { fallback }
        };
        policy.speed = [
            value(0, policy.speed[0]),
            value(2, policy.speed[1]),
            value(3, policy.speed[2]),
        ];
        policy.back_speed = if !vars.is_explicit(self.0[1])
            && !matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld)
        {
            policy.speed[0]
        } else {
            value(1, policy.back_speed)
        };
        policy.angle_speed = [
            value(4, policy.angle_speed[0]),
            value(5, policy.angle_speed[1]),
        ];
        policy.angle_multiplier = value(6, policy.angle_multiplier);
        policy.move_multiplier = value(7, policy.move_multiplier);
        policy.always_run = if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld)
            && !vars.is_explicit(self.0[8])
        {
            false
        } else if !matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            vars.integer_in(self.0[8], source) != 0
        } else {
            value(8, if policy.always_run { 1.0 } else { 0.0 }) != 0.0
        };
        policy.sensitivity = value(9, policy.sensitivity);
        policy.acceleration = value(10, policy.acceleration);
        policy.mouse_scale = [
            value(11, policy.mouse_scale[0]),
            value(12, policy.mouse_scale[1]),
        ];
        policy.mouse_side = value(13, policy.mouse_side);
        policy.mouse_forward = value(14, policy.mouse_forward);
        policy.filter = if rules == RuleSetId::Quake3 {
            vars.integer_in(self.0[15], source) != 0
        } else {
            value(15, 0.0) != 0.0
        };
        policy.freelook = if rules == RuleSetId::Quake3 {
            vars.integer_in(self.0[16], source) != 0
        } else {
            value(16, 1.0) != 0.0
        };
        policy.look_strafe = value(17, 0.0) != 0.0;
        policy
    }
}
