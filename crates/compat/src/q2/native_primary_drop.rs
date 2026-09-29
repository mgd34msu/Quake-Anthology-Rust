//! Port of `src/compat/q2/native-primary-drop.ts`.
//! Bridges original drop commands: guards, item callbacks and accepted decrements.

use qa_guest::core::contracts::{GuestAddress, GuestCallValue, NativeAbi};
use qa_world::combat::ItemId;

use super::native_primary_commands::CommandItem;
use super::native_primary_inventory::{
    ItemNameMatch, NativeInventoryRow, source_item_named,
};
use super::native_primary_reader::NativeRegion;
use super::native_primary_weapons::{HostResult, NativeActorId, NativeHostError, SyntheticHost};

/// Client offsets used by drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropClient {
    /// Client pointer offset within the entity.
    pub pointer: u32,
    /// Inventory offset within the client.
    pub inventory: u32,
    /// Cursor offset within the client.
    pub cursor: u32,
    /// Current weapon offset within the client.
    pub weapon: u32,
    /// Pending weapon offset within the client.
    pub pending: u32,
}

/// Inventory-drop entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryDrop {
    /// Entry RVA.
    pub entry: u32,
    /// Admitted-cursor observation RVA.
    pub admitted: u32,
}

/// Native primary drop profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePrimaryDropProfile {
    /// Artifact digest.
    pub digest: String,
    /// Executable ABI.
    pub abi: NativeAbi,
    /// Client offsets.
    pub client: DropClient,
    /// Named-drop entry RVA.
    pub named: u32,
    /// Inventory drop.
    pub inventory: InventoryDrop,
    /// Item-find entry RVA.
    pub find: u32,
    /// Find lookup-return RVA.
    pub lookup_return: u32,
    /// Allocation entry RVA.
    pub allocate: u32,
    /// Free entry RVA.
    pub free: u32,
    /// Optional consumer region.
    pub consumer: Option<NativeRegion>,
    /// Item callback regions.
    pub callbacks: Vec<NativeRegion>,
    /// Debit regions.
    pub debits: Vec<NativeRegion>,
}

/// Source projection for one drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropProjection {
    /// Source item descriptor.
    pub source: ItemId,
    /// Projected current weapon.
    pub current: Option<ItemId>,
    /// Projected pending weapon.
    pub pending: Option<ItemId>,
}

/// Approval gate for the consumer continuation. The hook calls
/// [`ConsumerGate::execute`] to approve the original continuation; the service
/// runs it afterwards since headless originals borrow the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerGate {
    /// Whether the hook approved the continuation.
    pub executed: bool,
}

impl ConsumerGate {
    /// Approve the consumer continuation.
    pub fn execute(&mut self) {
        self.executed = true;
    }
}

/// Canonical drop action replacing an original callback.
pub type DropAction = Box<dyn FnOnce()>;

/// Drop hooks owned by the caller.
pub struct DropHooks {
    /// Canonical rows, or `None` without a canonical inventory.
    pub rows: Box<dyn FnMut(NativeActorId) -> Option<Vec<NativeInventoryRow>>>,
    /// Canonical selection.
    pub selected: Box<dyn FnMut(NativeActorId) -> Option<ItemId>>,
    /// Source projection for a dropped item.
    pub projection: Box<dyn FnMut(NativeActorId, ItemId) -> DropProjection>,
    /// Accepted drop notice; returns false to refuse (frees the pickup).
    pub dropped: Box<dyn FnMut(NativeActorId, NativeActorId, ItemId, i32) -> bool>,
    /// Consumer continuation approval.
    pub consume: Box<dyn FnMut(NativeActorId, &mut ConsumerGate)>,
    /// Canonical action replacing an original callback.
    pub action: Box<dyn FnMut(NativeActorId, ItemId) -> Option<DropAction>>,
    /// Client print.
    pub print: Box<dyn FnMut(NativeActorId, String)>,
}

/// Drop frame kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropKind {
    /// Named drop (`drop <item>`).
    Named,
    /// Inventory (cursor) drop.
    Inventory,
}

