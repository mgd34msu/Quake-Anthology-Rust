//! Saved settings are a cold JSON boundary, never an input history or player.
//! Schema: C settings/codec.c and frontend/shared_storage.c/config_store.c.
use crate::Runtime;
use qa_console::{catalog::Scope, commands::Console, views::Context};
use qa_content::vfs::{FileRef, Vfs};
use qa_core::{
    names::canonical_path,
    primitives::{ClientPolicy, RuleSetId},
    sys_events::{EventTime, SeatId},
};
use qa_formats::archive::ArchiveReader;
use qa_input::{Target, keys};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Import {
    pub files: Vec<String>,
    pub cvars: u32,
    pub bindings: u32,
    pub unsupported: u32,
    pub seats: Vec<SeatId>,
}
impl Import {
    pub fn consumed(&self) -> bool {
        !self.files.is_empty() && (self.cvars + self.bindings != 0 || !self.seats.is_empty())
    }
    fn applied_to(&mut self, seat: SeatId) {
        if !self.seats.contains(&seat) {
            self.seats.push(seat);
        }
    }
}

fn document(vfs: &Vfs, file: &FileRef) -> Result<Value, String> {
    let bytes = vfs
        .read_to_vec(file, 4 * 1024 * 1024, &mut ArchiveReader::default())
        .map_err(|error| {
            if matches!(error, qa_content::vfs::VfsError::Limit) {
                "profile document exceeds 4 MiB".into()
            } else {
                format!("profile {file:?}: {error}")
            }
        })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("profile JSON: {e}"))?;
    if !value.is_object() || value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported profile schema".into());
    }
    Ok(value)
}

