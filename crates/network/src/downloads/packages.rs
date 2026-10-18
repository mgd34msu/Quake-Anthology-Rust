//! Original Q3 FS_PureServerSetReferencedPaks checksum/name columns.
use qa_formats::{FormatError, text::Tokens};

pub const Q3_PACKAGE_PATHS: usize = 4096;

/// Native signed checksum words, bounded by MAX_SEARCH_PATHS. The values are
/// existing wire metadata, never an engine content identity.
pub struct PackageChecksums<'a> {
    words: Tokens<'a>,
    remaining: usize,
}
impl<'a> PackageChecksums<'a> {
    fn new(text: &'a [u8]) -> Self {
        Self {
            words: Tokens::words(text),
            remaining: Q3_PACKAGE_PATHS,
        }
    }
    pub fn loaded(system_info: &'a [u8]) -> Self {
        Self::new(info_value(system_info, b"sv_paks"))
    }
    pub fn next(&mut self) -> Result<Option<u32>, FormatError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let Some(word) = self.words.next()? else {
            return Ok(None);
        };
        self.remaining -= 1;
        Ok(Some(qa_core::text::integer_prefix(word).0 as u32))
    }
}
fn info_value<'a>(info: &'a [u8], key: &[u8]) -> &'a [u8] {
    qa_core::text::info_value(info, key, true).map_or(&[], |range| &info[range])
}

pub struct ReferencedPackage<'a> {
    pub checksum: u32,
    pub name: Option<&'a [u8]>,
}

pub struct ReferencedPackages<'a> {
    checksums: PackageChecksums<'a>,
    names: Tokens<'a>,
}
impl<'a> ReferencedPackages<'a> {
    pub fn new(system_info: &'a [u8]) -> Self {
        Self {
            checksums: PackageChecksums::new(info_value(system_info, b"sv_referencedPaks")),
            names: Tokens::words(info_value(system_info, b"sv_referencedPakNames")),
        }
    }
    pub fn next(&mut self) -> Result<Option<ReferencedPackage<'a>>, FormatError> {
        let Some(checksum) = self.checksums.next()? else {
            return Ok(None);
        };
        Ok(Some(ReferencedPackage {
            checksum,
            name: self.names.next()?,
        }))
    }
}
