//! Product content inventory (donor `tools/inventory/products.ts`).
//!
//! Discovers corpus archives and loose gameplay programs, parses bounded
//! PAK and ZIP directories, classifies entry headers by magic, and maps
//! observed evidence onto the expected product domain. ZIP64, multi-disk,
//! encrypted, and non-store/deflate entries stay explicit inspection gaps.
//!
//! The bounded archive readers are intentionally module-local like the
//! donors: limits and error text differ per tool. The `generator` tag keeps
//! the donor string so `--check` validates donor-generated manifests.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use flate2::{Decompress, FlushDecompress, Status};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::reference::environment::{corpus_root, quake_typescript_root};
use crate::sha256::{to_hex, Hasher};
use crate::verify::hash::hash_bytes;

/// An expected product (donor `ExpectedProduct`).
#[derive(Debug, Clone)]
pub struct ExpectedProduct {
    /// Stable product id.
    pub id: String,
    /// Product family.
    pub family: &'static str,
    /// Product edition.
    pub edition: &'static str,
    /// Campaign directory.
    pub campaign: &'static str,
    /// Display title.
    pub title: String,
    /// Corpus content directory.
    pub content_directory: String,
    /// Base product id.
    pub base_product: Option<String>,
    /// Required archive path.
    pub required_archive: Option<String>,
    /// Required guest programs.
    pub required_programs: Vec<&'static str>,
    /// Witness map entry.
    pub map_witness: Option<String>,
    /// Wire protocols.
    pub protocols: Vec<&'static str>,
}

impl ExpectedProduct {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("family".to_owned(), Json::string(self.family)),
            ("edition".to_owned(), Json::string(self.edition)),
            ("campaign".to_owned(), Json::string(self.campaign)),
            ("title".to_owned(), Json::string(&self.title)),
            ("scope".to_owned(), Json::string("required")),
            ("contentDirectory".to_owned(), Json::string(&self.content_directory)),
            ("baseProduct".to_owned(), self.base_product.as_ref().map_or(Json::Null, Json::string)),
            ("requiredArchive".to_owned(), self.required_archive.as_ref().map_or(Json::Null, Json::string)),
            ("requiredPrograms".to_owned(), Json::array(self.required_programs.iter().copied().map(Json::string).collect())),
            ("mapWitness".to_owned(), self.map_witness.as_ref().map_or(Json::Null, Json::string)),
            ("protocols".to_owned(), Json::array(self.protocols.iter().copied().map(Json::string).collect())),
        ])
    }
}

/// The expected product domain (donor `expectedProducts`).
#[must_use]
pub fn expected_products() -> Vec<ExpectedProduct> {
    let mut products = Vec::new();
    let q1_titles =
        [("id1", "Quake"), ("hipnotic", "Scourge of Armagon"), ("rogue", "Dissolution of Eternity"), ("dopa", "Dimension of the Past"), ("mg1", "Dimension of the Machine"), ("mg3", "Dawn of the Machine"), ("ctf", "Capture the Flag")];
    for edition in ["classic", "rerelease"] {
        let campaigns: &[&str] =
            if edition == "classic" { &["id1", "hipnotic", "rogue"] } else { &["id1", "hipnotic", "rogue", "dopa", "mg1", "mg3", "ctf"] };
        for campaign in campaigns {
            let directory =
                if edition == "rerelease" { format!("q1/rerelease/{campaign}") } else { format!("q1/{campaign}") };
            let title = q1_titles.iter().find(|(name, _)| name == campaign).map_or(*campaign, |(_, title)| *title);
            products.push(ExpectedProduct {
                id: format!("q1-{edition}-{campaign}"),
                family: "q1",
                edition,
                campaign,
                title: title.to_owned(),
                content_directory: directory.clone(),
                base_product: if *campaign == "id1" { None } else { Some(format!("q1-{edition}-id1")) },
                required_archive: Some(format!("{directory}/pak0.pak")),
                required_programs: vec!["progs.dat"],
                map_witness: None,
                protocols: vec!["netquake"],
            });
        }
    }
    products.push(ExpectedProduct {
        id: "q1-quakeworld".to_owned(),
        family: "q1",
        edition: "quakeworld",
        campaign: "id1",
        title: "QuakeWorld".to_owned(),
        content_directory: "q1/qw".to_owned(),
        base_product: Some("q1-classic-id1".to_owned()),
        required_archive: None,
        required_programs: vec!["qwprogs.dat"],
        map_witness: None,
        protocols: vec!["quakeworld"],
    });
    products.push(ExpectedProduct {
        id: "q1-rerelease-quake64".to_owned(),
        family: "q1",
        edition: "rerelease",
        campaign: "quake64",
        title: "Quake 64 (unresolved content identity)".to_owned(),
        content_directory: "q1/rerelease/quake64".to_owned(),
        base_product: Some("q1-rerelease-id1".to_owned()),
        required_archive: None,
        required_programs: Vec::new(),
        map_witness: None,
        protocols: vec!["netquake"],
    });
    let q2_campaigns =
        [("baseq2", "Quake II", "base1"), ("xatrix", "The Reckoning", "xswamp"), ("rogue", "Ground Zero", "rmine1"), ("ctf", "Capture the Flag", "q2ctf1"), ("lmctf", "Loki's Minions CTF", "lmctf09"), ("mg2", "Call of the Machine", "mguhub"), ("n64", "Quake II 64", "q64/rtest")];
    for edition in ["classic", "rerelease"] {
        for (campaign, title, map) in q2_campaigns {
            if edition == "classic" && (campaign == "mg2" || campaign == "n64") {
                continue;
            }
            if edition == "rerelease" && campaign == "lmctf" {
                continue;
            }
            let directory =
                if edition == "rerelease" { "q2/rerelease/baseq2".to_owned() } else { format!("q2/{campaign}") };
            products.push(ExpectedProduct {
                id: format!("q2-{edition}-{campaign}"),
                family: "q2",
                edition,
                campaign,
                title: title.to_owned(),
                content_directory: directory.clone(),
                base_product: if campaign == "baseq2" {
                    None
                } else {
                    Some(format!("q2-{edition}-baseq2"))
                },
                required_archive: Some(format!("{directory}/pak0.pak")),
                required_programs: vec![if edition == "rerelease" { "game_x64.dll" } else { "gamex86.dll" }],
                map_witness: Some(format!("maps/{map}.bsp")),
                protocols: if edition == "rerelease" {
                    vec!["q2-rerelease"]
                } else {
                    vec!["q2-34", "q2repro-1038", "private-4038"]
                },
            });
        }
    }
    for campaign in ["baseq3", "missionpack"] {
        let directory = format!("q3a/{campaign}");
        products.push(ExpectedProduct {
            id: format!("q3-{campaign}"),
            family: "q3",
            edition: "classic",
            campaign,
            title: if campaign == "baseq3" { "Quake III Arena".to_owned() } else { "Team Arena".to_owned() },
            content_directory: directory.clone(),
            base_product: if campaign == "baseq3" { None } else { Some("q3-baseq3".to_owned()) },
            required_archive: Some(format!("{directory}/pak0.pk3")),
            required_programs: vec!["vm/qagame.qvm", "vm/cgame.qvm", "vm/ui.qvm"],
            map_witness: None,
            protocols: vec!["q3-68"],
        });
    }
    products
}

