//! Navigation asset inspection (donor `tools/navigation/inspect.ts`).
//!
//! Opens a content archive, parses an `.aas` or Kex navigation member with
//! the merged `qa-bots` parsers, and renders the donor's JSON report:
//! archive identity, asset counts, a travel-mode histogram, and the
//! inspection-only admission note. Members ending in `.aas` (case
//! sensitive, like the donor) parse as AAS; every other member parses as
//! Kex NAV2/NAV3. An optional BSP member supplies the expected checksum
//! for AAS verification.

use std::path::Path;

use qa_bots::md4::block_checksum;
use qa_bots::{aas_travel_mode, kex_travel_mode, parse_aas, parse_kex_navigation, BotsError, KexGeneration};
use qa_content::archive::{open_archive, ArchiveError, ArchivePathComparison, EntryRef, OpenArchive};
use thiserror::Error;

/// Usage line, preserved from the donor.
pub const USAGE: &str = "Usage: bun tools/navigation/inspect.ts <archive> <navigation-member> [bsp-member]";

/// Admission note, preserved from the donor report.
pub const ADMISSION_NOTE: &str =
    "Asset inspection only; route admission requires selected movement and live shared collision.";

/// Failure of navigation asset inspection.
#[derive(Debug, Error)]
pub enum InspectError {
    /// Wrong argument count; the message is [`USAGE`].
    #[error("{0}")]
    Usage(String),
    /// Navigation member absent from the archive.
    #[error("Navigation member missing: {0}")]
    NavigationMemberMissing(String),
    /// BSP member absent from the archive.
    #[error("Map member missing: {0}")]
    MapMemberMissing(String),
    /// Archive open, lookup, or entry-read failure.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// Navigation parse or checksum failure.
    #[error(transparent)]
    Bots(#[from] BotsError),
}

impl InspectError {
    /// Build the usage failure.
    #[must_use]
    pub fn usage() -> Self {
        Self::Usage(USAGE.to_string())
    }

    /// Build a missing-navigation-member failure.
    #[must_use]
    pub fn navigation_member_missing(member: impl Into<String>) -> Self {
        Self::NavigationMemberMissing(member.into())
    }

    /// Build a missing-BSP-member failure.
    #[must_use]
    pub fn map_member_missing(member: impl Into<String>) -> Self {
        Self::MapMemberMissing(member.into())
    }
}

/// Inspected asset family (donor `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationKind {
    /// Quake III AAS.
    Aas,
    /// Kex NAV2 (Quake I rerelease).
    Nav2,
    /// Kex NAV3 (Quake II rerelease).
    Nav3,
}

impl NavigationKind {
    /// Donor kind name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aas => "aas",
            Self::Nav2 => "nav2",
            Self::Nav3 => "nav3",
        }
    }
}

impl From<KexGeneration> for NavigationKind {
    fn from(generation: KexGeneration) -> Self {
        match generation {
            KexGeneration::Nav2 => Self::Nav2,
            KexGeneration::Nav3 => Self::Nav3,
        }
    }
}

/// Asset-specific section of the report (donor `source` object).
#[derive(Debug, Clone, PartialEq)]
pub enum NavigationSourceSummary {
    /// AAS checksum verification and cluster counts.
    Aas {
        /// Checksum the AAS was built against.
        bsp_checksum: i32,
        /// BSP member the checksum was verified against, if any.
        checked_map: Option<String>,
        /// Cluster count.
        clusters: usize,
        /// Portal count.
        portals: usize,
    },
    /// Kex traversal counts and cost multiplier.
    Kex {
        /// Traversal-hint count.
        traversals: usize,
        /// Link-entity count.
        entities: usize,
        /// Navigation cost multiplier.
        heuristic: f64,
    },
}

/// One travel-mode histogram bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelModeCount {
    /// Donor mode name.
    pub mode: String,
    /// Reachability/link count for the mode.
    pub count: usize,
}

