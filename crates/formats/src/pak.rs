use crate::{FormatError, span, word};

struct Entry<'a> {
    name: &'a [u8],
    data: &'a [u8],
    ordinal: usize,
}

pub struct Pak<'a> {
    entries: Vec<Entry<'a>>,
}

impl<'a> Pak<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if bytes.get(..4) != Some(b"PACK") {
            return Err(FormatError::Unsupported);
        }
        let offset = word(bytes, 4)?;
        let size = word(bytes, 8)?;
        if offset < 12 || size % 64 != 0 {
            return Err(FormatError::InvalidRecordSize);
        }
        let directory = span(bytes, offset, size)?;
        let mut entries = Vec::with_capacity(directory.len() / 64);
        for (ordinal, record) in directory.as_chunks::<64>().0.iter().enumerate() {
            let name = &record[..record[..56].iter().position(|v| *v == 0).unwrap_or(56)];
            let data = span(bytes, word(record, 56)?, word(record, 60)?)?;
            entries.push(Entry {
                name,
                data,
                ordinal,
            });
        }
        entries.sort_unstable_by(|a, b| a.name.cmp(b.name).then(a.ordinal.cmp(&b.ordinal)));
        Ok(Self { entries })
    }

    pub fn find(&self, name: &[u8]) -> Option<&'a [u8]> {
        let index = self.entries.partition_point(|entry| entry.name < name);
        self.entries
            .get(index)
            .filter(|entry| entry.name == name)
            .map(|entry| entry.data)
    }
}
