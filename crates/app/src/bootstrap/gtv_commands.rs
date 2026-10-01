//! Quake II GTV (multiview observer) console commands.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/gtv-commands.ts`
//! (`GtvConnectRequest`, `registerGtvCvars`, `parseGtvConnect`,
//! `matchesGtvDisconnect`). Direct port with no behavioral changes.

use qa_core::cvar::q2_flags;
use qa_core::cvar::{CvarError, CvarRegistry};
use thiserror::Error;

/// Failure of a GTV console command.
#[derive(Debug, Error)]
pub enum GtvError {
    /// Command line usage error.
    #[error("{0}")]
    Usage(String),
    /// Underlying cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Parsed `mvdconnect` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtvConnectRequest {
    /// Server address with optional port.
    pub address: String,
    /// Observer username.
    pub username: String,
    /// Observer password.
    pub password: String,
    /// Connection label, when named.
    pub label: Option<String>,
}

/// Register `mvd_username`/`mvd_password` on Quake II dialects; no-op elsewhere.
pub fn register_gtv_cvars(cvars: &mut CvarRegistry) -> Result<(), GtvError> {
    if !cvars.dialect().is_q2() {
        return Ok(());
    }
    cvars.register("mvd_username", "unnamed", 0)?;
    cvars.register("mvd_password", "", q2_flags::PRIVATE)?;
    Ok(())
}

/// Default credentials for `mvdconnect` option parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtvConnectDefaults {
    /// Default username.
    pub username: String,
    /// Default password.
    pub password: String,
}

/// Parse `mvdconnect [-u user] [-p password] [-n name] <address[:port]>`.
///
/// Returns `None` for `-h`/`--help`, matching the donor.
pub fn parse_gtv_connect(
    args: &[String],
    defaults: &GtvConnectDefaults,
) -> Result<Option<GtvConnectRequest>, GtvError> {
    let mut username = defaults.username.clone();
    let mut password = defaults.password.clone();
    let mut label: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let option = &args[index];
        if option == "--" {
            index += 1;
            break;
        }
        if !option.starts_with('-') {
            break;
        }
        if option == "-h" || option == "--help" {
            return Ok(None);
        }
        if option != "-u"
            && option != "--user"
            && option != "-p"
            && option != "--pass"
            && option != "-n"
            && option != "--name"
        {
            return Err(GtvError::Usage(format!("Unknown mvdconnect option: {option}")));
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| GtvError::Usage(format!("Missing value for {option}")))?;
        if value.contains('\0') || !value.is_ascii() {
            return Err(GtvError::Usage(
                "GTV options require non-NUL single-byte text".to_owned(),
            ));
        }
        if option == "-u" || option == "--user" {
            username = value.clone();
        } else if option == "-p" || option == "--pass" {
            password = value.clone();
        } else {
            label = Some(value.clone());
        }
        index += 1;
    }
    let address = args.get(index).filter(|text| !text.is_empty());
    if address.is_none() || index + 1 != args.len() {
        return Err(GtvError::Usage(
            "Usage: mvdconnect [-u user] [-p password] [-n name] <address[:port]>".to_owned(),
        ));
    }
    Ok(Some(GtvConnectRequest {
        address: address.expect("checked above").clone(),
        username,
        password,
        label,
    }))
}

/// Current GTV connection selected by `mvdisconnect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtvConnection {
    /// Connection id.
    pub id: u32,
    /// Connection label.
    pub label: String,
}

/// Whether `mvdisconnect [-a|--all] [conn_id]` selects the current connection.
///
/// Returns `false` for `-h`/`--help`, matching the donor.
pub fn matches_gtv_disconnect(args: &[String], current: Option<&GtvConnection>) -> Result<bool, GtvError> {
    if args.len() > 1 {
        return Err(GtvError::Usage("Usage: mvdisconnect [-a|--all] [conn_id]".to_owned()));
    }
    let selected = args.first();
    if selected.is_some_and(|text| text == "-h" || text == "--help") {
        return Ok(false);
    }
    let current = current.ok_or_else(|| GtvError::Usage("No GTV connections.".to_owned()))?;
    match selected {
        None => Ok(true),
        Some(text) if text == "-a" || text == "--all" || *text == current.id.to_string() || *text == current.label => {
            Ok(true)
        }
        Some(text) => Err(GtvError::Usage(format!("No such connection ID: {text}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn defaults() -> GtvConnectDefaults {
        GtvConnectDefaults {
            username: "observer".to_owned(),
            password: "secret".to_owned(),
        }
    }

    #[test]
    fn registers_credentials_only_for_q2() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_gtv_cvars(&mut cvars).expect("register");
        assert_eq!(cvars.variable_string("mvd_username"), "unnamed");
        assert_eq!(cvars.variable_string("mvd_password"), "");
        let mut q3 = CvarRegistry::new(Dialect::Q3);
        register_gtv_cvars(&mut q3).expect("no-op");
        assert!(q3.get("mvd_username").is_none());
    }

    #[test]
    fn parses_address_with_defaults_and_options() {
        let parsed = parse_gtv_connect(&args(&["demo.example:27910"]), &defaults())
            .expect("parse")
            .expect("request");
        assert_eq!(parsed.address, "demo.example:27910");
        assert_eq!(parsed.username, "observer");
        assert_eq!(parsed.label, None);
        let parsed = parse_gtv_connect(
            &args(&["-u", "cam", "-p", "pw", "-n", "feed", "demo.example"]),
            &defaults(),
        )
        .expect("parse")
        .expect("request");
        assert_eq!(parsed.username, "cam");
        assert_eq!(parsed.password, "pw");
        assert_eq!(parsed.label.as_deref(), Some("feed"));
        let parsed = parse_gtv_connect(&args(&["--", "-h"]), &defaults())
            .expect("parse")
            .expect("request");
        assert_eq!(parsed.address, "-h");
        assert!(parse_gtv_connect(&args(&["-h"]), &defaults()).expect("help").is_none());
    }

    #[test]
    fn rejects_bad_connect_requests() {
        assert!(parse_gtv_connect(&args(&["-z", "x", "y"]), &defaults()).is_err());
        assert!(parse_gtv_connect(&args(&["-u"]), &defaults()).is_err());
        assert!(parse_gtv_connect(&args(&[]), &defaults()).is_err());
        assert!(parse_gtv_connect(&args(&["a", "b"]), &defaults()).is_err());
        assert!(parse_gtv_connect(&args(&["-u", "caf\u{e9}", "a"]), &defaults()).is_err());
    }

    #[test]
    fn matches_disconnect_selection() {
        let current = GtvConnection {
            id: 3,
            label: "feed".to_owned(),
        };
        assert!(matches_gtv_disconnect(&args(&[]), Some(&current)).expect("all"));
        assert!(matches_gtv_disconnect(&args(&["-a"]), Some(&current)).expect("all"));
        assert!(matches_gtv_disconnect(&args(&["3"]), Some(&current)).expect("id"));
        assert!(matches_gtv_disconnect(&args(&["feed"]), Some(&current)).expect("label"));
        assert!(!matches_gtv_disconnect(&args(&["-h"]), Some(&current)).expect("help"));
        assert!(matches_gtv_disconnect(&args(&["9"]), Some(&current)).is_err());
        assert!(matches_gtv_disconnect(&args(&[]), None).is_err());
        assert!(matches_gtv_disconnect(&args(&["a", "b"]), Some(&current)).is_err());
    }
}
