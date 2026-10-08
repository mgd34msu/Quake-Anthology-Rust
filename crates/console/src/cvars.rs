use crate::text::MAX_TEXT;
use crate::{
    catalog::{Condition, ConversionKind, DefaultKind, Operation, Scope},
    conversion::{self, Input, Text},
    cvars_generated::{BINDINGS, CONVERSIONS, DEFAULTS, DEFINITIONS, FLAGS, OPERANDS},
    numbers::{integer, number},
    views::{Context, Role, Source},
};
use qa_core::primitives::CvarHandle;
use qa_core::text::FixedText;
use std::{borrow::Cow, fmt::Write};

pub use crate::catalog::Definition;
const EMPTY: u16 = u16::MAX;
const ROLES: [Role; 3] = [Role::Engine, Role::Game, Role::Cgame];

#[derive(Clone, Copy)]
struct NameSlot {
    first: u16,
    second: u16,
}
struct NameIndex {
    slots: Vec<NameSlot>,
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    lookups: std::cell::Cell<u64>,
}
impl NameIndex {
    fn load() -> Self {
        let mut index = Self {
            #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
            lookups: std::cell::Cell::new(0),
            slots: vec![
                NameSlot {
                    first: EMPTY,
                    second: EMPTY
                };
                4096
            ],
        };
        for (binding, b) in BINDINGS.iter().enumerate() {
            let mut bucket = index.bucket(b.name);
            loop {
                let slot = &mut index.slots[bucket];
                if slot.first == EMPTY {
                    slot.first = binding as u16;
                    break;
                }
                if BINDINGS[slot.first as usize]
                    .name
                    .eq_ignore_ascii_case(b.name)
                {
                    slot.second = binding as u16;
                    break;
                }
                bucket = (bucket + 1) & (index.slots.len() - 1);
            }
        }
        index
    }
    fn bucket(&self, name: &str) -> usize {
        // Bucket key only; this does not identify content or persist/cache a digest.
        let mut key = 2166136261u32;
        for byte in name.bytes() {
            key = (key ^ u32::from(byte.to_ascii_lowercase())).wrapping_mul(16777619);
        }
        key as usize & (self.slots.len() - 1)
    }
    fn lookup(&self, name: &str, side: Scope) -> Option<u16> {
        #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
        self.lookups.set(self.lookups.get().wrapping_add(1));
        let mut bucket = self.bucket(name);
        loop {
            let slot = self.slots[bucket];
            if slot.first == EMPTY {
                return None;
            }
            if BINDINGS[slot.first as usize]
                .name
                .eq_ignore_ascii_case(name)
            {
                let mut fallback = None;
                for id in [slot.first, slot.second] {
                    if id == EMPTY {
                        continue;
                    }
                    let scope = BINDINGS[id as usize].scope;
                    if scope == Scope::Any {
                        fallback = Some(id);
                    } else if scope == side || (scope == Scope::Client && side == Scope::Any) {
                        return Some(id);
                    }
                }
                return fallback;
            }
            bucket = (bucket + 1) & (self.slots.len() - 1);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct View {
    handle: CvarHandle,
    binding: u16,
    context: Context,
}
impl View {
    pub fn canonical(self) -> CvarHandle {
        self.handle
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    Conversion(conversion::Error),
    ReadOnly,
    InitOnly,
    Cheats,
    InvalidInfo,
}
impl From<conversion::Error> for WriteError {
    fn from(e: conversion::Error) -> Self {
        Self::Conversion(e)
    }
}
struct Value {
    name: &'static str,
    row: usize,
    seat: u8,
    explicit: bool,
    numbers: [f32; 5],
    integers: [i32; 5],
    revision: u64,
    command_flags: [Option<u32>; 5],
}
struct Detail {
    text: FixedText<MAX_TEXT>,
    revisions: [(CvarHandle, u64); 33],
    count: usize,
    present: bool,
}
impl Default for Detail {
    fn default() -> Self {
        Self {
            text: FixedText::default(),
            revisions: [(CvarHandle(0), 0); 33],
            count: 0,
            present: false,
        }
    }
}
struct PendingDetail {
    view: View,
    text: FixedText<MAX_TEXT>,
    modified: bool,
}
struct PendingWrite {
    first: Option<(CvarHandle, FixedText<MAX_TEXT>)>,
    changes: [Option<(CvarHandle, FixedText<64>)>; 32],
    detail: Option<PendingDetail>,
}
impl PendingWrite {
    fn handles(&self) -> impl Iterator<Item = CvarHandle> + '_ {
        self.first
            .iter()
            .map(|(h, _)| *h)
            .chain(self.changes.iter().flatten().map(|(h, _)| *h))
    }
}
type Projections = [[Result<f32, conversion::Error>; 3]; 5];

pub struct Cvars {
    values: Vec<Value>,
    texts: Box<[FixedText<MAX_TEXT>]>,
    offsets: Vec<usize>,
    defaults: Vec<[Option<Cow<'static, str>>; 5]>,
    index: NameIndex,
    details: Vec<[Option<Box<Detail>>; 5]>,
    pending: Vec<PendingWrite>,
    projections: Vec<Projections>,
    dependents: Vec<Vec<u16>>,
    dirty_handles: Vec<CvarHandle>,
    dirty_values: Vec<bool>,
    dirty_bindings: Vec<u16>,
    dirty_projections: Vec<bool>,
    context: Context,
    revision_clock: u64,
    pub server_active: bool,
    pub cheats: bool,
    pub initialized: bool,
}

impl Cvars {
    pub fn new() -> Self {
        Self::with_context(Context::default())
    }
    pub fn with_context(context: Context) -> Self {
        let index = NameIndex::load();
        let mut values =
            Vec::with_capacity(DEFINITIONS.iter().map(|d| d.family_count as usize).sum());
        let mut offsets = Vec::with_capacity(DEFINITIONS.len());
        for (row, definition) in DEFINITIONS.iter().enumerate() {
            offsets.push(values.len());
            for slot in 0..definition.family_count {
                let seat = if definition.family_count == 1 {
                    0
                } else {
                    slot + 1
                };
                let binding = BINDINGS
                    .iter()
                    .find(|b| b.canonical && b.row as usize == row && b.seat == seat);
                let name = binding.map_or(definition.name, |b| b.name);
                values.push(Value {
                    name,
                    row,
                    seat,
                    explicit: false,
                    numbers: [0.0; 5],
                    integers: [0; 5],
                    revision: 0,
                    command_flags: [None; 5],
                });
            }
        }
        let pending_capacity = values.len();
        let texts = (0..values.len()).map(|_| FixedText::default()).collect();
        let mut registry = Self {
            values,
            texts,
            offsets,
            index,
            context,
            revision_clock: 0,
            defaults: (0..DEFINITIONS.len())
                .map(|_| std::array::from_fn(|_| None))
                .collect(),
            details: BINDINGS
                .iter()
                .map(|b| {
                    std::array::from_fn(|source| {
                        let row = &DEFINITIONS[b.row as usize];
                        let conversion = &CONVERSIONS[if row.stored {
                            b.conversions[source]
                        } else {
                            row.rule_conversion
                        } as usize];
                        (conversion.detail || conversion.operation == Operation::MusicMute)
                            .then(Box::default)
                    })
                })
                .collect(),
            pending: Vec::with_capacity(pending_capacity),
            projections: vec![[[Ok(0.0); 3]; 5]; BINDINGS.len()],
            dependents: (0..pending_capacity).map(|_| Vec::new()).collect(),
            dirty_handles: Vec::with_capacity(pending_capacity),
            dirty_values: vec![false; pending_capacity],
            dirty_bindings: Vec::with_capacity(BINDINGS.len()),
            dirty_projections: vec![false; BINDINGS.len()],
            server_active: false,
            cheats: false,
            initialized: false,
        };
        for (i, binding) in BINDINGS.iter().enumerate() {
            let handle = registry.slot(binding.row as usize, binding.seat);
            registry.dependents[handle.0 as usize].push(i as u16);
            let definition = &DEFINITIONS[binding.row as usize];
            for source in 0..5 {
                let c = &CONVERSIONS[if definition.stored {
                    binding.conversions[source]
                } else {
                    definition.rule_conversion
                } as usize];
                for operand in &OPERANDS[c.operands.clone()] {
                    let handle = registry.slot(operand.row as usize, 0);
                    let dependents = &mut registry.dependents[handle.0 as usize];
                    if !dependents.contains(&(i as u16)) {
                        dependents.push(i as u16);
                    }
                }
            }
        }
        registry.resolve_defaults();
        registry.refresh_numbers();
        registry.refresh_projections();
        registry
    }
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn select_context(&mut self, context: Context) {
        if context == self.context {
            return;
        }
        let previous: Vec<_> = self
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                (
                    self.effective(CvarHandle(index as u32), self.context.source)
                        .to_owned(),
                    value.numbers[self.context.source as usize].to_bits(),
                    value.integers[self.context.source as usize],
                )
            })
            .collect();
        if self.context.side != context.side || self.context.dedicated != context.dedicated {
            self.context = context;
            self.resolve_defaults();
            self.refresh_numbers();
        } else {
            self.context = context;
        }
        for (index, (text, bits, integer)) in previous.into_iter().enumerate() {
            let handle = CvarHandle(index as u32);
            if text != self.text(handle)
                || bits != self.value(handle).to_bits()
                || integer != self.integer(handle)
            {
                self.mark_change(handle);
            }
        }
        self.refresh_projections();
    }
    fn slot(&self, row: usize, seat: u8) -> CvarHandle {
        CvarHandle(
            (self.offsets[row]
                + if DEFINITIONS[row].family_count > 1 {
                    seat.saturating_sub(1) as usize
                } else {
                    0
                }) as u32,
        )
    }
    pub fn bind(&self, name: &str, context: Context) -> Option<View> {
        let binding = self.index.lookup(name, context.side)?;
        let b = &BINDINGS[binding as usize];
        Some(View {
            handle: self.slot(b.row as usize, b.seat),
            binding,
            context,
        })
    }
    /// Canonical engine handle; converted boundaries retain a View instead.
    pub fn find(&self, name: &str) -> Option<CvarHandle> {
        self.bind(name, self.context).map(View::canonical)
    }
    fn effective(&self, handle: CvarHandle, source: Source) -> &str {
        let value = &self.values[handle.0 as usize];
        if value.explicit {
            self.texts[handle.0 as usize].as_str()
        } else {
            self.defaults[value.row][source as usize]
                .as_deref()
                .unwrap_or("")
        }
    }
    pub fn value(&self, handle: CvarHandle) -> f32 {
        self.values[handle.0 as usize].numbers[self.context.source as usize]
    }
    /// Per-client cached views keep movement/input defaults independent of the
    /// active map's console context; frame reads never parse text or bind names.
    pub fn value_in(&self, handle: CvarHandle, source: Source) -> f32 {
        self.values[handle.0 as usize].numbers[source as usize]
    }
    pub fn integer(&self, handle: CvarHandle) -> i32 {
        self.values[handle.0 as usize].integers[self.context.source as usize]
    }
    pub fn integer_in(&self, handle: CvarHandle, source: Source) -> i32 {
        self.values[handle.0 as usize].integers[source as usize]
    }
    pub fn generation(&self, handle: CvarHandle) -> u64 {
        self.values[handle.0 as usize].revision
    }
    /// Converted state can also depend on other canonical rows (colour, dmflags).
    pub fn view_generation(&self, view: View) -> u64 {
        OPERANDS[self.conversion(view).operands.clone()]
            .iter()
            .fold(self.generation(view.handle), |generation, operand| {
                generation.max(self.generation(self.slot(operand.row as usize, 0)))
            })
    }
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    pub fn reset_lookup_count(&self) {
        self.index.lookups.set(0);
    }
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    pub fn lookup_count(&self) -> u64 {
        self.index.lookups.get()
    }
    pub fn text(&self, handle: CvarHandle) -> &str {
        self.effective(handle, self.context.source)
    }
    pub fn is_explicit(&self, handle: CvarHandle) -> bool {
        self.values[handle.0 as usize].explicit
    }
    pub fn default_available(&self, handle: CvarHandle, source: Source) -> bool {
        self.defaults[self.values[handle.0 as usize].row][source as usize].is_some()
    }
    pub fn read(&self, view: View) -> Result<Text<'_>, conversion::Error> {
        let b = &BINDINGS[view.binding as usize];
        let definition = &DEFINITIONS[b.row as usize];
        let text = self.effective(view.handle, view.context.source);
        if b.canonical && view.context.role == Role::Engine && definition.stored {
            return Ok(Text::Borrowed(text));
        }
        let detail = self.detail(view);
        let operands = |row| self.effective(self.slot(row as usize, 0), view.context.source);
        conversion::read(Input {
            context: view.context,
            conversion: self.conversion(view),
            binding: b,
            value: text,
            current: text,
            detail,
            operand: &operands,
        })
    }
    pub fn numeric(&self, view: View) -> Result<f32, conversion::Error> {
        let b = &BINDINGS[view.binding as usize];
        if b.canonical && view.context.role == Role::Engine && DEFINITIONS[b.row as usize].stored {
            Ok(self.values[view.handle.0 as usize].numbers[view.context.source as usize])
        } else {
            self.projections[view.binding as usize][view.context.source as usize]
                [role_index(view.context.role)]
        }
    }
    fn conversion(&self, view: View) -> &'static crate::catalog::Conversion {
        let b = &BINDINGS[view.binding as usize];
        let row = &DEFINITIONS[b.row as usize];
        &CONVERSIONS[if row.stored {
            b.conversions[view.context.source as usize]
        } else {
            row.rule_conversion
        } as usize]
    }
    pub fn flags(&self, view: View) -> u32 {
        if let Some(flags) =
            self.values[view.handle.0 as usize].command_flags[view.context.source as usize]
        {
            return flags;
        }
        let b = &BINDINGS[view.binding as usize];
        let row = &DEFINITIONS[b.row as usize];
        let native = b.native_sources & (1 << view.context.source as u8) != 0;
        let source = if native {
            view.context.source as usize
        } else {
            row.home.map_or(view.context.source as usize, usize::from)
        };
        let mut flags = u32::from(row.archive);
        for clause in &FLAGS[row.defaults[source].flags.clone()] {
            if clause.issues == 0
                && (clause.member.is_empty()
                    || clause.member.eq_ignore_ascii_case(b.name)
                    || b.canonical
                    || !native)
            {
                flags |= clause.bits;
            }
        }
        if row.policies & 1 != 0 {
            flags |= 32;
        }
        flags
    }
    pub fn private(&self, view: View) -> bool {
        DEFINITIONS[BINDINGS[view.binding as usize].row as usize].policies & 2 != 0
    }
    fn check_write(&self, view: View, text: &str) -> Result<(), WriteError> {
        let flags = self.flags(view);
        if flags & 64 != 0 {
            return Err(WriteError::ReadOnly);
        }
        if flags & 16 != 0 && self.initialized {
            return Err(WriteError::InitOnly);
        }
        if flags & 512 != 0 && !self.cheats {
            return Err(WriteError::Cheats);
        }
        if flags & 6 != 0
            && text
                .bytes()
                .any(|b| matches!(b, b'\\' | b'"' | b';') || b < 32)
        {
            return Err(WriteError::InvalidInfo);
        }
        Ok(())
    }
    pub fn write(&mut self, view: View, text: &str) -> Result<(), WriteError> {
        self.write_inner(view, text, true)
    }
    /// Q2 Cvar_FullSet changes this source's flags and bypasses normal policy.
    pub fn full_set(&mut self, view: View, text: &str, flags: u32) -> Result<(), WriteError> {
        let source = view.context.source as usize;
        let previous = self.values[view.handle.0 as usize].command_flags[source];
        self.values[view.handle.0 as usize].command_flags[source] = Some(flags);
        if let Err(error) = self.write_inner(view, text, false) {
            self.values[view.handle.0 as usize].command_flags[source] = previous;
            return Err(error);
        }
        self.mark_change(view.handle);
        self.refresh_changes();
        Ok(())
    }
    fn write_inner(&mut self, view: View, text: &str, enforce: bool) -> Result<(), WriteError> {
        if enforce {
            self.check_write(view, text)?;
        }
        let binding = &BINDINGS[view.binding as usize];
        let current = self.effective(view.handle, view.context.source);
        let detail = self.detail(view);
        let operands = |row| self.effective(self.slot(row as usize, 0), view.context.source);
        let c = if binding.canonical
            && view.context.role == Role::Engine
            && DEFINITIONS[binding.row as usize].stored
        {
            &CONVERSIONS[0]
        } else {
            self.conversion(view)
        };
        let out = conversion::write(Input {
            context: view.context,
            conversion: c,
            binding,
            value: text,
            current,
            detail,
            operand: &operands,
        })?;
        let detail = if out.detail {
            let mut value = FixedText::default();
            value
                .set(out.detail_value.unwrap_or(text))
                .map_err(|_| conversion::Error::TextTooLong)?;
            Some(value)
        } else {
            None
        };
        let detail_modified = detail.as_ref().map(FixedText::as_str) != self.detail(view);
        let mut changes = PendingWrite {
            first: None,
            changes: std::array::from_fn(|_| None),
            detail: None,
        };
        let mut pending = enforce && self.flags(view) & 32 != 0 && self.server_active;
        if DEFINITIONS[binding.row as usize].stored {
            let mut value = FixedText::default();
            if let Some(prefix) = out.prefix {
                value
                    .write_str(prefix)
                    .map_err(|_| conversion::Error::TextTooLong)?;
            }
            value
                .write_str(out.text.as_str())
                .map_err(|_| conversion::Error::TextTooLong)?;
            changes.first = Some((view.handle, value));
        }
        for (i, change) in out
            .changes
            .into_iter()
            .take(out.change_count)
            .flatten()
            .enumerate()
        {
            let handle = self.slot(change.row as usize, 0);
            let target = self.values[handle.0 as usize].name;
            if let Some(target_view) = self.bind(target, view.context) {
                if enforce {
                    self.check_write(target_view, change.text.as_str())?;
                }
                pending |= enforce && self.flags(target_view) & 32 != 0 && self.server_active;
                let mut text = FixedText::default();
                text.set(change.text.as_str())
                    .map_err(|_| conversion::Error::TextTooLong)?;
                changes.changes[i] = Some((handle, text));
            }
        }
        self.pending
            .retain(|old| !old.handles().any(|h| changes.handles().any(|new| h == new)));
        changes.detail = detail.map(|text| PendingDetail {
            view,
            text,
            modified: detail_modified,
        });
        if pending {
            self.pending.push(changes);
            return Ok(());
        }
        self.publish_group(changes)?;
        self.refresh_changes();
        Ok(())
    }
    fn detail(&self, view: View) -> Option<&str> {
        self.details[view.binding as usize][view.context.source as usize]
            .as_ref()
            .filter(|d| {
                d.present
                    && d.revisions[..d.count]
                        .iter()
                        .all(|(h, revision)| self.values[h.0 as usize].revision == *revision)
            })
            .map(|d| d.text.as_str())
    }
    fn save_detail(
        &mut self,
        view: View,
        text: FixedText<MAX_TEXT>,
        modified: bool,
    ) -> Result<(), WriteError> {
        if modified {
            self.mark_change(view.handle);
        }
        let mut revisions = [(CvarHandle(0), 0); 33];
        revisions[0] = (view.handle, self.values[view.handle.0 as usize].revision);
        let mut count = 1;
        for operand in &OPERANDS[self.conversion(view).operands.clone()] {
            let handle = self.slot(operand.row as usize, 0);
            revisions[count] = (handle, self.values[handle.0 as usize].revision);
            count += 1;
        }
        let detail = self.details[view.binding as usize][view.context.source as usize]
            .as_mut()
            .ok_or(conversion::Error::PolicyRequired)?;
        detail.text = text;
        detail.revisions = revisions;
        detail.count = count;
        detail.present = true;
        Ok(())
    }
    fn clear_details(&mut self, handle: CvarHandle) {
        let mut modified = false;
        for &binding in &self.dependents[handle.0 as usize] {
            let source_details = &mut self.details[binding as usize];
            for detail in source_details.iter_mut().flatten() {
                if detail.present
                    && detail.revisions[..detail.count]
                        .iter()
                        .any(|(h, _)| *h == handle)
                {
                    detail.present = false;
                    modified = true;
                }
            }
        }
        if modified {
            self.mark_change(handle);
        }
    }
    fn cancel_pending(&mut self, handle: CvarHandle) {
        self.pending.retain(|p| !p.handles().any(|h| h == handle));
    }
    fn publish(&mut self, handle: CvarHandle, text: FixedText<MAX_TEXT>) {
        let value = &mut self.values[handle.0 as usize];
        if !value.explicit || self.texts[handle.0 as usize].as_str() != text.as_str() {
            value.explicit = true;
            self.texts[handle.0 as usize] = text;
            self.mark_change(handle);
        }
    }
    fn publish_group(&mut self, pending: PendingWrite) -> Result<(), WriteError> {
        if let Some((handle, text)) = pending.first {
            self.clear_details(handle);
            self.publish(handle, text);
        }
        for (handle, text) in pending.changes.into_iter().flatten() {
            let mut value = FixedText::default();
            value
                .set(text.as_str())
                .map_err(|_| conversion::Error::TextTooLong)?;
            self.clear_details(handle);
            self.publish(handle, value);
        }
        if let Some(detail) = pending.detail {
            self.save_detail(detail.view, detail.text, detail.modified)?;
        }
        Ok(())
    }
    fn mark_change(&mut self, handle: CvarHandle) {
        self.revision_clock = self.revision_clock.wrapping_add(1);
        self.values[handle.0 as usize].revision = self.revision_clock;
        if !self.dirty_values[handle.0 as usize] {
            self.dirty_values[handle.0 as usize] = true;
            self.dirty_handles.push(handle);
        }
    }
    /// Engine-owned updates bypass command policy (ROM values and telemetry).
    pub fn set(&mut self, handle: CvarHandle, value: f32) -> Result<(), WriteError> {
        let mut text = FixedText::<64>::default();
        write!(text, "{value}").map_err(|_| conversion::Error::TextTooLong)?;
        self.set_text(handle, text.as_str())
    }
    pub fn set_text(&mut self, handle: CvarHandle, text: &str) -> Result<(), WriteError> {
        let mut value = FixedText::default();
        value
            .set(text)
            .map_err(|_| conversion::Error::TextTooLong)?;
        self.cancel_pending(handle);
        self.clear_details(handle);
        self.publish(handle, value);
        self.refresh_changes();
        Ok(())
    }
    pub fn reset(&mut self, handle: CvarHandle) {
        self.cancel_pending(handle);
        self.clear_details(handle);
        let value = &mut self.values[handle.0 as usize];
        value.explicit = false;
        self.mark_change(handle);
        self.refresh_changes();
    }
    pub fn apply_latches(&mut self) -> Result<(), WriteError> {
        while !self.pending.is_empty() {
            let pending = self.pending.remove(0);
            self.publish_group(pending)?;
        }
        self.refresh_changes();
        Ok(())
    }
    pub fn entries(
        &self,
    ) -> impl Iterator<Item = (CvarHandle, &'static str, &str, &'static Definition, u8)> {
        self.values.iter().enumerate().map(|(index, v)| {
            let handle = CvarHandle(index as u32);
            (
                handle,
                v.name,
                self.text(handle),
                &DEFINITIONS[v.row],
                v.seat,
            )
        })
    }
    fn refresh_number_at(&mut self, index: usize) {
        let numbers = std::array::from_fn(|s| {
            number(
                self.effective(CvarHandle(index as u32), Source::ALL[s]),
                Source::ALL[s],
            )
        });
        let integers = std::array::from_fn(|s| {
            integer(self.effective(CvarHandle(index as u32), Source::ALL[s]))
        });
        self.values[index].numbers = numbers;
        self.values[index].integers = integers;
    }
    fn refresh_numbers(&mut self) {
        for index in 0..self.values.len() {
            self.refresh_number_at(index);
        }
    }
    fn refresh_projection_at(&mut self, index: usize) {
        let binding = &BINDINGS[index];
        let projections = std::array::from_fn(|source| {
            std::array::from_fn(|role| {
                let context = Context {
                    source: Source::ALL[source],
                    side: self.context.side,
                    role: ROLES[role],
                    dedicated: self.context.dedicated,
                    seat: qa_core::sys_events::SeatId::FIRST,
                    event_time: None,
                };
                let view = View {
                    handle: self.slot(binding.row as usize, binding.seat),
                    binding: index as u16,
                    context,
                };
                self.read(view)
                    .map(|text| number(text.as_str(), context.source))
            })
        });
        self.projections[index] = projections;
    }
    fn refresh_projections(&mut self) {
        for index in 0..BINDINGS.len() {
            self.refresh_projection_at(index);
        }
        self.dirty_values.fill(false);
        self.dirty_projections.fill(false);
        self.dirty_handles.clear();
        self.dirty_bindings.clear();
    }
    fn refresh_changes(&mut self) {
        for i in 0..self.dirty_handles.len() {
            let index = self.dirty_handles[i].0 as usize;
            self.refresh_number_at(index);
            for &binding in &self.dependents[index] {
                if !self.dirty_projections[binding as usize] {
                    self.dirty_projections[binding as usize] = true;
                    self.dirty_bindings.push(binding);
                }
            }
            self.dirty_values[index] = false;
        }
        for i in 0..self.dirty_bindings.len() {
            let index = self.dirty_bindings[i] as usize;
            self.refresh_projection_at(index);
            self.dirty_projections[index] = false;
        }
        self.dirty_handles.clear();
        self.dirty_bindings.clear();
    }
    fn resolve_defaults(&mut self) {
        for row in 0..DEFINITIONS.len() {
            for source in Source::ALL {
                self.defaults[row][source as usize] = self.resolve_default(row, source);
            }
        }
        // Native composites supply operands without their own native declaration.
        for definition in DEFINITIONS {
            if definition.stored {
                continue;
            }
            for source in Source::ALL {
                let context = Context {
                    source,
                    role: Role::Game,
                    ..self.context
                };
                for clause in &DEFAULTS[definition.defaults[source as usize].defaults.clone()] {
                    if clause.issues != 0
                        || clause.kind != DefaultKind::Native
                        || !condition(clause.condition, context)
                    {
                        continue;
                    }
                    let Some(binding) = self.index.lookup(clause.member, context.side) else {
                        continue;
                    };
                    let b = &BINDINGS[binding as usize];
                    let c = &CONVERSIONS[b.conversions[source as usize] as usize];
                    if c.kind != ConversionKind::Composite {
                        continue;
                    }
                    let operands = |r| self.effective(self.slot(r as usize, 0), source);
                    let out = conversion::write(Input {
                        context,
                        conversion: c,
                        binding: b,
                        value: clause.value,
                        current: clause.value,
                        detail: None,
                        operand: &operands,
                    });
                    if let Ok(out) = out {
                        let mut pending: [Option<(usize, String)>; 32] =
                            std::array::from_fn(|_| None);
                        for (i, change) in out
                            .changes
                            .into_iter()
                            .take(out.change_count)
                            .flatten()
                            .enumerate()
                        {
                            let own = &DEFINITIONS[change.row as usize].defaults[source as usize];
                            if !DEFAULTS[own.defaults.clone()]
                                .iter()
                                .any(|d| d.kind == DefaultKind::Native)
                            {
                                pending[i] =
                                    Some((change.row as usize, change.text.as_str().to_owned()));
                            }
                        }
                        for (r, text) in pending.into_iter().flatten() {
                            self.defaults[r][source as usize] = Some(Cow::Owned(text));
                        }
                    }
                }
            }
        }
    }
    fn resolve_default(&self, row: usize, source: Source) -> Option<Cow<'static, str>> {
        let context = Context {
            source,
            role: stock_role(row, source),
            ..self.context
        };
        let mut selected: Option<Cow<'static, str>> = None;
        for clause in &DEFAULTS[DEFINITIONS[row].defaults[source as usize].defaults.clone()] {
            if clause.issues != 0 || !condition(clause.condition, context) {
                continue;
            }
            let member = if clause.member.is_empty() {
                DEFINITIONS[row].name
            } else {
                clause.member
            };
            let binding = self.index.lookup(member, context.side);
            let Some(b) = binding.map(|i| &BINDINGS[i as usize]) else {
                if DEFINITIONS[row].family_count > 1 {
                    selected = Some(Cow::Borrowed(clause.value));
                }
                continue;
            };
            let c = &CONVERSIONS[b.conversions[source as usize] as usize];
            let joined = matches!(
                c.operation,
                Operation::Deathmatch | Operation::Coop | Operation::Teamplay
            );
            let current = if joined {
                selected.as_deref().unwrap_or(clause.value)
            } else {
                clause.value
            };
            let operands = |r| self.effective(self.slot(r as usize, 0), source);
            let Ok(out) = conversion::write(Input {
                context,
                conversion: c,
                binding: b,
                value: clause.value,
                current,
                detail: None,
                operand: &operands,
            }) else {
                continue;
            };
            if selected.as_deref().is_some_and(|s| s != out.text.as_str()) && !joined {
                return None;
            }
            selected = Some(Cow::Owned(out.into_owned()));
        }
        selected
    }
}
impl Default for Cvars {
    fn default() -> Self {
        Self::new()
    }
}
fn role_index(role: Role) -> usize {
    match role {
        Role::Engine => 0,
        Role::Game => 1,
        Role::Cgame => 2,
    }
}
fn condition(condition: Condition, context: Context) -> bool {
    match condition {
        Condition::Always => true,
        Condition::Engine => context.role == Role::Engine,
        Condition::Game => context.role == Role::Game,
        Condition::Cgame => context.role == Role::Cgame,
        Condition::Client => context.side != Scope::Server && !context.dedicated,
        Condition::Dedicated => context.dedicated,
        Condition::Linux => cfg!(target_os = "linux"),
        Condition::NotLinux => !cfg!(target_os = "linux"),
        Condition::Mac => cfg!(target_os = "macos"),
        Condition::NotMac => !cfg!(target_os = "macos"),
        Condition::Windows => cfg!(target_os = "windows"),
        Condition::NotWindows => !cfg!(target_os = "windows"),
        Condition::Unresolved => false,
    }
}
fn stock_role(row: usize, source: Source) -> Role {
    let mut role = Role::Game;
    for clause in &DEFAULTS[DEFINITIONS[row].defaults[source as usize].defaults.clone()] {
        if clause.issues != 0 {
            continue;
        }
        if clause.condition == Condition::Engine {
            role = Role::Engine;
        }
        if matches!(clause.condition, Condition::Cgame | Condition::Always)
            && let Some(b) = BINDINGS
                .iter()
                .find(|b| b.name.eq_ignore_ascii_case(clause.member) && b.scope != Scope::Server)
        {
            let c = &CONVERSIONS[b.conversions[source as usize] as usize];
            if c.cgame_only && c.kind == ConversionKind::Reciprocal {
                return Role::Cgame;
            }
        }
    }
    role
}
