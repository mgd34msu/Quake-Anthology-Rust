//! Additional fixture discovery (donor `tools/inventory/additional-fixtures.ts`).
//!
//! Walks Quake-related downloads and the Steam Quake directory for archive
//! candidates, hashes whole files, and parses bounded PAK and ZIP/ZIP64
//! directories without extracting payloads (except the stored PlanetQuake
//! CSV manifest, read in memory). Account/config/save/key names are omitted
//! from every report; nested archives and unsupported formats keep only
//! whole-file hash and header evidence.
//!
//! The bounded archive readers are intentionally module-local like the
//! donors: limits, ZIP64 support, and error text differ per tool. The
//! `generator` tag keeps the donor string so `--check` validates
//! donor-generated reports byte-for-byte.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{parse_json, Json};
use crate::reference::environment::quake_typescript_root;
use crate::sha256::{to_hex, Hasher};
use crate::verify::hash::hash_bytes;

/// Discovery bounds (donor `limits`).
#[derive(Debug, Clone, Copy)]
struct Limits {
    depth: usize,
    visited_entries: usize,
    archive_bytes: u64,
    directory_bytes: usize,
    metadata_bytes: usize,
    archive_members: usize,
    reported_members: usize,
    output_bytes: usize,
}

const LIMITS: Limits = Limits {
    depth: 12,
    visited_entries: 150_000,
    archive_bytes: 64 * 1024 * 1024 * 1024,
    directory_bytes: 64 * 1024 * 1024,
    metadata_bytes: 8 * 1024 * 1024,
    archive_members: 300_000,
    reported_members: 3000,
    output_bytes: 32 * 1024 * 1024,
};

fn limits_json() -> Json {
    Json::object(vec![
        ("depth".to_owned(), Json::uint(LIMITS.depth as u64)),
        ("visitedEntries".to_owned(), Json::uint(LIMITS.visited_entries as u64)),
        ("archiveBytes".to_owned(), Json::uint(LIMITS.archive_bytes)),
        ("directoryBytes".to_owned(), Json::uint(LIMITS.directory_bytes as u64)),
        ("metadataBytes".to_owned(), Json::uint(LIMITS.metadata_bytes as u64)),
        ("archiveMembers".to_owned(), Json::uint(LIMITS.archive_members as u64)),
        ("reportedMembers".to_owned(), Json::uint(LIMITS.reported_members as u64)),
        ("outputBytes".to_owned(), Json::uint(LIMITS.output_bytes as u64)),
    ])
}

fn downloads_root() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Downloads")
}

fn steam_quake_root() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/Steam/steamapps/common/Quake")
}

/// Whether a top-level downloads name is Quake-related (donor `relevantName`).
fn relevant_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.contains("quake") {
        return true;
    }
    if let Some(rest) = lower.strip_prefix('q') {
        if let Some(second) = rest.chars().next() {
            if matches!(second, '1' | '2' | '3') {
                let tail = &rest[second.len_utf8()..];
                if tail.starts_with("source") {
                    return true;
                }
                if let Some(next) = tail.chars().next() {
                    if next.is_ascii_alphanumeric() || next == '_' || next == '-' {
                        return true;
                    }
                }
            }
        }
    }
    if let Some(rest) = lower.strip_prefix("pak") {
        if let Some(next) = rest.chars().next() {
            if next.is_ascii_digit() {
                return true;
            }
        }
        if rest.starts_with("explr") {
            return true;
        }
    }
    lower.starts_with("glad") || lower.starts_with("lmctf")
}

/// Whether a path or member name is sensitive (donor `sensitiveName`).
fn sensitive_name(path: &str) -> bool {
    let lower = path.to_lowercase();
    for segment in lower.split('/') {
        if matches!(
            segment,
            "config"
                | "configs"
                | "save"
                | "saves"
                | "savedgames"
                | "account"
                | "accounts"
                | "credential"
                | "credentials"
                | "secret"
                | "secrets"
        ) {
            return true;
        }
    }
    lower.contains("cdkey")
        || lower.contains("q3key")
        || lower.contains("qzconfig")
        || lower.contains("steam_autocloud")
        || lower.ends_with(".cfg")
        || lower.ends_with(".ini")
        || lower.ends_with(".vdf")
        || lower.ends_with(".acf")
}

/// Whether an archive member is a targeted fixture (donor `wantedMember`).
fn wanted_member(path: &str) -> bool {
    let lower = path.to_lowercase();
    contains_fuzzy(&lower, "quake", "64")
        || contains_fuzzy(&lower, "q1", "n64")
        || contains_fuzzy(&lower, "quake", "n64")
        || lower.ends_with(".md4")
        || lower.split('/').next_back().is_some_and(|base| base == "gamex86.dll")
}

/// Case-folded `head` + optional character + `tail` search (donor `.?`).
fn contains_fuzzy(haystack: &str, head: &str, tail: &str) -> bool {
    let mut index = 0;
    while let Some(found) = haystack[index..].find(head) {
        let after = index + found + head.len();
        if haystack[after..].starts_with(tail) {
            return true;
        }
        if after < haystack.len() {
            let skip = haystack[after..].chars().next().map_or(1, |ch| ch.len_utf8());
            if haystack[after + skip..].starts_with(tail) {
                return true;
            }
        }
        index = after;
    }
    false
}

