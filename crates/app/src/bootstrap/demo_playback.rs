//! Demo family selection and mounted-demo opening.
//!
//! Donor provenance: `src/app/bootstrap/demo-playback.ts` (`demoFamily`,
//! `openDemoResource`, `DemoFamily`, `DemoRequest`, `DemoResource`).
//!
//! Sync port with no behavioral changes: the donor's async `read` becomes a
//! sync closure returning a result, and every message, candidate order, and
//! error string is preserved.

use qa_content::paths::{normalize_resource_path, PathError};
use thiserror::Error;

/// Demo packet family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoFamily {
    /// NetQuake (`.dem`).
    Q1,
    /// QuakeWorld (`.qwd`).
    Qw,
    /// Quake II (`.dm2`/`.mvd`).
    Q2,
    /// Quake III (`.dm_66`/`.dm_67`/`.dm_68`).
    Q3,
}

/// Demo open request (donor `DemoRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoRequest {
    /// Requested family.
    pub family: DemoFamily,
    /// Resource name, with or without an extension.
    pub name: String,
    /// Timedemo playback.
    pub timedemo: bool,
}

/// Supported Quake III demo protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3DemoProtocol {
    /// Protocol 66.
    P66,
    /// Protocol 67.
    P67,
    /// Protocol 68.
    P68,
}

impl Q3DemoProtocol {
    /// Protocol number.
    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Self::P66 => 66,
            Self::P67 => 67,
            Self::P68 => 68,
        }
    }

    /// Parse a protocol number.
    #[must_use]
    pub fn from_number(number: u64) -> Option<Self> {
        match number {
            66 => Some(Self::P66),
            67 => Some(Self::P67),
            68 => Some(Self::P68),
            _ => None,
        }
    }
}

/// Opened demo bytes (donor `DemoResource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoResource {
    /// Quake, QuakeWorld, or Quake II demo.
    Standard {
        /// Demo family.
        family: DemoFamily,
        /// Resolved resource path.
        path: String,
        /// Demo bytes.
        bytes: Vec<u8>,
    },
    /// Quake III demo with its protocol.
    Q3 {
        /// Resolved resource path.
        path: String,
        /// Demo bytes.
        bytes: Vec<u8>,
        /// Demo protocol.
        protocol: Q3DemoProtocol,
    },
}

