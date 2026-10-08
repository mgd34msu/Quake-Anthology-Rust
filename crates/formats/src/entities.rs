//! Borrowed map spawn text converted once to numeric keys and typed columns.
use crate::{FormatError, text::Tokens};
use qa_core::{names::NameTable, primitives::NameId};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitySyntax {
    Quake,
    Quake2,
    Quake3,
}
#[derive(Clone, Copy, Debug)]
pub struct SpawnField<'a> {
    pub key: NameId,
    pub value: &'a [u8],
}
pub struct EntityLump<'a> {
    pub names: NameTable,
    pub records: Vec<Range<usize>>,
    pub fields: Vec<SpawnField<'a>>,
}
pub struct SpawnColumns<'a, T> {
    pub values: Vec<T>,
    /// Remaining fields are for guest/module spawn only, never gameplay lookup.
    pub guest_records: Vec<Range<usize>>,
    pub guest_fields: Vec<SpawnField<'a>>,
}
impl<'a> EntityLump<'a> {
    pub fn parse(bytes: &'a [u8], syntax: EntitySyntax) -> Result<Self, FormatError> {
        let mut tokens = match syntax {
            EntitySyntax::Quake => Tokens::entity(bytes, b"{}()':", false, usize::MAX),
            EntitySyntax::Quake2 => Tokens::entity(bytes, b"", false, 1024),
            EntitySyntax::Quake3 => Tokens::entity(bytes, b"", true, 1024),
        };
        let mut pairs = Vec::new();
        let mut records = Vec::new();
        while let Some(open) = tokens.next()? {
            if open.first() != Some(&b'{') {
                return Err(FormatError::InvalidValue);
            }
            let first = pairs.len();
            loop {
                let key = tokens.value()?;
                if key.first() == Some(&b'}') {
                    break;
                }
                let value = tokens.value()?;
                if value.first() == Some(&b'}') {
                    return Err(FormatError::InvalidValue);
                }
                pairs.push((key, value));
            }
            records.push(first..pairs.len());
        }
        let keys = pairs.iter().map(|&(key, _)| key);
        let names = NameTable::load(keys).map_err(|_| FormatError::InvalidRange)?;
        let fields = pairs
            .into_iter()
            .map(|(key, value)| {
                Ok(SpawnField {
                    key: (if syntax == EntitySyntax::Quake {
                        names.find(key)
                    } else {
                        names.find_folded(key)
                    })
                    .ok_or(FormatError::InvalidValue)?,
                    value,
                })
            })
            .collect::<Result<_, FormatError>>()?;
        Ok(Self {
            names,
            records,
            fields,
        })
    }
    /// Each module resolves its field table to these NameIds once. The handler
    /// returns true after writing a typed column; only unhandled guest fields
    /// survive. Native underscore filtering is a module boundary choice.
    pub fn convert<T: Default>(
        &self,
        discard_utility: bool,
        mut field: impl FnMut(SpawnField<'a>, &mut T) -> Result<bool, FormatError>,
    ) -> Result<SpawnColumns<'a, T>, FormatError> {
        let mut values = Vec::with_capacity(self.records.len());
        let mut guest_records = Vec::with_capacity(self.records.len());
        let mut guest_fields = Vec::new();
        for record in &self.records {
            let mut value = T::default();
            let first = guest_fields.len();
            for &entry in &self.fields[record.clone()] {
                if discard_utility
                    && self
                        .names
                        .get(entry.key)
                        .is_some_and(|name| name.starts_with(b"_"))
                {
                    continue;
                }
                if !field(entry, &mut value)? {
                    guest_fields.push(entry);
                }
            }
            values.push(value);
            guest_records.push(first..guest_fields.len());
        }
        Ok(SpawnColumns {
            values,
            guest_records,
            guest_fields,
        })
    }
}
