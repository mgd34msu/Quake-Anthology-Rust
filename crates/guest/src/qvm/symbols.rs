//! QVM symbol lookup, `.map` loading, and execution profiles.
//!
//! Port of `src/compat/qvm/symbols.ts` (symbol lookup/loading, `ParseHex` and
//! `VM_VmProfile_f` from id Software's `qcommon/vm.c`; Copyright (C) 1999-2005
//! Id Software, Inc., GPL-2.0-or-later).
//!
//! Tokenizing mirrors the donor's `COM_Parse` subset used for map files
//! (whitespace/comment skipping, quoted and bare tokens, 1024-byte storage).
//! Symbol records keep the donor's release32 `vmSymbol_t` layout (16 bytes
//! plus the name at offset 12, value at 4, profile count at 8) in arena
//! bytes; the struct fields stay authoritative and sync into the backing so
//! allocation accounting observes the same retained blocks.

use std::cell::RefCell;

use crate::error::GuestError;

/// Retained symbol-file bytes (NUL-terminated by the filesystem layer).
#[derive(Debug, Clone)]
pub struct QvmSymbolFile {
    /// File bytes up to and including the terminator.
    pub terminated_bytes: Vec<u8>,
}

/// Filesystem surface used for `vm/*.map` loads.
pub trait QvmSymbolFiles {
    /// Read and retain `path`, or `None` when it does not exist.
    fn read_file_retained(&self, path: &str) -> Option<QvmSymbolFile>;
    /// Release a retained file.
    fn free_file(&self, file: &QvmSymbolFile);
}

/// Options for [`QvmSymbols::load`].
pub struct QvmSymbolLoadOptions {
    /// Module name the map path derives from.
    pub name: String,
    /// Developer level; zero skips loading entirely.
    pub developer: i32,
    /// Symbol-file source.
    pub files: Box<dyn QvmSymbolFiles>,
    /// Diagnostic sink.
    pub print: Box<dyn FnMut(&str)>,
}

impl std::fmt::Debug for QvmSymbolLoadOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmSymbolLoadOptions")
            .field("name", &self.name)
            .field("developer", &self.developer)
            .finish_non_exhaustive()
    }
}

/// One parsed symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSymbol {
    /// Instruction byte offset (program counter).
    pub value: i32,
    /// Symbol name.
    pub name: String,
    /// Profiled instruction count.
    pub profile_count: i32,
}

/// Mutable function symbol used for debug profiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmFunctionSymbol {
    /// Instruction byte offset (program counter).
    pub value: i32,
    /// Symbol name.
    pub name: String,
    /// Profiled instruction count.
    pub profile_count: i32,
}

#[derive(Debug, Clone)]
struct SymbolRecord {
    symbol: QvmFunctionSymbol,
    backing: Vec<u8>,
}

fn parse_hex(text: &str) -> i32 {
    let mut value: i32 = 0;
    for byte in text.bytes() {
        // Non-hex characters are ignored, matching the donor accumulator.
        if let Some(digit) = (byte as char).to_digit(16) {
            value = value.wrapping_mul(16).wrapping_add(digit as i32);
        }
    }
    value
}

/// Minimal `COM_Parse` over a Latin-1 byte string.
struct MapCursor<'a> {
    bytes: &'a [u8],
    offset: Option<usize>,
}

impl<'a> MapCursor<'a> {
    fn signed_byte(&self, offset: usize) -> i32 {
        if offset >= self.bytes.len() {
            return 0;
        }
        let byte = self.bytes[offset];
        if byte >= 128 {
            byte as i32 - 256
        } else {
            byte as i32
        }
    }