/// Failure opening a demo.
#[derive(Debug, Error)]
pub enum DemoPlaybackError {
    /// The demo could not be opened (carries the donor message).
    #[error("{0}")]
    Open(String),
    /// The mounted-file lookup failed.
    #[error("demo read failed: {0}")]
    Read(String),
    /// The request name was not a valid resource path.
    #[error(transparent)]
    Path(#[from] PathError),
}

/// Select the packet family from an explicit suffix, else the fallback.
#[must_use]
pub fn demo_family(name: &str, fallback: DemoFamily) -> DemoFamily {
    let lower = name.to_lowercase();
    if lower.ends_with(".qwd") {
        DemoFamily::Qw
    } else if lower.ends_with(".dem") {
        DemoFamily::Q1
    } else if lower.ends_with(".dm2") || lower.ends_with(".mvd") {
        DemoFamily::Q2
    } else if dm_suffix(&lower).is_some() {
        DemoFamily::Q3
    } else {
        fallback
    }
}

fn has_extension(path: &str) -> bool {
    match (path.rfind('.'), path.rfind('/')) {
        (Some(dot), Some(slash)) => dot > slash,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Split a trailing `.dm_<digits>` suffix into stem and digits.
fn dm_suffix(filename: &str) -> Option<(&str, &str)> {
    let dot = filename.rfind('.')?;
    let extension = &filename[dot + 1..];
    if extension.len() > 3 && extension.starts_with("dm_") && extension[3..].bytes().all(|byte| byte.is_ascii_digit()) {
        Some((&filename[..dot], &extension[3..]))
    } else {
        None
    }
}

/// Mounted-file lookup for demo bytes.
pub type DemoReader<'a, E> = &'a mut dyn FnMut(&str) -> Result<Option<Vec<u8>>, E>;

/// Open a demo through a mounted-file lookup with mod precedence.
pub fn open_demo_resource<E: std::fmt::Display>(
    request: &DemoRequest,
    read: DemoReader<'_, E>,
    print: &mut dyn FnMut(&str),
) -> Result<DemoResource, DemoPlaybackError> {
    let name = normalize_resource_path(&request.name)?;
    if request.family != DemoFamily::Q3 {
        let extension = match request.family {
            DemoFamily::Q1 => ".dem",
            DemoFamily::Qw => ".qwd",
            DemoFamily::Q2 => ".dm2",
            DemoFamily::Q3 => unreachable!("checked above"),
        };
        let filename = if has_extension(&name) {
            name.clone()
        } else {
            format!("{name}{extension}")
        };
        let path = if request.family == DemoFamily::Q2 && !filename.starts_with("demos/") {
            format!("demos/{filename}")
        } else {
            filename
        };
        let bytes = read(&path).map_err(|error| DemoPlaybackError::Read(error.to_string()))?;
        return match bytes {
            Some(bytes) => Ok(DemoResource::Standard {
                family: request.family,
                path,
                bytes,
            }),
            None => Err(DemoPlaybackError::Open(format!("Couldn't open demo {path}"))),
        };
    }

    let filename = name.strip_prefix("demos/").unwrap_or(&name);
    let (stem, digits) = match dm_suffix(filename) {
        Some((stem, digits)) => (stem, Some(digits)),
        None => (filename, None),
    };
    let requested = digits.and_then(|digits| digits.parse::<u64>().ok());
    let supported = matches!(requested, Some(66..=68));
    if digits.is_some() && !supported {
        let shown = requested
            .map(|number| number.to_string())
            .unwrap_or_else(|| digits.expect("digits checked above").to_string());
        print(&format!("Protocol {shown} not supported for demos\n"));
    }
    if supported {
        let number = requested.expect("supported implies a protocol");
        let path = format!("demos/{filename}");
        let bytes = read(&path).map_err(|error| DemoPlaybackError::Read(error.to_string()))?;
        return match bytes {
            Some(bytes) => {
                let protocol = Q3DemoProtocol::from_number(number)
                    .ok_or_else(|| DemoPlaybackError::Open("Invalid Quake III demo protocol selection".to_string()))?;
                Ok(DemoResource::Q3 { path, bytes, protocol })
            }
            None => Err(DemoPlaybackError::Open(format!("Couldn't open demo {name}"))),
        };
    }
    for number in [66u64, 67, 68] {
        let path = format!("demos/{stem}.dm_{number}");
        match read(&path).map_err(|error| DemoPlaybackError::Read(error.to_string()))? {
            Some(bytes) => {
                let protocol = Q3DemoProtocol::from_number(number)
                    .ok_or_else(|| DemoPlaybackError::Open("Invalid Quake III demo protocol selection".to_string()))?;
                return Ok(DemoResource::Q3 { path, bytes, protocol });
            }
            None => {
                print(&format!("Not found: {path}\n"));
            }
        }
    }
    Err(DemoPlaybackError::Open(format!("Couldn't open demo {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn read(files: &HashMap<String, Vec<u8>>) -> impl FnMut(&str) -> Result<Option<Vec<u8>>, String> + '_ {
        move |path: &str| Ok(files.get(path).cloned())
    }

    fn request(family: DemoFamily, name: &str) -> DemoRequest {
        DemoRequest {
            family,
            name: name.to_string(),
            timedemo: false,
        }
    }

    #[test]
    fn family_suffix_selection() {
        assert_eq!(demo_family("x.qwd", DemoFamily::Q1), DemoFamily::Qw);
        assert_eq!(demo_family("x.DEM", DemoFamily::Q3), DemoFamily::Q1);
        assert_eq!(demo_family("x.dm2", DemoFamily::Q1), DemoFamily::Q2);
        assert_eq!(demo_family("x.mvd", DemoFamily::Q1), DemoFamily::Q2);
        assert_eq!(demo_family("x.dm_68", DemoFamily::Q1), DemoFamily::Q3);
        assert_eq!(demo_family("x", DemoFamily::Qw), DemoFamily::Qw);
    }

    #[test]
    fn standard_families_resolve_paths() {
        let mut files = HashMap::new();
        files.insert("e1m1.dem".to_string(), vec![1]);
        files.insert("demos/q2.dm2".to_string(), vec![2]);
        let mut read = read(&files);
        let mut printed = Vec::new();
        let mut print = |text: &str| printed.push(text.to_string());
        let resource = open_demo_resource(&request(DemoFamily::Q1, "e1m1"), &mut read, &mut print).unwrap();
        assert!(matches!(
            resource,
            DemoResource::Standard {
                family: DemoFamily::Q1,
                ..
            }
        ));
        let resource = open_demo_resource(&request(DemoFamily::Q2, "q2"), &mut read, &mut print).unwrap();
        match resource {
            DemoResource::Standard { path, bytes, .. } => {
                assert_eq!(path, "demos/q2.dm2");
                assert_eq!(bytes, [2]);
            }
            DemoResource::Q3 { .. } => panic!("wrong kind"),
        }
        assert!(printed.is_empty());
    }

    #[test]
    fn missing_standard_demo_errors_with_path() {
        let files = HashMap::new();
        let mut read = read(&files);
        let mut print = |_: &str| {};
        let error = open_demo_resource(&request(DemoFamily::Q1, "absent"), &mut read, &mut print).unwrap_err();
        assert_eq!(error.to_string(), "Couldn't open demo absent.dem");
    }

    #[test]
    fn q3_explicit_protocol_and_fallback_chain() {
        let mut files = HashMap::new();
        files.insert("demos/a.dm_67".to_string(), vec![6]);
        files.insert("demos/b.dm_68".to_string(), vec![7]);
        let mut read = read(&files);
        let mut printed = Vec::new();
        {
            let mut print = |text: &str| printed.push(text.to_string());
            let resource = open_demo_resource(&request(DemoFamily::Q3, "a.dm_67"), &mut read, &mut print).unwrap();
            match resource {
                DemoResource::Q3 { protocol, bytes, .. } => {
                    assert_eq!(protocol, Q3DemoProtocol::P67);
                    assert_eq!(bytes, [6]);
                }
                _ => panic!("wrong kind"),
            }
        }
        assert!(printed.is_empty());
        {
            let mut print = |text: &str| printed.push(text.to_string());
            let resource = open_demo_resource(&request(DemoFamily::Q3, "b"), &mut read, &mut print).unwrap();
            assert!(matches!(
                resource,
                DemoResource::Q3 {
                    protocol: Q3DemoProtocol::P68,
                    ..
                }
            ));
        }
        assert_eq!(printed, ["Not found: demos/b.dm_66\n", "Not found: demos/b.dm_67\n",]);
    }

    #[test]
    fn q3_unsupported_protocol_and_missing() {
        let files = HashMap::new();
        let mut read = read(&files);
        let mut printed = Vec::new();
        {
            let mut print = |text: &str| printed.push(text.to_string());
            let error = open_demo_resource(&request(DemoFamily::Q3, "x.dm_99"), &mut read, &mut print).unwrap_err();
            assert_eq!(error.to_string(), "Couldn't open demo x.dm_99");
        }
        assert_eq!(
            printed,
            [
                "Protocol 99 not supported for demos\n",
                "Not found: demos/x.dm_66\n",
                "Not found: demos/x.dm_67\n",
                "Not found: demos/x.dm_68\n",
            ]
        );
    }

    #[test]
    fn read_errors_propagate() {
        let mut read = |_: &str| -> Result<Option<Vec<u8>>, String> { Err("boom".to_string()) };
        let mut print = |_: &str| {};
        let error = open_demo_resource(&request(DemoFamily::Q1, "x"), &mut read, &mut print).unwrap_err();
        assert!(matches!(error, DemoPlaybackError::Read(_)));
    }
}
