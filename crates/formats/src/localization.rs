//! Q2 rerelease key/format catalogs, using the common quoted-token reader.
use crate::{FormatError, text::Tokens};
pub type LocText = qa_core::text::FixedText<1023>;

pub struct Entry {
    pub key: Box<[u8]>,
    pub format: Box<[u8]>,
}

/// Platform-specific alternatives are skipped like q2repro Loc_Reload.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, FormatError> {
    let mut tokens = Tokens::entity(bytes, b"", true, usize::MAX).quoted_escapes();
    let mut entries = Vec::new();
    while let Some(key) = tokens.next()? {
        if key.is_empty() || key.len() >= 64 {
            return Err(FormatError::InvalidRange);
        }
        let mut equals = tokens.value()?;
        let platform = equals.starts_with(b"<");
        if platform {
            while !equals.ends_with(b">") {
                equals = tokens.value()?;
            }
            equals = tokens.value()?;
        }
        if equals != b"=" {
            return Err(FormatError::InvalidValue);
        }
        let raw = tokens.value()?;
        if platform {
            continue;
        }
        let mut format = LocText::default();
        let mut at = 0;
        while at < raw.len() {
            let mut byte = raw[at];
            at += 1;
            if byte == b'\\' {
                byte = *raw.get(at).ok_or(FormatError::Truncated)?;
                at += 1;
                byte = match byte {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    other => other,
                };
            }
            format
                .append_bytes(&[byte])
                .map_err(|_| FormatError::InvalidRange)?;
        }
        entries.push(Entry {
            key: key.into(),
            format: format.as_bytes().into(),
        });
    }
    Ok(entries)
}
