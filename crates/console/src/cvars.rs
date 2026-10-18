use crate::text::MAX_TEXT;
use crate::{
    catalog::{Condition, ConversionKind, DefaultKind, Operation, Scope},
    conversion::{self, Input, Text},
    cvars_generated::{BINDINGS, CONVERSIONS, DEFAULTS, DEFINITIONS, FLAGS, OPERANDS},
    numbers::{integer, number},
    views::{Context, Role, RuleSetId},
};
use qa_core::names::{NameTable, NamesError};
use qa_core::primitives::{CvarHandle, IndexSpan, NameId};
use qa_core::stamps::StampSet;
use qa_core::text::FixedText;
use std::{borrow::Cow, fmt::Write};

pub use crate::catalog::Definition;
mod clients;
use clients::{ClientColumn, native_context};
const EMPTY: u16 = u16::MAX;
const MODULE_CVARS: usize = 1024;
pub(crate) const USER_CREATED: u32 = 128;
const ROLES: [Role; 3] = [Role::Engine, Role::Game, Role::Cgame];
const INFO_FLAGS: [u32; 3] = [2, 4, 8];

/// Q3 Cvar_Get/Cvar_Set validate names before lookup; other native sources
/// keep their byte-exact spellings in the same table.
pub fn registration_name(name: &str, source: RuleSetId) -> &str {
    if source == RuleSetId::Quake3 && name.bytes().any(qa_core::text::info_delimiter) {
        "BADNAME"
    } else {
        name
    }
}

#[derive(Clone, Copy)]
struct NameSlot {
    first: u16,
    second: u16,
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
    /// Cached spelling/conversion identity within the one registry.
    pub fn binding_id(self) -> u16 {
        self.binding
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    Capacity,
    Conversion(conversion::Error),
    ReadOnly,
    InitOnly,
    Cheats,
    InvalidInfo,
    MissingDefault,
}

/// Live host facts, independent of the native flags selected by each view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimePolicy {
    pub initialized: bool,
    pub server_active: bool,
}
impl From<conversion::Error> for WriteError {
    fn from(e: conversion::Error) -> Self {
        Self::Conversion(e)
    }
}
#[derive(Clone)]
struct Value {
    module_defaults: [Option<NameId>; 5],
    row: usize,
    seat: u8,
    explicit: bool,
    numbers: [f32; 5],
    default_numbers: [f32; 5],
    integers: [i32; 5],
    name_id: NameId,
    revision: u64,
    pending_revision: u64,
    command_flags: [Option<u32>; 5],
    /// A loaded capability can hold LATCH writes independently of a server.
    latch_active: bool,
}
struct NativeView {
    view: View,
    next: Option<u32>,
    current: Result<FixedText<MAX_TEXT>, conversion::Error>,
    pending: Result<Option<FixedText<MAX_TEXT>>, conversion::Error>,
    modifications: u32,
}
fn native_text(value: Text<'_>) -> Result<FixedText<MAX_TEXT>, conversion::Error> {
    let mut text = FixedText::default();
    text.set(value.as_str())
        .map_err(|_| conversion::Error::TextTooLong)?;
    Ok(text)
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
    texts: [Option<(CvarHandle, FixedText<MAX_TEXT>)>; 2],
    changes: [Option<(CvarHandle, FixedText<64>)>; 33],
    details: [Option<PendingDetail>; 2],
}
impl PendingWrite {
    fn values(&self) -> impl Iterator<Item = (CvarHandle, &str)> {
        self.texts
            .iter()
            .flatten()
            .map(|(handle, text)| (*handle, text.as_str()))
            .chain(
                self.changes
                    .iter()
                    .flatten()
                    .map(|(handle, text)| (*handle, text.as_str())),
            )
    }
    fn value(&mut self, handle: CvarHandle, value: &str) -> Result<(), WriteError> {
        if let Some((_, text)) = self
            .texts
            .iter_mut()
            .flatten()
            .find(|(old, _)| *old == handle)
        {
            text.set(value)
                .map_err(|_| conversion::Error::TextTooLong)?;
            return Ok(());
        }
        if value.len() > 64 {
            let slot = self
                .texts
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(WriteError::Capacity)?;
            let mut text = FixedText::default();
            text.set(value)
                .map_err(|_| conversion::Error::TextTooLong)?;
            *slot = Some((handle, text));
            for slot in &mut self.changes {
                if slot.as_ref().is_some_and(|(old, _)| *old == handle) {
                    *slot = None;
                }
            }
            return Ok(());
        }
        let index = self
            .changes
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|(old, _)| *old == handle))
            .or_else(|| self.changes.iter().position(Option::is_none))
            .ok_or(WriteError::Capacity)?;
        let mut text = FixedText::default();
        text.set(value)
            .map_err(|_| conversion::Error::TextTooLong)?;
        self.changes[index] = Some((handle, text));
        Ok(())
    }
    fn detail(&mut self, view: View, text: &str, modified: bool) -> Result<(), WriteError> {
        let index = self
            .details
            .iter()
            .position(|slot| {
                slot.as_ref().is_some_and(|old| {
                    old.view.binding == view.binding && old.view.handle == view.handle
                })
            })
            .or_else(|| self.details.iter().position(Option::is_none))
            .ok_or(WriteError::Capacity)?;
        let detail = self.details[index].get_or_insert_with(|| PendingDetail {
            view,
            text: FixedText::default(),
            modified,
        });
        detail.view = view;
        detail
            .text
            .set(text)
            .map_err(|_| conversion::Error::TextTooLong)?;
        detail.modified = modified;
        Ok(())
    }
    fn append(&mut self, other: &Self) -> Result<(), WriteError> {
        for (handle, value) in other.values() {
            self.value(handle, value)?;
        }
        for detail in other.details.iter().flatten() {
            self.detail(detail.view, detail.text.as_str(), detail.modified)?;
        }
        Ok(())
    }
    fn handles(&self) -> impl Iterator<Item = CvarHandle> + '_ {
        self.values().map(|(handle, _)| handle).chain(
            self.details
                .iter()
                .flatten()
                .map(|detail| detail.view.handle),
        )
    }
}
type Projections = [[Result<(f32, i32), conversion::Error>; 3]; 5];