fn put(
    console: &mut Console<Runtime>,
    source: RuleSetId,
    name: &str,
    value: &str,
    selected: Option<SeatId>,
    report: &mut Import,
) {
    let context = Context {
        source,
        seat: selected.unwrap_or(console.cvars.context().seat),
        client_lifetime: None,
        cvar_seat: selected,
        ..console.cvars.context()
    };
    let server = Context {
        side: Scope::Server,
        ..context
    };
    let context = if console.cvars.bind(name, context).is_none()
        && console.cvars.bind(name, server).is_some()
    {
        if selected.is_some() {
            report.unsupported += 1;
            return;
        }
        server
    } else {
        context
    };
    if console
        .cvars
        .set_user(name, value, context, None, 0)
        .is_ok()
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
    selected: Option<SeatId>,
    report: &mut Import,
) -> Result<(), String> {
    if let Some(seat) = selected {
        console
            .cvars
            .load_seat_values(seat, source)
            .map_err(|e| format!("profile seat cvars: {e:?}"))?;
    }
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
        put(console, source, name, value, selected, report);
    }
    if let Some(seat) = selected {
        report.applied_to(seat);
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
            keys::mouse_button(u8::try_from(button).ok()?)
        }
        // Device-specific pad/axis tuning needs the later device settings gate.
        _ => None,
    }
}
struct ColdTarget;
impl Target for ColdTarget {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(
        &mut self,
        _: SeatId,
        _: EventTime,
        _: Option<qa_core::sys_events::KeySource>,
        _: Option<qa_core::primitives::CommandProducer>,
        _: &str,
    ) -> bool {
        true
    }
    fn cancel_commands(
        &mut self,
        _: qa_core::sys_events::KeySource,
        _: Option<qa_core::primitives::CommandProducer>,
    ) {
    }
}
fn seat(
    console: &mut Console<Runtime>,
    runtime: &mut Runtime,
    value: &Value,
    seat_id: SeatId,
    source: RuleSetId,
    apply_preferences: bool,
    report: &mut Import,
) -> Result<(), String> {
    console
        .cvars
        .load_seat_values(seat_id, source)
        .map_err(|e| format!("profile seat cvars: {e:?}"))?;
    if let Some(rows) = value.get("bindings").and_then(Value::as_array) {
        if rows.len() > 4096 {
            return Err("too many saved bindings".into());
        }
        runtime
            .input
            .unbind_all(seat_id, EventTime::default(), &mut ColdTarget);
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
                    .bind_text(
                        seat_id,
                        control,
                        &command,
                        EventTime::default(),
                        &mut ColdTarget,
                    )
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
    report.applied_to(seat_id);
    if !apply_preferences {
        return Ok(());
    }
    if let Some(always) = value.get("alwaysRun").and_then(Value::as_bool) {
        put(
            console,
            source,
            "cl_run",
            if always { "1" } else { "0" },
            Some(seat_id),
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
                put(
                    console,
                    source,
                    name,
                    &number.to_string(),
                    Some(seat_id),
                    report,
                );
            }
        }
        if let Some(number) = mouse.get("pitch").and_then(Value::as_f64) {
            let pitch = if mouse.get("invertPitch").and_then(Value::as_bool) == Some(true) {
                -number.abs()
            } else {
                number
            };
            put(
                console,
                source,
                "m_pitch",
                &pitch.to_string(),
                Some(seat_id),
                report,
            );
        }
        for (field, name) in [
            ("filter", "m_filter"),
            ("freeLook", "freelook"),
            ("lookStrafe", "lookstrafe"),
            ("lookSpring", "lookspring"),
        ] {
            if let Some(value) = mouse.get(field).and_then(Value::as_bool) {
                put(
                    console,
                    source,
                    name,
                    if value { "1" } else { "0" },
                    Some(seat_id),
                    report,
                );
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
    policies: &[ClientPolicy],
) -> Result<Import, String> {
    let Some(root) = qa_platform::saved_profile_root() else {
        return Ok(Import::default());
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
    let result = import_files(console, runtime, product, policies, &files);
    // Settings mounts never enter the live game asset namespace, even after a
    // malformed profile. Preserve the original import error after cleanup.
    let removed = runtime.vfs.unmount(mount);
    if !removed && result.is_ok() {
        return Err("profile mount lost during import".into());
    }
    result
}

fn import_files(
    console: &mut Console<Runtime>,
    runtime: &mut Runtime,
    product: &str,
    policies: &[ClientPolicy],
    files: &[(FileRef, Vec<u8>)],
) -> Result<Import, String> {
    let mut report = Import::default();
    let canonical = files
        .iter()
        .find(|(_, name)| name.as_slice() == b"cvars/shared/canonical.json");
    if policies.len() > SeatId::COUNT {
        return Err("too many local profile policies".into());
    }
    let primary = policies
        .first()
        .copied()
        .ok_or("profile requires a local seat policy")?;
    let policy = |seat: SeatId| policies.get(seat.index()).copied().unwrap_or(primary);
    let default_source = primary.client;
    let prefix = canonical_path(&format!("{product}/"));
    if let Some((file, name)) = canonical {
        let value = document(&runtime.vfs, file)?;
        if value.get("dialect").and_then(Value::as_str) != Some("q3") {
            return Err("canonical profile must use q3 names".into());
        }
        entries(
            console,
            value.get("entries").ok_or("canonical entries missing")?,
            RuleSetId::Quake3,
            None,
            &mut report,
        )?;
        if let Some(players) = value.get("players") {
            let players = players
                .as_array()
                .ok_or("canonical players must be a list")?;
            for player in players {
                let seat = player
                    .get("seat")
                    .and_then(Value::as_u64)
                    .and_then(|n| u8::try_from(n).ok())
                    .and_then(SeatId::new)
                    .ok_or("invalid canonical player seat")?;
                if report.seats.contains(&seat) {
                    return Err("duplicate canonical player seat".into());
                }
                entries(
                    console,
                    player.get("entries").ok_or("player entries missing")?,
                    RuleSetId::Quake3,
                    Some(seat),
                    &mut report,
                )?;
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
            for (file, name) in files {
                let Ok(path) = std::str::from_utf8(name) else {
                    continue;
                };
                let Some(relative) = path.strip_prefix(&prefix) else {
                    continue;
                };
                if !relative.starts_with(stage) || !relative.ends_with(".json") {
                    continue;
                }
                let value = document(&runtime.vfs, file)?;
                let file_source = value
                    .get("dialect")
                    .and_then(Value::as_str)
                    .and_then(RuleSetId::parse)
                    .ok_or("unknown profile cvar dialect")?;
                let role = |policy: ClientPolicy| match stage {
                    "cvars/movement/" => policy.movement,
                    "cvars/input/" => policy.input,
                    _ => policy.client,
                };
                let rows = value.get("entries").ok_or("profile entries missing")?;
                if stage == "cvars/client/" || stage == "cvars/input/" {
                    let Some(seat) = relative
                        .rsplit('/')
                        .next()
                        .and_then(|name| name.strip_suffix(".json"))
                        .and_then(|index| index.parse::<u8>().ok())
                        .and_then(SeatId::new)
                    else {
                        report.unsupported += 1;
                        continue;
                    };
                    if role(policy(seat)) != file_source {
                        continue;
                    }
                    entries(console, rows, file_source, Some(seat), &mut report)?;
                } else {
                    if role(primary) == file_source {
                        entries(console, rows, file_source, None, &mut report)?;
                    }
                    for (index, selected) in policies.iter().copied().enumerate() {
                        if role(selected) == file_source {
                            entries(
                                console,
                                rows,
                                file_source,
                                Some(SeatId::ALL[index]),
                                &mut report,
                            )?;
                        }
                    }
                }
                report.files.push(path.to_owned());
            }
        }
    }
    for seat_id in SeatId::ALL {
        let seat_name = format!("{prefix}input/seat-{}.json", seat_id.index() + 1);
        if let Some((file, _)) = files
            .iter()
            .find(|(_, name)| name.as_slice() == seat_name.as_bytes())
        {
            let value = document(&runtime.vfs, file)?;
            seat(
                console,
                runtime,
                &value,
                seat_id,
                policy(seat_id).input,
                canonical.is_none(),
                &mut report,
            )?;
            report.files.push(seat_name);
        }
    }
    let view_name = format!("{prefix}view.json");
    if canonical.is_none()
        && let Some((file, _)) = files
            .iter()
            .find(|(_, name)| name.as_slice() == view_name.as_bytes())
    {
        let value = document(&runtime.vfs, file)?;
        if let Some(fov) = value.get("fieldOfView").and_then(Value::as_f64) {
            put(
                console,
                default_source,
                "cg_fov",
                &fov.to_string(),
                None,
                &mut report,
            );
        }
        report.files.push(view_name);
    }
    Ok(report)
}

/// Startup overrides precede connection creation and apply to every prepared
/// local column as well as the inherited defaults. The ordinary setter still
/// owns all native alias conversion, flags and rejection behavior.
pub fn apply_overrides(
    console: &mut Console<Runtime>,
    settings: &[(String, String, Context)],
) -> Result<(), String> {
    for (name, value, context) in settings {
        let mut contexts = vec![*context];
        if context.side != Scope::Server {
            for seat in SeatId::ALL {
                if console.cvars.has_seat_values(seat) {
                    contexts.push(Context {
                        seat,
                        cvar_seat: Some(seat),
                        client_lifetime: None,
                        ..*context
                    });
                }
            }
        }
        for context in contexts {
            console
                .cvars
                .set_user(name, value, context, None, 0)
                .map_err(|e| format!("startup cvar {name}: {e:?}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/fixtures/profile.rs"]
mod tests;