/// Inventory bounds (donor `limits`).
struct Limits {
    directory_bytes: usize,
    archive_entries: usize,
    files: usize,
    depth: usize,
    header_compressed_bytes: usize,
    header_inflated_bytes: usize,
    program_bytes: usize,
    output_bytes: usize,
}

const LIMITS: Limits = Limits {
    directory_bytes: 32 * 1024 * 1024,
    archive_entries: 100_000,
    files: 20_000,
    depth: 16,
    header_compressed_bytes: 4096,
    header_inflated_bytes: 8 * 1024 * 1024,
    program_bytes: 64 * 1024 * 1024,
    output_bytes: 64 * 1024 * 1024,
};

fn limits_json() -> Json {
    Json::object(vec![
        ("directoryBytes".to_owned(), Json::uint(LIMITS.directory_bytes as u64)),
        ("archiveEntries".to_owned(), Json::uint(LIMITS.archive_entries as u64)),
        ("files".to_owned(), Json::uint(LIMITS.files as u64)),
        ("depth".to_owned(), Json::uint(LIMITS.depth as u64)),
        ("headerCompressedBytes".to_owned(), Json::uint(LIMITS.header_compressed_bytes as u64)),
        ("headerInflatedBytes".to_owned(), Json::uint(LIMITS.header_inflated_bytes as u64)),
        ("programBytes".to_owned(), Json::uint(LIMITS.program_bytes as u64)),
        ("outputBytes".to_owned(), Json::uint(LIMITS.output_bytes as u64)),
    ])
}
/// An archive entry.
#[derive(Debug, Clone)]
struct Entry {
    ordinal: usize,
    path: String,
    offset: u64,
    bytes: u64,
    compressed_bytes: u64,
    compression: u16,
    flags: u16,
    crc32: Option<String>,
}

/// A header observation.
#[derive(Debug, Clone)]
enum Observation {
    Header { format: String, header_hex: String },
    Extension { format: String },
    Uninspected { reason: String },
}

impl Observation {
    fn format(&self) -> Option<&str> {
        match self {
            Self::Header { format, .. } | Self::Extension { format } => Some(format),
            Self::Uninspected { .. } => None,
        }
    }

    fn to_json(&self) -> Json {
        match self {
            Self::Header { format, header_hex } => Json::object(vec![
                ("kind".to_owned(), Json::string("header")),
                ("format".to_owned(), Json::string(format)),
                ("headerHex".to_owned(), Json::string(header_hex)),
            ]),
            Self::Extension { format } => Json::object(vec![
                ("kind".to_owned(), Json::string("extension")),
                ("format".to_owned(), Json::string(format)),
            ]),
            Self::Uninspected { reason } => Json::object(vec![
                ("kind".to_owned(), Json::string("uninspected")),
                ("reason".to_owned(), Json::string(reason)),
            ]),
        }
    }
}

/// An observed entry occurrence.
#[derive(Debug, Clone)]
struct Occurrence {
    entry: Entry,
    observation: Observation,
    program_sha256: Option<String>,
}

impl Occurrence {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("ordinal".to_owned(), Json::uint(self.entry.ordinal as u64)),
            ("path".to_owned(), Json::string(&self.entry.path)),
            ("offset".to_owned(), Json::uint(self.entry.offset)),
            ("bytes".to_owned(), Json::uint(self.entry.bytes)),
            ("compressedBytes".to_owned(), Json::uint(self.entry.compressed_bytes)),
            ("compression".to_owned(), Json::uint(u64::from(self.entry.compression))),
            ("flags".to_owned(), Json::uint(u64::from(self.entry.flags))),
            ("crc32".to_owned(), self.entry.crc32.as_ref().map_or(Json::Null, Json::string)),
            ("observation".to_owned(), self.observation.to_json()),
            ("programSha256".to_owned(), self.program_sha256.as_ref().map_or(Json::Null, Json::string)),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Pak,
    Zip,
}

impl Container {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pak => "pak",
            Self::Zip => "zip",
        }
    }
}

/// An inspected archive.
#[derive(Debug, Clone)]
struct Archive {
    path: String,
    scope: &'static str,
    bytes: u64,
    sha256: String,
    container: Container,
    entries: Vec<Occurrence>,
    duplicate_paths: Vec<(String, Vec<usize>)>,
}

