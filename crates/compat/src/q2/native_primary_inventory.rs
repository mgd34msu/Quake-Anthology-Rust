//! Port of `src/compat/q2/native-primary-inventory.ts`.
//! Bridges original inventory scanners: usability filters, cursor and named use.

use std::collections::HashMap;

use qa_guest::core::contracts::{GuestAddress, GuestCallValue, NativeAbi};
use qa_world::combat::ItemId;

use super::native_primary_reader::NativeRegion;
use super::native_primary_weapons::{HostResult, NativeActorId, NativeHostError, SyntheticHost};

/// Canonical presentation of one inventory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryPresentation {
    /// Weapon row with its provider and weapon item.
    Weapon {
        /// Provider reference.
        source: String,
        /// Weapon item.
        weapon: ItemId,
    },
    /// Ammunition row with its provider and weapon item.
    Ammunition {
        /// Provider reference.
        source: String,
        /// Weapon item.
        weapon: ItemId,
    },
    /// Plain item row with an optional HUD icon path.
    Item {
        /// Provider reference.
        source: String,
        /// HUD icon path.
        icon: Option<String>,
    },
}

/// One canonical inventory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeInventoryRow {
    /// Canonical presentation.
    pub presentation: Option<InventoryPresentation>,
    /// Canonical item id.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Canonical count.
    pub count: i32,
    /// Original table slot.
    pub source_index: u32,
    /// Row is the canonical selection.
    pub selected: bool,
    /// Only presence is projected (0 or 1).
    pub presence_only: bool,
}

/// Readout item view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadoutItem {
    /// Canonical item id.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Canonical count.
    pub count: i32,
}

/// Inventory readout for an actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeInventoryReadout {
    /// Selected row presentation.
    pub presentation: Option<InventoryPresentation>,
    /// Non-empty rows.
    pub items: Vec<ReadoutItem>,
    /// Selected item.
    pub selected: Option<ItemId>,
}

/// Prototype items per inventory category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryPrototypes {
    /// Weapon prototype.
    pub weapon: ItemId,
    /// Ammunition prototype.
    pub ammunition: ItemId,
    /// Usable prototype.
    pub usable: ItemId,
    /// Passive prototype.
    pub passive: ItemId,
    /// Droppable prototype.
    pub droppable: ItemId,
    /// Undroppable prototype.
    pub undroppable: ItemId,
}

/// Cursor side-effect span restored after rejected evaluations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionWrite {
    /// Client offset.
    pub offset: u32,
    /// Span length in bytes.
    pub bytes: u32,
}

/// Next-item scanner declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextProfile {
    /// Entry RVA.
    pub entry: u32,
    /// Scan region entry RVA.
    pub scan: u32,
    /// Scan region join RVA.
    pub join: u32,
    /// Whether the entry takes the trailing menu argument.
    pub menu_argument: bool,
}

/// Previous-item scanner declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviousProfile {
    /// Entry RVA.
    pub entry: u32,
    /// Scan region entry RVA.
    pub scan: u32,
    /// Scan region join RVA.
    pub join: u32,
}

/// Validate-entry declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidateProfile {
    /// Entry RVA.
    pub entry: u32,
    /// Optional admitted scan region.
    pub scan: Option<NativeRegion>,
}

/// Use-entry declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseProfile {
    /// Entry RVA.
    pub entry: u32,
    /// Item-call RVA.
    pub call: u32,
    /// Join RVA.
    pub join: u32,
}

/// Named-use entry declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedUseProfile {
    /// Entry RVA.
    pub entry: u32,
    /// Lookup call RVA.
    pub lookup_call: u32,
    /// Lookup return RVA.
    pub lookup_return: u32,
    /// Item-call RVA.
    pub call: u32,
    /// Join RVA.
    pub join: u32,
}

/// Native primary inventory profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePrimaryInventoryProfile {
    /// Artifact digest.
    pub digest: String,
    /// Executable ABI.
    pub abi: NativeAbi,
    /// Client pointer offset within the entity.
    pub client: u32,
    /// Inventory offset within the client.
    pub inventory: u32,
    /// Inventory slot count.
    pub count: u32,
    /// Cursor offset within the client.
    pub cursor: u32,
    /// Empty-cursor sentinel.
    pub empty: i32,
    /// Category prototypes.
    pub prototypes: InventoryPrototypes,
    /// Cursor side-effect spans.
    pub selection_writes: Vec<SelectionWrite>,
    /// Next scanner.
    pub next: NextProfile,
    /// Previous scanner.
    pub previous: PreviousProfile,
    /// Validate entry.
    pub validate: ValidateProfile,
    /// Use entry.
    pub use_profile: UseProfile,
    /// Named-use entry.
    pub named_use: NamedUseProfile,
}