    fn parse(&mut self) -> Result<String, GuestError> {
        let Some(mut data) = self.offset else {
            return Ok(String::new());
        };
        let mut token = String::new();
        loop {
            loop {
                let byte = self.signed_byte(data);
                if byte > 32 {
                    break;
                }
                if byte == 0 {
                    self.offset = None;
                    return Ok(token);
                }
                data += 1;
            }
            let byte = self.signed_byte(data);
            if byte == b'/' as i32 && self.signed_byte(data + 1) == b'/' as i32 {
                data += 2;
                while {
                    let next = self.signed_byte(data);
                    next != 0 && next != b'\n' as i32
                } {
                    data += 1;
                }
            } else if byte == b'/' as i32 && self.signed_byte(data + 1) == b'*' as i32 {
                data += 2;
                while self.signed_byte(data) != 0
                    && (self.signed_byte(data) != b'*' as i32 || self.signed_byte(data + 1) != b'/' as i32)
                {
                    data += 1;
                }
                if self.signed_byte(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        if self.signed_byte(data) == b'"' as i32 {
            data += 1;
            loop {
                let byte = self.signed_byte(data);
                data += 1;
                if byte == b'"' as i32 || byte == 0 {
                    if token.len() == 1024 {
                        return Err(GuestError::invalid(
                            "COM_Parse quoted token terminator exceeds 1024-byte storage",
                        ));
                    }
                    self.offset = if byte == 0 { None } else { Some(data) };
                    return Ok(token);
                }
                if token.len() < 1024 {
                    token.push(self.bytes[data - 1] as char);
                }
            }
        }
        loop {
            if token.len() < 1024 {
                token.push(self.bytes[data] as char);
            }
            data += 1;
            if self.signed_byte(data) <= 32 {
                break;
            }
        }
        if token.len() == 1024 {
            token.clear();
        }
        self.offset = Some(data);
        Ok(token)
    }
}

/// Symbol records for one interpreter. Uses the release32 accounting profile.
pub struct QvmSymbols {
    records: Vec<SymbolRecord>,
    null_symbol: QvmFunctionSymbol,
    parsed_count: usize,
    print: RefCell<Box<dyn FnMut(&str)>>,
    instruction_pointers: Vec<i32>,
    allocate: Box<dyn FnMut(usize, &str) -> Result<Vec<u8>, GuestError>>,
    assert_live: Box<dyn Fn() -> Result<(), GuestError>>,
}

impl std::fmt::Debug for QvmSymbols {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmSymbols")
            .field("records", &self.records.len())
            .field("parsed_count", &self.parsed_count)
            .finish_non_exhaustive()
    }
}

impl QvmSymbols {
    /// Build symbol storage over prepared `instruction_pointers`.
    ///
    /// `allocate` issues the retained `vmSymbol_t` blocks; `assert_live`
    /// rejects use after the owning interpreter retires.
    pub fn new(
        instruction_pointers: Vec<i32>,
        allocate: Box<dyn FnMut(usize, &str) -> Result<Vec<u8>, GuestError>>,
        assert_live: Box<dyn Fn() -> Result<(), GuestError>>,
    ) -> Self {
        Self {
            records: Vec::new(),
            null_symbol: QvmFunctionSymbol {
                value: 0,
                name: String::new(),
                profile_count: 0,
            },
            parsed_count: 0,
            print: RefCell::new(Box::new(|_| {})),
            instruction_pointers,
            allocate,
            assert_live,
        }
    }

    /// Count parsed by the last [`Self::load`].
    pub fn count(&self) -> Result<usize, GuestError> {
        (self.assert_live)()?;
        Ok(self.parsed_count)
    }

    /// Snapshot of every record.
    pub fn entries(&mut self) -> Result<Vec<QvmSymbol>, GuestError> {
        (self.assert_live)()?;
        self.sync_backing();
        Ok(self
            .records
            .iter()
            .map(|record| {
                let symbol = &record.symbol;
                QvmSymbol {
                    value: symbol.value,
                    name: symbol.name.clone(),
                    profile_count: symbol.profile_count,
                }
            })
            .collect())
    }

    /// Index of the nearest function symbol at or before `value`.
    ///
    /// Returns `None` when no symbols are loaded (the donor's null symbol).
    /// The interpreter caches the index at function entry and bumps it per
    /// instruction with [`Self::add_profile_count`].
    pub fn function_symbol_index(&self, value: i32) -> Result<Option<usize>, GuestError> {
        (self.assert_live)()?;
        if self.records.is_empty() {
            return Ok(None);
        }
        let mut selected = 0;
        for (index, record) in self.records.iter().enumerate().skip(1) {
            if record.symbol.value > value {
                break;
            }
            selected = index;
        }
        Ok(Some(selected))
    }

    /// Add `delta` to a profile count with 32-bit wraparound.
    pub fn add_profile_count(&mut self, index: usize, delta: i32) {
        if let Some(record) = self.records.get_mut(index) {
            record.symbol.profile_count = record.symbol.profile_count.wrapping_add(delta);
        } else {
            self.null_symbol.profile_count = self.null_symbol.profile_count.wrapping_add(delta);
        }
    }

    /// `value` rendered as `name` or `name+offset` (`"NO SYMBOLS"` when empty).
    pub fn value_to_symbol(&self, value: i32) -> Result<String, GuestError> {
        (self.assert_live)()?;
        if self.records.is_empty() {
            return Ok("NO SYMBOLS".to_string());
        }
        let mut selected = &self.records[0].symbol;
        for record in self.records.iter().skip(1) {
            if record.symbol.value > value {
                break;
            }
            selected = &record.symbol;
        }
        if value == selected.value {
            return Ok(selected.name.clone());
        }
        let text = format!("{}+{}", selected.name, value.wrapping_sub(selected.value));
        if text.len() >= 1024 {
            let overflow = text.len();
            (self.print.borrow_mut())(&format!("Com_sprintf: overflow of {overflow} in 1024\n"));
        }
        Ok(text.chars().take(1023).collect())
    }

    /// Byte offset of `name`, or zero when unknown.
    pub fn symbol_to_value(&self, name: &str) -> Result<i32, GuestError> {
        (self.assert_live)()?;
        let end = name.find('\0').unwrap_or(name.len());
        let symbol_name = &name[..end];
        Ok(self
            .records
            .iter()
            .find(|record| record.symbol.name == symbol_name)
            .map_or(0, |record| record.symbol.value))
    }

    /// Load `vm/<base>.map` when `developer` is nonzero.
    pub fn load(&mut self, options: QvmSymbolLoadOptions) -> Result<(), GuestError> {
        if options.developer == 0 {
            return Ok(());
        }
        (self.assert_live)()?;
        let mut print = options.print;
        let end = options.name.find('\0').unwrap_or(options.name.len());
        let name = &options.name[..end];
        let base = name.find('.').map_or(name, |dot| &name[..dot]);
        let requested = format!("vm/{base}.map");
        if requested.len() >= 64 {
            let overflow = requested.len();
            print(&format!("Com_sprintf: overflow of {overflow} in 64\n"));
        }
        let path: String = requested.chars().take(63).collect();
        let Some(file) = options.files.read_file_retained(&path) else {
            print(&format!("Couldn't load symbol file: {path}\n"));
            self.print = RefCell::new(print);
            return Ok(());
        };
        let mut cursor = MapCursor {
            bytes: &file.terminated_bytes,
            offset: Some(0),
        };
        let mut count = 0;
        loop {
            let segment = cursor.parse()?;
            if segment.is_empty() {
                break;
            }
            if parse_hex(&segment) != 0 {
                cursor.parse()?;
                cursor.parse()?;
                continue;
            }
            let address = cursor.parse()?;
            if address.is_empty() {
                print("WARNING: incomplete line at end of file\n");
                break;
            }
            let mut value = parse_hex(&address);
            let symbol_name = cursor.parse()?;
            if symbol_name.is_empty() {
                print("WARNING: incomplete line at end of file\n");
                break;
            }
            // vmSymbol_t is 16 bytes in the selected release32 profile,
            // including symName[1] and tail padding.
            let backing = (self.allocate)(16 + symbol_name.len(), &path)?;
            if count == 0 {
                self.records.clear();
            }
            if value >= 0 && (value as usize) < self.instruction_pointers.len() {
                value = self.instruction_pointers[value as usize];
            }
            let mut record = SymbolRecord {
                symbol: QvmFunctionSymbol {
                    value,
                    name: symbol_name,
                    profile_count: 0,
                },
                backing,
            };
            sync_record(&mut record);
            self.records.push(record);
            count += 1;
        }
        self.parsed_count = count;
        print(&format!("{count} symbols parsed from {path}\n"));
        options.files.free_file(&file);
        self.print = RefCell::new(print);
        Ok(())
    }

    /// Print the execution profile and reset every count.
    pub fn print_profile(&mut self, print: &mut dyn FnMut(&str), debug_enabled: bool) -> Result<(), GuestError> {
        (self.assert_live)()?;
        if self.parsed_count == 0 {
            return Ok(());
        }
        let len = self.parsed_count.min(self.records.len());
        let mut order: Vec<usize> = (0..len).collect();
        order.sort_by_key(|index| self.records[*index].symbol.profile_count);
        let total: i64 = order
            .iter()
            .map(|index| i64::from(self.records[*index].symbol.profile_count))
            .sum();
        if total == 0 {
            // C's NaN-to-int conversion has no defined percentage, including
            // after resetting a debug profile.
            print(if debug_enabled {
                "vmprofile: percentages are undefined with zero total instructions.\n"
            } else {
                "vmprofile: percentages are undefined with zero total instructions; DEBUG_VM is disabled.\n"
            });
        }
        for index in order {
            (self.assert_live)()?;
            let count = self.records[index].symbol.profile_count;
            let prefix = if total == 0 {
                "    ".to_string()
            } else {
                // Donor: Math.trunc(100 * fround(count) / total), padded to 2.
                let percent = ((100.0 * f64::from(count)) / total as f64).trunc() as i64;
                format!("{percent:>2}% ")
            };
            let name = &self.records[index].symbol.name;
            print(&format!("{prefix}{count:>9} {name}\n"));
            (self.assert_live)()?;
            self.records[index].symbol.profile_count = 0;
        }
        print(&format!("    {total:>9} total\n"));
        self.sync_backing();
        Ok(())
    }

    fn sync_backing(&mut self) {
        for record in &mut self.records {
            sync_record(record);
        }
    }
}

fn sync_record(record: &mut SymbolRecord) {
    if record.backing.len() >= 12 + record.symbol.name.len() {
        record.backing[4..8].copy_from_slice(&record.symbol.value.to_le_bytes());
        record.backing[8..12].copy_from_slice(&record.symbol.profile_count.to_le_bytes());
        record.backing[12..12 + record.symbol.name.len()].copy_from_slice(record.symbol.name.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct MapFiles {
        files: HashMap<String, Vec<u8>>,
        freed: Rc<RefCell<usize>>,
    }

    impl QvmSymbolFiles for MapFiles {
        fn read_file_retained(&self, path: &str) -> Option<QvmSymbolFile> {
            self.files.get(path).map(|bytes| QvmSymbolFile {
                terminated_bytes: bytes.clone(),
            })
        }

        fn free_file(&self, _file: &QvmSymbolFile) {
            *self.freed.borrow_mut() += 1;
        }
    }

    fn symbols() -> (QvmSymbols, Rc<RefCell<Vec<String>>>) {
        let printed = Rc::new(RefCell::new(Vec::new()));
        let symbols = QvmSymbols::new(vec![0, 5, 10], Box::new(|len, _| Ok(vec![0; len])), Box::new(|| Ok(())));
        (symbols, printed)
    }

    fn load(symbols: &mut QvmSymbols, files: MapFiles, printed: Rc<RefCell<Vec<String>>>, name: &str) {
        let printed_clone = Rc::clone(&printed);
        symbols
            .load(QvmSymbolLoadOptions {
                name: name.to_string(),
                developer: 1,
                files: Box::new(files),
                print: Box::new(move |text| printed_clone.borrow_mut().push(text.to_string())),
            })
            .unwrap();
    }

    #[test]
    fn loads_map_symbols_and_resolves_values() {
        let (mut symbols, printed) = symbols();
        let mut files = HashMap::new();
        files.insert("vm/qagame.map".to_string(), b"0 0 vmMain\n0 1 Other\n".to_vec());
        load(
            &mut symbols,
            MapFiles {
                files,
                freed: Rc::new(RefCell::new(0)),
            },
            Rc::clone(&printed),
            "qagame.qvm",
        );
        assert_eq!(symbols.count().unwrap(), 2);
        assert_eq!(symbols.symbol_to_value("Other").unwrap(), 5);
        assert_eq!(symbols.value_to_symbol(5).unwrap(), "Other");
        assert_eq!(symbols.value_to_symbol(7).unwrap(), "Other+2");
        assert_eq!(symbols.symbol_to_value("missing").unwrap(), 0);
        assert!(printed.borrow().iter().any(|line| line.contains("2 symbols parsed")));
    }

    #[test]
    fn skips_loading_without_developer_and_reports_missing_files() {
        let (mut symbols, printed) = symbols();
        symbols
            .load(QvmSymbolLoadOptions {
                name: "qagame.qvm".to_string(),
                developer: 0,
                files: Box::new(MapFiles {
                    files: HashMap::new(),
                    freed: Rc::new(RefCell::new(0)),
                }),
                print: Box::new(|_| {}),
            })
            .unwrap();
        assert_eq!(symbols.count().unwrap(), 0);
        load(
            &mut symbols,
            MapFiles {
                files: HashMap::new(),
                freed: Rc::new(RefCell::new(0)),
            },
            Rc::clone(&printed),
            "qagame.qvm",
        );
        assert!(printed
            .borrow()
            .iter()
            .any(|line| line.contains("Couldn't load symbol file")));
        assert_eq!(symbols.value_to_symbol(0).unwrap(), "NO SYMBOLS");
    }

    #[test]
    fn data_segments_skip_and_profile_resets() {
        let (mut symbols, _) = symbols();
        let mut files = HashMap::new();
        files.insert(
            "vm/qagame.map".to_string(),
            b"1 0 ignored\n0 2 Leaf\n// comment\n".to_vec(),
        );
        let printed = Rc::new(RefCell::new(Vec::new()));
        load(
            &mut symbols,
            MapFiles {
                files,
                freed: Rc::new(RefCell::new(0)),
            },
            Rc::clone(&printed),
            "qagame",
        );
        assert_eq!(symbols.count().unwrap(), 1);
        assert_eq!(symbols.symbol_to_value("Leaf").unwrap(), 10);
        let index = symbols.function_symbol_index(10).unwrap().unwrap();
        symbols.add_profile_count(index, 3);
        assert!(symbols.function_symbol_index(0).unwrap().is_some());
        let output = Rc::new(RefCell::new(Vec::new()));
        let output_clone = Rc::clone(&output);
        symbols
            .print_profile(&mut move |text| output_clone.borrow_mut().push(text.to_string()), false)
            .unwrap();
        assert!(output.borrow().iter().any(|line| line.contains("Leaf")));
        assert_eq!(symbols.entries().unwrap()[0].profile_count, 0);
    }

    #[test]
    fn zero_total_reports_undefined_percentages() {
        let (mut symbols, _) = symbols();
        let mut files = HashMap::new();
        files.insert("vm/qagame.map".to_string(), b"0 0 vmMain\n".to_vec());
        load(
            &mut symbols,
            MapFiles {
                files,
                freed: Rc::new(RefCell::new(0)),
            },
            Rc::new(RefCell::new(Vec::new())),
            "qagame",
        );
        let output = Rc::new(RefCell::new(Vec::new()));
        let output_clone = Rc::clone(&output);
        symbols
            .print_profile(&mut move |text| output_clone.borrow_mut().push(text.to_string()), true)
            .unwrap();
        assert!(output
            .borrow()
            .iter()
            .any(|line| line.contains("percentages are undefined")));
    }
}
