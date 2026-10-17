//! Original Q3 FS_PureServerSetReferencedPaks checksum/name columns.
use qa_formats::{FormatError, text::Tokens};

pub struct ReferencedPackage<'a> {
    pub checksum: u32,
    pub name: Option<&'a [u8]>,
}

pub struct ReferencedPackages<'a> {
    checksums: Tokens<'a>,
    names: Tokens<'a>,
    remaining: usize,
}
impl<'a> ReferencedPackages<'a> {
    pub fn new(system_info: &'a [u8]) -> Self {
        let value = |key| {
            qa_core::text::info_value(system_info, key, false)
                .map_or(&b""[..], |range| &system_info[range])
        };
        Self {
            checksums: Tokens::words(value(b"sv_referencedPaks")),
            names: Tokens::words(value(b"sv_referencedPakNames")),
            remaining: 4096,
        }
    }
    pub fn next(&mut self) -> Result<Option<ReferencedPackage<'a>>, FormatError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let Some(checksum) = self.checksums.next()? else {
            return Ok(None);
        };
        self.remaining -= 1;
        Ok(Some(ReferencedPackage {
            checksum: qa_core::text::integer_prefix(checksum).0 as u32,
            name: self.names.next()?,
        }))
    }
}