/// Whether an FTP manifest line is targeted (donor line filter).
fn wanted_manifest_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    if sensitive_name(&lower) {
        return false;
    }
    if contains_fuzzy(&lower, "quake", "64")
        || contains_fuzzy(&lower, "q1", "n64")
        || contains_fuzzy(&lower, "quake", "n64")
    {
        return true;
    }
    if let Some(position) = lower.find(".md4") {
        let after = &lower[position + 4..];
        if after.is_empty()
            || after.starts_with(['"', ';', ','])
            || after.chars().next().is_some_and(char::is_whitespace)
        {
            return true;
        }
    }
    lower.contains("gamex86.dll")
}

/// Node-style extension: from the last dot in the final segment, or empty.
fn extension(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rfind('.') {
        Some(dot) if dot > 0 => &base[dot..],
        _ => "",
    }
}

/// Read an exact range, failing on short reads (donor `read`).
fn read_range(file: &mut File, path: &str, offset: u64, length: usize, size: u64) -> Result<Vec<u8>, ToolsError> {
    if offset.saturating_add(length as u64) > size {
        return Err(ToolsError::invalid(format!(
            "Invalid file range {offset}+{length}/{size}"
        )));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| ToolsError::io(format!("seeking {path}"), error))?;
    let mut bytes = vec![0u8; length];
    let mut done = 0;
    while done < length {
        let count = file
            .read(&mut bytes[done..])
            .map_err(|error| ToolsError::io(format!("reading {path}"), error))?;
        if count == 0 {
            return Err(ToolsError::invalid("File shortened while reading"));
        }
        done += count;
    }
    Ok(bytes)
}

/// Stream-hash a whole file in 4 MiB chunks (donor `hash`).
fn hash_file(file: &mut File, path: &str, size: u64) -> Result<String, ToolsError> {
    let mut hasher = Hasher::new();
    let mut offset = 0;
    while offset < size {
        let length = (size - offset).min(4 * 1024 * 1024) as usize;
        hasher.update(&read_range(file, path, offset, length, size)?);
        offset += length as u64;
    }
    Ok(to_hex(&hasher.finish()))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

/// A safe-range u64 (donor `safe64`).
fn safe_u64(bytes: &[u8], offset: usize) -> Result<u64, ToolsError> {
    let value = u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ]);
    if value > 9_007_199_254_740_991 {
        return Err(ToolsError::invalid("ZIP64 integer exceeds safe range"));
    }
    Ok(value)
}

/// An archive member.
#[derive(Debug, Clone)]
struct Member {
    ordinal: usize,
    path: String,
    bytes: u64,
    compressed_bytes: u64,
    offset: u64,
    compression: u16,
    flags: u16,
    crc32: Option<String>,
}

impl Member {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("ordinal".to_owned(), Json::uint(self.ordinal as u64)),
            ("path".to_owned(), Json::string(&self.path)),
            ("bytes".to_owned(), Json::uint(self.bytes)),
            ("compressedBytes".to_owned(), Json::uint(self.compressed_bytes)),
            ("offset".to_owned(), Json::uint(self.offset)),
            ("compression".to_owned(), Json::uint(u64::from(self.compression))),
            ("flags".to_owned(), Json::uint(u64::from(self.flags))),
            ("crc32".to_owned(), self.crc32.as_ref().map_or(Json::Null, Json::string)),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Pak,
    Zip,
    Zip64,
}

impl Container {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pak => "pak",
            Self::Zip => "zip",
            Self::Zip64 => "zip64",
        }
    }
}

struct Directory {
    kind: Container,
    sha256: String,
    members: Vec<Member>,
}