/// Navigation inspection report (donor `console.log` object).
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationReport {
    /// Archive path label, as passed on the command line.
    pub archive: String,
    /// Inspected member path.
    pub member: String,
    /// Member ordinal within the archive.
    pub ordinal: usize,
    /// Member payload length in bytes.
    pub byte_length: usize,
    /// Inspected asset family.
    pub kind: NavigationKind,
    /// Asset format version.
    pub version: i32,
    /// Node count: AAS areas minus the dummy area zero (which is `-1`
    /// for a degenerate empty AAS, exactly like the donor's
    /// `areas.length - 1`), or Kex node count.
    pub nodes: i64,
    /// Reachability (AAS) or link (Kex) count.
    pub reachabilities: usize,
    /// Travel-mode histogram in first-seen order.
    pub travel_modes: Vec<TravelModeCount>,
    /// Asset-specific summary.
    pub source: NavigationSourceSummary,
}

/// Count travel modes in first-seen order (donor `Map` insertion order).
fn count_modes<'a>(modes: impl Iterator<Item = &'a str>) -> Vec<TravelModeCount> {
    let mut counts: Vec<TravelModeCount> = Vec::new();
    for mode in modes {
        match counts.iter_mut().find(|bucket| bucket.mode == mode) {
            Some(bucket) => bucket.count += 1,
            None => counts.push(TravelModeCount {
                mode: mode.to_string(),
                count: 1,
            }),
        }
    }
    counts
}

/// Inspect parsed member bytes, verifying an AAS asset against optional
/// BSP bytes (donor parse/dispatch/count section).
pub fn inspect_navigation_member(
    archive: &str,
    member: &str,
    ordinal: usize,
    bytes: &[u8],
    map: Option<(&str, &[u8])>,
) -> Result<NavigationReport, InspectError> {
    if member.ends_with(".aas") {
        let expected = map
            .map(|(_, map_bytes)| Ok::<i32, BotsError>(block_checksum(map_bytes)? as i32))
            .transpose()?;
        let asset = parse_aas(bytes, member, expected)?;
        let travel_modes = count_modes(
            asset
                .reachability
                .iter()
                .map(|reach| aas_travel_mode(reach.travel_type).as_str()),
        );
        Ok(NavigationReport {
            archive: archive.to_string(),
            member: member.to_string(),
            ordinal,
            byte_length: bytes.len(),
            kind: NavigationKind::Aas,
            version: asset.version,
            nodes: asset.areas.len() as i64 - 1,
            reachabilities: asset.reachability.len(),
            travel_modes,
            source: NavigationSourceSummary::Aas {
                bsp_checksum: asset.bsp_checksum,
                checked_map: map.map(|(name, _)| name.to_string()),
                clusters: asset.clusters.len(),
                portals: asset.portals.len(),
            },
        })
    } else {
        let asset = parse_kex_navigation(bytes, member)?;
        let travel_modes = count_modes(asset.links.iter().map(|link| kex_travel_mode(link.link_type).as_str()));
        Ok(NavigationReport {
            archive: archive.to_string(),
            member: member.to_string(),
            ordinal,
            byte_length: bytes.len(),
            kind: NavigationKind::from(asset.kind),
            version: asset.version,
            nodes: asset.nodes.len() as i64,
            reachabilities: asset.links.len(),
            travel_modes,
            source: NavigationSourceSummary::Kex {
                traversals: asset.traversals.len(),
                entities: asset.entities.len(),
                heuristic: asset.heuristic,
            },
        })
    }
}

/// Look up the navigation (and optional BSP) member of an open archive and
/// inspect it (donor `findEntries`/`readEntry` section).
pub fn inspect_open_archive(
    archive: &OpenArchive,
    archive_label: &str,
    member: &str,
    bsp_member: Option<&str>,
) -> Result<NavigationReport, InspectError> {
    let entry = archive
        .find_entries(member, ArchivePathComparison::Exact)?
        .into_iter()
        .next()
        .ok_or_else(|| InspectError::navigation_member_missing(member))?;
    let map = match bsp_member {
        Some(map_path) => {
            let map_entry = archive
                .find_entries(map_path, ArchivePathComparison::Exact)?
                .into_iter()
                .next()
                .ok_or_else(|| InspectError::map_member_missing(map_path))?;
            Some((map_path, archive.read_entry(EntryRef::Entry(&map_entry))?))
        }
        None => None,
    };
    let bytes = archive.read_entry(EntryRef::Entry(&entry))?;
    inspect_navigation_member(
        archive_label,
        member,
        entry.ordinal(),
        &bytes,
        map.as_ref().map(|(name, bytes)| (*name, bytes.as_slice())),
    )
}

