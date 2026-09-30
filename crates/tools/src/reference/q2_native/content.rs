//! Retail content staging (donor `tools/reference/q2-native/content.ts`).
//!
//! Symlinks PAK archives into an isolated mount and selects map entries by
//! parsing the PAK directory: a `PACK` magic, a little-endian directory
//! offset/size, and 64-byte entries (56-byte NUL-padded name plus offset
//! and size). Every selected entry is SHA-256 hashed; archives hash low to
//! high priority.

use std::path::Path;

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::reference::environment::identify_file;
use crate::reference::schema::FileIdentity;
use crate::verify::hash::hash_bytes;

/// A selected archive entry.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// Archive the entry was selected from.
    pub archive: String,
    /// Entry name.
    pub name: String,
    /// Entry byte offset.
    pub offset: u64,
    /// Entry byte size.
    pub size: u64,
    /// SHA-256 hex of the entry bytes.
    pub sha256: String,
}

impl ArchiveEntry {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("archive".to_owned(), Json::string(&self.archive)),
            ("name".to_owned(), Json::string(&self.name)),
            ("offset".to_owned(), Json::uint(self.offset)),
            ("size".to_owned(), Json::uint(self.size)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
        ])
    }
}

/// Staged content: archive identities, selected entries, and the mount.
#[derive(Debug, Clone)]
pub struct StagedContent {
    /// Archive identities, low to high priority.
    pub archives_low_to_high_priority: Vec<FileIdentity>,
    /// Selected entries.
    pub entries: Vec<ArchiveEntry>,
    /// Mount directory.
    pub mount: String,
    /// Selection policy statement.
    pub selection: String,
}

impl StagedContent {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            (
                "archivesLowToHighPriority".to_owned(),
                Json::array(self.archives_low_to_high_priority.iter().map(FileIdentity::to_json).collect()),
            ),
            ("entries".to_owned(), Json::array(self.entries.iter().map(ArchiveEntry::to_json).collect())),
            ("mount".to_owned(), Json::string(&self.mount)),
            ("selection".to_owned(), Json::string(&self.selection)),
        ])
    }
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

/// Stage `archives` into `directory`, selecting `maps` entries from each.
pub fn stage_content(archives: &[String], directory: &str, maps: &[String]) -> Result<StagedContent, ToolsError> {
    let mut identities = Vec::with_capacity(archives.len());
    let mut selected: Vec<ArchiveEntry> = Vec::new();
    let archive_directory = Path::new(directory).join("baseq2");
    std::fs::create_dir_all(&archive_directory)
        .map_err(|error| ToolsError::io(format!("creating {}", archive_directory.display()), error))?;
    for archive in archives {
        identities.push(identify_file(archive)?);
        let file_name = Path::new(archive)
            .file_name()
            .ok_or_else(|| ToolsError::invalid(format!("Invalid PAK path: {archive}")))?;
        fsutil::create_symlink(Path::new(archive), &archive_directory.join(file_name))?;
        let bytes = fsutil::read_bytes(Path::new(archive))?;
        if bytes.len() < 12 || &bytes[0..4] != b"PACK" {
            return Err(ToolsError::invalid(format!("Invalid PAK: {archive}")));
        }
        let offset = read_u32_le(&bytes, 4) as usize;
        let size = read_u32_le(&bytes, 8) as usize;
        if size % 64 != 0 || offset.saturating_add(size) > bytes.len() {
            return Err(ToolsError::invalid(format!("Invalid PAK directory: {archive}")));
        }
        let entries = &bytes[offset..offset + size];
        for chunk in entries.chunks_exact(64) {
            let end = chunk[..56].iter().position(|byte| *byte == 0).unwrap_or(56);
            let name = String::from_utf8_lossy(&chunk[..end]).into_owned();
            if !maps.iter().any(|map| map == &name) {
                continue;
            }
            let entry_offset = read_u32_le(chunk, 56) as usize;
            let entry_size = read_u32_le(chunk, 60) as usize;
            if entry_offset.saturating_add(entry_size) > bytes.len() {
                return Err(ToolsError::invalid(format!("Invalid PAK entry: {name}")));
            }
            let data = &bytes[entry_offset..entry_offset + entry_size];
            let entry =
                ArchiveEntry { archive: archive.clone(), name: name.clone(), offset: entry_offset as u64, size: entry_size as u64, sha256: hash_bytes(data) };
            if let Some(prior) = selected.iter_mut().find(|entry| entry.name == name) {
                *prior = entry;
            } else {
                selected.push(entry);
            }
        }
    }
    for name in maps {
        if !selected.iter().any(|entry| &entry.name == name) {
            return Err(ToolsError::invalid(format!("Missing selected content: {name}")));
        }
    }
    Ok(StagedContent {
        archives_low_to_high_priority: identities,
        entries: selected,
        mount: directory.to_owned(),
        selection: "Only explicitly listed archive symlinks; no corpus loose files or user configs.".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut records: Vec<(&str, usize, usize)> = Vec::new();
        for (name, data) in entries {
            bytes.extend_from_slice(data);
            records.push((name, offset, data.len()));
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
    fn stages_and_selects_entries() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-stage-").expect("temp dir");
        let pak = directory.join("pak0.pak");
        let bytes = write_pak(&[("maps/q2dm1.bsp", b"map-bytes"), ("maps/other.bsp", b"other")]);
        fsutil::write_bytes(&pak, &bytes).expect("write pak");
        let pak_text = pak.to_string_lossy().into_owned();
        let mount = directory.join("mount").to_string_lossy().into_owned();
        let staged =
            stage_content(&[pak_text.clone()], &mount, &["maps/q2dm1.bsp".to_owned()]).expect("stage");
        assert_eq!(staged.entries.len(), 1);
        assert_eq!(staged.entries[0].name, "maps/q2dm1.bsp");
        assert_eq!(staged.entries[0].archive, pak_text);
        assert_eq!(staged.entries[0].sha256, hash_bytes(b"map-bytes"));
        assert_eq!(staged.mount, mount);
        assert_eq!(staged.archives_low_to_high_priority.len(), 1);
        let link = Path::new(&mount).join("baseq2/pak0.pak");
        assert!(link.is_symlink());
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn rejects_invalid_archives_and_missing_maps() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-stage-bad-").expect("temp dir");
        let bad = directory.join("bad.pak");
        fsutil::write_bytes(&bad, b"NOPE").expect("write");
        let bad_text = bad.to_string_lossy().into_owned();
        let mount = directory.join("mount").to_string_lossy().into_owned();
        let error = stage_content(&[bad_text], &mount, &["maps/q2dm1.bsp".to_owned()]).expect_err("invalid pak");
        assert!(error.to_string().starts_with("Invalid PAK"), "{error}");
        let pak = directory.join("pak0.pak");
        let bytes = write_pak(&[("maps/other.bsp", b"other")]);
        fsutil::write_bytes(&pak, &bytes).expect("write pak");
        let error = stage_content(&[pak.to_string_lossy().into_owned()], &mount, &["maps/q2dm1.bsp".to_owned()])
            .expect_err("missing map");
        assert!(error.to_string().starts_with("Missing selected content"), "{error}");
        fsutil::remove_forced(&directory);
    }
}