/// Parse a PAK or ZIP/ZIP64 directory (donor `directory`).
fn parse_directory(file: &mut File, path: &str, size: u64, magic: &str) -> Result<Directory, ToolsError> {
    if magic == "PACK" {
        let header = read_range(file, path, 0, 12.min(size as usize), size)?;
        if header.len() < 12 {
            return Err(ToolsError::invalid("PAK directory exceeds bounds"));
        }
        let offset = read_u32_le(&header, 4) as u64;
        let length = read_u32_le(&header, 8) as usize;
        if length > LIMITS.directory_bytes
            || !length.is_multiple_of(64)
            || length / 64 > LIMITS.archive_members
            || offset.saturating_add(length as u64) > size
        {
            return Err(ToolsError::invalid("PAK directory exceeds bounds"));
        }
        let data = read_range(file, path, offset, length, size)?;
        let mut members = Vec::new();
        for (ordinal, chunk) in data.as_chunks::<64>().0.iter().enumerate() {
            let end = chunk[..56].iter().position(|byte| *byte == 0).unwrap_or(56);
            let position = read_u32_le(chunk, 56) as u64;
            let bytes = read_u32_le(chunk, 60) as u64;
            if position.saturating_add(bytes) > size {
                return Err(ToolsError::invalid("PAK member exceeds file bounds"));
            }
            members.push(Member {
                ordinal,
                path: String::from_utf8_lossy(&chunk[..end]).into_owned(),
                bytes,
                compressed_bytes: bytes,
                offset: position,
                compression: 0,
                flags: 0,
                crc32: None,
            });
        }
        return Ok(Directory {
            kind: Container::Pak,
            sha256: hash_bytes(&data),
            members,
        });
    }
    let tail_start = size.saturating_sub(65_557);
    let tail = read_range(file, path, tail_start, (size - tail_start) as usize, size)?;
    let mut end: Option<usize> = None;
    if tail.len() >= 22 {
        let mut index = tail.len() - 22;
        loop {
            if read_u32_le(&tail, index) == 0x0605_4b50
                && index + 22 + read_u16_le(&tail, index + 20) as usize == tail.len()
            {
                end = Some(index);
                break;
            }
            if index == 0 {
                break;
            }
            index -= 1;
        }
    }
    let end = end.ok_or_else(|| ToolsError::invalid("ZIP end directory absent"))?;
    if read_u16_le(&tail, end + 4) != 0
        || read_u16_le(&tail, end + 6) != 0
        || read_u16_le(&tail, end + 8) != read_u16_le(&tail, end + 10)
    {
        return Err(ToolsError::invalid("Multi-disk ZIP unsupported"));
    }
    let mut count = read_u16_le(&tail, end + 10) as u64;
    let mut length = read_u32_le(&tail, end + 12) as u64;
    let mut offset = read_u32_le(&tail, end + 16) as u64;
    let mut kind = Container::Zip;
    if count == 0xffff || length == 0xffff_ffff || offset == 0xffff_ffff {
        let locator_at = tail_start as i64 + end as i64 - 20;
        if locator_at >= 0 {
            let locator = read_range(file, path, locator_at as u64, 20, size)?;
            if read_u32_le(&locator, 0) != 0x0706_4b50
                || read_u32_le(&locator, 4) != 0
                || read_u32_le(&locator, 16) != 1
            {
                return Err(ToolsError::invalid("Invalid ZIP64 locator"));
            }
            let extended = read_range(file, path, safe_u64(&locator, 8)?, 56, size)?;
            if extended.len() < 56
                || read_u32_le(&extended, 0) != 0x0606_4b50
                || read_u32_le(&extended, 16) != 0
                || read_u32_le(&extended, 20) != 0
                || safe_u64(&extended, 24)? != safe_u64(&extended, 32)?
            {
                return Err(ToolsError::invalid("Invalid ZIP64 end directory"));
            }
            count = safe_u64(&extended, 32)?;
            length = safe_u64(&extended, 40)?;
            offset = safe_u64(&extended, 48)?;
            kind = Container::Zip64;
        } else {
            return Err(ToolsError::invalid(format!(
                "Invalid file range {locator_at}+20/{size}"
            )));
        }
    }
    if length > LIMITS.directory_bytes as u64
        || count > LIMITS.archive_members as u64
        || offset.saturating_add(length) > tail_start + end as u64
    {
        return Err(ToolsError::invalid("ZIP directory exceeds bounds"));
    }
    let length = length as usize;
    let data = read_range(file, path, offset, length, size)?;
    let mut members = Vec::new();
    let mut position = 0;
    for ordinal in 0..count as usize {
        if position + 46 > length || read_u32_le(&data, position) != 0x0201_4b50 {
            return Err(ToolsError::invalid("Invalid ZIP directory member"));
        }
        let name_bytes = read_u16_le(&data, position + 28) as usize;
        let extra_bytes = read_u16_le(&data, position + 30) as usize;
        let next = position + 46 + name_bytes + extra_bytes + read_u16_le(&data, position + 32) as usize;
        if next > length {
            return Err(ToolsError::invalid("ZIP member metadata exceeds directory"));
        }
        let mut bytes = read_u32_le(&data, position + 24) as u64;
        let mut compressed_bytes = read_u32_le(&data, position + 20) as u64;
        let mut local_offset = read_u32_le(&data, position + 42) as u64;
        let mut disk = read_u16_le(&data, position + 34) as u64;
        let extra_end = position + 46 + name_bytes + extra_bytes;
        let mut extra = position + 46 + name_bytes;
        while extra < extra_end {
            if extra + 4 > extra_end {
                return Err(ToolsError::invalid("Truncated ZIP extra field"));
            }
            let id = read_u16_le(&data, extra);
            let field_end = extra + 4 + read_u16_le(&data, extra + 2) as usize;
            if field_end > extra_end {
                return Err(ToolsError::invalid("ZIP extra field exceeds record"));
            }
            if id == 1 {
                let mut cursor = extra + 4;
                let next64 = |cursor: &mut usize| -> Result<u64, ToolsError> {
                    if *cursor + 8 > field_end {
                        return Err(ToolsError::invalid("Truncated ZIP64 value"));
                    }
                    let value = safe_u64(&data, *cursor)?;
                    *cursor += 8;
                    Ok(value)
                };
                if bytes == 0xffff_ffff {
                    bytes = next64(&mut cursor)?;
                }
                if compressed_bytes == 0xffff_ffff {
                    compressed_bytes = next64(&mut cursor)?;
                }
                if local_offset == 0xffff_ffff {
                    local_offset = next64(&mut cursor)?;
                }
                if disk == 0xffff {
                    if cursor + 4 > field_end {
                        return Err(ToolsError::invalid("Truncated ZIP64 disk"));
                    }
                    disk = read_u32_le(&data, cursor) as u64;
                }
            }
            extra = field_end;
        }
        if disk != 0
            || local_offset == 0xffff_ffff
            || bytes == 0xffff_ffff
            || compressed_bytes == 0xffff_ffff
            || local_offset.saturating_add(30).saturating_add(compressed_bytes) > size
        {
            return Err(ToolsError::invalid("ZIP member range/disk invalid or unresolved"));
        }
        members.push(Member {
            ordinal,
            path: String::from_utf8_lossy(&data[position + 46..position + 46 + name_bytes]).into_owned(),
            bytes,
            compressed_bytes,
            offset: local_offset,
            compression: read_u16_le(&data, position + 10),
            flags: read_u16_le(&data, position + 8),
            crc32: Some(format!("{:08x}", read_u32_le(&data, position + 16))),
        });
        position = next;
    }
    if position != length {
        return Err(ToolsError::invalid("Unconsumed ZIP directory data"));
    }
    Ok(Directory {
        kind,
        sha256: hash_bytes(&data),
        members,
    })
}