/// Open an archive file, inspect one navigation member, and close the
/// archive on every path (donor `main` with its `finally` close).
pub fn inspect_archive_member(
    archive_path: &str,
    member: &str,
    bsp_member: Option<&str>,
) -> Result<NavigationReport, InspectError> {
    let archive = open_archive(Path::new(archive_path), None)?;
    let outcome = inspect_open_archive(&archive, archive_path, member, bsp_member);
    archive.close();
    outcome
}

/// Run the donor command line over positional arguments (without the
/// program name): `[archive, member, bsp?]`. Extra arguments are ignored,
/// like the donor. Returns the pretty-printed JSON report.
pub fn run(args: &[String]) -> Result<String, InspectError> {
    if args.len() < 2 {
        return Err(InspectError::usage());
    }
    let report = inspect_archive_member(&args[0], &args[1], args.get(2).map(String::as_str))?;
    Ok(render_report(&report))
}

/// Escape a string the way `JSON.stringify` does.
fn push_json_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0A}' => out.push_str("\\n"),
            '\u{0C}' => out.push_str("\\f"),
            '\u{0D}' => out.push_str("\\r"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Render an `f64` the way `JSON.stringify` renders finite numbers.
fn render_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return "null".to_owned();
    }
    let abs = value.abs();
    if abs >= 1e21 || abs < 1e-6 {
        return render_exponential(value);
    }
    let plain = format!("{value}");
    if plain.contains(['e', 'E']) {
        return render_exponential(value);
    }
    plain
}

fn render_exponential(value: f64) -> String {
    let raw = format!("{value:.17e}");
    let exp_pos = raw.find('e').unwrap_or(raw.len());
    let base_exponent: i32 = raw[exp_pos + 1..].parse().unwrap_or(0);
    let negative = raw.starts_with('-');
    let digits: Vec<u8> = raw[..exp_pos].bytes().filter(|byte| byte.is_ascii_digit()).collect();
    for precision in 1..=digits.len().max(17) {
        let mut kept: Vec<u8> = digits.iter().copied().take(precision).collect();
        kept.resize(precision, b'0');
        let mut exponent = base_exponent;
        let next = digits.get(precision).copied().unwrap_or(b'0');
        if next >= b'5' {
            let mut index = kept.len();
            loop {
                if index == 0 {
                    kept = vec![b'0'; precision];
                    kept[0] = b'1';
                    exponent += 1;
                    break;
                }
                index -= 1;
                if kept[index] < b'9' {
                    kept[index] += 1;
                    break;
                }
                kept[index] = b'0';
            }
        }
        while kept.len() > 1 && kept.last() == Some(&b'0') {
            kept.pop();
        }
        let mantissa: Vec<char> = kept.iter().map(|byte| *byte as char).collect();
        let candidate = render_exp_candidate(negative, &mantissa, exponent);
        if candidate.parse::<f64>().unwrap_or(f64::NAN).to_bits() == value.to_bits() {
            return candidate;
        }
    }
    let mantissa: Vec<char> = digits.iter().map(|byte| *byte as char).collect();
    render_exp_candidate(negative, &mantissa, base_exponent)
}

