//! Borrowed native comma-separated surface-to-shader mappings.
use crate::{FormatError, text::Tokens};

pub struct SkinMapping<'a> {
    pub surface: &'a [u8],
    pub shader: &'a [u8],
}
pub struct SkinMappings<'a> {
    tokens: Tokens<'a>,
    finished: bool,
}
impl<'a> SkinMappings<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            tokens: Tokens::comma(qa_core::text::until_nul(bytes)),
            finished: false,
        }
    }
    fn mapping(&mut self) -> Result<Option<SkinMapping<'a>>, FormatError> {
        loop {
            let Some(surface) = self.tokens.next()? else {
                return Ok(None);
            };
            if surface.is_empty() {
                return Ok(None);
            }
            if self.tokens.bytes.get(self.tokens.at) == Some(&b',') {
                self.tokens.at += 1;
            }
            // RE_RegisterSkin checks the original token before lowercasing.
            if surface.windows(4).any(|part| part == b"tag_") {
                continue;
            }
            let shader = self.tokens.next()?.unwrap_or(b"");
            return Ok(Some(SkinMapping {
                surface: &surface[..surface.len().min(63)],
                shader,
            }));
        }
    }
}
impl<'a> Iterator for SkinMappings<'a> {
    type Item = Result<SkinMapping<'a>, FormatError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.mapping() {
            Ok(Some(mapping)) => Some(Ok(mapping)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}