/// Archive inspection outcome.
#[derive(Debug, Clone)]
enum Inspection {
    Inspected {
        container: Container,
        directory_sha256: String,
        member_count: usize,
        sensitive_names_omitted: usize,
        extension_counts: Vec<(String, usize)>,
        targeted_members: Vec<Member>,
        reported_members: Vec<Member>,
        omitted_members: usize,
    },
    Unsupported {
        reason: String,
    },
    Error {
        reason: String,
    },
}

impl Inspection {
    fn to_json(&self) -> Json {
        match self {
            Self::Inspected {
                container,
                directory_sha256,
                member_count,
                sensitive_names_omitted,
                extension_counts,
                targeted_members,
                reported_members,
                omitted_members,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("inspected")),
                ("container".to_owned(), Json::string(container.as_str())),
                ("directorySha256".to_owned(), Json::string(directory_sha256)),
                ("memberCount".to_owned(), Json::uint(*member_count as u64)),
                (
                    "sensitiveNamesOmitted".to_owned(),
                    Json::uint(*sensitive_names_omitted as u64),
                ),
                (
                    "extensionCounts".to_owned(),
                    Json::object(
                        extension_counts
                            .iter()
                            .map(|(name, count)| (name.clone(), Json::uint(*count as u64)))
                            .collect(),
                    ),
                ),
                (
                    "targetedMemberCount".to_owned(),
                    Json::uint(targeted_members.len() as u64),
                ),
                (
                    "targetedMembers".to_owned(),
                    Json::array(
                        targeted_members
                            .iter()
                            .take(LIMITS.reported_members)
                            .map(Member::to_json)
                            .collect(),
                    ),
                ),
                (
                    "reportedMembers".to_owned(),
                    Json::array(
                        reported_members
                            .iter()
                            .take(LIMITS.reported_members)
                            .map(Member::to_json)
                            .collect(),
                    ),
                ),
                ("omittedMembers".to_owned(), Json::uint(*omitted_members as u64)),
            ]),
            Self::Unsupported { reason } | Self::Error { reason } => Json::object(vec![
                (
                    "kind".to_owned(),
                    Json::string(if matches!(self, Self::Unsupported { .. }) {
                        "unsupported"
                    } else {
                        "error"
                    }),
                ),
                ("reason".to_owned(), Json::string(reason)),
            ]),
        }
    }

    fn targeted_members(&self) -> Vec<Member> {
        match self {
            Self::Inspected { targeted_members, .. } => targeted_members.clone(),
            _ => Vec::new(),
        }
    }
}

/// In-memory PlanetQuake CSV manifest record.
#[derive(Debug, Clone)]
struct MetadataManifest {
    member: Member,
    sha256: String,
    line_count: usize,
    header: String,
    targeted_line_count: usize,
    targeted_lines: Vec<String>,
}

impl MetadataManifest {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("member".to_owned(), self.member.to_json()),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("lineCount".to_owned(), Json::uint(self.line_count as u64)),
            ("header".to_owned(), Json::string(&self.header)),
            (
                "targetedLineCount".to_owned(),
                Json::uint(self.targeted_line_count as u64),
            ),
            (
                "targetedLines".to_owned(),
                Json::array(self.targeted_lines.iter().map(Json::string).collect()),
            ),
        ])
    }
}

/// Whole-file evidence plus inspection.
#[derive(Debug, Clone)]
struct FileEvidence {
    path: String,
    bytes: u64,
    sha256: String,
    header_hex: String,
    qfiles_hash_matches: Vec<String>,
    inspection: Inspection,
    metadata_manifests: Vec<MetadataManifest>,
}

impl FileEvidence {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("bytes".to_owned(), Json::uint(self.bytes)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("headerHex".to_owned(), Json::string(&self.header_hex)),
            (
                "qfilesHashMatches".to_owned(),
                Json::array(self.qfiles_hash_matches.iter().map(Json::string).collect()),
            ),
            ("inspection".to_owned(), self.inspection.to_json()),
            (
                "metadataManifests".to_owned(),
                Json::array(self.metadata_manifests.iter().map(MetadataManifest::to_json).collect()),
            ),
        ])
    }
}