fn render_exp_candidate(negative: bool, mantissa: &[char], exponent: i32) -> String {
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push(mantissa[0]);
    if mantissa.len() > 1 {
        out.push('.');
        out.extend(mantissa[1..].iter());
    }
    out.push('e');
    out.push_str(&format!("{exponent:+}"));
    out
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_key(out: &mut String, depth: usize, key: &str) {
    push_indent(out, depth);
    push_json_string(key, out);
    out.push_str(": ");
}

/// Render the report as two-space indented JSON
/// (`JSON.stringify(report, null, 2)` style).
#[must_use]
pub fn render_report(report: &NavigationReport) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_key(&mut out, 1, "archive");
    push_json_string(&report.archive, &mut out);
    out.push_str(",\n");
    push_key(&mut out, 1, "member");
    push_json_string(&report.member, &mut out);
    out.push_str(",\n");
    push_key(&mut out, 1, "ordinal");
    out.push_str(&report.ordinal.to_string());
    out.push_str(",\n");
    push_key(&mut out, 1, "bytes");
    out.push_str(&report.byte_length.to_string());
    out.push_str(",\n");
    push_key(&mut out, 1, "kind");
    push_json_string(report.kind.as_str(), &mut out);
    out.push_str(",\n");
    push_key(&mut out, 1, "version");
    out.push_str(&report.version.to_string());
    out.push_str(",\n");
    push_key(&mut out, 1, "nodes");
    out.push_str(&report.nodes.to_string());
    out.push_str(",\n");
    push_key(&mut out, 1, "reachabilities");
    out.push_str(&report.reachabilities.to_string());
    out.push_str(",\n");
    push_key(&mut out, 1, "travelModes");
    if report.travel_modes.is_empty() {
        out.push_str("{}");
    } else {
        out.push_str("{\n");
        for (index, bucket) in report.travel_modes.iter().enumerate() {
            push_key(&mut out, 2, &bucket.mode);
            out.push_str(&bucket.count.to_string());
            if index + 1 < report.travel_modes.len() {
                out.push(',');
            }
            out.push('\n');
        }
        push_indent(&mut out, 1);
        out.push('}');
    }
    out.push_str(",\n");
    push_key(&mut out, 1, "source");
    out.push_str("{\n");
    match &report.source {
        NavigationSourceSummary::Aas {
            bsp_checksum,
            checked_map,
            clusters,
            portals,
        } => {
            push_key(&mut out, 2, "bspChecksum");
            out.push_str(&bsp_checksum.to_string());
            out.push_str(",\n");
            push_key(&mut out, 2, "checkedMap");
            match checked_map {
                Some(map) => push_json_string(map, &mut out),
                None => out.push_str("null"),
            }
            out.push_str(",\n");
            push_key(&mut out, 2, "clusters");
            out.push_str(&clusters.to_string());
            out.push_str(",\n");
            push_key(&mut out, 2, "portals");
            out.push_str(&portals.to_string());
            out.push('\n');
        }
        NavigationSourceSummary::Kex {
            traversals,
            entities,
            heuristic,
        } => {
            push_key(&mut out, 2, "traversals");
            out.push_str(&traversals.to_string());
            out.push_str(",\n");
            push_key(&mut out, 2, "entities");
            out.push_str(&entities.to_string());
            out.push_str(",\n");
            push_key(&mut out, 2, "heuristic");
            out.push_str(&render_number(*heuristic));
            out.push('\n');
        }
    }
    push_indent(&mut out, 1);
    out.push_str("},\n");
    push_key(&mut out, 1, "admission");
    push_json_string(ADMISSION_NOTE, &mut out);
    out.push_str("\n}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::archive::decode_archive;
    use std::fs;

    fn push_i32(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn crc32_ieee(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xffff_ffff;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                if crc & 1 == 0 {
                    crc >>= 1;
                } else {
                    crc = (crc >> 1) ^ 0xedb8_8320;
                }
            }
        }
        crc ^ 0xffff_ffff
    }

    /// Minimal AAS v4: `area_count` empty areas with matching settings and
    /// one reachability per travel type.
    fn fixture_aas(checksum: i32, area_count: usize, travel_types: &[i32]) -> Vec<u8> {
        let areas_len = area_count * 48;
        let settings_len = area_count * 28;
        let reach_len = travel_types.len() * 44;
        let areas_offset = 124;
        let settings_offset = areas_offset + areas_len;
        let reach_offset = settings_offset + settings_len;
        let mut out = Vec::new();
        push_u32(&mut out, 0x5341_4145);
        push_i32(&mut out, 4);
        push_i32(&mut out, checksum);
        for lump in 0..14 {
            let (offset, length) = match lump {
                7 => (areas_offset, areas_len),
                8 => (settings_offset, settings_len),
                9 => (reach_offset, reach_len),
                _ => (0, 0),
            };
            push_i32(&mut out, offset as i32);
            push_i32(&mut out, length as i32);
        }
        for number in 0..area_count {
            push_i32(&mut out, number as i32);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            for _ in 0..9 {
                push_f32(&mut out, 0.0);
            }
        }
        for setting in 0..area_count {
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, if setting == 0 { travel_types.len() as i32 } else { 0 });
            push_i32(&mut out, 0);
        }
        for travel_type in travel_types {
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            for _ in 0..6 {
                push_f32(&mut out, 0.0);
            }
            push_i32(&mut out, *travel_type);
            push_u16(&mut out, 0);
            push_u16(&mut out, 0);
        }
        out
    }

    /// Minimal Kex NAV2 v12: one node with one link per link type.
    fn fixture_nav2(link_types: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"NAV2");
        push_i32(&mut out, 12);
        push_i32(&mut out, 1);
        push_i32(&mut out, link_types.len() as i32);
        push_i32(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, link_types.len() as u16);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        for _ in 0..3 {
            push_f32(&mut out, 0.0);
        }
        for link_type in link_types {
            push_u16(&mut out, 0);
            out.push(*link_type);
            out.push(0);
            push_u16(&mut out, 0xffff);
        }
        push_i32(&mut out, 0);
        out
    }

    /// Minimal stored (uncompressed) ZIP with no prefix or comment.
    fn fixture_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, content) in entries {
            let offset = out.len() as u32;
            let crc = crc32_ieee(content);
            push_u32(&mut out, 0x0403_4b50);
            push_u16(&mut out, 20);
            push_u16(&mut out, 0);
            push_u16(&mut out, 0);
            push_u16(&mut out, 0);
            push_u16(&mut out, 0);
            push_u32(&mut out, crc);
            push_u32(&mut out, content.len() as u32);
            push_u32(&mut out, content.len() as u32);
            push_u16(&mut out, name.len() as u16);
            push_u16(&mut out, 0);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(content);
            push_u32(&mut central, 0x0201_4b50);
            push_u16(&mut central, 20);
            push_u16(&mut central, 20);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u32(&mut central, crc);
            push_u32(&mut central, content.len() as u32);
            push_u32(&mut central, content.len() as u32);
            push_u16(&mut central, name.len() as u16);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u32(&mut central, 0);
            push_u32(&mut central, offset);
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = out.len() as u32;
        let central_size = central.len() as u32;
        out.extend_from_slice(&central);
        push_u32(&mut out, 0x0605_4b50);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, entries.len() as u16);
        push_u16(&mut out, entries.len() as u16);
        push_u32(&mut out, central_size);
        push_u32(&mut out, central_offset);
        push_u16(&mut out, 0);
        out
    }

    const MAP_BYTES: &[u8] = b"fake bsp bytes for checksum";

    fn map_checksum() -> i32 {
        block_checksum(MAP_BYTES).unwrap() as i32
    }

    #[test]
    fn usage_error_without_two_args() {
        for args in [vec![], vec!["archive.pk3".to_string()]] {
            let error = run(&args).unwrap_err();
            assert_eq!(error.to_string(), USAGE);
            assert!(matches!(error, InspectError::Usage(_)));
        }
    }

    #[test]
    fn missing_navigation_member() {
        let zip = fixture_zip(&[("maps/other.bsp", b"bsp")]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let error = inspect_open_archive(&archive, "fixture.zip", "maps/test.aas", None).unwrap_err();
        assert_eq!(error.to_string(), "Navigation member missing: maps/test.aas");
        assert!(matches!(error, InspectError::NavigationMemberMissing(_)));
    }

    #[test]
    fn missing_map_member() {
        let aas = fixture_aas(map_checksum(), 1, &[2]);
        let zip = fixture_zip(&[("maps/test.aas", &aas)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let error = inspect_open_archive(&archive, "fixture.zip", "maps/test.aas", Some("maps/test.bsp")).unwrap_err();
        assert_eq!(error.to_string(), "Map member missing: maps/test.bsp");
        assert!(matches!(error, InspectError::MapMemberMissing(_)));
    }

    #[test]
    fn checksum_mismatch_rejected() {
        let aas = fixture_aas(map_checksum().wrapping_add(1), 1, &[2]);
        let zip = fixture_zip(&[("maps/test.aas", &aas), ("maps/test.bsp", MAP_BYTES)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let error = inspect_open_archive(&archive, "fixture.zip", "maps/test.aas", Some("maps/test.bsp")).unwrap_err();
        assert!(error.to_string().contains("AAS belongs to a different BSP checksum"));
    }

    #[test]
    fn inspect_aas_with_map() {
        let aas = fixture_aas(map_checksum(), 1, &[99, 2, 2]);
        let zip = fixture_zip(&[("maps/test.aas", &aas), ("maps/test.bsp", MAP_BYTES)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let report = inspect_open_archive(&archive, "fixture.zip", "maps/test.aas", Some("maps/test.bsp")).unwrap();
        assert_eq!(report.archive, "fixture.zip");
        assert_eq!(report.member, "maps/test.aas");
        assert_eq!(report.ordinal, 0);
        assert_eq!(report.byte_length, 332);
        assert_eq!(report.kind, NavigationKind::Aas);
        assert_eq!(report.version, 4);
        assert_eq!(report.nodes, 0);
        assert_eq!(report.reachabilities, 3);
        assert_eq!(
            report.travel_modes,
            vec![
                TravelModeCount {
                    mode: "unknown".to_string(),
                    count: 1,
                },
                TravelModeCount {
                    mode: "walk".to_string(),
                    count: 2,
                },
            ]
        );
        assert_eq!(
            report.source,
            NavigationSourceSummary::Aas {
                bsp_checksum: map_checksum(),
                checked_map: Some("maps/test.bsp".to_string()),
                clusters: 0,
                portals: 0,
            }
        );
        let expected = format!(
            "{{\n  \"archive\": \"fixture.zip\",\n  \"member\": \"maps/test.aas\",\n  \"ordinal\": 0,\n  \"bytes\": 332,\
             \n  \"kind\": \"aas\",\n  \"version\": 4,\n  \"nodes\": 0,\n  \"reachabilities\": 3,\
             \n  \"travelModes\": {{\n    \"unknown\": 1,\n    \"walk\": 2\n  }},\
             \n  \"source\": {{\n    \"bspChecksum\": {},\n    \"checkedMap\": \"maps/test.bsp\",\
             \n    \"clusters\": 0,\n    \"portals\": 0\n  }},\
             \n  \"admission\": \"{ADMISSION_NOTE}\"\n}}",
            map_checksum()
        );
        assert_eq!(render_report(&report), expected);
    }

    #[test]
    fn inspect_aas_without_map_skips_checksum() {
        let aas = fixture_aas(0x1234_5678, 1, &[4]);
        let zip = fixture_zip(&[("maps/test.aas", &aas)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let report = inspect_open_archive(&archive, "fixture.zip", "maps/test.aas", None).unwrap();
        assert_eq!(report.kind, NavigationKind::Aas);
        assert_eq!(
            report.travel_modes,
            vec![TravelModeCount {
                mode: "jump".to_string(),
                count: 1,
            }]
        );
        assert!(matches!(
            report.source,
            NavigationSourceSummary::Aas {
                bsp_checksum: 0x1234_5678,
                checked_map: None,
                ..
            }
        ));
        assert!(render_report(&report).contains("\"checkedMap\": null"));
    }

    #[test]
    fn inspect_kex_nav2() {
        let nav = fixture_nav2(&[1, 0]);
        let zip = fixture_zip(&[("maps/test.nav", &nav)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let report = inspect_open_archive(&archive, "fixture.zip", "maps/test.nav", None).unwrap();
        assert_eq!(report.kind, NavigationKind::Nav2);
        assert_eq!(report.version, 12);
        assert_eq!(report.nodes, 1);
        assert_eq!(report.reachabilities, 2);
        assert_eq!(
            report.source,
            NavigationSourceSummary::Kex {
                traversals: 0,
                entities: 0,
                heuristic: 1.0,
            }
        );
        let expected = "{\n  \"archive\": \"fixture.zip\",\n  \"member\": \"maps/test.nav\",\n  \"ordinal\": 0,\
         \n  \"bytes\": 56,\n  \"kind\": \"nav2\",\n  \"version\": 12,\n  \"nodes\": 1,\n  \"reachabilities\": 2,\
         \n  \"travelModes\": {\n    \"jump\": 1,\n    \"walk\": 1\n  },\
         \n  \"source\": {\n    \"traversals\": 0,\n    \"entities\": 0,\n    \"heuristic\": 1\n  },\
         \n  \"admission\": \"Asset inspection only; route admission requires selected movement and live shared collision.\"\n}";
        assert_eq!(render_report(&report), expected);
    }

    #[test]
    fn member_suffix_is_case_sensitive() {
        let aas = fixture_aas(map_checksum(), 1, &[2]);
        let zip = fixture_zip(&[("maps/test.AAS", &aas)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let error = inspect_open_archive(&archive, "fixture.zip", "maps/test.AAS", None).unwrap_err();
        assert!(error.to_string().contains("expected NAV2 or NAV3"));
    }

    #[test]
    fn empty_aas_reports_negative_one_nodes() {
        let aas = fixture_aas(0, 0, &[]);
        let zip = fixture_zip(&[("maps/empty.aas", &aas)]);
        let archive = decode_archive(&zip, None, "fixture.zip").unwrap();
        let report = inspect_open_archive(&archive, "fixture.zip", "maps/empty.aas", None).unwrap();
        assert_eq!(report.nodes, -1);
        assert_eq!(report.reachabilities, 0);
        assert!(report.travel_modes.is_empty());
        let json = render_report(&report);
        assert!(json.contains("\"nodes\": -1"));
        assert!(json.contains("\"travelModes\": {}"));
    }

    #[test]
    fn render_number_matches_json_stringify() {
        assert_eq!(render_number(0.30000000000000004), "0.30000000000000004");
        assert_eq!(render_number(-0.0), "0");
        assert_eq!(render_number(1e21), "1e+21");
        assert_eq!(render_number(1e-7), "1e-7");
        assert_eq!(render_number(0.10000000149011612), "0.10000000149011612");
        assert_eq!(render_number(123.0), "123");
        assert_eq!(render_number(1.0), "1");
        assert_eq!(render_number(1.5), "1.5");
    }

    #[test]
    fn push_json_string_escapes_like_json_stringify() {
        let mut out = String::new();
        push_json_string("a\"b\\c\nd\u{1}e\u{7f}f", &mut out);
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001e\u{7f}f\"");
    }

    #[test]
    fn run_end_to_end_over_file() {
        let aas = fixture_aas(map_checksum(), 1, &[2]);
        let zip = fixture_zip(&[("maps/test.aas", &aas), ("maps/test.bsp", MAP_BYTES)]);
        let path = std::env::temp_dir().join(format!("qa-tools-inspect-{}-e2e.pk3", std::process::id()));
        fs::write(&path, &zip).unwrap();
        let outcome = run(&[
            path.to_string_lossy().into_owned(),
            "maps/test.aas".to_string(),
            "maps/test.bsp".to_string(),
            "ignored-extra".to_string(),
        ]);
        let _ = fs::remove_file(&path);
        let json = outcome.unwrap();
        assert!(json.contains("\"kind\": \"aas\""));
        assert!(json.contains("\"walk\": 1"));
        assert!(json.contains(ADMISSION_NOTE));
    }
}