/// Debit state of one drop frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebitState {
    /// Debit region has not run.
    Pending,
    /// Drop accepted by the caller.
    Accepted,
    /// Drop refused; the pickup was freed.
    Refused,
}

struct DropFrame {
    actor: Option<NativeActorId>,
    kind: DropKind,
    row: Option<NativeInventoryRow>,
    counter: Option<GuestAddress>,
    restore: Option<Vec<(GuestAddress, Vec<u8>)>>,
    pickup: Option<NativeActorId>,
    debit: DebitState,
}

/// Named-find outcome for the emulated original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindOutcome {
    /// Return the projected descriptor.
    Project {
        /// Projected source descriptor.
        descriptor: GuestAddress,
    },
    /// Run the original find untouched.
    PassThrough,
}

/// Original command guards, item callbacks and accepted decrements own each
/// drop. Entry wrappers are explicit methods; tests emulate originals.
pub struct NativePrimaryDrop {
    profile: NativePrimaryDropProfile,
    command_items: Vec<CommandItem>,
    hooks: DropHooks,
    frames: Vec<DropFrame>,
    closed: bool,
}

impl NativePrimaryDrop {
    /// Build the service over a snapshot of the command item table.
    pub fn new(
        profile: NativePrimaryDropProfile,
        command_items: Vec<CommandItem>,
        hooks: DropHooks,
    ) -> Self {
        Self {
            profile,
            command_items,
            hooks,
            frames: Vec::new(),
            closed: false,
        }
    }

    /// Borrow the profile.
    #[must_use]
    pub fn profile(&self) -> &NativePrimaryDropProfile {
        &self.profile
    }

    fn check_host(&self, host: &SyntheticHost) -> HostResult<()> {
        if self.closed {
            return Err(NativeHostError::Fault("native drop service is closed".to_string()));
        }
        if host.core.digest != self.profile.digest {
            return Err(NativeHostError::Fault(
                "native drop profile belongs to another artifact".to_string(),
            ));
        }
        Ok(())
    }

    fn current(&self, host: &SyntheticHost, actor: NativeActorId) -> bool {
        !self.closed && host.core.is_live(actor)
    }

    /// Enter a drop scope (headless entry wrapper).
    pub fn begin(&mut self, actor: Option<NativeActorId>, kind: DropKind) {
        self.frames.push(DropFrame {
            actor,
            kind,
            row: None,
            counter: None,
            restore: None,
            pickup: None,
            debit: DebitState::Pending,
        });
    }

