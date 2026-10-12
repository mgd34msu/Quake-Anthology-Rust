//! Cold map fields use the VM's checked entity bytes and existing definitions.
use super::{Trap, Vm, definition};
use qa_core::{
    names::NameTable,
    primitives::{CallbackId, NameId, RuleSetId, ThinkTime},
    text::{atof_prefix_bytes, atoi_prefix},
};
use qa_formats::{
    entities::{SpawnField, level_string},
    program::quakec::{Definition, Image},
};

pub(super) struct MapGlobals {
    map_name: Option<usize>,
    pub(super) server_flags: Option<usize>,
}
impl MapGlobals {
    pub(super) fn load(image: &Image) -> Self {
        Self {
            map_name: definition(image, false, b"mapname", 1),
            server_flags: definition(image, false, b"serverflags", 2),
        }
    }
}

#[derive(Clone, Copy)]
enum FieldBinding<'a> {
    Utility,
    Unknown(&'a [u8]),
    Value { definition: Definition, angle: bool },
}
/// Exact map NameIds are resolved against the program once, before callbacks.
/// Borrowing the map keys does not borrow the VM or cache mutable global words.
pub struct MapFields<'a> {
    keys: Box<[FieldBinding<'a>]>,
    classname: Option<usize>,
    spawn_flags: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapAdmission {
    pub has_pairs: bool,
    /// Zero means absent. An explicitly inserted empty string has a nonzero
    /// native offset and may resolve an empty-named program function.
    pub classname: Option<u32>,
    pub spawn_flags: i32,
}

fn optional_definition(
    image: &Image,
    fields: bool,
    name: &[u8],
    kind: u16,
) -> Result<Option<usize>, Trap> {
    let present = if fields {
        image.field(name)
    } else {
        image.global(name)
    };
    match present {
        None => Ok(None),
        Some(_) => definition(image, fields, name, kind)
            .map(Some)
            .ok_or(Trap::Memory),
    }
}

/// ED_ParseEpair splits exactly three components on literal spaces. Missing
/// components are zero; the native C reader's out-of-buffer reads are rejected
/// as a source of additional data.
fn vector_words(mut text: &[u8]) -> [u32; 3] {
    std::array::from_fn(|_| {
        let split = text.iter().position(|&byte| byte == b' ');
        let part = split.map_or(text, |end| &text[..end]);
        let value = (atof_prefix_bytes(part).0 as f32).to_bits();
        text = split.map_or(&[], |end| &text[end + 1..]);
        value
    })
}

impl Vm {
    pub fn bind_map_fields<'a>(
        &self,
        names: &'a NameTable,
        native_rules: RuleSetId,
    ) -> Result<MapFields<'a>, Trap> {
        let mut keys = Vec::with_capacity(names.len());
        for index in 0..names.len() {
            let original = names.get(NameId(index as u32)).ok_or(Trap::String)?;
            // NQ rewrites editor aliases before trimming trailing key spaces.
            // QW has the same aliases but does not trim those spaces.
            let angle = original == b"angle";
            let key = match original {
                b"angle" => &b"angles"[..],
                b"light" => &b"light_lev"[..],
                _ => original,
            };
            let key = if native_rules == RuleSetId::Quake {
                &key[..key
                    .iter()
                    .rposition(|&byte| byte != b' ')
                    .map_or(0, |last| last + 1)]
            } else {
                key
            };
            let binding = if key.starts_with(b"_") {
                FieldBinding::Utility
            } else {
                self.image
                    .names
                    .find(key)
                    .and_then(|name| {
                        self.image
                            .fielddefs
                            .iter()
                            .find(|definition| definition.name == name)
                    })
                    .copied()
                    .map_or(FieldBinding::Unknown(key), |definition| {
                        FieldBinding::Value { definition, angle }
                    })
            };
            keys.push(binding);
        }
        Ok(MapFields {
            keys: keys.into_boxed_slice(),
            classname: optional_definition(&self.image, true, b"classname", 1)?,
            spawn_flags: optional_definition(&self.image, true, b"spawnflags", 2)?,
        })
    }

    /// `native` is a native byte offset. The existing allocator already clears
    /// ordinary rows; the seeded world row is intentionally not cleared here.
    pub fn admit_map_record(
        &mut self,
        bindings: &MapFields<'_>,
        native: u32,
        fields: &[SpawnField<'_>],
        mut warn: impl FnMut(&[u8]),
    ) -> Result<MapAdmission, Trap> {
        self.entity_number(native)?;
        for field in fields {
            let binding = *bindings
                .keys
                .get(field.key.0 as usize)
                .ok_or(Trap::String)?;
            let FieldBinding::Value { definition, angle } = binding else {
                if let FieldBinding::Unknown(key) = binding {
                    warn(key);
                }
                continue;
            };
            let kind = definition.value_type & 0x7fff;
            let value = if angle {
                let capacity = field.value.len().checked_add(4).ok_or(Trap::String)?;
                let mut text = Vec::with_capacity(capacity);
                text.extend_from_slice(b"0 ");
                text.extend_from_slice(field.value);
                text.extend_from_slice(b" 0");
                std::borrow::Cow::Owned(text)
            } else {
                std::borrow::Cow::Borrowed(field.value)
            };
            let mut words = [0u32; 3];
            let width = match kind {
                0 | 7 => continue,
                1 => {
                    words[0] = self.insert_string(&level_string(&value))? as u32;
                    1
                }
                2 => {
                    words[0] = (atof_prefix_bytes(&value).0 as f32).to_bits();
                    1
                }
                3 => {
                    words = vector_words(&value);
                    3
                }
                4 => {
                    let ordinal = usize::try_from(atoi_prefix(&value)).map_err(|_| Trap::Memory)?;
                    let value = ordinal
                        .checked_mul(self.stride)
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or(Trap::Memory)?;
                    // Forward references need backing capacity, not a current
                    // common lifetime or already allocated native row.
                    self.entity_number(value)?;
                    words[0] = value;
                    1
                }
                5 => {
                    let offset = self.image.field(&value).ok_or(Trap::Memory)?;
                    // ED_ParseEpair reads G_INT(def->ofs) at admission. An
                    // earlier callback may have changed this global word.
                    words[0] = *self.image.globals.get(offset).ok_or(Trap::Memory)?;
                    1
                }
                6 => {
                    words[0] = self.image.function(&value).ok_or(Trap::Function)?;
                    1
                }
                _ => return Err(Trap::Memory),
            };
            let address = self.field_address(native, definition.offset as u32, width)?;
            for (index, word) in words[..width].iter().copied().enumerate() {
                self.store_entity_word(address + index as u64 * 4, word)?;
            }
        }
        let read = |field| -> Result<u32, Trap> {
            let address = self.field_address(native, field as u32, 1)?;
            Ok(self.entities.read_word(address)? as u32)
        };
        let classname = bindings
            .classname
            .map(read)
            .transpose()?
            .filter(|&value| value != 0);
        let spawn_flags = bindings
            .spawn_flags
            .map(read)
            .transpose()?
            .map_or(0, |word| {
                super::native_integer(f64::from(f32::from_bits(word)))
            });
        Ok(MapAdmission {
            has_pairs: !fields.is_empty(),
            classname,
            spawn_flags,
        })
    }

    pub fn map_spawn_callback(&self, classname: u32) -> Result<Option<CallbackId>, Trap> {
        let classname = i32::try_from(classname).map_err(|_| Trap::String)?;
        let Some(function) = self.image.function(self.string(classname)?) else {
            return Ok(None);
        };
        self.projection
            .quakec_callback(function)
            .map(Some)
            .map_err(|_| Trap::Function)
    }

    pub fn initialize_map(
        &mut self,
        time: ThinkTime,
        map_name: &[u8],
        server_flags: f32,
    ) -> Result<(), Trap> {
        let time_global = self.call_fields.time_global.ok_or(Trap::Memory)?;
        let map_name_global = self.map_globals.map_name.ok_or(Trap::Memory)?;
        let server_flags_global = self.map_globals.server_flags.ok_or(Trap::Memory)?;
        let name = self.insert_string(map_name)?;
        self.image.globals[time_global] = (time.seconds() as f32).to_bits();
        self.image.globals[map_name_global] = name as u32;
        self.image.globals[server_flags_global] = server_flags.to_bits();
        Ok(())
    }

    pub fn set_map_time(&mut self, time: ThinkTime) -> Result<(), Trap> {
        let global = self.call_fields.time_global.ok_or(Trap::Memory)?;
        self.image.globals[global] = (time.seconds() as f32).to_bits();
        Ok(())
    }

    /// Optional native startup globals are absent in some programs. A named
    /// incompatible definition is an admission error, not a float overwrite.
    pub fn set_map_number(&mut self, name: &[u8], value: f32) -> Result<(), Trap> {
        if let Some(global) = optional_definition(&self.image, false, name, 2)? {
            self.image.globals[global] = value.to_bits();
        }
        Ok(())
    }

    pub fn seed_map_world(&mut self, world: u32, model_name: &[u8]) -> Result<(), Trap> {
        let addresses = self
            .projection
            .quakec_world_fields(&self.entities, world)
            .map_err(|_| Trap::Memory)?;
        let model = self.insert_string(model_name)? as u32;
        for (address, word) in
            addresses
                .into_iter()
                .zip([model, 1.0f32.to_bits(), 4.0f32.to_bits(), 7.0f32.to_bits()])
        {
            self.store_entity_word(address, word)?;
        }
        Ok(())
    }
}