pub struct Cvars {
    catalog_values: usize,
    registered_values: usize,
    stride: usize,
    clients: Vec<ClientColumn>,
    shared_values: StampSet,
    values: Vec<Value>,
    texts: Vec<qa_core::text::TextSlots>,
    offsets: Vec<usize>,
    defaults: Vec<[Option<Cow<'static, str>>; 5]>,
    pub(crate) names: NameTable,
    name_bindings: Box<[NameSlot]>,
    binding_names: Box<[NameId]>,
    autoswitch_text_name: NameId,
    flag_names: Box<[Option<NameId>]>,
    stock_roles: Box<[[Role; 5]]>,
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    lookups: std::cell::Cell<u64>,
    details: Vec<[Option<Box<Detail>>; 5]>,
    pending: Vec<PendingWrite>,
    projections: Vec<Projections>,
    projection_rows: Box<[IndexSpan]>,
    dependents: Vec<Vec<u16>>,
    dirty_handles: Vec<CvarHandle>,
    dirty_values: StampSet,
    dirty_bindings: Vec<usize>,
    dirty_projections: StampSet,
    context: Context,
    revision_clock: u64,
    info_revisions: Vec<[[u64; INFO_FLAGS.len()]; 5]>,
    native_views: Vec<NativeView>,
    native_heads: Box<[Option<u32>]>,
    runtime_policy: RuntimePolicy,
    cheats_handle: CvarHandle,
    download_handle: CvarHandle,
    video_policy: Option<crate::video::VideoPolicy>,
}

impl Cvars {
    pub fn new() -> Result<Self, NamesError> {
        Self::with_context(Context::default())
    }
    pub fn with_context(context: Context) -> Result<Self, NamesError> {
        let context = cvar_context(context);
        let names = NameTable::load_reserved(
            BINDINGS.iter().map(|b| b.name.as_bytes()),
            32768,
            1024 * 1024,
        )?;
        let mut name_bindings = vec![
            NameSlot {
                first: EMPTY,
                second: EMPTY
            };
            names.capacity()
        ]
        .into_boxed_slice();
        for (binding, b) in BINDINGS.iter().enumerate() {
            let id = names
                .find_folded(b.name.as_bytes())
                .ok_or(NamesError::Capacity)?;
            let slot = &mut name_bindings[id.0 as usize];
            if slot.first == EMPTY {
                slot.first = binding as u16;
            } else {
                slot.second = binding as u16;
            }
        }
        let mut binding_names: Vec<NameId> = BINDINGS
            .iter()
            .map(|b| {
                names
                    .find_folded(b.name.as_bytes())
                    .ok_or(NamesError::Capacity)
            })
            .collect::<Result<Vec<_>, _>>()?;
        binding_names.resize(BINDINGS.len() + MODULE_CVARS, NameId(0));
        let autoswitch_text_name = names
            .find_folded(b"qts_weapon_autoswitch")
            .ok_or(NamesError::Capacity)?;
        let flag_names = FLAGS
            .iter()
            .map(|c| names.find_folded(c.member.as_bytes()))
            .collect();
        let stock_roles = (0..DEFINITIONS.len())
            .map(|row| {
                RuleSetId::ALL
                    .map(|source| stock_role(&names, &binding_names[..BINDINGS.len()], row, source))
            })
            .collect();
        let mut values = Vec::with_capacity(
            DEFINITIONS
                .iter()
                .map(|d| d.family_count as usize)
                .sum::<usize>()
                + MODULE_CVARS,
        );
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
                    name_id: names.find(name.as_bytes()).ok_or(NamesError::Capacity)?,
                    module_defaults: [None; 5],
                    row,
                    seat,
                    explicit: false,
                    numbers: [0.0; 5],
                    default_numbers: [0.0; 5],
                    integers: [0; 5],
                    revision: 0,
                    pending_revision: 0,
                    command_flags: [None; 5],
                    latch_active: false,
                });
            }
        }
        let catalog_values = values.len();
        let pending_capacity = catalog_values + MODULE_CVARS;
        let texts = qa_core::text::TextSlots::load(
            (0..pending_capacity).map(|_| MAX_TEXT),
            qa_core::text::SlotsLimits {
                rows: pending_capacity,
                width: MAX_TEXT,
                bytes: 512 * 1024 * 1024,
            },
        )
        .map_err(|_| NamesError::Capacity)?;
        let mut projection_count = 0;
        let projection_rows: Vec<_> = BINDINGS
            .iter()
            .map(|binding| {
                let family = DEFINITIONS[binding.row as usize].family_count;
                let count = if binding.seat == 0 {
                    u32::from(family).min(qa_core::sys_events::SeatId::COUNT as u32)
                } else {
                    1
                };
                let span = IndexSpan {
                    first: projection_count,
                    count,
                };
                projection_count += count;
                span
            })
            .collect();
        let mut registry = Self {
            catalog_values,
            registered_values: catalog_values,
            stride: pending_capacity,
            clients: Vec::new(),
            shared_values: StampSet::new(pending_capacity),
            values,
            texts: vec![texts],
            offsets,
            names,
            name_bindings,
            binding_names: binding_names.into_boxed_slice(),
            autoswitch_text_name,
            flag_names,
            stock_roles,
            #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
            lookups: std::cell::Cell::new(0),
            context,
            revision_clock: 0,
            info_revisions: vec![[[0; INFO_FLAGS.len()]; 5]],
            native_views: Vec::with_capacity(BINDINGS.len() + MODULE_CVARS * ROLES.len()),
            native_heads: vec![None; BINDINGS.len() + MODULE_CVARS].into_boxed_slice(),
            defaults: (0..DEFINITIONS.len())
                .map(|_| std::array::from_fn(|_| None))
                .collect(),
            details: BINDINGS
                .iter()
                .enumerate()
                .flat_map(|(index, b)| (0..projection_rows[index].count).map(move |_| b))
                .map(|b| {
                    std::array::from_fn(|source| {
                        let row = &DEFINITIONS[b.row as usize];
                        let conversion = &CONVERSIONS[if row.stored {
                            b.conversions[source]
                        } else {
                            row.rule_conversion
                        } as usize];
                        (conversion.detail
                            || matches!(
                                conversion.operation,
                                Operation::MusicMute
                                    | Operation::VideoMode
                                    | Operation::Fullscreen
                                    | Operation::WindowGeometry
                            )
                            || row.name == "r_mode")
                            .then(Box::default)
                    })
                })
                .collect(),
            pending: Vec::with_capacity(pending_capacity),
            projections: vec![[[Ok((0.0, 0)); 3]; 5]; projection_count as usize],
            projection_rows: projection_rows.into_boxed_slice(),
            dependents: (0..pending_capacity).map(|_| Vec::new()).collect(),
            dirty_handles: Vec::with_capacity(pending_capacity),
            dirty_values: StampSet::new(pending_capacity),
            dirty_bindings: Vec::with_capacity(BINDINGS.len()),
            dirty_projections: StampSet::new(BINDINGS.len()),
            runtime_policy: RuntimePolicy::default(),
            cheats_handle: CvarHandle(0),
            download_handle: CvarHandle(0),
            video_policy: None,
        };
        for (i, binding) in BINDINGS.iter().enumerate() {
            let handle = registry.slot(binding.row as usize, binding.seat);
            for offset in 0..registry.projection_rows[i].count {
                registry.dependents[handle.0 as usize + offset as usize].push(i as u16);
            }
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
        let context = Context {
            side: Scope::Client,
            client_lifetime: None,
            cvar_client: None,
            ..context
        };
        let [cheats, download] =
            ["sv_cheats", "cl_allowDownload"].map(|name| registry.bind(name, context));
        registry.cheats_handle = cheats.ok_or(NamesError::Capacity)?.canonical();
        registry.download_handle = download.ok_or(NamesError::Capacity)?.canonical();
        registry.pad_values();
        registry.resolve_defaults();
        registry.refresh_numbers();
        registry.refresh_projections();
        Ok(registry)
    }
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn runtime_policy(&self) -> RuntimePolicy {
        self.runtime_policy
    }
    pub fn set_runtime_policy(&mut self, policy: RuntimePolicy) {
        self.runtime_policy = policy;
    }
    pub fn select_context(&mut self, context: Context) {
        let context = cvar_context(context);
        if context == self.context {
            return;
        }
        let previous: Vec<_> = self
            .values
            .iter()
            .enumerate()
            .filter(|(index, _)| self.defined(*index))
            .map(|(index, value)| {
                (
                    self.effective(CvarHandle(index as u32), self.context.source)
                        .to_owned(),
                    value.numbers[self.context.source as usize].to_bits(),
                    value.integers[self.context.source as usize],
                    index,
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
        for (text, bits, integer, index) in previous {
            let handle = CvarHandle(index as u32);
            if text != self.text(handle)
                || bits != self.value(handle).to_bits()
                || integer != self.integer(handle)
            {
                self.mark_change(handle);
            }
        }
        self.refresh_projections();
        // Changing the registry's default policy may change info projections
        // in sources other than the newly selected console context.
        self.revision_clock = self.revision_clock.wrapping_add(1);
        self.info_revisions
            .fill([[self.revision_clock; INFO_FLAGS.len()]; 5]);
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
    fn binding_slot(
        &self,
        binding: &crate::catalog::Binding,
        context: Context,
    ) -> Option<CvarHandle> {
        let family = DEFINITIONS[binding.row as usize].family_count;
        let seat = if binding.seat == 0 && family > 1 {
            context.seat.index() as u8 + 1
        } else {
            binding.seat
        };
        (seat <= family).then(|| self.slot(binding.row as usize, seat))
    }
    fn projection_index(&self, view: View) -> Option<usize> {
        let span = *self.projection_rows.get(view.binding as usize)?;
        let offset = if span.count > 1 {
            self.values[view.handle.0 as usize].seat.saturating_sub(1) as usize
        } else {
            0
        };
        Some(self.column(view.handle) * self.projection_count() + span.first as usize + offset)
    }
    fn projected_number(&self, view: View) -> Result<(f32, i32), conversion::Error> {
        let index = self
            .projection_index(view)
            .ok_or(conversion::Error::PolicyRequired)?;
        self.projections[index][view.context.source as usize][role_index(view.context.role)]
    }
    pub(crate) fn lookup_name(&self, name: &str) -> Option<NameId> {
        #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
        self.lookups.set(self.lookups.get().wrapping_add(1));
        self.names.find_folded(name.as_bytes())
    }
    fn binding_id(&self, id: NameId, side: Scope) -> Option<u16> {
        let slot = *self.name_bindings.get(id.0 as usize)?;
        let mut fallback = None;
        for binding in [slot.first, slot.second] {
            if binding == EMPTY {
                continue;
            }
            let scope = BINDINGS
                .get(binding as usize)
                .map_or(Scope::Any, |b| b.scope);
            if scope == Scope::Any {
                fallback = Some(binding);
            } else if scope == side || (scope == Scope::Client && side == Scope::Any) {
                return Some(binding);
            }
        }
        fallback
    }
    fn lookup_binding(&self, name: &str, side: Scope) -> Option<u16> {
        self.binding_id(self.lookup_name(name)?, side)
    }
    pub(crate) fn bind_id(&self, id: NameId, context: Context) -> Option<View> {
        let binding = self.binding_id(self.names.folded(id)?, context.side)?;
        let handle = BINDINGS.get(binding as usize).map_or_else(
            || {
                Some(CvarHandle(
                    (self.catalog_values + binding as usize - BINDINGS.len()) as u32,
                ))
            },
            |b| self.binding_slot(b, context),
        )?;
        let handle = self.context_handle(handle, context)?;
        Some(View {
            handle,
            binding,
            context,
        })
    }
    pub fn bind(&self, name: &str, context: Context) -> Option<View> {
        self.bind_id(self.lookup_name(name)?, context)
    }

    /// Console and saved-profile writes share native user-created ownership
    /// and the same capacity reserved for module registrations at load.
    pub fn set_user(
        &mut self,
        name: &str,
        text: &str,
        context: Context,
        full_flags: Option<u32>,
        added_flags: u32,
    ) -> Result<View, WriteError> {
        let name = registration_name(name, context.source);
        let view = match self.bind(name, context) {
            Some(view) => view,
            None => self.register(
                name,
                Some(text),
                full_flags.unwrap_or(if context.source == RuleSetId::Quake3 {
                    USER_CREATED
                } else {
                    0
                }),
                context,
            )?,
        };
        let written = if let Some(flags) = full_flags {
            self.full_set(view, text, flags)
        } else {
            self.write(view, text)
        };
        if added_flags != 0 {
            // Q3 adds these flags after an ordinary write even when policy
            // rejects it. Register retains an existing catalog alias View.
            self.register(name, None, added_flags, context)?;
        }
        written?;
        Ok(view)
    }

    /// Module names enter the existing table using capacity reserved at load.
    /// Catalog defaults and previously written values survive registration.
    /// Flags use the registry's common meanings; ABIs convert native bits.
    pub fn register(
        &mut self,
        name: &str,
        default: Option<&str>,
        flags: u32,
        context: Context,
    ) -> Result<View, WriteError> {
        self.context_column(context).ok_or(WriteError::Capacity)?;
        let name = registration_name(name, context.source);
        if name.is_empty()
            || (flags & 6 != 0
                && (!info_text(name) || default.is_some_and(|text| !info_text(text))))
        {
            return Err(WriteError::InvalidInfo);
        }
        if default.is_some_and(|text| text.len() > MAX_TEXT) {
            return Err(conversion::Error::TextTooLong.into());
        }
        if let Some(view) = self.bind(name, context) {
            let previous_flags = self.flags(view);
            let adopt_user_default = context.source == RuleSetId::Quake3
                && view.binding as usize >= BINDINGS.len()
                && previous_flags & USER_CREATED != 0
                && flags & USER_CREATED == 0
                && default.is_some_and(|value| !value.is_empty());
            let user_value = if adopt_user_default && !self.values[view.handle.0 as usize].explicit
            {
                Some(native_text(self.read(view)?)?)
            } else {
                None
            };
            // The catalog reserves names even when a source's audited default
            // is unresolved. Native Cvar_Get supplies that missing default;
            // an explicit user value and other sources keep their own values.
            if let Some(default) = default
                && (adopt_user_default || !self.default_available(view.handle, context.source))
            {
                let mut changes = PendingWrite {
                    texts: [None, None],
                    changes: std::array::from_fn(|_| None),
                    details: [None, None],
                };
                self.prepare_write(view, default, false, &mut changes)?;
                for (handle, text) in changes.values() {
                    if (adopt_user_default && handle == view.handle)
                        || !self.default_available(handle, context.source)
                    {
                        let id = self
                            .names
                            .intern(text.as_bytes())
                            .map_err(|_| WriteError::Capacity)?;
                        self.values[handle.0 as usize].module_defaults[context.source as usize] =
                            Some(id);
                        self.mark_change(handle);
                    }
                }
            }
            if let Some(user_value) = user_value {
                self.publish(view.handle, user_value.as_str())?;
            }
            // Q3's first non-user registration takes ownership of the reset
            // default while retaining the console value and existing flags.
            let merged = (if adopt_user_default {
                previous_flags & !USER_CREATED
            } else {
                previous_flags
            }) | flags;
            if merged != previous_flags {
                self.values[view.handle.0 as usize].command_flags[context.source as usize] =
                    Some(merged);
                self.mark_change(view.handle);
            }
            self.refresh_changes();
            return Ok(view);
        }
        let default = default.ok_or(WriteError::MissingDefault)?;
        let module_index = self.registered_values - self.catalog_values;
        if module_index >= MODULE_CVARS {
            return Err(WriteError::Capacity);
        }
        let binding =
            u16::try_from(BINDINGS.len() + module_index).map_err(|_| WriteError::Capacity)?;
        let name_id = self
            .names
            .intern(name.as_bytes())
            .map_err(|_| WriteError::Capacity)?;
        let default_id = self
            .names
            .intern(default.as_bytes())
            .map_err(|_| WriteError::Capacity)?;
        let folded = self.names.folded(name_id).ok_or(WriteError::Capacity)?;
        let handle = CvarHandle(self.registered_values as u32);
        self.values[handle.0 as usize] = Value {
            module_defaults: [Some(default_id); 5],
            row: DEFINITIONS.len(),
            seat: 0,
            explicit: false,
            name_id,
            numbers: [0.0; 5],
            default_numbers: [0.0; 5],
            integers: [0; 5],
            revision: 0,
            pending_revision: 0,
            command_flags: [Some(flags); 5],
            latch_active: false,
        };
        self.registered_values += 1;
        self.add_client_definition(handle);
        self.name_bindings[folded.0 as usize].first = binding;
        self.binding_names[binding as usize] = folded;
        let handle = self
            .context_handle(handle, context)
            .ok_or(WriteError::Capacity)?;
        if self.column(handle) != 0 {
            self.values[handle.0 as usize].module_defaults = [Some(default_id); 5];
            self.values[handle.0 as usize].command_flags = [Some(flags); 5];
            let root = self.root(handle).0 as usize;
            self.values[root].module_defaults = [None; 5];
            self.values[root].command_flags = [None; 5];
        }
        self.refresh_number_at(handle.0 as usize);
        self.mark_change(handle);
        self.refresh_changes();
        Ok(View {
            handle,
            binding,
            context,
        })
    }

    fn name_text(&self, id: NameId) -> &str {
        std::str::from_utf8(self.names.get(id).unwrap_or_default()).unwrap_or_default()
    }

    /// Canonical engine handle; converted boundaries retain a View instead.
    pub fn find(&self, name: &str) -> Option<CvarHandle> {
        self.bind(name, self.context).map(View::canonical)
    }
    fn effective(&self, handle: CvarHandle, source: RuleSetId) -> &str {
        let value = &self.values[handle.0 as usize];
        if value.explicit {
            self.texts[self.column(handle)]
                .text(self.root(handle).0 as usize)
                .unwrap_or_default()
        } else if self.column(handle) != 0 && self.values[self.root(handle).0 as usize].explicit {
            self.effective(self.root(handle), source)
        } else {
            self.default_text(handle, source)
        }
    }
    fn default_text(&self, handle: CvarHandle, source: RuleSetId) -> &str {
        let value = &self.values[handle.0 as usize];
        if let Some(default) = value.module_defaults[source as usize] {
            self.name_text(default)
        } else if self.column(handle) != 0 {
            self.default_text(self.root(handle), source)
        } else {
            self.defaults
                .get(value.row)
                .and_then(|row| row[source as usize].as_deref())
                .unwrap_or("")
        }
    }
    pub fn value(&self, handle: CvarHandle) -> f32 {
        self.values[handle.0 as usize].numbers[self.context.source as usize]
    }
    /// Per-client cached views keep movement/input defaults independent of the
    /// active map's console context; frame reads never parse text or bind names.
    pub fn value_in(&self, handle: CvarHandle, source: RuleSetId) -> f32 {
        self.values[handle.0 as usize].numbers[source as usize]
    }
    /// Finite effective value, falling back to the same catalog's cached default.
    /// This does not change native registration/admission for other consumers.
    pub fn finite_value_in(&self, handle: CvarHandle, source: RuleSetId) -> f32 {
        let value = &self.values[handle.0 as usize];
        let number = value.numbers[source as usize];
        if number.is_finite() {
            number
        } else {
            value.default_numbers[source as usize]
        }
    }
    pub fn integer(&self, handle: CvarHandle) -> i32 {
        self.values[handle.0 as usize].integers[self.context.source as usize]
    }
    pub fn integer_in(&self, handle: CvarHandle, source: RuleSetId) -> i32 {
        self.values[handle.0 as usize].integers[source as usize]
    }
    pub fn generation(&self, handle: CvarHandle) -> u64 {
        let revision = self.values[handle.0 as usize].revision;
        if self.column(handle) == 0 {
            revision
        } else {
            revision.max(self.values[self.root(handle).0 as usize].revision)
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision_clock
    }
    /// Applied native USERINFO (2), SERVERINFO (4) or SYSTEMINFO (8) changes.
    /// A source stamp covers all seats and roles; unrelated cvars and pending
    /// latches do not invalidate info strings. This is registry state, not a
    /// fingerprint of their contents.
    pub fn info_generation(&self, source: RuleSetId, flags: u32) -> u64 {
        self.info_generation_at(0, source, flags)
    }
    pub fn info_generation_in(&self, context: Context, flags: u32) -> u64 {
        let global = self.info_generation(context.source, flags);
        self.context_column(context).map_or(global, |column| {
            global.max(self.info_generation_at(column, context.source, flags))
        })
    }
    fn info_generation_at(&self, column: usize, source: RuleSetId, flags: u32) -> u64 {
        INFO_FLAGS
            .into_iter()
            .zip(self.info_revisions[column][source as usize])
            .filter_map(|(flag, generation)| (flags & flag != 0).then_some(generation))
            .max()
            .unwrap_or(0)
    }
    fn mark_info(&mut self, column: usize, source: RuleSetId, flags: u32, generation: u64) {
        for (flag, revision) in INFO_FLAGS
            .into_iter()
            .zip(&mut self.info_revisions[column][source as usize])
        {
            if flags & flag != 0 {
                *revision = (*revision).max(generation);
            }
        }
    }
    /// Native handles refer to cached views of this registry, never another
    /// name/value table. Registration uses capacity reserved at registry load.
    pub fn native_view(&mut self, mut view: View) -> Result<u32, WriteError> {
        self.validate_view(view)?;
        view.context = cvar_context(view.context);
        view.context.event_time = None;
        view.context.key_source = None;
        view.context.producer = None;
        let binding = view.binding as usize;
        let mut cursor = self.native_heads[binding];
        while let Some(index) = cursor {
            let row = &self.native_views[index as usize];
            if row.view.handle == view.handle
                && native_context(row.view.context) == native_context(view.context)
            {
                self.native_views[index as usize].view = view;
                self.refresh_native(binding);
                return Ok(index);
            }
            cursor = row.next;
        }
        if self.native_views.len() == self.native_views.capacity() {
            return Err(WriteError::Capacity);
        }
        let current = self.native_current(view);
        let pending = self.native_pending(view);
        let index = self.native_views.len() as u32;
        self.native_views.push(NativeView {
            view,
            next: self.native_heads[binding],
            current,
            pending,
            modifications: 1,
        });
        self.native_heads[binding] = Some(index);
        Ok(index)
    }
    pub fn native_snapshot(&self, handle: u32) -> Option<(View, i32)> {
        let row = self.native_views.get(handle as usize)?;
        Some((row.view, row.modifications as i32))
    }
    fn native_current(&self, view: View) -> Result<FixedText<MAX_TEXT>, conversion::Error> {
        self.read(view).and_then(native_text)
    }
    fn native_pending(&self, view: View) -> Result<Option<FixedText<MAX_TEXT>>, conversion::Error> {
        self.latched(view)?.map(native_text).transpose()
    }
    fn refresh_native(&mut self, binding: usize) {
        let mut cursor = self.native_heads[binding];
        while let Some(index) = cursor {
            let view = self.native_views[index as usize].view;
            let current = self.native_current(view);
            let pending = self.native_pending(view);
            let row = &mut self.native_views[index as usize];
            let current_changed = current.as_ref().map(FixedText::as_str)
                != row.current.as_ref().map(FixedText::as_str);
            let pending_changed = pending
                .as_ref()
                .map(|value| value.as_ref().map(FixedText::as_str))
                != row
                    .pending
                    .as_ref()
                    .map(|value| value.as_ref().map(FixedText::as_str));
            // Q3 counts a new latch, and its later application, separately.
            // Merely cancelling a latch does not modify the current value.
            if current_changed || (pending_changed && pending.as_ref().is_ok_and(Option::is_some)) {
                row.modifications = row.modifications.wrapping_add(1);
            }
            row.current = current;
            row.pending = pending;
            cursor = row.next;
        }
    }
    pub fn name(&self, view: View) -> &str {
        BINDINGS.get(view.binding as usize).map_or_else(
            || self.name_text(self.values[view.handle.0 as usize].name_id),
            |binding| binding.name,
        )
    }
    fn dependency_rows(&self, view: View) -> impl Iterator<Item = u16> + '_ {
        let conversion = self.conversion(view);
        OPERANDS[conversion.operands.clone()]
            .iter()
            .map(|operand| operand.row)
            .chain(
                self.video_policy
                    .as_ref()
                    .into_iter()
                    .flat_map(move |policy| {
                        policy.dependencies(conversion.operation).iter().copied()
                    }),
            )
    }

    pub fn set_video_policy(
        &mut self,
        capabilities: std::sync::Arc<qa_core::presentation::VideoCapabilities>,
        windowed: qa_core::presentation::DisplayMode,
    ) -> Result<(), conversion::Error> {
        let policy = crate::video::VideoPolicy::load(capabilities, windowed)?;
        for (index, binding) in BINDINGS.iter().enumerate() {
            for &conversion in &binding.conversions {
                for &row in policy.dependencies(CONVERSIONS[conversion as usize].operation) {
                    let handle = self.slot(row as usize, 0);
                    let dependents = &mut self.dependents[handle.0 as usize];
                    if !dependents.contains(&(index as u16)) {
                        dependents.push(index as u16);
                    }
                }
            }
        }
        for &row in policy
            .rows
            .iter()
            .chain(&policy.position)
            .chain([&policy.geometry, &policy.aspect])
        {
            self.shared_values
                .mark(self.slot(row as usize, 0).0 as usize);
        }
        self.video_policy = Some(policy);
        self.resolve_defaults();
        self.refresh_numbers();
        self.refresh_projections();
        Ok(())
    }
    pub fn window_views(&self, gl: bool) -> Result<Vec<View>, conversion::Error> {
        self.video_policy
            .as_ref()
            .ok_or(conversion::Error::VideoPolicyRequired)?;
        let custom = self.custom_aspect(true);
        crate::video::WINDOW_CVARS
            .iter()
            .filter(|&&name| gl || name != crate::video::WINDOW_CVARS[5])
            .chain(crate::video::GL_CVARS.iter().filter(|_| gl))
            .chain(std::iter::once(&"r_customaspect").filter(|_| custom))
            .map(|name| {
                self.bind(
                    name,
                    Context {
                        role: Role::Engine,
                        ..self.context
                    },
                )
                .ok_or(conversion::Error::VideoPolicyRequired)
            })
            .collect()
    }
    pub fn window_request(
        &self,
        pending: bool,
        gl: bool,
        baseline: Option<qa_core::presentation::WindowRequest>,
    ) -> Result<qa_core::presentation::WindowRequest, conversion::Error> {
        let policy = self
            .video_policy
            .as_ref()
            .ok_or(conversion::Error::VideoPolicyRequired)?;
        let source = self.context.source;
        let value = |row| {
            let handle = self.slot(row as usize, 0);
            if pending {
                self.pending_text(handle, source)
            } else {
                self.effective(handle, source)
            }
        };
        let detail_for = |name, source| {
            let view = self.bind(
                name,
                Context {
                    source,
                    role: Role::Engine,
                    ..self.context
                },
            )?;
            if pending {
                self.pending_detail(view)
            } else {
                self.detail(view)
            }
        };
        let fullscreen = detail_for("vid_fullscreen", RuleSetId::Quake2Rerelease);
        let mut geometry = self
            .retained_detail("vid_geometry", pending)
            .map(|(_, text)| text);
        let custom = self.custom_aspect(pending);
        let native_mode = [
            ("vid_mode", RuleSetId::Quake),
            ("vid_mode", RuleSetId::QuakeWorld),
            ("gl_mode", RuleSetId::Quake2),
        ]
        .into_iter()
        .find_map(|(name, source)| detail_for(name, source).map(|detail| (source, detail)));
        let mode = self.slot(policy.rows[0] as usize, 0);
        let mode_pending = pending
            && self
                .pending
                .iter()
                .any(|entry| entry.handles().any(|handle| handle == mode));
        if geometry.is_none()
            && source == RuleSetId::Quake2Rerelease
            && !self.is_explicit(mode)
            && !mode_pending
        {
            geometry = Some(value(policy.geometry));
        }
        let positioned = policy.position.iter().any(|&row| {
            let handle = self.slot(row as usize, 0);
            self.is_explicit(handle)
                || pending
                    && self
                        .pending
                        .iter()
                        .any(|entry| entry.handles().any(|h| h == handle))
        }) || policy
            .position
            .iter()
            .all(|&row| self.native_default_available(self.slot(row as usize, 0), source));
        let mut default = policy.windowed();
        if source == RuleSetId::Quake2Rerelease {
            default.width = 640;
            default.height = 480;
        }
        let default_window = !self.is_explicit(mode)
            && !mode_pending
            && (!self.native_default_available(mode, source)
                || matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld)
                    && integer(value(policy.rows[0])) == -1);
        let mut width = FixedText::<64>::default();
        let mut height = FixedText::<64>::default();
        if default_window {
            write!(width, "{}", default.width).map_err(|_| conversion::Error::TextTooLong)?;
            write!(height, "{}", default.height).map_err(|_| conversion::Error::TextTooLong)?;
        }
        policy.request(
            &|row| {
                if default_window {
                    if row == policy.rows[0] {
                        return "-1";
                    }
                    if row == policy.rows[1] {
                        return width.as_str();
                    }
                    if row == policy.rows[2] {
                        return height.as_str();
                    }
                }
                if row == policy.aspect {
                    let handle = self.slot(row as usize, 0);
                    return if pending {
                        self.pending_text(handle, RuleSetId::Quake3)
                    } else {
                        self.effective(handle, RuleSetId::Quake3)
                    };
                }
                value(row)
            },
            fullscreen,
            geometry,
            native_mode,
            custom,
            positioned,
            source,
            baseline,
            gl,
        )
    }
    fn custom_aspect(&self, pending: bool) -> bool {
        let Some(policy) = &self.video_policy else {
            return false;
        };
        let mode = self.slot(policy.rows[0] as usize, 0);
        let text = if pending {
            self.pending_text(mode, self.context.source)
        } else {
            self.effective(mode, self.context.source)
        };
        let aspect = self.slot(policy.aspect as usize, 0);
        integer(text) == -1
            && (self.retained_detail("r_mode", pending).is_some()
                || self.is_explicit(aspect)
                || pending
                    && self
                        .pending
                        .iter()
                        .any(|group| group.handles().any(|handle| handle == aspect)))
    }
    fn retained_detail(&self, name: &str, pending: bool) -> Option<(View, &str)> {
        RuleSetId::ALL.into_iter().find_map(|source| {
            let view = self.bind(
                name,
                Context {
                    source,
                    role: Role::Engine,
                    ..self.context
                },
            )?;
            let detail = if pending {
                self.pending_detail(view)
            } else {
                self.detail(view)
            }?;
            Some((view, detail))
        })
    }
    fn pending_text(&self, handle: CvarHandle, source: RuleSetId) -> &str {
        self.pending
            .iter()
            .flat_map(PendingWrite::values)
            .find_map(|(h, text)| (h == handle).then_some(text))
            .or_else(|| {
                self.inherits(handle)
                    .then(|| self.pending_text(self.root(handle), source))
            })
            .unwrap_or_else(|| self.effective(handle, source))
    }
    fn pending_detail(&self, view: View) -> Option<&str> {
        self.pending
            .iter()
            .flat_map(|pending| pending.details.iter().flatten())
            .find(|detail| {
                detail.view.binding == view.binding
                    && detail.view.handle == view.handle
                    && detail.view.context.source == view.context.source
            })
            .map(|detail| detail.text.as_str())
            .or_else(|| {
                let replaced = self.pending.iter().any(|pending| {
                    pending.handles().any(|handle| {
                        self.pending_affects(handle, view.handle)
                            || self.dependency_rows(view).any(|row| {
                                self.pending_affects(handle, self.slot_in(row as usize, view))
                            })
                    })
                });
                (!replaced).then(|| self.detail(view)).flatten()
            })
    }
    /// Converted state can also depend on other canonical rows (colour, dmflags).
    pub fn view_generation(&self, view: View) -> u64 {
        self.dependency_rows(view)
            .fold(self.generation(view.handle), |generation, row| {
                generation.max(self.generation(self.slot_in(row as usize, view)))
            })
    }
    /// Native cvar layouts also expose a pending latch, without marking the
    /// current native value modified until that latch is applied.
    pub fn view_update_generation(&self, view: View) -> u64 {
        let generation = |handle: CvarHandle| {
            let value = &self.values[handle.0 as usize];
            let pending = if self.inherits(handle) {
                self.values[self.root(handle).0 as usize].pending_revision
            } else {
                0
            };
            self.generation(handle)
                .max(value.pending_revision)
                .max(pending)
        };
        self.dependency_rows(view)
            .fold(generation(view.handle), |revision, row| {
                revision.max(generation(self.slot_in(row as usize, view)))
            })
    }
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    pub fn reset_lookup_count(&self) {
        self.lookups.set(0);
    }
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    pub fn lookup_count(&self) -> u64 {
        self.lookups.get()
    }
    pub fn text(&self, handle: CvarHandle) -> &str {
        self.effective(handle, self.context.source)
    }
    pub fn is_explicit(&self, handle: CvarHandle) -> bool {
        self.values[handle.0 as usize].explicit
            || self.column(handle) != 0 && self.values[self.root(handle).0 as usize].explicit
    }
    /// Cached native-role value only when explicitly set or registered by that engine.
    pub fn native_number(&self, handle: CvarHandle, source: RuleSetId) -> Option<f32> {
        self.native_setting_available(handle, source)
            .then(|| self.value_in(handle, source))
    }
    pub fn native_integer(&self, handle: CvarHandle, source: RuleSetId) -> Option<i32> {
        self.native_setting_available(handle, source)
            .then(|| self.integer_in(handle, source))
    }
    pub fn explicit_number(&self, handle: CvarHandle, source: RuleSetId) -> Option<f32> {
        self.is_explicit(handle)
            .then(|| self.value_in(handle, source))
    }
    fn native_setting_available(&self, handle: CvarHandle, source: RuleSetId) -> bool {
        self.is_explicit(handle) || self.native_default_available(handle, source)
    }
    pub fn default_available(&self, handle: CvarHandle, source: RuleSetId) -> bool {
        if self.values[handle.0 as usize].module_defaults[source as usize].is_some() {
            return true;
        }
        if self.column(handle) != 0 {
            return self.default_available(self.root(handle), source);
        }
        self.defaults
            .get(self.values[handle.0 as usize].row)
            .is_some_and(|row| row[source as usize].is_some())
    }
    /// Load-time consumers may retain native behavior for settings that the
    /// unified catalog accepts but the selected engine never registered.
    pub fn native_default_available(&self, handle: CvarHandle, source: RuleSetId) -> bool {
        if self.values[handle.0 as usize].module_defaults[source as usize].is_some() {
            return true;
        }
        if self.column(handle) != 0 {
            return self.native_default_available(self.root(handle), source);
        }
        let row = self.values[handle.0 as usize].row;
        if row >= DEFINITIONS.len() {
            return false;
        }
        let context = Context {
            source,
            role: self.stock_roles[row][source as usize],
            ..self.context
        };
        self.default_available(handle, source)
            && DEFAULTS[DEFINITIONS[row].defaults[source as usize].defaults.clone()]
                .iter()
                .any(|clause| {
                    clause.issues == 0
                        && clause.kind == DefaultKind::Native
                        && condition(clause.condition, context)
                })
    }
    pub fn read(&self, view: View) -> Result<Text<'_>, conversion::Error> {
        self.validate_view(view)?;
        let text = self.effective(view.handle, view.context.source);
        let detail = self.detail(view);
        let operands = |row| self.effective(self.slot_in(row as usize, view), view.context.source);
        self.project(view, text, detail, &operands)
    }
    fn project<'a>(
        &'a self,
        view: View,
        text: &'a str,
        detail: Option<&'a str>,
        operands: &dyn Fn(u16) -> &'a str,
    ) -> Result<Text<'a>, conversion::Error> {
        if self.reads_stored(view).unwrap_or(true) {
            return Ok(Text::Borrowed(text));
        }
        conversion::read(Input {
            context: view.context,
            conversion: self.conversion(view),
            name: self.binding_names[view.binding as usize],
            autoswitch_text_name: self.autoswitch_text_name,
            value: text,
            current: text,
            detail,
            operand: operands,
            video: self.video_policy.as_ref(),
        })
    }
    pub fn latched(&self, view: View) -> Result<Option<Text<'_>>, conversion::Error> {
        self.validate_view(view)?;
        if !self.pending.iter().any(|pending| {
            pending.handles().any(|handle| {
                self.pending_affects(handle, view.handle)
                    || self
                        .dependency_rows(view)
                        .any(|row| self.pending_affects(handle, self.slot_in(row as usize, view)))
            })
        }) {
            return Ok(None);
        }
        let text_for = |handle| self.pending_text(handle, view.context.source);
        let detail = self.pending_detail(view);
        self.project(view, text_for(view.handle), detail, &|row| {
            text_for(self.slot_in(row as usize, view))
        })
        .map(Some)
    }
    pub fn numeric(&self, view: View) -> Result<f32, conversion::Error> {
        self.validate_view(view)?;
        if self.reads_stored(view).unwrap_or(true) {
            Ok(self.value_in(view.handle, view.context.source))
        } else {
            self.projected_number(view).map(|(number, _)| number)
        }
    }
    pub fn integer_view(&self, view: View) -> Result<i32, conversion::Error> {
        self.validate_view(view)?;
        if self.reads_stored(view).unwrap_or(true) {
            Ok(self.integer_in(view.handle, view.context.source))
        } else {
            self.projected_number(view).map(|(_, integer)| integer)
        }
    }
    fn reads_stored(&self, view: View) -> Option<bool> {
        BINDINGS.get(view.binding as usize).map(|binding| {
            binding.canonical
                && self.conversion(view).operation != Operation::WindowGeometry
                && view.context.role == Role::Engine
                && DEFINITIONS[binding.row as usize].stored
        })
    }
    fn conversion(&self, view: View) -> &'static crate::catalog::Conversion {
        if view.binding as usize >= BINDINGS.len() {
            return &CONVERSIONS[0];
        }
        let b = &BINDINGS[view.binding as usize];
        let row = &DEFINITIONS[b.row as usize];
        &CONVERSIONS[if row.stored {
            b.conversions[view.context.source as usize]
        } else {
            row.rule_conversion
        } as usize]
    }
    pub fn flags(&self, view: View) -> u32 {
        if let Some(flags) = self.values[view.handle.0 as usize].command_flags
            [view.context.source as usize]
            .or(self.values[self.root(view.handle).0 as usize].command_flags
                [view.context.source as usize])
        {
            return flags;
        }
        let Some(b) = BINDINGS.get(view.binding as usize) else {
            return 0;
        };
        let row = &DEFINITIONS[b.row as usize];
        let native = b.native_sources & (1 << view.context.source as u8) != 0;
        let source = if native {
            view.context.source as usize
        } else {
            row.home.map_or(view.context.source as usize, usize::from)
        };
        let mut flags = u32::from(row.archive);
        for index in row.defaults[source].flags.clone() {
            let clause = &FLAGS[index];
            if clause.issues == 0
                && (clause.member.is_empty()
                    || self.flag_names[index] == Some(self.binding_names[view.binding as usize])
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
        if view.binding as usize >= BINDINGS.len() {
            return false;
        }
        DEFINITIONS[BINDINGS[view.binding as usize].row as usize].policies & 2 != 0
    }
    pub fn macro_value(&self, name: &[u8], context: Context) -> Option<Text<'_>> {
        let view = self.bind(std::str::from_utf8(name).ok()?, context)?;
        (!self.private(view))
            .then(|| self.read(view).ok())
            .flatten()
    }
    fn check_write(&self, view: View, text: &str) -> Result<(), WriteError> {
        let flags = self.flags(view);
        if flags & 64 != 0 {
            return Err(WriteError::ReadOnly);
        }
        if flags & 16 != 0 && self.runtime_policy.initialized {
            return Err(WriteError::InitOnly);
        }
        // Q3 Cvar_Set2 returns from its LATCH branch before checking CHEAT.
        // The eagerly registered catalog retains its initial setup window.
        let native_latch = self.runtime_policy.initialized
            && view.context.source == RuleSetId::Quake3
            && flags & 32 != 0;
        if flags & 512 != 0
            && !native_latch
            && self.integer_in(
                self.in_column(self.cheats_handle, self.column(view.handle)),
                view.context.source,
            ) == 0
        {
            return Err(WriteError::Cheats);
        }
        if flags & 6 != 0 && !info_text(text) {
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
        // A new flag override also replaces other aliases' catalog flags.
        // Invalidate all native info categories when that membership changes.
        if previous != Some(flags) {
            self.mark_info(
                self.column(view.handle),
                view.context.source,
                2 | 4 | 8,
                self.revision_clock,
            );
        }
        self.refresh_changes();
        Ok(())
    }
    /// Native force-set preserves flags and still uses the same conversion and
    /// publication path as console writes.
    pub fn force_write(&mut self, view: View, text: &str) -> Result<(), WriteError> {
        self.write_inner(view, text, false)
    }
    /// A native reset writes this source/alias's default through its ordinary
    /// conversion path; it does not clear explicit values for every source.
    pub fn reset_view(&mut self, view: View, force: bool) -> Result<(), WriteError> {
        let mut text = FixedText::<MAX_TEXT>::default();
        let source = view.context.source;
        let default = self.project(view, self.default_text(view.handle, source), None, &|row| {
            self.default_text(self.slot_in(row as usize, view), source)
        })?;
        text.set(default.as_str())
            .map_err(|_| conversion::Error::TextTooLong)?;
        // Cvar_Set2 returns before protection checks or latch cancellation
        // when the reset string already equals the current native value.
        if self
            .read(view)
            .is_ok_and(|current| current.as_str() == text.as_str())
        {
            return Ok(());
        }
        self.write_inner(view, text.as_str(), !force)
    }
    fn prepare_write(
        &self,
        view: View,
        text: &str,
        enforce: bool,
        changes: &mut PendingWrite,
    ) -> Result<bool, WriteError> {
        let fullscreen_group = self.video_policy.as_ref().and_then(|policy| {
            if view.handle != self.slot(policy.geometry as usize, 0) {
                return None;
            }
            let fullscreen = self.slot(policy.rows[3] as usize, 0);
            self.pending
                .iter()
                .find(|group| group.handles().any(|handle| handle == fullscreen))
                .map(|group| {
                    (
                        group,
                        policy.rows[3],
                        self.pending_text(fullscreen, view.context.source),
                    )
                })
        });
        let operand = fullscreen_group.map(|(_, row, value)| (row, value));
        if let Some((previous, _, _)) = fullscreen_group {
            changes.append(previous)?;
        }
        let mut pending = self.prepare_write_value(view, text, enforce, operand, changes)?
            || fullscreen_group.is_some();
        if let Some(policy) = &self.video_policy
            && view.handle == self.slot(policy.rows[6] as usize, 0)
            && let Some(selector) = self.bind(
                "vid_fullscreen",
                Context {
                    source: RuleSetId::Quake2Rerelease,
                    role: Role::Engine,
                    ..view.context
                },
            )
            && let Some(index) = self.pending_detail(selector)
            && integer(index) != 0
        {
            pending |= self.prepare_write_value(
                selector,
                index,
                enforce,
                Some((policy.rows[6], text)),
                changes,
            )?;
        }
        if let Some(policy) = &self.video_policy {
            let fullscreen = self.slot(policy.rows[3] as usize, 0);
            let selected = changes
                .values()
                .find_map(|(handle, value)| (handle == fullscreen).then_some(integer(value)))
                .map(|value| if value == 0 { "0" } else { "1" });
            if let Some(selected) = selected
                && let Some((geometry, raw)) = self.retained_detail("vid_geometry", true)
                && view.handle != geometry.handle
            {
                pending |= self.prepare_write_value(
                    geometry,
                    raw,
                    enforce,
                    Some((policy.rows[3], selected)),
                    changes,
                )?;
            }
        }
        Ok(pending)
    }
    fn prepare_write_value(
        &self,
        view: View,
        text: &str,
        enforce: bool,
        operand_override: Option<(u16, &str)>,
        changes: &mut PendingWrite,
    ) -> Result<bool, WriteError> {
        if enforce {
            self.check_write(view, text)?;
        }
        let binding = BINDINGS.get(view.binding as usize);
        let current = self.effective(view.handle, view.context.source);
        let detail = self.detail(view);
        let operands = |row| {
            operand_override
                .filter(|(overridden, _)| *overridden == row)
                .map_or_else(
                    || self.effective(self.slot_in(row as usize, view), view.context.source),
                    |(_, value)| value,
                )
        };
        let c = if self.reads_stored(view).unwrap_or(true) {
            &CONVERSIONS[0]
        } else {
            self.conversion(view)
        };
        let mut out = conversion::write(Input {
            context: view.context,
            conversion: c,
            name: self.binding_names[view.binding as usize],
            autoswitch_text_name: self.autoswitch_text_name,
            value: text,
            current,
            detail,
            operand: &operands,
            video: self.video_policy.as_ref(),
        })?;
        if self.video_policy.as_ref().is_some_and(|policy| {
            view.handle == self.slot(policy.rows[0] as usize, 0)
                && binding.is_some_and(|binding| binding.canonical)
        }) {
            out.detail = true;
        }
        let mut pending = enforce && self.latch_pending(view);
        if binding.is_none_or(|b| DEFINITIONS[b.row as usize].stored) {
            let mut value = FixedText::<MAX_TEXT>::default();
            if let Some(prefix) = out.prefix {
                value
                    .write_str(prefix)
                    .map_err(|_| conversion::Error::TextTooLong)?;
            }
            value
                .write_str(out.text.as_str())
                .map_err(|_| conversion::Error::TextTooLong)?;
            changes.value(view.handle, value.as_str())?;
        }
        for change in out.changes.into_iter().take(out.change_count).flatten() {
            let handle = self.slot_in(change.row as usize, view);
            let target = self.values[handle.0 as usize].name_id;
            if let Some(target_view) = self.bind_id(target, view.context) {
                if enforce {
                    self.check_write(target_view, change.text.as_str())?;
                }
                pending |= enforce && self.latch_pending(target_view);
                changes.value(handle, change.text.as_str())?;
            }
        }
        if out.detail {
            let detail = out.detail_value.unwrap_or(text);
            changes.detail(view, detail, Some(detail) != self.detail(view))?;
        }
        Ok(pending)
    }
    fn write_inner(&mut self, view: View, text: &str, enforce: bool) -> Result<(), WriteError> {
        self.validate_view(view)?;
        let native_q3 = self.runtime_policy.initialized && view.context.source == RuleSetId::Quake3;
        // Even a forced equal write leaves Q3's outstanding latch untouched.
        if native_q3
            && self
                .read(view)
                .is_ok_and(|current| current.as_str() == text)
        {
            return Ok(());
        }
        let mut changes = PendingWrite {
            texts: [None, None],
            changes: std::array::from_fn(|_| None),
            details: [None, None],
        };
        let pending = self.prepare_write(view, text, enforce, &mut changes)?;
        if native_q3
            && pending
            && self
                .latched(view)?
                .is_some_and(|current| current.as_str() == text)
        {
            return Ok(());
        }
        self.remove_pending(|old| old.handles().any(|h| changes.handles().any(|new| h == new)));
        if pending {
            self.mark_pending(&changes);
            self.pending.push(changes);
            self.refresh_changes();
            return Ok(());
        }
        self.publish_group(&changes)?;
        self.refresh_changes();
        Ok(())
    }
    fn detail(&self, view: View) -> Option<&str> {
        self.details.get(self.projection_index(view)?)?[view.context.source as usize]
            .as_ref()
            .filter(|d| {
                d.present
                    && d.revisions[..d.count]
                        .iter()
                        .all(|(h, revision)| self.generation(*h) == *revision)
            })
            .map(|d| d.text.as_str())
    }
    fn save_detail(&mut self, view: View, text: &str, modified: bool) -> Result<(), WriteError> {
        if modified {
            self.mark_change(view.handle);
        }
        let mut revisions = [(CvarHandle(0), 0); 33];
        revisions[0] = (view.handle, self.generation(view.handle));
        let mut count = 1;
        for row in self.dependency_rows(view) {
            let handle = self.slot_in(row as usize, view);
            revisions[count] = (handle, self.generation(handle));
            count += 1;
        }
        let index = self
            .projection_index(view)
            .ok_or(conversion::Error::PolicyRequired)?;
        let detail = self.details[index][view.context.source as usize]
            .as_mut()
            .ok_or(conversion::Error::PolicyRequired)?;
        detail
            .text
            .set(text)
            .map_err(|_| conversion::Error::TextTooLong)?;
        detail.revisions = revisions;
        detail.count = count;
        detail.present = true;
        Ok(())
    }
    fn clear_details(&mut self, handle: CvarHandle) {
        let mut modified = false;
        let column = self.column(handle);
        for &binding in &self.dependents[self.root(handle).0 as usize] {
            for relative in self.projection_rows[binding as usize].range() {
                let index = column * self.projection_count() + relative;
                for detail in self.details[index].iter_mut().flatten() {
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
        }
        if modified {
            self.mark_change(handle);
        }
    }
    fn cancel_pending(&mut self, handle: CvarHandle) {
        self.remove_pending(|pending| pending.handles().any(|h| h == handle));
    }
    fn remove_pending(&mut self, matches: impl Fn(&PendingWrite) -> bool) {
        let mut index = 0;
        while index < self.pending.len() {
            if matches(&self.pending[index]) {
                let old = self.pending.remove(index);
                self.mark_pending(&old);
            } else {
                index += 1;
            }
        }
    }
    fn mark_pending(&mut self, pending: &PendingWrite) {
        for handle in pending.handles() {
            self.revision_clock = self.revision_clock.wrapping_add(1);
            self.values[handle.0 as usize].pending_revision = self.revision_clock;
            self.mark_dirty(handle);
        }
    }
    fn publish(&mut self, handle: CvarHandle, text: &str) -> Result<(), WriteError> {
        let column = self.column(handle);
        let root = self.root(handle).0 as usize;
        let changed = self.texts[column]
            .set(root, text.as_bytes())
            .map_err(|_| WriteError::Capacity)?;
        let value = &mut self.values[handle.0 as usize];
        if !value.explicit || changed {
            value.explicit = true;
            self.mark_change(handle);
        }
        Ok(())
    }

    fn publish_group(&mut self, pending: &PendingWrite) -> Result<(), WriteError> {
        for (handle, text) in pending.values() {
            self.clear_details(handle);
            self.publish(handle, text)?;
        }
        for detail in pending.details.iter().flatten() {
            self.save_detail(detail.view, detail.text.as_str(), detail.modified)?;
        }
        Ok(())
    }
    fn mark_change(&mut self, handle: CvarHandle) {
        self.revision_clock = self.revision_clock.wrapping_add(1);
        self.values[handle.0 as usize].revision = self.revision_clock;
        self.mark_dirty(handle);
    }
    fn mark_dirty(&mut self, handle: CvarHandle) {
        let root = self.root(handle);
        let first = self.column(handle);
        let end = if first == 0 {
            self.texts.len()
        } else {
            first + 1
        };
        for column in first..end {
            let handle = self.in_column(root, column);
            if !self.dirty_values.test_and_set(handle.0 as usize) {
                self.dirty_handles.push(handle);
            }
        }
    }
    /// Engine-owned updates bypass command policy (ROM values and telemetry).
    pub fn set(&mut self, handle: CvarHandle, value: f32) -> Result<(), WriteError> {
        let mut text = FixedText::<64>::default();
        write!(text, "{value}").map_err(|_| conversion::Error::TextTooLong)?;
        self.set_text(handle, text.as_str())
    }
    pub fn set_text(&mut self, handle: CvarHandle, text: &str) -> Result<(), WriteError> {
        if text.len() > MAX_TEXT {
            return Err(conversion::Error::TextTooLong.into());
        }
        self.cancel_pending(handle);
        self.clear_details(handle);
        self.publish(handle, text)?;
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
    /// Set at a capability's load/unload boundary. Native server-wide latches
    /// retain their existing behavior; this also serves client-only renderers.
    pub fn set_latch_active(&mut self, handle: CvarHandle, active: bool) {
        self.values[handle.0 as usize].latch_active = active;
    }
    fn latch_pending(&self, view: View) -> bool {
        self.flags(view) & 32 != 0
            && (self.runtime_policy.server_active
                || self.values[view.handle.0 as usize].latch_active
                || (self.runtime_policy.initialized && view.context.source == RuleSetId::Quake3))
    }
    pub fn apply_latches(&mut self) -> Result<(), WriteError> {
        while !self.pending.is_empty() {
            self.apply_pending(0)?;
        }
        self.refresh_changes();
        Ok(())
    }
    /// Native Cvar_Get activates only the requested variable's pending group.
    pub fn apply_latch(&mut self, view: View) -> Result<(), WriteError> {
        self.validate_view(view)?;
        while let Some(index) = self
            .pending
            .iter()
            .position(|pending| pending.handles().any(|handle| handle == view.handle))
        {
            self.apply_pending(index)?;
        }
        self.refresh_changes();
        Ok(())
    }
    fn apply_pending(&mut self, index: usize) -> Result<(), WriteError> {
        let pending = self.pending.remove(index);
        self.mark_pending(&pending);
        self.publish_group(&pending)
    }
    pub fn entries(
        &self,
    ) -> impl Iterator<Item = (CvarHandle, &str, &str, Option<&'static Definition>, u8)> {
        self.entries_in(self.context)
    }
    pub fn entries_in(
        &self,
        context: Context,
    ) -> impl Iterator<Item = (CvarHandle, &str, &str, Option<&'static Definition>, u8)> {
        self.values[..self.registered_values]
            .iter()
            .enumerate()
            .filter_map(move |(index, value)| {
                let handle = self.context_handle(CvarHandle(index as u32), context)?;
                Some((
                    handle,
                    self.name_text(value.name_id),
                    self.effective(handle, context.source),
                    DEFINITIONS.get(value.row),
                    value.seat,
                ))
            })
    }
    /// Native info keys use their source names and converted values. Build at
    /// connect or info-change boundaries, never while reading module cvars.
    pub fn info_string(
        &self,
        context: Context,
        flag: u32,
        rules: qa_core::text::InfoRules,
        output: &mut [u8],
    ) -> Result<usize, WriteError> {
        if rules.bytes == 0 || output.len() < rules.bytes {
            return Err(WriteError::Capacity);
        }
        output[0] = 0;
        let mut length = 0;
        let mut append = |view: View| -> Result<(), WriteError> {
            if self.flags(view) & flag == 0 {
                return Ok(());
            }
            let name = self.name(view);
            let value = self.read(view)?;
            if value.as_str().is_empty() {
                return Ok(());
            }
            let _ = qa_core::text::info_set(
                output,
                &mut length,
                name.as_bytes(),
                value.as_str().as_bytes(),
                rules,
            );
            Ok(())
        };
        for (index, binding) in BINDINGS.iter().enumerate() {
            if binding.native_sources & (1 << context.source as u8) != 0
                && let Some(view) = self.bind_id(self.binding_names[index], context)
                && view.binding as usize == index
            {
                append(view)?;
            }
        }
        for value in &self.values[self.catalog_values..self.registered_values] {
            if let Some(view) = self.bind_id(value.name_id, context) {
                append(view)?;
            }
        }
        Ok(length)
    }
    fn refresh_number_at(&mut self, index: usize) {
        let default_numbers = std::array::from_fn(|s| {
            number(
                self.default_text(CvarHandle(index as u32), RuleSetId::ALL[s]),
                RuleSetId::ALL[s],
            )
        });
        let numbers = std::array::from_fn(|s| {
            number(
                self.effective(CvarHandle(index as u32), RuleSetId::ALL[s]),
                RuleSetId::ALL[s],
            )
        });
        let integers = std::array::from_fn(|s| {
            integer(self.effective(CvarHandle(index as u32), RuleSetId::ALL[s]))
        });
        self.values[index].numbers = numbers;
        self.values[index].default_numbers = default_numbers;
        self.values[index].integers = integers;
    }
    fn refresh_numbers(&mut self) {
        for index in 0..self.values.len() {
            if self.defined(index) {
                self.refresh_number_at(index);
            }
        }
    }
    fn refresh_projection_at(&mut self, index: usize, column: usize) {
        let binding = &BINDINGS[index];
        let span = self.projection_rows[index];
        for projection in span.range() {
            let projections = std::array::from_fn(|source| {
                let context = Context {
                    source: RuleSetId::ALL[source],
                    role: Role::Engine,
                    seat: qa_core::sys_events::SeatId::new(
                        (projection - span.first as usize) as u8,
                    )
                    .unwrap_or(qa_core::sys_events::SeatId::FIRST),
                    event_time: None,
                    key_source: None,
                    producer: None,
                    side: if column == 0 {
                        self.context.side
                    } else {
                        Scope::Client
                    },
                    cvar_client: None,
                    client_lifetime: self.column_owner(column),
                    client_policy: None,
                    ..self.context
                };
                let Some(handle) = self
                    .binding_slot(binding, context)
                    .and_then(|handle| self.context_handle(handle, context))
                else {
                    return [Err(conversion::Error::PolicyRequired); ROLES.len()];
                };
                let view = View {
                    handle,
                    binding: index as u16,
                    context,
                };
                if binding.native_sources & (1 << source as u8) != 0 {
                    let flags = self.flags(view);
                    if flags & (2 | 4 | 8) != 0 {
                        self.mark_info(column, context.source, flags, self.view_generation(view));
                    }
                }
                std::array::from_fn(|role| {
                    let view = View {
                        context: Context {
                            role: ROLES[role],
                            ..context
                        },
                        ..view
                    };
                    self.read(view).map(|text| {
                        (
                            number(text.as_str(), context.source),
                            integer(text.as_str()),
                        )
                    })
                })
            });
            let index = column * self.projection_count() + projection;
            self.projections[index] = projections;
        }
    }
    fn refresh_projections(&mut self) {
        for column in 0..self.texts.len() {
            for index in 0..BINDINGS.len() {
                self.refresh_projection_at(index, column);
            }
        }
        for index in 0..self.native_heads.len() {
            self.refresh_native(index);
        }
        self.dirty_values.begin();
        self.dirty_projections.begin();
        self.dirty_handles.clear();
        self.dirty_bindings.clear();
    }
    fn refresh_changes(&mut self) {
        if self.dirty_handles.is_empty() {
            return;
        }
        for i in 0..self.dirty_handles.len() {
            let index = self.dirty_handles[i].0 as usize;
            self.refresh_number_at(index);
            let root = index % self.stride;
            let column = index / self.stride;
            for &binding in &self.dependents[root] {
                let projection = column * BINDINGS.len() + binding as usize;
                if !self.dirty_projections.test_and_set(projection) {
                    self.dirty_bindings.push(projection);
                }
            }
            if root >= self.catalog_values {
                self.refresh_native(BINDINGS.len() + root - self.catalog_values);
                let value = &self.values[index];
                let flags = value.command_flags;
                let generation = value.revision;
                for source in RuleSetId::ALL {
                    self.mark_info(
                        column,
                        source,
                        flags[source as usize].unwrap_or(0),
                        generation,
                    );
                }
            }
        }
        for i in 0..self.dirty_bindings.len() {
            let index = self.dirty_bindings[i] as usize;
            self.refresh_projection_at(index % BINDINGS.len(), index / BINDINGS.len());
            self.refresh_native(index % BINDINGS.len());
        }
        self.dirty_values.begin();
        self.dirty_projections.begin();
        self.dirty_handles.clear();
        self.dirty_bindings.clear();
    }
    fn resolve_defaults(&mut self) {
        for row in 0..DEFINITIONS.len() {
            for source in RuleSetId::ALL {
                self.defaults[row][source as usize] = self.resolve_default(row, source);
            }
        }
        // Native composites supply operands without their own native declaration.
        for definition in DEFINITIONS {
            if definition.stored {
                continue;
            }
            for source in RuleSetId::ALL {
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
                    let Some(binding) = self.lookup_binding(clause.member, context.side) else {
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
                        name: self.binding_names[binding as usize],
                        autoswitch_text_name: self.autoswitch_text_name,
                        value: clause.value,
                        current: clause.value,
                        detail: None,
                        operand: &operands,
                        video: self.video_policy.as_ref(),
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
    fn resolve_default(&self, row: usize, source: RuleSetId) -> Option<Cow<'static, str>> {
        let context = Context {
            source,
            role: self.stock_roles[row][source as usize],
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
            let binding = self.lookup_binding(member, context.side);
            let Some(binding) = binding else {
                if DEFINITIONS[row].family_count > 1 {
                    selected = Some(Cow::Borrowed(clause.value));
                }
                continue;
            };
            let b = &BINDINGS[binding as usize];
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
                name: self.binding_names[binding as usize],
                autoswitch_text_name: self.autoswitch_text_name,
                value: clause.value,
                current,
                detail: None,
                operand: &operands,
                video: self.video_policy.as_ref(),
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
fn stock_role(names: &NameTable, binding_names: &[NameId], row: usize, source: RuleSetId) -> Role {
    let mut role = Role::Game;
    for clause in &DEFAULTS[DEFINITIONS[row].defaults[source as usize].defaults.clone()] {
        if clause.issues != 0 {
            continue;
        }
        if clause.condition == Condition::Engine {
            role = Role::Engine;
        }
        if matches!(clause.condition, Condition::Cgame | Condition::Always)
            && let Some(id) = names.find_folded(clause.member.as_bytes())
            && let Some(binding) = binding_names
                .iter()
                .enumerate()
                .find(|&(index, &name)| name == id && BINDINGS[index].scope != Scope::Server)
                .map(|(index, _)| index)
        {
            let b = &BINDINGS[binding];
            let c = &CONVERSIONS[b.conversions[source as usize] as usize];
            if c.cgame_only && c.kind == ConversionKind::Reciprocal {
                return Role::Cgame;
            }
        }
    }
    role
}

fn info_text(text: &str) -> bool {
    qa_core::text::info_valid_field(text.as_bytes(), qa_core::text::InfoBytes::Exact)
}

fn cvar_context(context: Context) -> Context {
    Context {
        file_search: None,
        ..context
    }
}
