//! One exact-name localization catalog with bounded, allocation-free formatting.
use qa_core::{
    names::NameTable,
    primitives::IndexSpan,
    text::{integer_prefix, until_nul},
};
use qa_formats::localization::LocText;

#[derive(Clone, Copy, Default)]
struct Argument {
    span: IndexSpan,
    index: usize,
}
#[derive(Default)]
struct Arguments {
    fields: [Argument; 8],
    count: usize,
}
impl Arguments {
    fn parse(format: &[u8]) -> Option<Self> {
        let mut result = Self::default();
        let mut sequential = 0;
        let mut positional = false;
        let mut at = 0;
        while at < format.len() {
            if format[at] != b'{' {
                at += 1;
                continue;
            }
            let start = at;
            at += 1;
            if format.get(at) == Some(&b'{') {
                continue;
            }
            if result.count == result.fields.len() {
                return None;
            }
            let (value, consumed) = integer_prefix(&format[at..]);
            let index = if consumed == 0 {
                if positional {
                    return None;
                }
                let index = sequential;
                sequential += 1;
                index
            } else {
                if sequential != 0 {
                    return None;
                }
                positional = true;
                // Loc_Parse stores strtol's result in an unsigned byte.
                usize::from(value as u8)
            };
            at += consumed;
            at -= 1;
            let end = loop {
                if at >= format.len() || format[at] == 0 {
                    return None;
                }
                at += 1;
                if format.get(at) != Some(&b'}') {
                    continue;
                }
                let end = at + 1;
                at += 1;
                if format.get(at) != Some(&b'}') {
                    break end;
                }
            };
            result.fields[result.count] = Argument {
                span: IndexSpan {
                    first: start as u32,
                    count: (end - start) as u32,
                },
                index,
            };
            result.count += 1;
        }
        Some(result)
    }
    fn fields(&self) -> &[Argument] {
        &self.fields[..self.count]
    }
}
struct Template {
    format: Box<[u8]>,
    arguments: Arguments,
}

#[derive(Default)]
pub struct Catalog {
    names: Option<NameTable>,
    entries: Box<[Option<Template>]>,
}
impl Catalog {
    pub fn load(bytes: &[u8]) -> Result<Self, qa_formats::FormatError> {
        let parsed = qa_formats::localization::read(bytes)?;
        let names = NameTable::load(parsed.iter().map(|entry| &*entry.key))
            .map_err(|_| qa_formats::FormatError::InvalidRange)?;
        let mut entries = std::iter::repeat_with(|| None)
            .take(names.len())
            .collect::<Vec<_>>();
        for entry in parsed {
            let Some(arguments) = Arguments::parse(until_nul(&entry.format)) else {
                continue;
            };
            let id = names
                .find(&entry.key)
                .ok_or(qa_formats::FormatError::InvalidValue)?;
            entries[id.0 as usize] = Some(Template {
                format: entry.format,
                arguments,
            });
        }
        Ok(Self {
            names: Some(names),
            entries: entries.into_boxed_slice(),
        })
    }
    fn find(&self, key: &[u8]) -> Option<&Template> {
        let id = self.names.as_ref()?.find(key)?;
        self.entries.get(id.0 as usize)?.as_ref()
    }
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Missing keys and malformed/incomplete arguments retain native fallback text.
    /// Argument strings localize keys, but are never parsed as inline formats.
    pub fn format<'a, E>(
        &self,
        base: &[u8],
        count: usize,
        mut argument: impl FnMut(usize) -> Result<Option<&'a [u8]>, E>,
        output: &mut LocText,
    ) -> Result<(), E> {
        output.clear();
        let base = until_nul(base);
        let inline;
        let (fallback, format, fields) = if let Some(key) = base.strip_prefix(b"$") {
            let Some(template) = self.find(key) else {
                append(output, key);
                return Ok(());
            };
            (key, until_nul(&template.format), &template.arguments)
        } else if has_arguments(base) {
            let format = &base[..base.len().min(LocText::CAPACITY)];
            let Some(parsed) = Arguments::parse(format) else {
                append(output, base);
                return Ok(());
            };
            inline = parsed;
            (base, format, &inline)
        } else {
            append(output, base);
            return Ok(());
        };
        if fields.count == 0 {
            append(output, format);
            return Ok(());
        }
        if fields.fields().iter().any(|field| field.index >= count) {
            append(output, fallback);
            return Ok(());
        }
        for index in 0..count {
            if argument(index)?.is_none() {
                append(output, fallback);
                return Ok(());
            }
        }
        let mut first = 0;
        for field in fields.fields() {
            let range = field.span.range();
            append(output, &format[first..range.start]);
            if let Some(text) = argument(field.index)? {
                let text = until_nul(text);
                let text = if let Some(key) = text.strip_prefix(b"$") {
                    self.find(key)
                        .filter(|t| t.arguments.count == 0)
                        .map_or(key, |t| until_nul(&t.format))
                } else {
                    text
                };
                append(output, text);
            }
            first = range.end;
        }
        append(output, &format[first..]);
        Ok(())
    }
}
fn has_arguments(base: &[u8]) -> bool {
    let mut at = 0;
    while at < base.len() {
        if base[at] == b'{' {
            at += 1;
            match base.get(at) {
                None => return false,
                Some(b'{') => {}
                Some(_) => return true,
            }
        }
        at += 1;
    }
    false
}
fn append(output: &mut LocText, bytes: &[u8]) {
    let remaining = LocText::CAPACITY - output.as_bytes().len();
    let _ = output.append_bytes(&bytes[..bytes.len().min(remaining)]);
}