impl Archive {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("scope".to_owned(), Json::string(self.scope)),
            ("bytes".to_owned(), Json::uint(self.bytes)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("container".to_owned(), Json::string(self.container.as_str())),
            ("entries".to_owned(), Json::array(self.entries.iter().map(Occurrence::to_json).collect())),
            (
                "duplicatePaths".to_owned(),
                Json::array(
                    self.duplicate_paths
                        .iter()
                        .map(|(path, ordinals)| {
                            Json::object(vec![
                                ("path".to_owned(), Json::string(path)),
                                ("ordinals".to_owned(), Json::array(ordinals.iter().map(|ordinal| Json::uint(*ordinal as u64)).collect())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// A loose gameplay program.
#[derive(Debug, Clone)]
struct Program {
    path: String,
    bytes: u64,
    sha256: String,
    observation: Observation,
}

impl Program {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("bytes".to_owned(), Json::uint(self.bytes)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("observation".to_owned(), self.observation.to_json()),
        ])
    }
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn read_u32_be(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

/// Read an exact range, failing on short reads (donor `readRange`).
fn read_range(file: &mut File, path: &str, offset: u64, length: usize, size: u64) -> Result<Vec<u8>, ToolsError> {
    if offset.saturating_add(length as u64) > size {
        return Err(ToolsError::invalid(format!("Invalid file range {offset}+{length}/{size}")));
    }
    file.seek(SeekFrom::Start(offset)).map_err(|error| ToolsError::io(format!("seeking {path}"), error))?;
    let mut bytes = vec![0u8; length];
    let mut done = 0;
    while done < length {
        let count =
            file.read(&mut bytes[done..]).map_err(|error| ToolsError::io(format!("reading {path}"), error))?;
        if count == 0 {
            return Err(ToolsError::invalid("File shortened during inspection"));
        }
        done += count;
    }
    Ok(bytes)
}

/// Stream-hash a whole file in 1 MiB chunks (donor `sha256`).
fn hash_file(file: &mut File, path: &str, size: u64) -> Result<String, ToolsError> {
    let mut hasher = Hasher::new();
    let mut offset = 0;
    while offset < size {
        let length = (size - offset).min(1024 * 1024) as usize;
        hasher.update(&read_range(file, path, offset, length, size)?);
        offset += length as u64;
    }
    Ok(to_hex(&hasher.finish()))
}

/// Parse a PAK directory (donor `pakDirectory`).
fn pak_directory(file: &mut File, path: &str, size: u64) -> Result<Vec<Entry>, ToolsError> {
    let header = read_range(file, path, 0, 12.min(size as usize), size)?;
    if header.len() < 12 {
        return Err(ToolsError::invalid("PAK directory exceeds bounds or has partial records"));
    }
    let offset = read_u32_le(&header, 4) as u64;
    let length = read_u32_le(&header, 8) as usize;
    if length % 64 != 0 || length > LIMITS.directory_bytes || length / 64 > LIMITS.archive_entries {
        return Err(ToolsError::invalid("PAK directory exceeds bounds or has partial records"));
    }
    let directory = read_range(file, path, offset, length, size)?;
    let mut entries = Vec::new();
    for (ordinal, chunk) in directory.chunks_exact(64).enumerate() {
        let end = chunk[..56].iter().position(|byte| *byte == 0).unwrap_or(56);
        let entry = Entry {
            ordinal,
            path: String::from_utf8_lossy(&chunk[..end]).into_owned(),
            offset: read_u32_le(chunk, 56) as u64,
            bytes: read_u32_le(chunk, 60) as u64,
            compressed_bytes: read_u32_le(chunk, 60) as u64,
            compression: 0,
            flags: 0,
            crc32: None,
        };
        if entry.offset.saturating_add(entry.bytes) > size {
            return Err(ToolsError::invalid(format!("PAK entry outside archive: {}", entry.path)));
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Parse a ZIP central directory without ZIP64 (donor `zipDirectory`).
fn zip_directory(file: &mut File, path: &str, size: u64) -> Result<Vec<Entry>, ToolsError> {
    let tail_offset = size.saturating_sub(65_557);
    let tail = read_range(file, path, tail_offset, (size - tail_offset) as usize, size)?;
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
    let end = end.ok_or_else(|| ToolsError::invalid("ZIP end directory not found"))?;
    let count = read_u16_le(&tail, end + 10) as usize;
    let length = read_u32_le(&tail, end + 12) as u64;
    let offset = read_u32_le(&tail, end + 16) as u64;
    if read_u16_le(&tail, end + 4) != 0
        || read_u16_le(&tail, end + 6) != 0
        || read_u16_le(&tail, end + 8) as usize != count
        || count == 0xffff
        || length == 0xffff_ffff
        || offset == 0xffff_ffff
    {
        return Err(ToolsError::invalid("Multi-disk/ZIP64 archives require a separate bounded reader"));
    }
    if count > LIMITS.archive_entries || length > LIMITS.directory_bytes as u64 || offset.saturating_add(length) > tail_offset + end as u64 {
        return Err(ToolsError::invalid("ZIP directory exceeds bounds"));
    }
    let length = length as usize;
    let directory = read_range(file, path, offset, length, size)?;
    let mut entries = Vec::new();
    let mut position = 0;
    for ordinal in 0..count {
        if position + 46 > length || read_u32_le(&directory, position) != 0x0201_4b50 {
            return Err(ToolsError::invalid("Invalid ZIP central directory record"));
        }
        let name_bytes = read_u16_le(&directory, position + 28) as usize;
        let extra_bytes = read_u16_le(&directory, position + 30) as usize;
        let comment_bytes = read_u16_le(&directory, position + 32) as usize;
        let next = position + 46 + name_bytes + extra_bytes + comment_bytes;
        if next > length {
            return Err(ToolsError::invalid("ZIP directory record exceeds directory bounds"));
        }
        entries.push(Entry {
            ordinal,
            path: String::from_utf8_lossy(&directory[position + 46..position + 46 + name_bytes]).into_owned(),
            offset: read_u32_le(&directory, position + 42) as u64,
            bytes: read_u32_le(&directory, position + 24) as u64,
            compressed_bytes: read_u32_le(&directory, position + 20) as u64,
            compression: read_u16_le(&directory, position + 10),
            flags: read_u16_le(&directory, position + 8),
            crc32: Some(format!("{:08x}", read_u32_le(&directory, position + 16))),
        });
        position = next;
    }
    if position != length {
        return Err(ToolsError::invalid("Unconsumed ZIP directory bytes"));
    }
    Ok(entries)
}

/// Raw-inflate `input` with an output cap, matching `inflateRawSync` bounds.
fn inflate_raw_capped(input: &[u8], full: bool, cap: usize) -> Result<Vec<u8>, ToolsError> {
    let mut decoder = Decompress::new(false);
    let mut output = vec![0u8; 64.min(cap)];
    let mut produced = 0_usize;
    let mut consumed = 0_usize;
    let flush = if full { FlushDecompress::Finish } else { FlushDecompress::Sync };
    loop {
        if produced == output.len() {
            if output.len() >= cap {
                return Err(ToolsError::invalid("Inflated entry exceeds bounded output"));
            }
            output.resize((output.len() * 2 + 64).min(cap), 0);
        }
        let before_in = decoder.total_in() as usize;
        let before_out = decoder.total_out() as usize;
        let status = decoder
            .decompress(&input[consumed..], &mut output[produced..], flush)
            .map_err(|error| ToolsError::invalid(format!("Inflate failed: {error}")))?;
        consumed += decoder.total_in() as usize - before_in;
        produced += decoder.total_out() as usize - before_out;
        match status {
            Status::StreamEnd => break,
            Status::Ok => {
                if consumed >= input.len() {
                    break;
                }
            }
            Status::BufError => break,
        }
    }
    output.truncate(produced);
    Ok(output)
}

/// Read entry payload bytes, inflating deflate entries (donor `entryBytes`).
fn entry_bytes(
    file: &mut File,
    path: &str,
    size: u64,
    container: Container,
    entry: &Entry,
    full: bool,
) -> Result<Vec<u8>, ToolsError> {
    if entry.flags & 1 != 0 {
        return Err(ToolsError::invalid("Encrypted archive entry"));
    }
    let mut offset = entry.offset;
    if container == Container::Zip {
        let local = read_range(file, path, offset, 30, size)?;
        if local.len() < 30 || read_u32_le(&local, 0) != 0x0403_4b50 || read_u16_le(&local, 8) != entry.compression {
            return Err(ToolsError::invalid("ZIP local header disagrees with directory"));
        }
        offset += 30 + read_u16_le(&local, 26) as u64 + read_u16_le(&local, 28) as u64;
    }
    if offset.saturating_add(entry.compressed_bytes) > size {
        return Err(ToolsError::invalid("Entry outside archive"));
    }
    if full && (entry.bytes > LIMITS.program_bytes as u64 || entry.compressed_bytes > LIMITS.program_bytes as u64) {
        return Err(ToolsError::invalid("Program exceeds bounded hash/decode limit"));
    }
    if entry.compression == 0 {
        let length = if full { entry.bytes } else { 64.min(entry.bytes) };
        return read_range(file, path, offset, length as usize, size);
    }
    if entry.compression != 8 {
        return Err(ToolsError::invalid(format!("Unsupported ZIP compression {}", entry.compression)));
    }
    let take = if full { entry.compressed_bytes } else { (LIMITS.header_compressed_bytes as u64).min(entry.compressed_bytes) };
    let compressed = read_range(file, path, offset, take as usize, size)?;
    let cap = if full { LIMITS.program_bytes } else { LIMITS.header_inflated_bytes };
    let decoded = inflate_raw_capped(&compressed, full, cap)?;
    if full && decoded.len() as u64 != entry.bytes {
        return Err(ToolsError::invalid("Decoded program size disagrees with directory"));
    }
    Ok(if full { decoded } else { decoded[..64.min(decoded.len())].to_vec() })
}
/// Node-style extension: from the last dot in the final segment, or empty.
fn extension(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    if base == ".." {
        return "";
    }
    match base.rfind('.') {
        Some(dot) if dot > 0 => &base[dot..],
        _ => "",
    }
}

/// Node-style dirname.
fn dirname(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[..index],
        None => ".",
    }
}

/// Whether an entry is a gameplay program (donor `isProgram`).
fn is_program(path: &str) -> bool {
    let lower = path.to_lowercase();
    (lower.ends_with("/qwprogs.dat")
        || lower == "qwprogs.dat"
        || lower.ends_with("/progs.dat")
        || lower == "progs.dat")
        || lower.ends_with(".qvm")
        || lower.ends_with(".dll")
        || lower.ends_with(".so")
}

fn inspected_extension(lower: &str) -> bool {
    matches!(
        lower,
        ".bsp" | ".mdl" | ".spr" | ".md2" | ".sp2" | ".md3" | ".md4" | ".md5mesh" | ".md5anim" | ".cin" | ".ogv" | ".roq" | ".wav" | ".ogg" | ".mp3" | ".flac" | ".qvm" | ".dll" | ".so"
    )
}

/// Node `ascii` decoding: bytes masked to 7 bits.
fn ascii(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| (byte & 0x7f) as char).collect()
}

/// Classify a header by magic (donor `observe`).
fn observe(path: &str, header: &[u8]) -> Observation {
    let magic = ascii(&header[..4.min(header.len())]);
    let version: i64 = if header.len() >= 8 { i64::from(read_u32_le(header, 4)) } else { -1 };
    let mut format: Option<String> = None;
    if extension(path).to_lowercase() == ".bsp" && header.len() >= 4 {
        format = Some(
            if read_u32_le(header, 0) == 29 {
                "bsp29".to_owned()
            } else if magic == "BSP2" {
                "bsp2".to_owned()
            } else if magic == "2PSB" {
                "bsp2-rmq".to_owned()
            } else if magic == "IBSP" {
                format!("ibsp{version}")
            } else if magic == "QBSP" {
                format!("qbsp{version}")
            } else {
                "unknown-bsp".to_owned()
            },
        );
    } else if magic == "IDPO" {
        format = Some(format!("mdl{version}"));
    } else if magic == "IDSP" {
        format = Some(format!("spr{version}"));
    } else if magic == "IDP2" {
        format = Some(format!("md2-v{version}"));
    } else if magic == "IDS2" {
        format = Some(format!("sp2-v{version}"));
    } else if magic == "IDP3" {
        format = Some(format!("md3-v{version}"));
    } else if magic == "IDP4" {
        format = Some(format!("md4-v{version}"));
    } else if ascii(&header[..10.min(header.len())]) == "MD5Version" {
        format = Some("md5".to_owned());
    } else if magic == "OggS" {
        format = Some("ogg-container".to_owned());
    } else if magic == "RIFF" && header.len() >= 12 && ascii(&header[8..12]) == "WAVE" {
        format = Some("wav".to_owned());
    } else if magic == "fLaC" {
        format = Some("flac".to_owned());
    } else if header.len() >= 4 && [0x1272_1444, 0x1272_1445].contains(&read_u32_le(header, 0)) {
        format = Some("qvm".to_owned());
    } else if header.len() >= 4 && read_u32_be(header, 0) == 0x7f45_4c46 {
        format = Some(format!(
            "elf{}",
            if header.get(4) == Some(&1) {
                "32"
            } else if header.get(4) == Some(&2) {
                "64"
            } else {
                "-unknown"
            }
        ));
    } else if ascii(&header[..2.min(header.len())]) == "MZ" {
        let pe_offset = if header.len() >= 64 { read_u32_le(header, 60) as usize } else { header.len() };
        if pe_offset + 26 <= header.len() && read_u32_le(header, pe_offset) == 0x0000_4550 {
            let machine = read_u16_le(header, pe_offset + 4);
            let optional = read_u16_le(header, pe_offset + 24);
            format = Some(format!(
                "pe{}-{}",
                if optional == 0x10b {
                    "32"
                } else if optional == 0x20b {
                    "32+"
                } else {
                    "-unknown"
                },
                if machine == 0x14c {
                    "i386".to_owned()
                } else if machine == 0x8664 {
                    "x86-64".to_owned()
                } else {
                    format!("machine-{machine}")
                }
            ));
        } else {
            format = Some("pe-dos-header".to_owned());
        }
    } else if header.len() >= 4 && (path.to_lowercase().ends_with("qwprogs.dat") || path.to_lowercase().ends_with("progs.dat")) {
        format = Some(format!("quakec-v{}", read_u32_le(header, 0)));
    } else if header.len() >= 2 && read_u16_le(header, 0) == 0x1084 {
        format = Some("roq".to_owned());
    }
    match format {
        Some(format) => Observation::Header { format, header_hex: to_hex(&header[..16.min(header.len())]) },
        None => {
            let without_dot = extension(path).to_lowercase().strip_prefix('.').unwrap_or("").to_owned();
            Observation::Extension { format: if without_dot.is_empty() { "no-extension".to_owned() } else { without_dot } }
        }
    }
}

#[cfg(unix)]
fn change_key(meta: &std::fs::Metadata) -> (u64, std::time::SystemTime, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.ctime(), meta.ctime_nsec())
}

#[cfg(not(unix))]
fn change_key(meta: &std::fs::Metadata) -> (u64, std::time::SystemTime, i64, i64) {
    (meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH), 0, 0)
}

#[cfg(unix)]
fn identity_key(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.ino(), meta.dev())
}

#[cfg(not(unix))]
fn identity_key(_meta: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// Inspect one archive (donor `inspectArchive`).
fn inspect_archive(path: &str, absolute: &str) -> Result<Archive, ToolsError> {
    let mut file = File::open(absolute).map_err(|error| ToolsError::io(format!("opening {absolute}"), error))?;
    let before_meta = file.metadata().map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?;
    let size = before_meta.len();
    let magic = ascii(&read_range(&mut file, absolute, 0, (4 as u64).min(size) as usize, size)?);
    let container = if magic == "PACK" { Container::Pak } else { Container::Zip };
    let directory = if container == Container::Pak {
        pak_directory(&mut file, absolute, size)?
    } else {
        zip_directory(&mut file, absolute, size)?
    };
    let mut entries = Vec::with_capacity(directory.len());
    for entry in &directory {
        let program = is_program(&entry.path);
        let fallback = extension(&entry.path).to_lowercase().strip_prefix('.').unwrap_or("").to_owned();
        let mut observation = Observation::Extension {
            format: if fallback.is_empty() { "no-extension".to_owned() } else { fallback },
        };
        let mut program_sha256 = None;
        if program || inspected_extension(&extension(&entry.path).to_lowercase()) {
            match entry_bytes(&mut file, absolute, size, container, entry, program) {
                Ok(bytes) => {
                    observation = observe(&entry.path, &bytes);
                    if program {
                        program_sha256 = Some(hash_bytes(&bytes));
                    }
                }
                Err(error) => {
                    observation = Observation::Uninspected { reason: error.to_string() };
                }
            }
        }
        entries.push(Occurrence { entry: entry.clone(), observation, program_sha256 });
    }
    let mut names: Vec<(String, Vec<usize>)> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();
    for entry in &entries {
        if let Some(index) = indexes.get(&entry.entry.path) {
            names[*index].1.push(entry.entry.ordinal);
        } else {
            indexes.insert(entry.entry.path.clone(), names.len());
            names.push((entry.entry.path.clone(), vec![entry.entry.ordinal]));
        }
    }
    let sha256 = hash_file(&mut file, absolute, size)?;
    let after_meta = file.metadata().map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?;
    if change_key(&before_meta) != change_key(&after_meta) {
        return Err(ToolsError::invalid(format!("Archive changed during inspection: {path}")));
    }
    Ok(Archive {
        path: path.to_owned(),
        scope: if path.starts_with("quakelive/") || path == "q1-paks.zip" { "provenance-only" } else { "product-content" },
        bytes: size,
        sha256,
        container,
        entries,
        duplicate_paths: names.into_iter().filter(|(_, ordinals)| ordinals.len() > 1).collect(),
    })
}

fn selected_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".pak") || lower.ends_with(".pk3") || lower.ends_with(".kpf") || lower.ends_with(".zip") || lower == "qwprogs.dat" || lower == "progs.dat" || lower.starts_with("game") && (lower.ends_with(".dll") || lower.ends_with(".so"))
}

/// Discover selected corpus files (donor `discover`).
fn discover(root: &str) -> Result<Vec<String>, ToolsError> {
    let root = root.trim_end_matches('/').to_owned();
    let root = if root.is_empty() { "/".to_owned() } else { root };
    let mut selected = Vec::new();
    let mut count = 0_usize;
    fn walk(directory: &str, root: &str, depth: usize, count: &mut usize, selected: &mut Vec<String>) -> Result<(), ToolsError> {
        if depth > LIMITS.depth {
            return Err(ToolsError::invalid("Inventory traversal depth exceeded"));
        }
        let mut entries: Vec<(String, std::fs::FileType)> = Vec::new();
        let listing =
            std::fs::read_dir(directory).map_err(|error| ToolsError::io(format!("listing {directory}"), error))?;
        for entry in listing {
            let entry = entry.map_err(|error| ToolsError::io(format!("listing {directory}"), error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
            entries.push((entry.file_name().to_string_lossy().into_owned(), file_type));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, file_type) in entries {
            *count += 1;
            if *count > LIMITS.files {
                return Err(ToolsError::invalid("Inventory file count exceeded"));
            }
            let path = format!("{directory}/{name}");
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                walk(&path, root, depth + 1, count, selected)?;
            } else if file_type.is_file() && selected_name(&name) {
                selected.push(path.strip_prefix(&format!("{root}/")).unwrap_or(&path).to_owned());
            }
        }
        Ok(())
    }
    walk(&root, &root, 0, &mut count, &mut selected)?;
    selected.sort();
    Ok(selected)
}
fn evidence_limits() -> Vec<String> {
    [
        "Static directories and bounded headers do not prove runtime compatibility or campaign completion.",
        "Product status describes installed content evidence only. requiredPrograms and programFixtureStatus describe separate guest/reference fixtures; missing native binaries do not block official TypeScript gameplay.",
        "Entry ordinals preserve duplicate names. Counts include overridden content and are not counts of unique playable maps.",
        "Inventory path order is not mount precedence. Product/base identities and all source occurrences remain separate.",
        "ZIP64, multi-disk, encrypted entries, and compression other than store/deflate are explicit inspection gaps.",
        "Nested archives are listed but not recursively decoded. q1-paks.zip is backup provenance, not an installed mount.",
        "Loose assets other than selected gameplay programs are not read. Configurations, saves, credentials, CD keys, OS libraries, and executables are not inspected.",
        "Header classification identifies observed magic only; extensions alone do not verify format variants. SHA256 covers complete archives and selected gameplay programs.",
        "Only three example occurrences per format are repeated in the summary; complete archive directories remain below.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn required_archives(product: &ExpectedProduct) -> Vec<String> {
    let mut required = Vec::new();
    if let Some(archive) = &product.required_archive {
        required.push(archive.clone());
    }
    if product.id == "q1-classic-id1" {
        required.push("q1/id1/pak1.pak".to_owned());
    }
    required
}

fn product_json(
    product: &ExpectedProduct,
    expected: &[ExpectedProduct],
    archives: &[Archive],
    programs: &[Program],
) -> Json {
    let base = expected.iter().find(|candidate| candidate.id == product.base_product.as_deref().unwrap_or(""));
    let mut required_content: Vec<String> = required_archives(product);
    if let Some(base) = base {
        required_content.extend(required_archives(base));
    }
    let own: Vec<&Archive> =
        archives.iter().filter(|archive| dirname(&archive.path) == product.content_directory).collect();
    let base_evidence: Vec<Json> = base
        .map(|base| {
            archives
                .iter()
                .filter(|archive| dirname(&archive.path) == base.content_directory)
                .map(|archive| Json::string(&archive.path))
                .collect()
        })
        .unwrap_or_default();
    let program_evidence: Vec<Json> = product
        .required_programs
        .iter()
        .map(|name| {
            let mut occurrences = Vec::new();
            for program in programs.iter().filter(|program| program.path == format!("{}/{}", product.content_directory, name)) {
                occurrences.push(Json::object(vec![
                    ("source".to_owned(), Json::string(&program.path)),
                    ("ordinal".to_owned(), Json::Null),
                    ("sha256".to_owned(), Json::string(&program.sha256)),
                ]));
            }
            for archive in &own {
                for entry in archive.entries.iter().filter(|entry| {
                    entry.entry.path.to_lowercase() == name.to_lowercase() && entry.program_sha256.is_some()
                }) {
                    occurrences.push(Json::object(vec![
                        ("source".to_owned(), Json::string(&archive.path)),
                        ("ordinal".to_owned(), Json::uint(entry.entry.ordinal as u64)),
                        ("sha256".to_owned(), Json::string(entry.program_sha256.as_ref().expect("checked sha"))),
                    ]));
                }
            }
            Json::object(vec![
                ("name".to_owned(), Json::string(*name)),
                ("occurrences".to_owned(), Json::array(occurrences)),
            ])
        })
        .collect();
    let map_evidence: Vec<Json> = own
        .iter()
        .flat_map(|archive| {
            archive
                .entries
                .iter()
                .filter(|entry| match &product.map_witness {
                    None => entry.entry.path.to_lowercase().ends_with(".bsp"),
                    Some(witness) => entry.entry.path.to_lowercase() == witness.to_lowercase(),
                })
                .map(|entry| {
                    Json::object(vec![
                        ("source".to_owned(), Json::string(&archive.path)),
                        ("ordinal".to_owned(), Json::uint(entry.entry.ordinal as u64)),
                        ("path".to_owned(), Json::string(&entry.entry.path)),
                    ])
                })
                .collect::<Vec<Json>>()
        })
        .collect();
    let mut missing = Vec::new();
    for path in &required_content {
        if !archives.iter().any(|archive| archive.path.to_lowercase() == path.to_lowercase()) {
            missing.push(format!("archive:{path}"));
        }
    }
    if product.map_witness.is_some() && map_evidence.is_empty() {
        missing.push(format!("map:{}", product.map_witness.as_ref().expect("witness")));
    }
    let missing_programs: Vec<&str> = product
        .required_programs
        .iter()
        .filter(|name| {
            !programs.iter().any(|program| program.path == format!("{}/{}", product.content_directory, name))
                && !own.iter().any(|archive| {
                    archive.entries.iter().any(|entry| {
                        entry.entry.path.to_lowercase() == name.to_lowercase() && entry.program_sha256.is_some()
                    })
                })
        })
        .copied()
        .collect();
    let status =
        if product.campaign == "quake64" { "unresolved" } else if missing.is_empty() { "observed" } else { "missing-evidence" };
    let fixture_status = if product.campaign == "quake64" {
        "unresolved"
    } else if missing_programs.is_empty() {
        "observed"
    } else {
        "missing-reference-fixture"
    };
    let mut pairs = Vec::new();
    if let Json::Object(base) = product.to_json() {
        pairs.extend(base);
    }
    pairs.extend(vec![
        ("status".to_owned(), Json::string(status)),
        ("missing".to_owned(), Json::array(missing.iter().map(Json::string).collect())),
        ("requiredContentArchives".to_owned(), Json::array(required_content.iter().map(Json::string).collect())),
        ("archiveEvidence".to_owned(), Json::array(own.iter().map(|archive| Json::string(&archive.path)).collect())),
        ("baseArchiveEvidence".to_owned(), Json::array(base_evidence)),
        ("programFixtureStatus".to_owned(), Json::string(fixture_status)),
        ("missingProgramFixtures".to_owned(), Json::array(missing_programs.iter().copied().map(Json::string).collect())),
        ("programEvidence".to_owned(), Json::array(program_evidence)),
        ("mapEvidence".to_owned(), Json::array(map_evidence)),
        (
            "mount".to_owned(),
            Json::object(vec![
                ("contentDirectory".to_owned(), Json::string(&product.content_directory)),
                ("baseProduct".to_owned(), product.base_product.as_ref().map_or(Json::Null, Json::string)),
                ("archiveOrder".to_owned(), Json::string("inventory-path-order-only; runtime precedence remains profile-specific")),
                (
                    "campaignProgram".to_owned(),
                    Json::array(
                        if product.family == "q1" {
                            product.required_programs.iter().copied().map(Json::string).collect()
                        } else {
                            Vec::new()
                        },
                    ),
                ),
                (
                    "crossEditionMapOverride".to_owned(),
                    if product.family == "q2" {
                        Json::string("When rerelease gameplay uses classic maps, classic maps/ takes precedence while rerelease assets retain priority; do not merge filenames globally.")
                    } else {
                        Json::Null
                    },
                ),
            ]),
        ),
    ]);
    Json::object(pairs)
}

/// Inventory the corpus under `root` (donor `inventoryProducts`).
pub fn inventory_products(root: &str) -> Result<Json, ToolsError> {
    let mut archives: Vec<Archive> = Vec::new();
    let mut programs: Vec<Program> = Vec::new();
    let mut inspection_errors: Vec<Json> = Vec::new();
    let mut opaque_fixtures: Vec<Json> = Vec::new();
    let paths = discover(root)?;
    let mut snapshots = Vec::with_capacity(paths.len());
    for path in &paths {
        let absolute = format!("{}/{}", root.trim_end_matches('/'), path);
        snapshots.push((path.clone(), std::fs::metadata(&absolute).map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?));
    }
    for path in &paths {
        let absolute = format!("{}/{}", root.trim_end_matches('/'), path);
        let outcome: Result<(), ToolsError> = (|| {
            if path == "quakelive/web.pak" {
                let mut file = File::open(&absolute).map_err(|error| ToolsError::io(format!("opening {absolute}"), error))?;
                let size = file.metadata().map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?.len();
                opaque_fixtures.push(Json::object(vec![
                    ("path".to_owned(), Json::string(path)),
                    ("bytes".to_owned(), Json::uint(size)),
                    ("sha256".to_owned(), Json::string(hash_file(&mut file, &absolute, size)?)),
                    ("headerHex".to_owned(), Json::string(to_hex(&read_range(&mut file, &absolute, 0, (16 as u64).min(size) as usize, size)?))),
                    (
                        "reason".to_owned(),
                        Json::string("Quake Live web resource bundle; observed header is neither PACK nor ZIP. Opaque provenance only; no game archive directory claimed."),
                    ),
                ]));
            } else if [".pak", ".pk3", ".kpf", ".zip"].into_iter().any(|extension| path.to_lowercase().ends_with(extension)) {
                archives.push(inspect_archive(path, &absolute)?);
            } else {
                let mut file = File::open(&absolute).map_err(|error| ToolsError::io(format!("opening {absolute}"), error))?;
                let before = file.metadata().map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?;
                let row = Program {
                    path: path.clone(),
                    bytes: before.len(),
                    sha256: hash_file(&mut file, &absolute, before.len())?,
                    observation: observe(path, &read_range(&mut file, &absolute, 0, (65_536 as u64).min(before.len()) as usize, before.len())?),
                };
                let after = file.metadata().map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?;
                if change_key(&before) != change_key(&after) {
                    return Err(ToolsError::invalid("Program changed during inspection"));
                }
                programs.push(row);
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            inspection_errors.push(Json::object(vec![
                ("path".to_owned(), Json::string(path)),
                ("reason".to_owned(), Json::string(error.to_string())),
            ]));
        }
    }
    if paths != discover(root)? {
        return Err(ToolsError::invalid("Selected corpus paths changed during inventory"));
    }
    for (path, before) in &snapshots {
        let absolute = format!("{}/{}", root.trim_end_matches('/'), path);
        let after = std::fs::metadata(&absolute).map_err(|error| ToolsError::io(format!("stating {absolute}"), error))?;
        if identity_key(before) != identity_key(&after) || change_key(before) != change_key(&after) {
            return Err(ToolsError::invalid(format!("Corpus input changed during inventory: {path}")));
        }
    }
    let expected = expected_products();
    let products: Vec<Json> =
        expected.iter().map(|product| product_json(product, &expected, &archives, &programs)).collect();
    let mut formats: Vec<(String, usize, usize, Vec<Json>)> = Vec::new();
    let mut format_indexes: HashMap<String, usize> = HashMap::new();
    for archive in archives.iter().filter(|archive| archive.scope == "product-content") {
        for entry in &archive.entries {
            let Some(format) = entry.observation.format() else {
                continue;
            };
            let index = *format_indexes.get(format).unwrap_or(&formats.len());
            if index == formats.len() {
                format_indexes.insert(format.to_owned(), index);
                formats.push((format.to_owned(), 0, 0, Vec::new()));
            }
            if matches!(entry.observation, Observation::Header { .. }) {
                formats[index].1 += 1;
            } else {
                formats[index].2 += 1;
            }
            if formats[index].3.len() < 3 {
                formats[index].3.push(Json::object(vec![
                    ("source".to_owned(), Json::string(&archive.path)),
                    ("ordinal".to_owned(), Json::uint(entry.entry.ordinal as u64)),
                    ("path".to_owned(), Json::string(&entry.entry.path)),
                ]));
            }
        }
    }
    formats.sort_by(|left, right| left.0.cmp(&right.0));
    let mut fixtures: Vec<Json> = archives
        .iter()
        .filter(|archive| archive.scope == "provenance-only")
        .map(|archive| {
            Json::object(vec![
                ("source".to_owned(), Json::string(&archive.path)),
                ("sha256".to_owned(), Json::string(&archive.sha256)),
                ("scope".to_owned(), Json::string("provenance-only")),
                (
                    "reason".to_owned(),
                    Json::string(if archive.path.starts_with("quakelive/") {
                        "Quake Live data and native ABI fixtures; outside required product scope"
                    } else {
                        "Backup archive; nested PAKs are not installed mounts"
                    }),
                ),
            ])
        })
        .collect();
    for fixture in &opaque_fixtures {
        let empty = Json::Null;
        fixtures.push(Json::object(vec![
            ("source".to_owned(), fixture.get("path").unwrap_or(&empty).clone()),
            ("sha256".to_owned(), fixture.get("sha256").unwrap_or(&empty).clone()),
            ("scope".to_owned(), Json::string("provenance-only")),
            ("reason".to_owned(), fixture.get("reason").unwrap_or(&empty).clone()),
        ]));
    }
    let gaps: Vec<Json> = ["bsp2", "md4-v1"]
        .into_iter()
        .filter(|format| formats.iter().find(|(name, _, _, _)| name == format).map_or(0, |(_, header, _, _)| *header) == 0)
        .map(|format| {
            Json::object(vec![
                ("format".to_owned(), Json::string(format)),
                ("status".to_owned(), Json::string("no-confirmed-header-fixture")),
                ("owner".to_owned(), Json::string("W07")),
                ("required".to_owned(), Json::boolean(true)),
            ])
        })
        .collect();
    let summary = Json::object(vec![
        ("expectedProducts".to_owned(), Json::uint(products.len() as u64)),
        (
            "observedProducts".to_owned(),
            Json::uint(products.iter().filter(|product| product.get("status").and_then(Json::as_str) == Some("observed")).count() as u64),
        ),
        (
            "unresolvedProducts".to_owned(),
            Json::uint(products.iter().filter(|product| product.get("status").and_then(Json::as_str) == Some("unresolved")).count() as u64),
        ),
        ("archives".to_owned(), Json::uint(archives.len() as u64)),
        ("archiveEntries".to_owned(), Json::uint(archives.iter().map(|archive| archive.entries.len() as u64).sum::<u64>())),
        ("standalonePrograms".to_owned(), Json::uint(programs.len() as u64)),
        (
            "uninspectedEntries".to_owned(),
            Json::uint(
                archives
                    .iter()
                    .map(|archive| archive.entries.iter().filter(|entry| matches!(entry.observation, Observation::Uninspected { .. })).count() as u64)
                    .sum::<u64>(),
            ),
        ),
        (
            "duplicateNamesWithinArchives".to_owned(),
            Json::uint(archives.iter().map(|archive| archive.duplicate_paths.len() as u64).sum::<u64>()),
        ),
    ]);
    Ok(Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("generator".to_owned(), Json::string("bun run tools/inventory/products.ts --write")),
        ("corpusRoot".to_owned(), Json::string("../qfiles")),
        (
            "expectationSources".to_owned(),
            Json::array(
                [
                    "docs/work-packages.json#W01",
                    "docs/source-assessment.md#data-coverage-and-provenance",
                    "../quake-1-re-ts/src/client/menu_content.ts",
                    "../quake-2-re-ts/src/client/menu_content.ts",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
        ("limits".to_owned(), limits_json()),
        ("evidenceLimits".to_owned(), Json::array(evidence_limits().iter().map(Json::string).collect())),
        ("products".to_owned(), Json::array(products)),
        ("fixtures".to_owned(), Json::array(fixtures)),
        ("requiredFixtureGaps".to_owned(), Json::array(gaps)),
        (
            "formatCounts".to_owned(),
            Json::array(
                formats
                    .iter()
                    .map(|(format, header, extension, examples)| {
                        Json::object(vec![
                            ("format".to_owned(), Json::string(format)),
                            ("headerOccurrences".to_owned(), Json::uint(*header as u64)),
                            ("extensionOccurrences".to_owned(), Json::uint(*extension as u64)),
                            ("examples".to_owned(), Json::array(examples.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("summary".to_owned(), summary),
        ("archives".to_owned(), Json::array(archives.iter().map(Archive::to_json).collect())),
        ("programs".to_owned(), Json::array(programs.iter().map(Program::to_json).collect())),
        ("opaqueFixtures".to_owned(), Json::array(opaque_fixtures)),
        ("inspectionErrors".to_owned(), Json::array(inspection_errors)),
    ]))
}

fn write_u32_le(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn find_product<'a>(products: &'a [Json], id: &str) -> Option<&'a Json> {
    products.iter().find(|row| row.get("id").and_then(Json::as_str) == Some(id))
}

/// Self-test the inventory reader on a synthetic corpus (donor `verifyReader`).
pub fn verify_reader() -> Result<(), ToolsError> {
    let root = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-product-inventory-")?;
    let root = root.to_string_lossy().into_owned();
    let outcome = (|| -> Result<(), ToolsError> {
        let directory = format!("{root}/q2/baseq2");
        std::fs::create_dir_all(&directory).map_err(|error| ToolsError::io(format!("creating {directory}"), error))?;
        let mut pak = vec![0u8; 148];
        pak[0..4].copy_from_slice(b"PACK");
        write_u32_le(&mut pak, 4, 20);
        write_u32_le(&mut pak, 8, 128);
        pak[12..16].copy_from_slice(b"IBSP");
        write_u32_le(&mut pak, 16, 38);
        for offset in [20_usize, 84] {
            pak[offset..offset + 14].copy_from_slice(b"maps/base1.bsp");
            write_u32_le(&mut pak, offset + 56, 12);
            write_u32_le(&mut pak, offset + 60, 8);
        }
        let path = format!("{directory}/pak0.pak");
        std::fs::write(&path, &pak).map_err(|error| ToolsError::io(format!("writing {path}"), error))?;
        let manifest = inventory_products(&root)?;
        let empty = Json::Null;
        let archives = manifest.get("archives").and_then(Json::as_array).map_or(&[][..], |list| list);
        let archive = archives.first().unwrap_or(&empty);
        let entries = archive.get("entries").and_then(Json::as_array).map_or(0, <[Json]>::len);
        let ordinals = archive
            .get("duplicatePaths")
            .and_then(Json::as_array)
            .and_then(|paths| paths.first())
            .and_then(|row| row.get("ordinals"))
            .and_then(Json::as_array)
            .map(|ordinals| ordinals.iter().map(Json::render).collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        if entries != 2 || ordinals != "0,1" {
            return Err(ToolsError::invalid("Duplicate archive occurrences were lost"));
        }
        let expected_hash = hash_bytes(&pak);
        if archive.get("sha256").and_then(Json::as_str) != Some(expected_hash.as_str()) {
            return Err(ToolsError::invalid("Archive hash mismatch"));
        }
        let products = manifest.get("products").and_then(Json::as_array).map_or(&[][..], |list| list);
        let product = find_product(products, "q2-classic-baseq2").unwrap_or(&empty);
        let maps = product.get("mapEvidence").and_then(Json::as_array).map_or(0, <[Json]>::len);
        if product.get("status").and_then(Json::as_str) != Some("observed")
            || product.get("programFixtureStatus").and_then(Json::as_str) != Some("missing-reference-fixture")
            || maps != 2
        {
            return Err(ToolsError::invalid("Content availability was conflated with guest program fixtures"));
        }
        if products.len() != expected_products().len()
            || find_product(products, "q1-rerelease-quake64").and_then(|row| row.get("status")).and_then(Json::as_str) != Some("unresolved")
        {
            return Err(ToolsError::invalid("Missing content reduced the expected product domain"));
        }
        if find_product(products, "q1-quakeworld").and_then(|row| row.get("status")).and_then(Json::as_str) != Some("missing-evidence") {
            return Err(ToolsError::invalid("An absent base mount was reported available"));
        }
        let headers = manifest
            .get("formatCounts")
            .and_then(Json::as_array)
            .and_then(|counts| counts.iter().find(|row| row.get("format").and_then(Json::as_str) == Some("ibsp38")))
            .and_then(|row| row.get("headerOccurrences"))
            .and_then(Json::as_f64);
        if headers != Some(2.0) {
            return Err(ToolsError::invalid("Map headers were not independently classified"));
        }
        write_u32_le(&mut pak, 8, LIMITS.directory_bytes as u32 + 64);
        std::fs::write(&path, &pak).map_err(|error| ToolsError::io(format!("writing {path}"), error))?;
        let invalid = inventory_products(&root)?;
        let error_count = invalid.get("inspectionErrors").and_then(Json::as_array).map_or(0, <[Json]>::len);
        let archive_count = invalid.get("archives").and_then(Json::as_array).map_or(0, <[Json]>::len);
        if error_count != 1 || archive_count != 0 {
            return Err(ToolsError::invalid("Oversized archive directory was accepted"));
        }
        println!("Product inventory reader checks passed: duplicates, hashes, headers, finite expected domain, content/program separation, bounded directory rejection.");
        Ok(())
    })();
    fsutil::remove_forced(Path::new(&root));
    outcome
}

/// CLI entry point returning a process exit code (donor `import.meta.main`).
pub fn run(args: &[String]) -> Result<i32, ToolsError> {
    if args.iter().any(|argument| argument != "--write" && argument != "--check" && argument != "--self-test") || args.len() > 1 {
        return Err(ToolsError::invalid("Usage: qa-tools inventory::products --write|--check|--self-test"));
    }
    if args.contains(&"--self-test".to_owned()) {
        verify_reader()?;
        return Ok(0);
    }
    let root = corpus_root();
    let metadata = std::fs::metadata(&root).map_err(|error| ToolsError::io(format!("stating {root}"), error))?;
    if !metadata.is_dir() {
        return Err(ToolsError::invalid("Reference corpus is not a directory"));
    }
    let manifest = inventory_products(&root)?;
    let serialized = format!("{}\n", manifest.render_pretty());
    if serialized.len() > LIMITS.output_bytes {
        return Err(ToolsError::invalid("Manifest exceeds output limit"));
    }
    let destination = quake_typescript_root().join("verification/product-manifest.json");
    if args.contains(&"--write".to_owned()) {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
        }
        fsutil::write_text(&destination, &serialized)?;
    } else if args.contains(&"--check".to_owned()) && fsutil::read_text(&destination)? != serialized {
        return Err(ToolsError::invalid("Product manifest differs from current corpus; run --write to regenerate"));
    }
    let empty = Json::Null;
    let mut pairs = match manifest.get("summary") {
        Some(Json::Object(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    let products = manifest.get("products").and_then(Json::as_array).map_or(&[][..], |list| list);
    pairs.push(("manifestBytes".to_owned(), Json::uint(serialized.len() as u64)));
    pairs.push(("inspectionErrors".to_owned(), manifest.get("inspectionErrors").unwrap_or(&empty).clone()));
    pairs.push((
        "missing".to_owned(),
        Json::array(
            products
                .iter()
                .filter(|product| product.get("status").and_then(Json::as_str) != Some("observed"))
                .map(|product| {
                    Json::object(vec![
                        ("id".to_owned(), product.get("id").unwrap_or(&empty).clone()),
                        ("status".to_owned(), product.get("status").unwrap_or(&empty).clone()),
                        ("missing".to_owned(), product.get("missing").unwrap_or(&empty).clone()),
                    ])
                })
                .collect(),
        ),
    ));
    pairs.push((
        "missingProgramFixtures".to_owned(),
        Json::array(
            products
                .iter()
                .filter(|product| product.get("missingProgramFixtures").and_then(Json::as_array).is_some_and(|list| !list.is_empty()))
                .map(|product| {
                    Json::object(vec![
                        ("id".to_owned(), product.get("id").unwrap_or(&empty).clone()),
                        ("missing".to_owned(), product.get("missingProgramFixtures").unwrap_or(&empty).clone()),
                    ])
                })
                .collect(),
        ),
    ));
    pairs.push(("requiredFixtureGaps".to_owned(), manifest.get("requiredFixtureGaps").unwrap_or(&empty).clone()));
    println!("{}", Json::object(pairs).render());
    let errors = manifest.get("inspectionErrors").and_then(Json::as_array).map_or(0, <[Json]>::len);
    let uninspected =
        manifest.get("summary").and_then(|summary| summary.get("uninspectedEntries")).and_then(Json::as_f64).unwrap_or(0.0);
    if errors > 0 || uninspected > 0.0 {
        return Ok(1);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_checks_pass() {
        verify_reader().expect("reader self-test");
    }

    #[test]
    fn expected_domain_has_25_products() {
        assert_eq!(expected_products().len(), 25);
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(run(&["--bogus".to_owned()]).is_err());
        assert!(run(&["--write".to_owned(), "--check".to_owned()]).is_err());
    }

    #[test]
    fn dotdot_has_no_extension() {
        assert_eq!(extension(".."), "");
    }
}