/// Inventory hooks owned by the caller.
pub struct InventoryHooks {
    /// Canonical rows, or `None` when the actor has no canonical inventory.
    pub rows: Box<dyn FnMut(NativeActorId) -> Option<Vec<NativeInventoryRow>>>,
    /// Accepted use of a canonical item.
    pub use_item: Box<dyn FnMut(NativeActorId, ItemId)>,
    /// Source descriptor for a table slot.
    pub descriptor: Box<dyn FnMut(u32) -> GuestAddress>,
    /// Canonical item behind a descriptor address.
    pub item_at: Box<dyn FnMut(GuestAddress) -> Option<ItemId>>,
    /// Client print.
    pub print: Box<dyn FnMut(NativeActorId, String)>,
}

struct Frame {
    actor: Option<NativeActorId>,
    flags: i32,
    named: bool,
    row: Option<NativeInventoryRow>,
    restore: Option<(GuestAddress, Vec<u8>)>,
}

/// Named-item resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemNameMatch {
    /// Unique match with its exactness.
    Match {
        /// Matched row.
        row: NativeInventoryRow,
        /// Exact id match rather than label or original fallback.
        exact: bool,
    },
    /// Several label matches.
    Ambiguous(Vec<NativeInventoryRow>),
}

/// Resolve a typed name against canonical rows: exact id first, then the
/// original descriptor fallback, then unique label match.
pub fn source_item_named(
    rows: &[NativeInventoryRow],
    text: &str,
    original: Option<&ItemId>,
) -> Option<ItemNameMatch> {
    let requested: String = text
        .chars()
        .filter(|char| *char != ' ')
        .collect::<String>()
        .to_lowercase();
    if let Some(exact) = rows.iter().find(|row| row.item.to_lowercase() == requested) {
        return Some(ItemNameMatch::Match {
            row: exact.clone(),
            exact: true,
        });
    }
    if let Some(original) = original {
        if let Some(primary) = rows.iter().find(|row| &row.item == original) {
            return Some(ItemNameMatch::Match {
                row: primary.clone(),
                exact: false,
            });
        }
    }
    let matches: Vec<NativeInventoryRow> = rows
        .iter()
        .filter(|row| {
            row.label
                .chars()
                .filter(|char| *char != ' ')
                .collect::<String>()
                .to_lowercase()
                == requested
        })
        .cloned()
        .collect();
    if matches.len() > 1 {
        Some(ItemNameMatch::Ambiguous(matches))
    } else {
        matches.into_iter().next().map(|row| ItemNameMatch::Match {
            row,
            exact: false,
        })
    }
}

/// Named-lookup outcome for the emulated original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupOutcome {
    /// Project the row and return its descriptor.
    Project {
        /// Projected row.
        row: NativeInventoryRow,
        /// Source descriptor the original returns.
        descriptor: GuestAddress,
    },
    /// Ambiguous or unmatched; run the original lookup untouched.
    PassThrough,
}

/// Original scanners own usability, category filters and accepted cursor side
/// effects. Entry wrappers are explicit methods; tests emulate originals.
pub struct NativePrimaryInventory {
    profile: NativePrimaryInventoryProfile,
    hooks: InventoryHooks,
    selected: HashMap<NativeActorId, (ItemId, u32)>,
    frames: Vec<Frame>,
    evaluating: u32,
    closed: bool,
}

impl NativePrimaryInventory {
    /// Build the service.
    pub fn new(profile: NativePrimaryInventoryProfile, hooks: InventoryHooks) -> Self {
        Self {
            profile,
            hooks,
            selected: HashMap::new(),
            frames: Vec::new(),
            evaluating: 0,
            closed: false,
        }
    }

    /// Borrow the profile.
    #[must_use]
    pub fn profile(&self) -> &NativePrimaryInventoryProfile {
        &self.profile
    }