    fn restore_frame(&self, host: &mut SyntheticHost, frame: &mut DropFrame) -> HostResult<()> {
        if let Some(saved) = frame.restore.take() {
            frame.counter = None;
            if let Some(actor) = frame.actor {
                if self.current(host, actor) {
                    for (address, bytes) in saved {
                        host.core.memory.write(address, &bytes)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Leave a drop scope, restoring any projection.
    pub fn end(&mut self, host: &mut SyntheticHost) -> HostResult<()> {
        if let Some(mut frame) = self.frames.pop() {
            self.restore_frame(host, &mut frame)?;
        }
        Ok(())
    }

    /// Resolve a named find, projecting selected rows into the frame.
    pub fn find_named(
        &mut self,
        host: &mut SyntheticHost,
        text: &str,
        original: Option<GuestAddress>,
    ) -> HostResult<FindOutcome> {
        self.check_host(host)?;
        let (actor, named) = match self.frames.last() {
            Some(frame) if frame.kind == DropKind::Named => (frame.actor, true),
            _ => (None, false),
        };
        let actor = match actor {
            Some(actor) if named && self.current(host, actor) => actor,
            _ => return Ok(FindOutcome::PassThrough),
        };
        let original_item = original
            .and_then(|address| self.command_items.iter().find(|row| row.address == address))
            .map(|row| row.item.clone());
        let rows = (self.hooks.rows)(actor).unwrap_or_default();
        match source_item_named(&rows, &text.to_lowercase(), original_item.as_ref()) {
            Some(ItemNameMatch::Ambiguous(items)) => {
                if original.is_none() {
                    let names: Vec<&str> = items.iter().map(|row| row.item.as_str()).collect();
                    (self.hooks.print)(
                        actor,
                        format!("Ambiguous item \"{text}\"; use {}\n", names.join(", ")),
                    );
                }
                Ok(FindOutcome::PassThrough)
            }
            Some(ItemNameMatch::Match { row, .. }) if row.selected => {
                let descriptor = self.project(host, actor, &row, false)?;
                Ok(FindOutcome::Project { descriptor })
            }
            _ => Ok(FindOutcome::PassThrough),
        }
    }

    /// Observe the admitted inventory cursor, projecting the selected row.
    pub fn observe_admitted(&mut self, host: &mut SyntheticHost) -> HostResult<()> {
        self.check_host(host)?;
        let actor = match self.frames.last() {
            Some(frame) if frame.kind == DropKind::Inventory => frame.actor,
            _ => None,
        };
        let actor = match actor {
            Some(actor) if self.current(host, actor) => actor,
            _ => return Ok(()),
        };
        let chosen = (self.hooks.selected)(actor);
        let row = (self.hooks.rows)(actor)
            .and_then(|rows| {
                rows.into_iter()
                    .find(|row| Some(&row.item) == chosen.as_ref() && row.selected)
            });
        if let Some(row) = row {
            self.project(host, actor, &row, true)?;
        }
        Ok(())
    }

    /// Observe an allocation result for the current frame.
    pub fn note_allocate(
        &mut self,
        host: &mut SyntheticHost,
        owner: Option<NativeActorId>,
        result: Option<GuestAddress>,
    ) -> HostResult<()> {
        self.check_host(host)?;
        let frame = match self.frames.last_mut() {
            Some(frame) if frame.counter.is_some() => frame,
            _ => return Ok(()),
        };
        if let (Some(actor), Some(result)) = (frame.actor, result) {
            if owner == Some(actor) {
                frame.pickup = host.core.actor_for(result);
            }
        }
        Ok(())
    }

    /// Run a callback region; returns true when a canonical action skipped it.
    pub fn callback_region(&mut self, host: &mut SyntheticHost) -> HostResult<bool> {
        self.check_host(host)?;
        let (actor, item) = match self.frames.last() {
            Some(frame)
                if frame.row.is_some() && frame.actor.is_some_and(|actor| self.current(host, actor)) =>
            {
                (frame.actor.expect("actor"), frame.row.clone().expect("row").item)
            }
            _ => return Ok(false),
        };
        match (self.hooks.action)(actor, item) {
            Some(action) => {
                if let Some(frame) = self.frames.last_mut() {
                    self.restore_frame(host, frame)?;
                }
                action();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Run a debit region around the original decrement.
    pub fn debit_region(
        &mut self,
        host: &mut SyntheticHost,
        execute: impl FnOnce(&mut SyntheticHost) -> HostResult<()>,
    ) -> HostResult<DebitState> {
        self.check_host(host)?;
        let (actor, counter, row) = match self.frames.last() {
            Some(frame)
                if frame.counter.is_some()
                    && frame.row.is_some()
                    && frame.actor.is_some_and(|actor| self.current(host, actor)) =>
            {
                (
                    frame.actor.expect("actor"),
                    frame.counter.expect("counter"),
                    frame.row.clone().expect("row"),
                )
            }
            _ => {
                execute(&mut *host)?;
                return Ok(DebitState::Pending);
            }
        };
        let before = host.core.memory.read_i32(counter)?;
        execute(&mut *host)?;
        let after = host.core.memory.read_i32(counter)?;
        if let Some(frame) = self.frames.last_mut() {
            self.restore_frame(host, frame)?;
        }
        let count = before - after;
        let pickup = self.frames.last().and_then(|frame| frame.pickup);
        match pickup {
            Some(pickup) if count > 0 && self.current(host, pickup) => {
                let accepted = (self.hooks.dropped)(actor, pickup, row.item, count);
                let debit = if accepted {
                    DebitState::Accepted
                } else {
                    DebitState::Refused
                };
                if let Some(frame) = self.frames.last_mut() {
                    frame.debit = debit;
                }
                if !accepted {
                    if let Ok(entity) = host.core.entity_of(pickup) {
                        let free = host.core.at(self.profile.free)?;
                        host.invoke(free, &[GuestCallValue::Pointer(Some(entity))])?;
                    }
                }
                Ok(debit)
            }
            _ => Err(NativeHostError::Fault(
                "accepted original drop lacks its source allocation or positive debit".to_string(),
            )),
        }
    }

    /// Run the consumer region; returns true when the original ran.
    pub fn consumer_region(
        &mut self,
        host: &mut SyntheticHost,
        execute: impl FnOnce(&mut SyntheticHost) -> HostResult<()>,
    ) -> HostResult<bool> {
        self.check_host(host)?;
        if self.profile.consumer.is_none() {
            execute(host)?;
            return Ok(true);
        }
        let (actor, debit) = match self.frames.last() {
            Some(frame) => (frame.actor, frame.debit),
            None => {
                execute(host)?;
                return Ok(true);
            }
        };
        if debit == DebitState::Refused {
            return Ok(false);
        }
        let actor = match actor {
            Some(actor) if debit == DebitState::Accepted && self.current(host, actor) => actor,
            _ => {
                execute(host)?;
                return Ok(true);
            }
        };
        let mut gate = ConsumerGate { executed: false };
        (self.hooks.consume)(actor, &mut gate);
        if gate.executed {
            execute(host)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn source_weapon(&self, item: Option<&ItemId>) -> HostResult<Option<GuestAddress>> {
        match item {
            None => Ok(None),
            Some(item) => match self.command_items.iter().find(|row| &row.item == item) {
                Some(row) => Ok(Some(row.address)),
                None => Err(NativeHostError::Fault(
                    "selected drop weapon has no source descriptor".to_string(),
                )),
            },
        }
    }

    fn project(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        row: &NativeInventoryRow,
        cursor: bool,
    ) -> HostResult<GuestAddress> {
        if self.frames.last().is_some_and(|frame| frame.restore.is_some()) {
            return Err(NativeHostError::Fault(
                "original drop projection has no unique actor scope".to_string(),
            ));
        }
        let projection = (self.hooks.projection)(actor, row.item.clone());
        let descriptor = self
            .command_items
            .iter()
            .find(|row| row.item == projection.source)
            .cloned()
            .ok_or_else(|| {
                NativeHostError::Fault("selected drop has no source item or client".to_string())
            })?;
        let entity = host.core.entity_of(actor).map_err(|_| {
            NativeHostError::Fault("selected drop has no source item or client".to_string())
        })?;
        let client = host.core.memory.read_pointer(
            host.core
                .memory
                .offset(entity, i64::from(self.profile.client.pointer))?,
        )?;
        let client = client.ok_or_else(|| {
            NativeHostError::Fault("selected drop has no source item or client".to_string())
        })?;
        let counter = host.core.memory.offset(
            client,
            i64::from(self.profile.client.inventory) + i64::from(descriptor.index) * 4,
        )?;
        let weapon = host
            .core
            .memory
            .offset(client, i64::from(self.profile.client.weapon))?;
        let pending = host
            .core
            .memory
            .offset(client, i64::from(self.profile.client.pending))?;
        let selection = host
            .core
            .memory
            .offset(client, i64::from(self.profile.client.cursor))?;
        let mut saved = vec![
            (counter, host.core.memory.copy(counter, 4)?),
            (weapon, host.core.memory.copy(weapon, host.core.pointer_bytes())?),
            (pending, host.core.memory.copy(pending, host.core.pointer_bytes())?),
        ];
        if cursor {
            saved.push((selection, host.core.memory.copy(selection, 4)?));
        }
        let current = self.source_weapon(projection.current.as_ref())?;
        let pending_weapon = self.source_weapon(projection.pending.as_ref())?;
        host.core.memory.write_i32(
            counter,
            if row.presence_only {
                i32::from(row.count != 0)
            } else {
                row.count
            },
        )?;
        host.core.memory.write_pointer(weapon, current)?;
        host.core.memory.write_pointer(pending, pending_weapon)?;
        if cursor {
            host.core.memory.write_i32(selection, descriptor.index as i32)?;
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.row = Some(row.clone());
            frame.counter = Some(counter);
            frame.restore = Some(saved);
        }
        Ok(descriptor.address)
    }

    /// Close the service, restoring every frame.
    pub fn close(&mut self, host: &mut SyntheticHost) -> HostResult<()> {
        while let Some(mut frame) = self.frames.pop() {
            self.restore_frame(host, &mut frame)?;
        }
        self.closed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::CLASSIC_DIGEST;
    use super::*;

    fn profile() -> NativePrimaryDropProfile {
        NativePrimaryDropProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            client: DropClient {
                pointer: 84,
                inventory: 0x100,
                cursor: 0xF0,
                weapon: 0x80,
                pending: 0x88,
            },
            named: 0x100,
            inventory: InventoryDrop { entry: 0x110, admitted: 0x120 },
            find: 0x130,
            lookup_return: 0x140,
            allocate: 0x150,
            free: 0x160,
            consumer: Some(NativeRegion { entry: 0x170, join: 0x180 }),
            callbacks: vec![NativeRegion { entry: 0x190, join: 0x1A0 }],
            debits: vec![NativeRegion { entry: 0x1B0, join: 0x1C0 }],
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
        ]
    }

    fn items(host: &mut SyntheticHost) -> Vec<CommandItem> {
        vec![
            CommandItem {
                item: "q2:weapon_blaster".to_string(),
                index: 1,
                address: host.core.at(0x400).expect("descriptor"),
                weapon: true,
                ammunition: false,
                icon: String::new(),
                ammo: Some("q2:ammo_shells".to_string()),
            },
            CommandItem {
                item: "q2:ammo_shells".to_string(),
                index: 2,
                address: host.core.at(0x420).expect("descriptor"),
                weapon: false,
                ammunition: true,
                icon: String::new(),
                ammo: None,
            },
        ]
    }

    fn hooks() -> DropHooks {
        let table = rows();
        let moved = table.clone();
        DropHooks {
            rows: Box::new(move |_| Some(moved.clone())),
            selected: Box::new(|_| Some("q2:weapon_blaster".to_string())),
            projection: Box::new(|_, item| DropProjection {
                source: item,
                current: Some("q2:weapon_blaster".to_string()),
                pending: None,
            }),
            dropped: Box::new(|_, _, _, _| true),
            consume: Box::new(|_, gate| gate.execute()),
            action: Box::new(|_, _| None),
            print: Box::new(|_, _| {}),
        }
    }

    fn linked(host: &mut SyntheticHost) -> NativeActorId {
        let actor = host.core.spawn_actor(896, 512).expect("actor");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host.core.client_of(actor).expect("client");
        host.core.set_client(entity, 84, client).expect("link");
        actor
    }

    #[test]
    fn projects_named_finds_with_restoration() {
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let table = items(&mut host);
        let mut drop = NativePrimaryDrop::new(profile, table, hooks());
        let actor = linked(&mut host);
        drop.begin(Some(actor), DropKind::Named);
        let outcome = drop.find_named(&mut host, "blaster", None).expect("find");
        let descriptor = match outcome {
            FindOutcome::Project { descriptor } => descriptor,
            FindOutcome::PassThrough => panic!("expected projection"),
        };
        assert_eq!(descriptor, host.core.at(0x400).expect("rva"));
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host
            .core
            .memory
            .read_pointer(host.core.memory.offset(entity, 84).expect("link"))
            .expect("read")
            .expect("client");
        let counter = host.core.memory.offset(client, 0x104).expect("counter");
        assert_eq!(host.core.memory.read_i32(counter).expect("count"), 1);
        let weapon = host.core.memory.offset(client, 0x80).expect("weapon");
        assert!(host.core.memory.read_pointer(weapon).expect("read").is_some());
        assert!(!drop.callback_region(&mut host).expect("callback"));
        drop.end(&mut host).expect("end");
        assert_eq!(host.core.memory.read_i32(counter).expect("restored"), 0);
        assert!(host.core.memory.read_pointer(weapon).expect("read").is_none());
    }

    #[test]
    fn accepts_debits_and_runs_consumers() {
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let table = items(&mut host);
        let mut drop = NativePrimaryDrop::new(profile, table, hooks());
        let actor = linked(&mut host);
        let pickup = linked(&mut host);
        drop.begin(Some(actor), DropKind::Inventory);
        drop.observe_admitted(&mut host).expect("admitted");
        let pickup_entity = host.core.entity_of(pickup).expect("entity");
        drop.note_allocate(&mut host, Some(actor), Some(pickup_entity)).expect("allocate");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host
            .core
            .memory
            .read_pointer(host.core.memory.offset(entity, 84).expect("link"))
            .expect("read")
            .expect("client");
        let counter = host.core.memory.offset(client, 0x104).expect("counter");
        let debit = drop
            .debit_region(&mut host, |host| {
                let before = host.core.memory.read_i32(counter).expect("before");
                host.core.memory.write_i32(counter, before - 1).expect("debit");
                Ok(())
            })
            .expect("debit");
        assert_eq!(debit, DebitState::Accepted);
        let ran = drop
            .consumer_region(&mut host, |_| Ok(()))
            .expect("consumer");
        assert!(ran);
        drop.end(&mut host).expect("end");
    }

    #[test]
    fn refuses_debits_by_freeing_pickups() {
        use std::sync::{Arc, Mutex};
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let table = items(&mut host);
        let freed: Arc<Mutex<Vec<GuestCallValue>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = freed.clone();
        host.on_rva(
            0x160,
            Box::new(move |_, values| {
                *capture.lock().expect("lock") = values.to_vec();
                Ok(qa_guest::core::contracts::GuestCallResult::Void)
            }),
        )
        .expect("handler");
        let mut drop_hooks = hooks();
        drop_hooks.dropped = Box::new(|_, _, _, _| false);
        let mut drop = NativePrimaryDrop::new(profile, table, drop_hooks);
        let actor = linked(&mut host);
        let pickup = linked(&mut host);
        drop.begin(Some(actor), DropKind::Inventory);
        drop.observe_admitted(&mut host).expect("admitted");
        let pickup_entity = host.core.entity_of(pickup).expect("entity");
        drop.note_allocate(&mut host, Some(actor), Some(pickup_entity)).expect("allocate");
        let debit = drop
            .debit_region(&mut host, |host| {
                let entity = host.core.entity_of(actor).expect("entity");
                let client = host
                    .core
                    .memory
                    .read_pointer(host.core.memory.offset(entity, 84).expect("link"))
                    .expect("read")
                    .expect("client");
                let counter = host.core.memory.offset(client, 0x104).expect("counter");
                host.core.memory.write_i32(counter, 0).expect("debit");
                Ok(())
            })
            .expect("debit");
        assert_eq!(debit, DebitState::Refused);
        assert_eq!(freed.lock().expect("lock").len(), 1);
        let ran = drop
            .consumer_region(&mut host, |_| Ok(()))
            .expect("consumer");
        assert!(!ran);
        drop.close(&mut host).expect("close");
    }

    #[test]
    fn reports_ambiguous_names_without_projecting() {
        use std::sync::{Arc, Mutex};
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let table = items(&mut host);
        let printed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = printed.clone();
        let mut drop_hooks = hooks();
        drop_hooks.rows = Box::new(|_| {
            Some(vec![
                row("a_one", "Same", 1, 1, true),
                row("a_two", "Same", 1, 2, false),
            ])
        });
        drop_hooks.print = Box::new(move |_, text| capture.lock().expect("lock").push(text));
        let mut drop = NativePrimaryDrop::new(profile, table, drop_hooks);
        let actor = linked(&mut host);
        drop.begin(Some(actor), DropKind::Named);
        let outcome = drop.find_named(&mut host, "same", None).expect("find");
        assert_eq!(outcome, FindOutcome::PassThrough);
        assert_eq!(printed.lock().expect("lock").len(), 1);
        drop.end(&mut host).expect("end");
    }
}