/// Known corpus hashes by sha256 (donor `knownHashes`).
fn known_hashes(project_root: &Path) -> Result<HashMap<String, Vec<String>>, ToolsError> {
    let value = parse_json(&fsutil::read_text(
        &project_root.join("verification/product-manifest.json"),
    )?)?;
    let Some(entries) = value.as_object() else {
        return Err(ToolsError::invalid("Product manifest lacks archive evidence"));
    };
    let Some(archives) = entries
        .iter()
        .find(|(key, _)| key == "archives")
        .and_then(|(_, found)| found.as_array())
    else {
        return Err(ToolsError::invalid("Product manifest lacks archive evidence"));
    };
    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    for item in archives {
        let (Some(path), Some(sha)) = (
            item.get("path").and_then(Json::as_str),
            item.get("sha256").and_then(Json::as_str),
        ) else {
            return Err(ToolsError::invalid("Invalid known archive identity"));
        };
        result.entry(sha.to_owned()).or_default().push(path.to_owned());
    }
    Ok(result)
}

struct Walker {
    visited_entries: usize,
    errors: Vec<(String, String)>,
}

fn walk_file_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".pak") || lower.ends_with(".pk3") || lower.ends_with(".md4") || lower == "gamex86.dll"
}

/// Bounded discovery walk (donor `walk`).
fn walk(root: &str, depth: usize, files: &mut Vec<String>, walker: &mut Walker) -> Result<(), ToolsError> {
    if depth > LIMITS.depth {
        walker
            .errors
            .push((root.to_owned(), "Directory depth bound reached".to_owned()));
        return Ok(());
    }
    let mut entries: Vec<(String, std::fs::FileType)> = Vec::new();
    let listing = std::fs::read_dir(root).map_err(|error| ToolsError::io(format!("listing {root}"), error))?;
    for entry in listing {
        let entry = entry.map_err(|error| ToolsError::io(format!("listing {root}"), error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
        entries.push((entry.file_name().to_string_lossy().into_owned(), file_type));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, file_type) in entries {
        walker.visited_entries += 1;
        if walker.visited_entries > LIMITS.visited_entries {
            return Err(ToolsError::invalid("Discovery entry bound reached"));
        }
        let path = format!("{root}/{name}");
        if sensitive_name(&path) {
            continue;
        }
        if file_type.is_dir() && name != ".git" && name != "node_modules" {
            walk(&path, depth + 1, files, walker)?;
        } else if file_type.is_file() && walk_file_name(&name) {
            files.push(path);
        }
    }
    Ok(())
}

/// Read the stored PlanetQuake CSV manifest in memory.
fn planet_manifest(
    file: &mut File,
    path: &str,
    size: u64,
    directory: &Directory,
) -> Result<Vec<MetadataManifest>, ToolsError> {
    let mut manifests = Vec::new();
    if Path::new(path).file_name().and_then(|name| name.to_str()) != Some("planetquake_ftp.zip") {
        return Ok(manifests);
    }
    let Some(member) = directory.members.iter().find(|item| item.path == "planetquake_ftp.csv") else {
        return Ok(manifests);
    };
    if member.compression != 0
        || member.bytes != member.compressed_bytes
        || member.bytes > LIMITS.metadata_bytes as u64
        || member.flags & 1 != 0
    {
        return Err(ToolsError::invalid("FTP manifest exceeds stored metadata bounds"));
    }
    let local = read_range(file, path, member.offset, 30, size)?;
    if local.len() < 30 || read_u32_le(&local, 0) != 0x0403_4b50 || read_u16_le(&local, 8) != 0 {
        return Err(ToolsError::invalid("Invalid stored manifest local header"));
    }
    let payload_at = member.offset + 30 + read_u16_le(&local, 26) as u64 + read_u16_le(&local, 28) as u64;
    let payload = read_range(file, path, payload_at, member.bytes as usize, size)?;
    let text = String::from_utf8_lossy(&payload).replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let targeted: Vec<String> = lines
        .iter()
        .filter(|line| wanted_manifest_line(line))
        .map(|line| (*line).to_owned())
        .collect();
    manifests.push(MetadataManifest {
        member: member.clone(),
        sha256: hash_bytes(&payload),
        line_count: lines.len(),
        header: lines
            .first()
            .map_or(String::new(), |line| line.chars().take(256).collect()),
        targeted_line_count: targeted.len(),
        targeted_lines: targeted.into_iter().take(LIMITS.reported_members).collect(),
    });
    Ok(manifests)
}

/// Inspect one candidate file (donor `inspect`).
fn inspect(path: &str, known: &HashMap<String, Vec<String>>) -> Result<FileEvidence, ToolsError> {
    let mut file = File::open(path).map_err(|error| ToolsError::io(format!("opening {path}"), error))?;
    let bytes = file
        .metadata()
        .map_err(|error| ToolsError::io(format!("stating {path}"), error))?
        .len();
    if bytes > LIMITS.archive_bytes {
        return Err(ToolsError::invalid("File exceeds hashing limit"));
    }
    let header = read_range(&mut file, path, 0, 64_u64.min(bytes) as usize, bytes)?;
    let sha256 = hash_file(&mut file, path, bytes)?;
    let magic = String::from_utf8_lossy(&header[..4.min(header.len())]).into_owned();
    let mut metadata_manifests = Vec::new();
    let inspection = if magic == "PACK" || [".zip", ".pk3"].contains(&extension(path).to_lowercase().as_str()) {
        match parse_directory(&mut file, path, bytes, &magic) {
            Ok(observed) => match planet_manifest(&mut file, path, bytes, &observed) {
                Ok(manifests) => {
                    metadata_manifests = manifests;
                    let public: Vec<&Member> = observed
                        .members
                        .iter()
                        .filter(|member| !sensitive_name(&member.path))
                        .collect();
                    let mut extension_counts: Vec<(String, usize)> = Vec::new();
                    let mut extension_indexes: HashMap<String, usize> = HashMap::new();
                    for member in &public {
                        let name = extension(&member.path).to_lowercase();
                        let name = if name.is_empty() { "[none]".to_owned() } else { name };
                        if let Some(index) = extension_indexes.get(&name) {
                            extension_counts[*index].1 += 1;
                        } else {
                            extension_indexes.insert(name.clone(), extension_counts.len());
                            extension_counts.push((name, 1));
                        }
                    }
                    let targeted: Vec<Member> = public
                        .iter()
                        .filter(|member| wanted_member(&member.path))
                        .map(|member| (*member).clone())
                        .collect();
                    let reported: Vec<Member> = public.into_iter().cloned().collect();
                    Inspection::Inspected {
                        container: observed.kind,
                        directory_sha256: observed.sha256,
                        member_count: observed.members.len(),
                        sensitive_names_omitted: observed.members.len() - reported.len(),
                        extension_counts,
                        targeted_members: targeted,
                        reported_members: reported.clone(),
                        omitted_members: reported.len().saturating_sub(LIMITS.reported_members),
                    }
                }
                Err(error) => Inspection::Error {
                    reason: error.to_string(),
                },
            },
            Err(error) => Inspection::Error {
                reason: error.to_string(),
            },
        }
    } else {
        Inspection::Unsupported {
            reason: "Hash and header only; this discovery reader parses PAK and ZIP/ZIP64 directories".to_owned(),
        }
    };
    Ok(FileEvidence {
        path: path.to_owned(),
        bytes,
        sha256: sha256.clone(),
        header_hex: to_hex(&header),
        qfiles_hash_matches: known.get(&sha256).cloned().unwrap_or_default(),
        inspection,
        metadata_manifests,
    })
}
fn evidence_limits() -> Vec<String> {
    [
        "Downloads discovery starts with Quake-related top-level filenames. Unrelated directories and personal file contents are not searched.",
        "Archive hashing reads raw container bytes only. The bounded PlanetQuake CSV member manifest is read in memory; assets and executables are not extracted or executed, and account/config/save/key names are omitted from reported directories.",
        "Nested archives are manifest candidates only. Filename matches do not establish a campaign or model format identity.",
        "Steam scope is Quake title content archives only. Steam executable inventory is owned by W02.",
        "PAK/ZIP/ZIP64 directories are range-bounded. Unsupported archive formats retain only whole-file hash and header evidence.",
        "Known qfiles duplicate identities are compared against the recorded product manifest hashes, not rehashed from qfiles in this command.",
        "Directory details are capped per archive; complete directory byte hashes and member counts permit reproducible verification. No runtime compatibility is inferred.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn unresolved_fixtures() -> Json {
    Json::array(vec![
        Json::object(vec![
            ("id".to_owned(), Json::string("q1-rerelease-quake64")),
            ("status".to_owned(), Json::string("unresolved")),
            (
                "reason".to_owned(),
                Json::string("No confirmed Quake 64 content identity in this bounded discovery. Nested archive payloads remain uninspected."),
            ),
        ]),
        Json::object(vec![
            ("id".to_owned(), Json::string("md4-v1")),
            ("status".to_owned(), Json::string("no-confirmed-header-fixture")),
            (
                "reason".to_owned(),
                Json::string("This additional discovery did not establish an MD4 header fixture."),
            ),
        ]),
        Json::object(vec![
            ("id".to_owned(), Json::string("q2-classic-baseq2/gamex86.dll")),
            ("status".to_owned(), Json::string("no-confirmed-retail-base-fixture")),
            (
                "reason".to_owned(),
                Json::string("Gladiator DLLs and q2test archive members are provenance candidates; they do not establish the retail baseQ2 guest identity."),
            ),
        ]),
    ])
}

fn main_file_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["zip", "pk3", "pak", "7z", "gz", "bundle"]
        .into_iter()
        .any(|extension| lower.ends_with(&format!(".{extension}")))
}
/// Build the discovery report over explicit roots (donor `main` core).
pub fn discover_fixtures(downloads: &Path, steam_quake: &Path, project_root: &Path) -> Result<Json, ToolsError> {
    let mut files: Vec<String> = Vec::new();
    let mut candidates: Vec<Json> = Vec::new();
    let mut walker = Walker {
        visited_entries: 0,
        errors: Vec::new(),
    };
    let downloads_text = downloads.to_string_lossy().into_owned();
    let mut top: Vec<(String, std::fs::FileType)> = Vec::new();
    let listing =
        std::fs::read_dir(downloads).map_err(|error| ToolsError::io(format!("listing {downloads_text}"), error))?;
    for entry in listing {
        let entry = entry.map_err(|error| ToolsError::io(format!("listing {downloads_text}"), error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
        top.push((entry.file_name().to_string_lossy().into_owned(), file_type));
    }
    top.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, file_type) in top {
        if !relevant_name(&name) {
            continue;
        }
        let path = format!("{downloads_text}/{name}");
        candidates.push(Json::object(vec![
            ("path".to_owned(), Json::string(&path)),
            (
                "kind".to_owned(),
                Json::string(if file_type.is_dir() {
                    "directory"
                } else if file_type.is_file() {
                    "file"
                } else {
                    "unfollowed-special-file"
                }),
            ),
        ]));
        if file_type.is_dir() {
            walk(&path, 1, &mut files, &mut walker)?;
        } else if file_type.is_file() && main_file_name(&name) {
            files.push(path);
        }
    }
    let steam_meta = std::fs::symlink_metadata(steam_quake)
        .map_err(|error| ToolsError::io(format!("stating {}", steam_quake.display()), error))?;
    if steam_meta.file_type().is_dir() {
        walk(&steam_quake.to_string_lossy(), 0, &mut files, &mut walker)?;
    }
    let known = known_hashes(project_root)?;
    let mut seen = HashSet::new();
    let mut unique: Vec<String> = Vec::new();
    for path in files {
        if seen.insert(path.clone()) {
            unique.push(path);
        }
    }
    unique.sort();
    let mut evidence = Vec::new();
    for path in &unique {
        let base = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(path.clone());
        eprintln!("Inspecting {base}");
        match inspect(path, &known) {
            Ok(file) => evidence.push(file),
            Err(error) => walker.errors.push((path.clone(), error.to_string())),
        }
    }
    let mut target_matches = Vec::new();
    for file in &evidence {
        for member in file.inspection.targeted_members() {
            target_matches.push(Json::object(vec![
                ("archive".to_owned(), Json::string(&file.path)),
                ("archiveSha256".to_owned(), Json::string(&file.sha256)),
                ("member".to_owned(), member.to_json()),
            ]));
        }
    }
    let summary = Json::object(vec![
        ("files".to_owned(), Json::uint(evidence.len() as u64)),
        (
            "bytesHashed".to_owned(),
            Json::uint(evidence.iter().map(|file| file.bytes).sum::<u64>()),
        ),
        (
            "inspectedArchives".to_owned(),
            Json::uint(
                evidence
                    .iter()
                    .filter(|file| matches!(file.inspection, Inspection::Inspected { .. }))
                    .count() as u64,
            ),
        ),
        (
            "duplicateQfilesArchives".to_owned(),
            Json::uint(
                evidence
                    .iter()
                    .filter(|file| !file.qfiles_hash_matches.is_empty())
                    .count() as u64,
            ),
        ),
        ("targetMatches".to_owned(), Json::uint(target_matches.len() as u64)),
    ]);
    Ok(Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        (
            "generator".to_owned(),
            Json::string("bun tools/inventory/additional-fixtures.ts --write"),
        ),
        (
            "roots".to_owned(),
            Json::object(vec![
                ("downloads".to_owned(), Json::string(downloads.to_string_lossy())),
                ("steamQuake".to_owned(), Json::string(steam_quake.to_string_lossy())),
                (
                    "knownCorpusManifest".to_owned(),
                    Json::string("verification/product-manifest.json"),
                ),
            ]),
        ),
        ("limits".to_owned(), limits_json()),
        (
            "evidenceLimits".to_owned(),
            Json::array(evidence_limits().iter().map(Json::string).collect()),
        ),
        ("unresolvedFixtures".to_owned(), unresolved_fixtures()),
        ("candidates".to_owned(), Json::array(candidates)),
        ("visitedEntries".to_owned(), Json::uint(walker.visited_entries as u64)),
        ("summary".to_owned(), summary),
        ("targetMatches".to_owned(), Json::array(target_matches)),
        (
            "evidence".to_owned(),
            Json::array(evidence.iter().map(FileEvidence::to_json).collect()),
        ),
        (
            "errors".to_owned(),
            Json::array(
                walker
                    .errors
                    .iter()
                    .map(|(path, reason)| {
                        Json::object(vec![
                            ("path".to_owned(), Json::string(path)),
                            ("reason".to_owned(), Json::string(reason)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

/// Run discovery (donor `main`): `--write` or `--check` the report.
pub fn run(args: &[String]) -> Result<(), ToolsError> {
    if args.len() != 1 || (args[0] != "--write" && args[0] != "--check") {
        return Err(ToolsError::invalid(
            "Usage: qa-tools inventory::additional_fixtures --write|--check",
        ));
    }
    let project_root = quake_typescript_root();
    let report = discover_fixtures(&downloads_root(), &steam_quake_root(), &project_root)?;
    let output = format!("{}\n", report.render_pretty());
    if output.len() > LIMITS.output_bytes {
        return Err(ToolsError::invalid("Report exceeds output bound"));
    }
    let destination = project_root.join("verification/additional-fixtures.json");
    if args[0] == "--write" {
        fsutil::write_text(&destination, &output)?;
    } else if fsutil::read_text(&destination)? != output {
        return Err(ToolsError::invalid("Additional fixture report differs; rerun --write"));
    }
    println!("{}", report.get("summary").unwrap_or(&Json::Null).render());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_relevant_names() {
        for name in [
            "quake-stuff",
            "Quake2",
            "q2repro",
            "q1source",
            "q3a",
            "pak0.pak",
            "pakexplr.zip",
            "gladachar",
            "lmctf09",
            "quake",
            "quake64",
        ] {
            assert!(relevant_name(name), "{name}");
        }
        for name in ["q4data", "pak.zip", "random", "q", "gl"] {
            assert!(!relevant_name(name), "{name}");
        }
    }

    #[test]
    fn classifies_sensitive_names() {
        for name in [
            "/root/config/x",
            "a/save",
            "accounts",
            "Credential",
            "cdkey.txt",
            "user/q3key",
            "x.vdf",
            "app.acf",
            "game.cfg",
            "setup.ini",
        ] {
            assert!(sensitive_name(name), "{name}");
        }
        for name in ["/root/maps/q3dm1.bsp", "pak0.pak", "configuration.txt", "savedgames2/x"] {
            assert!(!sensitive_name(name), "{name}");
        }
    }

    #[test]
    fn classifies_wanted_members() {
        for name in [
            "quake64/maps/x.bsp",
            "QUAKE-64/x",
            "q1n64/player.mdl",
            "quake_n64/x",
            "models/x.md4",
            "baseq2/gamex86.dll",
            "a/b/gamex86.dll",
        ] {
            assert!(wanted_member(name), "{name}");
        }
        for name in ["quake/maps/x.bsp", "q2dm1.bsp", "gamex86.dll.bak"] {
            assert!(!wanted_member(name), "{name}");
        }
    }

    #[test]
    fn extracts_extensions() {
        assert_eq!(extension("a/b.PK3"), ".PK3");
        assert_eq!(extension("noext"), "");
        assert_eq!(extension(".bashrc"), "");
        assert_eq!(extension("a.b/c"), "");
    }

    fn write_pak(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut data_size = 0_usize;
        for (_, data) in entries {
            data_size += data.len();
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PACK");
        bytes.extend_from_slice(&(12 + data_size as u32).to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u32 * 64).to_le_bytes());
        let mut offset = 12_usize;
        let mut records = Vec::new();
        for (name, data) in entries {
            bytes.extend_from_slice(data);
            records.push((*name, offset, data.len()));
            offset += data.len();
        }
        for (name, entry_offset, size) in &records {
            let mut slot = [0u8; 56];
            slot[..name.len()].copy_from_slice(name.as_bytes());
            bytes.extend_from_slice(&slot);
            bytes.extend_from_slice(&(*entry_offset as u32).to_le_bytes());
            bytes.extend_from_slice(&(*size as u32).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn inspects_pak_archives() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-fixture-").expect("temp dir");
        let pak = directory.join("quake-test.pak");
        fsutil::write_bytes(
            &pak,
            &write_pak(&[
                ("maps/q2dm1.bsp", b"map"),
                ("config.cfg", b"cfg"),
                ("quake64/x.md4", b"md4"),
            ]),
        )
        .expect("write");
        let evidence = inspect(&pak.to_string_lossy(), &HashMap::new()).expect("inspect");
        let Inspection::Inspected {
            member_count,
            sensitive_names_omitted,
            targeted_members,
            reported_members,
            omitted_members,
            ..
        } = &evidence.inspection
        else {
            panic!("expected inspection");
        };
        assert_eq!(*member_count, 3);
        assert_eq!(*sensitive_names_omitted, 1);
        assert_eq!(targeted_members.len(), 1);
        assert_eq!(reported_members.len(), 2);
        assert_eq!(*omitted_members, 0);
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn rejects_invalid_archives_and_usage() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-fixture-bad-").expect("temp dir");
        let bad = directory.join("quake-bad.pak");
        fsutil::write_bytes(&bad, b"PACK").expect("write");
        let evidence = inspect(&bad.to_string_lossy(), &HashMap::new()).expect("inspect");
        assert!(matches!(evidence.inspection, Inspection::Error { .. }));
        let dll = directory.join("gamex86.dll");
        fsutil::write_bytes(&dll, b"MZ-binary").expect("write");
        let evidence = inspect(&dll.to_string_lossy(), &HashMap::new()).expect("inspect");
        assert!(matches!(evidence.inspection, Inspection::Unsupported { .. }));
        assert!(run(&[]).is_err());
        assert!(run(&["--bogus".to_owned()]).is_err());
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn discovers_over_explicit_roots() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-fixture-roots-").expect("temp dir");
        let downloads = directory.join("dl");
        let quake_dir = downloads.join("quake-stuff");
        std::fs::create_dir_all(&quake_dir).expect("mkdir");
        fsutil::write_bytes(&quake_dir.join("data.pak"), &write_pak(&[("maps/x.bsp", b"x")])).expect("write");
        fsutil::write_text(&downloads.join("unrelated.zip"), "zip").expect("write");
        let project = directory.join("project/verification");
        std::fs::create_dir_all(&project).expect("mkdir");
        fsutil::write_text(&project.join("product-manifest.json"), r#"{"archives": []}"#).expect("write");
        let nosteam = directory.join("nosteam");
        std::fs::create_dir_all(&nosteam).expect("mkdir");
        let report = discover_fixtures(&downloads, &nosteam, &directory.join("project")).expect("discover");
        assert_eq!(
            report
                .get("summary")
                .and_then(|summary| summary.get("files"))
                .and_then(Json::as_f64),
            Some(1.0)
        );
        assert_eq!(
            report.get("candidates").and_then(Json::as_array).map(<[Json]>::len),
            Some(1)
        );
        fsutil::remove_forced(&directory);
    }
}