    fn check_host(&self, host: &SyntheticHost) -> HostResult<()> {
        if self.closed {
            return Err(NativeHostError::Fault(
                "native inventory service is closed".to_string(),
            ));
        }
        if host.core.digest != self.profile.digest {
            return Err(NativeHostError::Fault(
                "native inventory profile belongs to another artifact".to_string(),
            ));
        }
        Ok(())
    }

    fn current(&self, host: &SyntheticHost, actor: NativeActorId) -> bool {
        !self.closed && host.core.is_live(actor)
    }

    fn source_count(row: &NativeInventoryRow) -> i32 {
        if row.presence_only {
            i32::from(row.count != 0)
        } else {
            row.count
        }
    }

    fn client(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<GuestAddress> {
        let entity = host.core.entity_of(actor).map_err(|_| {
            NativeHostError::Fault("native inventory actor has no source client".to_string())
        })?;
        let client = host.core.memory.read_pointer(
            host.core
                .memory
                .offset(entity, i64::from(self.profile.client))?,
        )?;
        client.ok_or_else(|| {
            NativeHostError::Fault("native inventory actor has no source client".to_string())
        })
    }

    /// Resolve the cursor to its canonical row, tracking selections.
    pub fn cursor(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        rows: &[NativeInventoryRow],
    ) -> HostResult<Option<NativeInventoryRow>> {
        self.check_host(host)?;
        let client = self.client(host, actor)?;
        let cursor = host
            .core
            .memory
            .offset(client, i64::from(self.profile.cursor))?;
        let index = host.core.memory.read_i32(cursor)?;
        if let Some((item, slot)) = self.selected.get(&actor) {
            if *slot as i32 == index {
                if let Some(row) = rows.iter().find(|row| &row.item == item) {
                    return Ok(Some(row.clone()));
                }
                self.selected.remove(&actor);
                host.core.memory.write_i32(cursor, self.profile.empty)?;
                return Ok(None);
            }
        }
        self.selected.remove(&actor);
        Ok(rows
            .iter()
            .find(|row| !row.selected && row.source_index as i32 == index)
            .cloned())
    }

    /// Enter an entry scope (headless entry wrapper).
    pub fn enter(&mut self, actor: Option<NativeActorId>, flags: i32, named: bool) {
        self.frames.push(Frame {
            actor,
            flags,
            named,
            row: None,
            restore: None,
        });
    }

    /// Leave an entry scope, restoring any named projection.
    pub fn exit(&mut self, host: &mut SyntheticHost) -> HostResult<()> {
        if let Some(frame) = self.frames.pop() {
            if let (Some(actor), Some((address, bytes))) = (frame.actor, frame.restore) {
                if self.current(host, actor) {
                    host.core.memory.write(address, &bytes)?;
                }
            }
        }
        Ok(())
    }

    /// Run the validate entry around the original with selected-row projection.
    pub fn validate_entry(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        run_original: impl FnOnce(&mut SyntheticHost) -> HostResult<()>,
    ) -> HostResult<()> {
        self.check_host(host)?;
        if self.evaluating != 0 {
            return run_original(host);
        }
        let rows = (self.hooks.rows)(actor);
        let chosen = match &rows {
            Some(rows) => self.cursor(host, actor, rows)?,
            None => None,
        };
        match chosen {
            Some(row) if row.selected => {
                let client = self.client(host, actor)?;
                let address = host.core.memory.offset(
                    client,
                    i64::from(self.profile.inventory) + i64::from(row.source_index) * 4,
                )?;
                let count = host.core.memory.read_i32(address)?;
                host.core.memory.write_i32(address, Self::source_count(&row))?;
                let outcome = run_original(host);
                if self.current(host, actor) {
                    host.core.memory.write_i32(address, count)?;
                }
                outcome
            }
            _ => run_original(host),
        }
    }

    /// Run an admitted scan region: navigate canonically and skip the original.
    /// The scan flags come from the current entry frame, defaulting to -1.
    pub fn admitted_scan(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        rows: &[NativeInventoryRow],
        direction: i32,
    ) -> HostResult<()> {
        self.check_host(host)?;
        if !self.current(host, actor) {
            return Err(NativeHostError::Fault(
                "admitted inventory scan lost its actor".to_string(),
            ));
        }
        let flags = self.frames.last().map_or(-1, |frame| frame.flags);
        self.navigate(host, actor, rows, direction, flags)
    }

    fn navigate(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        rows: &[NativeInventoryRow],
        direction: i32,
        flags: i32,
        ) -> HostResult<()> {
        let chosen = self.cursor(host, actor, rows)?;
        let start = match &chosen {
            None => {
                if direction > 0 {
                    -1
                } else {
                    0
                }
            }
            Some(row) => rows
                .iter()
                .position(|candidate| candidate.item == row.item && candidate.source_index == row.source_index)
                .unwrap_or(0) as i32,
        };
        let mut refused: HashMap<u32, Vec<i32>> = HashMap::new();
        for step in 1..=rows.len() {
            let index = (start + direction * step as i32).rem_euclid(rows.len() as i32) as usize;
            let row = rows.get(index).ok_or_else(|| {
                NativeHostError::Fault("native inventory traversal lost its row".to_string())
            })?;
            if row.count == 0
                || refused
                    .get(&row.source_index)
                    .is_some_and(|counts| counts.contains(&Self::source_count(row)))
            {
                continue;
            }
            if self.evaluate(host, actor, row, direction, flags)? {
                self.selected.insert(actor, (row.item.clone(), row.source_index));
                return Ok(());
            }
            refused
                .entry(row.source_index)
                .or_default()
                .push(Self::source_count(row));
        }
        let client = self.client(host, actor)?;
        let cursor = host
            .core
            .memory
            .offset(client, i64::from(self.profile.cursor))?;
        host.core.memory.write_i32(cursor, self.profile.empty)?;
        self.selected.remove(&actor);
        Ok(())
    }

    fn evaluate(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        row: &NativeInventoryRow,
        direction: i32,
        flags: i32,
    ) -> HostResult<bool> {
        let client = self.client(host, actor)?;
        let entity = host.core.entity_of(actor).map_err(|_| {
            NativeHostError::Fault("native inventory actor has no source client".to_string())
        })?;
        if row.source_index == 0 || row.source_index >= self.profile.count {
            return Err(NativeHostError::Fault(
                "canonical inventory row has no valid original item slot".to_string(),
            ));
        }
        let inventory = host
            .core
            .memory
            .offset(client, i64::from(self.profile.inventory))?;
        let saved_inventory = host
            .core
            .memory
            .copy(inventory, self.profile.count as usize * 4)?;
        let mut saved_selection = Vec::with_capacity(self.profile.selection_writes.len());
        for span in &self.profile.selection_writes {
            let address = host
                .core
                .memory
                .offset(client, i64::from(span.offset))?;
            saved_selection.push((address, host.core.memory.copy(address, span.bytes as usize)?));
        }
        self.evaluating += 1;
        let accepted = (|| {
            host.core.memory.write(inventory, &vec![0u8; saved_inventory.len()])?;
            host.core.memory.write_i32(
                host.core.memory.offset(
                    inventory,
                    i64::from(row.source_index) * 4,
                )?,
                Self::source_count(row),
            )?;
            host.core.memory.write_i32(
                host.core.memory.offset(client, i64::from(self.profile.cursor))?,
                0,
            )?;
            let mut values = vec![
                GuestCallValue::Pointer(Some(entity)),
                GuestCallValue::Int32(flags),
            ];
            if direction > 0 && self.profile.next.menu_argument {
                values.push(GuestCallValue::Uint32(0));
            }
            let entry = if direction > 0 {
                self.profile.next.entry
            } else {
                self.profile.previous.entry
            };
            host.invoke(host.core.at(entry)?, &values)?;
            Ok::<bool, NativeHostError>(
                host.core.memory.read_i32(
                    host.core.memory.offset(client, i64::from(self.profile.cursor))?,
                )? == row.source_index as i32,
            )
        })();
        self.evaluating -= 1;
        let accepted = accepted?;
        if self.current(host, actor) {
            host.core.memory.write(inventory, &saved_inventory)?;
            if !accepted {
                for (address, bytes) in saved_selection {
                    host.core.memory.write(address, &bytes)?;
                }
            }
        }
        Ok(accepted)
    }

    /// Run a use-call region; returns true when the canonical use skipped it.
    /// Named frames use their projected lookup row, plain frames the cursor.
    pub fn use_call(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        rows: &[NativeInventoryRow],
    ) -> HostResult<bool> {
        self.check_host(host)?;
        if !self.current(host, actor) {
            return Ok(false);
        }
        let named = self.frames.last().is_some_and(|frame| frame.named);
        let chosen = if named {
            self.frames.last().and_then(|frame| frame.row.clone())
        } else {
            self.cursor(host, actor, rows)?
        };
        match chosen {
            Some(row) if row.selected => {
                if let Some(frame) = self.frames.last_mut() {
                    if let (Some((address, bytes)), true) =
                        (frame.restore.take(), self.current(host, actor))
                    {
                        host.core.memory.write(address, &bytes)?;
                    }
                }
                (self.hooks.use_item)(actor, row.item);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Resolve a named-use lookup, projecting selected rows into the frame.
    pub fn named_lookup(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        text: &str,
        original: Option<GuestAddress>,
    ) -> HostResult<LookupOutcome> {
        self.check_host(host)?;
        if self.frames.last().is_some_and(|frame| !frame.named) {
            return Ok(LookupOutcome::PassThrough);
        }
        let original_item = match original {
            Some(address) => (self.hooks.item_at)(address),
            None => None,
        };
        let rows = (self.hooks.rows)(actor).unwrap_or_default();
        match source_item_named(&rows, text, original_item.as_ref()) {
            Some(ItemNameMatch::Ambiguous(items)) => {
                if original.is_none() {
                    let names: Vec<&str> = items.iter().map(|row| row.item.as_str()).collect();
                    (self.hooks.print)(
                        actor,
                        format!("Ambiguous item \"{text}\"; use {}\n", names.join(", ")),
                    );
                }
                Ok(LookupOutcome::PassThrough)
            }
            Some(ItemNameMatch::Match { row, .. }) if row.selected => {
                let client = self.client(host, actor)?;
                let address = host.core.memory.offset(
                    client,
                    i64::from(self.profile.inventory) + i64::from(row.source_index) * 4,
                )?;
                let count = host.core.memory.copy(address, 4)?;
                let descriptor = (self.hooks.descriptor)(row.source_index);
                host.core.memory.write_i32(address, Self::source_count(&row))?;
                if let Some(frame) = self.frames.last_mut() {
                    frame.row = Some(row.clone());
                    frame.restore = Some((address, count));
                }
                Ok(LookupOutcome::Project { row, descriptor })
            }
            _ => Ok(LookupOutcome::PassThrough),
        }
    }

    /// Read the canonical readout, tracking the cursor.
    pub fn read(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
    ) -> HostResult<Option<NativeInventoryReadout>> {
        self.check_host(host)?;
        if !self.current(host, actor) {
            return Ok(None);
        }
        let rows = (self.hooks.rows)(actor);
        let rows = match rows {
            Some(rows) => rows,
            None => {
                if let Some((_, index)) = self.selected.remove(&actor) {
                    let client = self.client(host, actor)?;
                    let cursor = host
                        .core
                        .memory
                        .offset(client, i64::from(self.profile.cursor))?;
                    if host.core.memory.read_i32(cursor)? == index as i32 {
                        host.core.memory.write_i32(cursor, self.profile.empty)?;
                    }
                }
                return Ok(None);
            }
        };
        let chosen = self.cursor(host, actor, &rows)?;
        Ok(Some(NativeInventoryReadout {
            presentation: chosen.as_ref().and_then(|row| row.presentation.clone()),
            items: rows
                .iter()
                .filter(|row| row.count != 0)
                .map(|row| ReadoutItem {
                    item: row.item.clone(),
                    label: row.label.clone(),
                    count: row.count,
                })
                .collect(),
            selected: chosen.map(|row| row.item),
        }))
    }

    /// Restore a saved cursor.
    pub fn restore(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        item: Option<&ItemId>,
    ) -> HostResult<()> {
        self.check_host(host)?;
        self.selected.remove(&actor);
        let client = self.client(host, actor)?;
        let cursor = host
            .core
            .memory
            .offset(client, i64::from(self.profile.cursor))?;
        match item {
            None => {
                host.core.memory.write_i32(cursor, self.profile.empty)?;
                Ok(())
            }
            Some(item) => {
                let row = (self.hooks.rows)(actor)
                    .and_then(|rows| rows.into_iter().find(|row| &row.item == item))
                    .ok_or_else(|| {
                        NativeHostError::Fault(
                            "saved native inventory cursor is absent from its canonical rows"
                                .to_string(),
                        )
                    })?;
                host.core.memory.write_i32(cursor, row.source_index as i32)?;
                self.selected.insert(actor, (item.clone(), row.source_index));
                Ok(())
            }
        }
    }

    /// Forget tracked selection for an actor.
    pub fn release(&mut self, actor: NativeActorId) {
        self.selected.remove(&actor);
    }

    /// Close the service.
    pub fn close(&mut self) {
        self.closed = true;
        self.selected.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::CLASSIC_DIGEST;
    use super::*;

    fn profile() -> NativePrimaryInventoryProfile {
        let item = |name: &str| format!("q2:{name}");
        NativePrimaryInventoryProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            client: 84,
            inventory: 0x100,
            count: 8,
            cursor: 0xF0,
            empty: -1,
            prototypes: InventoryPrototypes {
                weapon: item("weapon_blaster"),
                ammunition: item("ammo_shells"),
                usable: item("item_quad"),
                passive: item("key_data_cd"),
                droppable: item("item_quad"),
                undroppable: item("weapon_blaster"),
            },
            selection_writes: vec![SelectionWrite { offset: 0xF0, bytes: 4 }],
            next: NextProfile { entry: 0x100, scan: 0x110, join: 0x120, menu_argument: false },
            previous: PreviousProfile { entry: 0x130, scan: 0x140, join: 0x150 },
            validate: ValidateProfile { entry: 0x160, scan: None },
            use_profile: UseProfile { entry: 0x170, call: 0x180, join: 0x190 },
            named_use: NamedUseProfile {
                entry: 0x1A0,
                lookup_call: 0x1B0,
                lookup_return: 0x1C0,
                call: 0x1D0,
                join: 0x1E0,
            },
        }
    }

    fn row(item: &str, label: &str, count: i32, source_index: u32, selected: bool) -> NativeInventoryRow {
        NativeInventoryRow {
            presentation: None,
            item: format!("q2:{item}"),
            label: label.to_string(),
            count,
            source_index,
            selected,
            presence_only: false,
        }
    }

    fn rows() -> Vec<NativeInventoryRow> {
        vec![
            row("weapon_blaster", "Blaster", 1, 1, true),
            row("ammo_shells", "Shells", 10, 2, false),
            row("item_quad", "Quad", 0, 3, false),
        ]
    }

    fn linked(host: &mut SyntheticHost) -> NativeActorId {
        let actor = host.core.spawn_actor(896, 512).expect("actor");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host.core.client_of(actor).expect("client");
        host.core.set_client(entity, 84, client).expect("link");
        host.core
            .memory
            .write_i32(
                host.core.memory.offset(client.expect("client"), 0xF0).expect("cursor"),
                -1,
            )
            .expect("empty");
        actor
    }

    fn accept_original(host: &mut SyntheticHost, profile: &NativePrimaryInventoryProfile) {
        let inventory = profile.inventory;
        let cursor = profile.cursor;
        let count = profile.count;
        for entry in [profile.next.entry, profile.previous.entry] {
            host.on_rva(
                entry,
                Box::new(move |core, values| {
                    let entity = match values[0] {
                        GuestCallValue::Pointer(Some(address)) => address,
                        _ => return Err(NativeHostError::Fault("no entity".to_string())),
                    };
                    let client = core
                        .memory
                        .read_pointer(core.memory.offset(entity, 84)?)?
                        .expect("client");
                    for slot in 1..count {
                        let counter =
                            core.memory.offset(client, i64::from(inventory) + i64::from(slot) * 4)?;
                        if core.memory.read_i32(counter)? != 0 {
                            core.memory.write_i32(core.memory.offset(client, i64::from(cursor))?, slot as i32)?;
                            return Ok(qa_guest::core::contracts::GuestCallResult::Void);
                        }
                    }
                    Ok(qa_guest::core::contracts::GuestCallResult::Void)
                }),
            )
            .expect("handler");
        }
    }

    #[test]
    fn navigates_to_accepted_rows() {
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        accept_original(&mut host, &profile);
        let table = rows();
        let moved = table.clone();
        let hooks = InventoryHooks {
            rows: Box::new(move |_| Some(moved.clone())),
            use_item: Box::new(|_, _| {}),
            descriptor: Box::new(|_| GuestAddress { space: 0, offset: 0 }),
            item_at: Box::new(|_| None),
            print: Box::new(|_, _| {}),
        };
        let mut inventory = NativePrimaryInventory::new(profile, hooks);
        let actor = linked(&mut host);
        inventory.enter(Some(actor), 0, false);
        inventory.admitted_scan(&mut host, actor, &table, 1).expect("scan");
        inventory.exit(&mut host).expect("exit");
        let readout = inventory.read(&mut host, actor).expect("read").expect("rows");
        assert_eq!(readout.selected.as_deref(), Some("q2:weapon_blaster"));
        assert_eq!(readout.items.len(), 2);
    }

    #[test]
    fn projects_named_lookups_and_reports_ambiguity() {
        use std::sync::{Arc, Mutex};
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let table = rows();
        let moved = table.clone();
        let printed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = printed.clone();
        let hooks = InventoryHooks {
            rows: Box::new(move |_| Some(moved.clone())),
            use_item: Box::new(|_, _| {}),
            descriptor: Box::new(|index| GuestAddress { space: 1, offset: u64::from(index) * 32 }),
            item_at: Box::new(|_| None),
            print: Box::new(move |_, text| capture.lock().expect("lock").push(text)),
        };
        let mut inventory = NativePrimaryInventory::new(profile, hooks);
        let actor = linked(&mut host);
        inventory.enter(Some(actor), 0, true);
        let outcome = inventory.named_lookup(&mut host, actor, "blaster", None).expect("lookup");
        match outcome {
            LookupOutcome::Project { row, descriptor } => {
                assert_eq!(row.item, "q2:weapon_blaster");
                assert_eq!(descriptor.offset, 32);
            }
            LookupOutcome::PassThrough => panic!("expected projection"),
        }
        let used = inventory.use_call(&mut host, actor, &table).expect("use");
        assert!(used);
        inventory.exit(&mut host).expect("exit");
        inventory.enter(Some(actor), 0, false);
        let skipped = inventory.use_call(&mut host, actor, &table).expect("use");
        assert!(!skipped);
        inventory.exit(&mut host).expect("exit");
        let outcome = inventory.named_lookup(&mut host, actor, "s", None).expect("lookup");
        assert_eq!(outcome, LookupOutcome::PassThrough);
        assert!(printed.lock().expect("lock").is_empty());
    }

    #[test]
    fn validates_selected_rows_with_restoration() {
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let table = rows();
        let moved = table.clone();
        let hooks = InventoryHooks {
            rows: Box::new(move |_| Some(moved.clone())),
            use_item: Box::new(|_, _| {}),
            descriptor: Box::new(|_| GuestAddress { space: 0, offset: 0 }),
            item_at: Box::new(|_| None),
            print: Box::new(|_, _| {}),
        };
        let mut inventory = NativePrimaryInventory::new(profile, hooks);
        let actor = linked(&mut host);
        inventory.restore(&mut host, actor, Some(&"q2:weapon_blaster".to_string())).expect("restore");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host.core.memory.read_pointer(host.core.memory.offset(entity, 84).expect("link")).expect("read").expect("client");
        let counter = host.core.memory.offset(client, 0x100 + 4).expect("counter");
        host.core.memory.write_i32(counter, 0).expect("zero");
        inventory
            .validate_entry(&mut host, actor, |host| {
                assert_eq!(host.core.memory.read_i32(counter).expect("count"), 1);
                Ok(())
            })
            .expect("validate");
        assert_eq!(host.core.memory.read_i32(counter).expect("restored"), 0);
        let readout = inventory.read(&mut host, actor).expect("read").expect("rows");
        assert_eq!(readout.selected.as_deref(), Some("q2:weapon_blaster"));
        inventory.release(actor);
        inventory.close();
    }

    #[test]
    fn resolves_names_like_the_source_matcher() {
        let table = rows();
        let exact = source_item_named(&table, "Q2:WEAPON_blaster", None).expect("exact");
        assert!(matches!(exact, ItemNameMatch::Match { exact: true, .. }));
        let label = source_item_named(&table, "shells", None).expect("label");
        assert!(matches!(label, ItemNameMatch::Match { exact: false, .. }));
        assert!(source_item_named(&table, "nope", None).is_none());
        let doubled = vec![
            row("a_one", "Same", 1, 1, false),
            row("a_two", "Same", 1, 2, false),
        ];
        assert!(matches!(
            source_item_named(&doubled, "same", None),
            Some(ItemNameMatch::Ambiguous(_))
        ));
    }
}
