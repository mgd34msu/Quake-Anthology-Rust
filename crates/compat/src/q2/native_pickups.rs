//! Port of `src/compat/q2/native-primary-pickups.ts`.
//! Bridges original pickup touch/grant/supply calls over synthetic entities.

use qa_guest::core::contracts::{
    GuestAddress, GuestCallResult, GuestCallValue, GuestCallSignature, GuestRegister, NativeAbi,
};
use qa_world::combat::ItemId;

use super::native_primary_reader::NativeRegion;
use super::native_primary_weapons::{HostResult, NativeActorId, NativeHostError, SyntheticHost};

/// Grant resource channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupResource {
    /// Regular (protection) resource.
    Regular,
    /// Inventory resource.
    Inventory,
}

/// Protection channel of one pickup consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionChannel {
    /// Regular armor channel.
    Regular,
    /// Powered channel.
    Powered,
}

/// One protection consumer invoked around a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupConsumer {
    /// Consumer entry RVA.
    pub entry: u32,
    /// Consumer signature.
    pub signature: GuestCallSignature,
    /// Protection channel.
    pub protection: ProtectionChannel,
}

/// Supply capture mode of one grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupSupply {
    /// Ammo supply captured at a region entry with an amount register.
    Ammo {
        /// Capture entry RVA.
        entry: u32,
        /// Amount register.
        amount: GuestRegister,
    },
    /// Weapon supply settled through ammo capture and autoswitch skip.
    Weapon {
        /// Ammo-return observation RVA.
        ammo_return: u32,
        /// Settle observation RVA.
        settle: u32,
        /// Autoswitch skip region.
        autoswitch: NativeRegion,
    },
}

/// One original pickup grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePickupGrant {
    /// Grant entry RVA.
    pub entry: u32,
    /// Recipient skip region.
    pub recipient: NativeRegion,
    /// Default resource channel.
    pub resource: PickupResource,
    /// Protection consumers.
    pub consumers: Vec<PickupConsumer>,
    /// Supply capture mode.
    pub supply: Option<PickupSupply>,
}

/// Source item table view for pickups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupItems {
    /// Table base RVA.
    pub table: u32,
    /// Row stride in bytes.
    pub stride: u32,
    /// Row count.
    pub count: u32,
    /// Classname pointer offset.
    pub classname: u32,
    /// Pickup-function pointer offset.
    pub pickup: u32,
}

/// Entity offsets for pickups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item descriptor offset.
    pub item: u32,
    /// Count offset.
    pub count: u32,
    /// Spawnflags offset.
    pub spawnflags: u32,
    /// Inuse offset.
    pub inuse: u32,
    /// Inuse width: 1 or 4 bytes.
    pub inuse_bytes: u8,
    /// Generation offset, when stored.
    pub generation: Option<u32>,
}

/// Source clock storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeStorage {
    /// Float seconds.
    FloatSeconds,
    /// Integer milliseconds.
    Int64Milliseconds,
}

/// Source clock declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupTime {
    /// Clock address RVA.
    pub address: u32,
    /// Clock storage.
    pub storage: TimeStorage,
}

/// Ammo supply declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmoSupply {
    /// Ammo-grant entry RVA.
    pub entry: u32,
    /// Ammo-grant signature.
    pub signature: GuestCallSignature,
    /// Optional stop RVA following successful bookkeeping.
    pub stop: Option<u32>,
    /// Ammo tag offset within the descriptor.
    pub tag: u32,
    /// Capacity offsets within the client per tag.
    pub capacities: Vec<u32>,
    /// Capacity width: 2 or 4 bytes.
    pub capacity_bytes: u8,
}

/// Supply profile shared with commands and inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupSupplyProfile {
    /// Client pointer offset within the entity.
    pub client: u32,
    /// Inventory offset within the client.
    pub inventory: u32,
    /// Flags offset within the descriptor.
    pub flags: u32,
    /// Weapon flag bit.
    pub weapon_flag: u32,
    /// Ammo supply.
    pub ammo: AmmoSupply,
}

/// Native pickup profile. Each region leaves the original stack, saved
/// registers and map continuation intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePickupProfile {
    /// Artifact digest.
    pub digest: String,
    /// Executable ABI.
    pub abi: NativeAbi,
    /// Touch entry RVA.
    pub touch: u32,
    /// Grant return RVA.
    pub grant_return: u32,
    /// Targets return RVA.
    pub targets_return: u32,
    /// Touch signature.
    pub touch_signature: GuestCallSignature,
    /// Grant signature.
    pub grant_signature: GuestCallSignature,
    /// Original grants.
    pub grants: Vec<NativePickupGrant>,
    /// Item table.
    pub items: PickupItems,
    /// Entity offsets.
    pub entity: PickupEntity,
    /// Source clock.
    pub time: PickupTime,
    /// Supply profile.
    pub supply: PickupSupplyProfile,
}

/// Default resource of one pickup offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferResource {
    /// Protection channel resource.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory item resource.
    Inventory {
        /// Item.
        item: ItemId,
    },
}

/// Pickup count override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupCount {
    /// Default count.
    Default,
    /// Explicit count.
    Override(i32),
}

/// Pickup clock reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickupTimeValue {
    /// Seconds.
    Seconds(f64),
    /// Milliseconds.
    Milliseconds(i64),
}

/// One source pickup offer.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupOffer {
    /// Recipient actor.
    pub recipient: NativeActorId,
    /// Pickup actor.
    pub pickup: NativeActorId,
    /// Source module id.
    pub source: String,
    /// Canonical item.
    pub item: ItemId,
    /// Default resource.
    pub default_resource: OfferResource,
    /// Count override.
    pub count: PickupCount,
    /// Dropped (not map-placed) item.
    pub dropped: bool,
    /// Clock reading.
    pub time: PickupTimeValue,
}

/// Admission selection for one offer.
pub enum PickupSelection {
    /// Stale; touch with no effect.
    Stale,
    /// Blocked; grant refuses.
    Blocked,
    /// Original; run the original grant untouched.
    Original,
    /// Replacement owned by the caller.
    Replacement {
        /// Resource grant.
        grant: Box<dyn FnMut() -> PickupOutcome>,
        /// Replacement liveness.
        current: Box<dyn Fn() -> bool>,
    },
}

/// Resource grant outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupOutcome {
    /// Resources granted.
    Accepted,
    /// Resources refused.
    Refused,
    /// Stale; cancel the touch.
    Stale,
}

/// Touch description outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum TouchDescribe {
    /// Run the original touch without a frame.
    PassThrough,
    /// Stale records; touch with no effect.
    Stale,
    /// Described offer pushed as the current frame.
    Offer {
        /// Pickup offer.
        offer: PickupOffer,
        /// Grant index.
        grant_index: usize,
        /// Item descriptor.
        descriptor: GuestAddress,
    },
}

/// Grant outcome for the emulated original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantOutcome {
    /// Grant an int32 1.
    Accepted,
    /// Grant an int32 0.
    Refused,
    /// Cancel the touch.
    Cancelled,
}

/// Supply offer settled from a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupplyOffer {
    /// Weapon offer with bundled ammo.
    Weapon {
        /// Weapon item.
        item: ItemId,
        /// Bundled ammo.
        ammo: Vec<(ItemId, i32)>,
    },
    /// Ammo offer.
    Ammo {
        /// Ammo item.
        item: ItemId,
        /// Amount.
        amount: i32,
    },
    /// Ammo offer from a weapon descriptor.
    AmmoWeapon {
        /// Ammo item.
        item: ItemId,
        /// Amount.
        amount: i32,
        /// Weapon item.
        weapon: ItemId,
    },
}

/// Captured supply ammo for quantity projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupplyAmmo {
    /// Ammo descriptor.
    pub descriptor: GuestAddress,
    /// Offered amount.
    pub amount: i32,
}

/// Settled supply evaluation held by a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyEvaluation {
    /// Supply offer.
    pub offer: SupplyOffer,
    /// Captured ammo for quantity projections.
    pub ammo: Option<SupplyAmmo>,
}

/// Projected quantity result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupQuantity {
    /// Granted amount.
    pub amount: i32,
    /// Whether the original accepted.
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    Original,
    Replacement,
    Blocked,
}

struct PickupFrame {
    pickup: GuestAddress,
    recipient: GuestAddress,
    offer: PickupOffer,
    grant_index: usize,
    descriptor: GuestAddress,
    kind: SelectionKind,
    invalid: bool,
    called: bool,
    supply: Option<SupplyEvaluation>,
}

/// The complete original caller owns eligibility, feedback, targets and item
/// lifetime. Touch/grant phases are explicit methods; tests emulate originals.
pub struct NativePrimaryPickups {
    profile: NativePickupProfile,
    consumer_hook: Option<Box<dyn FnMut(NativeActorId, ProtectionChannel)>>,
    supply_owner: Option<Box<dyn Fn(NativeActorId, &ItemId, NativeActorId) -> bool>>,
    frames: Vec<PickupFrame>,
    closed: bool,
}

impl NativePrimaryPickups {
    /// Build the service. Grants with consumers require a protection hook.
    pub fn new(
        profile: NativePickupProfile,
        consumer_hook: Option<Box<dyn FnMut(NativeActorId, ProtectionChannel)>>,
    ) -> HostResult<Self> {
        if profile.grants.iter().any(|grant| !grant.consumers.is_empty()) && consumer_hook.is_none()
        {
            return Err(NativeHostError::Fault(
                "native pickup consumer has no protection owner binding".to_string(),
            ));
        }
        Ok(Self {
            profile,
            consumer_hook,
            supply_owner: None,
            frames: Vec::new(),
            closed: false,
        })
    }

    /// Borrow the profile.
    #[must_use]
    pub fn profile(&self) -> &NativePickupProfile {
        &self.profile
    }

    fn check_host(&self, host: &SyntheticHost) -> HostResult<()> {
        if self.closed {
            return Err(NativeHostError::Fault("native pickup service is closed".to_string()));
        }
        if host.core.digest != self.profile.digest {
            return Err(NativeHostError::Fault(
                "native pickup profile does not identify this artifact".to_string(),
            ));
        }
        Ok(())
    }

    /// Fail when a touch frame is still active.
    pub fn assert_idle(&self) -> HostResult<()> {
        if self.frames.is_empty() {
            Ok(())
        } else {
            Err(NativeHostError::Fault(
                "native pickup caller is still active".to_string(),
            ))
        }
    }

    /// Bind the weapon-supply owner; fails when busy, closed or already bound.
    pub fn bind_supply(
        &mut self,
        owner: Box<dyn Fn(NativeActorId, &ItemId, NativeActorId) -> bool>,
    ) -> HostResult<()> {
        self.assert_idle()?;
        if self.closed || self.supply_owner.is_some() {
            return Err(NativeHostError::Fault(
                "native pickups already have a supply owner or are closed".to_string(),
            ));
        }
        self.supply_owner = Some(owner);
        Ok(())
    }

    /// Release the weapon-supply owner.
    pub fn unbind_supply(&mut self) {
        self.supply_owner = None;
    }

    fn item_index(&self, host: &SyntheticHost, descriptor: GuestAddress) -> HostResult<u32> {
        let table = host.core.at(self.profile.items.table)?;
        let offset = descriptor.offset.wrapping_sub(table.offset);
        let stride = u64::from(self.profile.items.stride);
        if descriptor.offset < table.offset
            || stride == 0
            || offset % stride != 0
            || offset / stride >= u64::from(self.profile.items.count)
        {
            return Err(NativeHostError::Fault(
                "native pickup descriptor is outside its original item table".to_string(),
            ));
        }
        Ok((offset / stride) as u32)
    }

    fn item_name(&self, host: &mut SyntheticHost, descriptor: GuestAddress) -> HostResult<ItemId> {
        self.item_index(host, descriptor)?;
        let slot = host.core.memory.offset(descriptor, i64::from(self.profile.items.classname))?;
        let name = host.core.memory.read_pointer(slot)?;
        match name {
            Some(name) => {
                let text = host.core.read_c_string(name, 256)?;
                Ok(format!("q2:{text}"))
            }
            None => Err(NativeHostError::Fault(
                "native pickup descriptor has no classname".to_string(),
            )),
        }
    }

    fn client(&self, host: &mut SyntheticHost, recipient: GuestAddress) -> HostResult<GuestAddress> {
        let slot = host.core.memory.offset(recipient, i64::from(self.profile.supply.client))?;
        host.core.memory.read_pointer(slot)?.ok_or_else(|| {
            NativeHostError::Fault("native pickup recipient has no original client".to_string())
        })
    }

    fn counter(
        &self,
        host: &mut SyntheticHost,
        frame: &PickupFrame,
        descriptor: GuestAddress,
    ) -> HostResult<GuestAddress> {
        let client = self.client(host, frame.recipient)?;
        let index = self.item_index(host, descriptor)?;
        Ok(host.core.memory.offset(
            client,
            i64::from(self.profile.supply.inventory) + i64::from(index) * 4,
        )?)
    }

    fn live(&self, host: &mut SyntheticHost, record: GuestAddress) -> HostResult<bool> {
        let address = host
            .core
            .memory
            .offset(record, i64::from(self.profile.entity.inuse))?;
        match self.profile.entity.inuse_bytes {
            1 => Ok(host.core.memory.read_u8(address)? != 0),
            _ => Ok(host.core.memory.read_i32(address)? != 0),
        }
    }

    fn frame_current(&self, host: &mut SyntheticHost, frame: &PickupFrame) -> HostResult<bool> {
        if self.closed || frame.invalid {
            return Ok(false);
        }
        if !self.live(host, frame.pickup)? || !self.live(host, frame.recipient)? {
            return Ok(false);
        }
        if host.core.actor_for(frame.pickup) != Some(frame.offer.pickup)
            || host.core.actor_for(frame.recipient) != Some(frame.offer.recipient)
        {
            return Ok(false);
        }
        let slot = host
            .core
            .memory
            .offset(frame.pickup, i64::from(self.profile.entity.item))?;
        if host.core.memory.read_pointer(slot)? != Some(frame.descriptor) {
            return Ok(false);
        }
        Ok(true)
    }

    /// Describe a touch; offers push the current frame.
    pub fn begin_touch(
        &mut self,
        host: &mut SyntheticHost,
        pickup: GuestAddress,
        recipient: GuestAddress,
    ) -> HostResult<TouchDescribe> {
        self.check_host(host)?;
        let descriptor = host
            .core
            .memory
            .read_pointer(host.core.memory.offset(pickup, i64::from(self.profile.entity.item))?)?;
        let descriptor = match descriptor {
            Some(descriptor) => descriptor,
            None => return Ok(TouchDescribe::PassThrough),
        };
        if self.item_index(host, descriptor).is_err() {
            return Ok(TouchDescribe::PassThrough);
        }
        let entry = host.core.memory.read_pointer(
            host.core
                .memory
                .offset(descriptor, i64::from(self.profile.items.pickup))?,
        )?;
        let grant_index = match entry {
            Some(entry) => self.profile.grants.iter().position(|grant| {
                host.core.at(grant.entry).map_or(false, |address| address == entry)
            }),
            None => None,
        };
        let grant_index = match grant_index {
            Some(index) => index,
            None => return Ok(TouchDescribe::PassThrough),
        };
        let (pickup_actor, recipient_actor) =
            match (host.core.actor_for(pickup), host.core.actor_for(recipient)) {
                (Some(pickup), Some(recipient)) => (pickup, recipient),
                _ => return Ok(TouchDescribe::Stale),
            };
        if !self.live(host, pickup)? || !self.live(host, recipient)? {
            return Ok(TouchDescribe::Stale);
        }
        let item = self.item_name(host, descriptor)?;
        let count = host.core.memory.read_i32(
            host.core
                .memory
                .offset(pickup, i64::from(self.profile.entity.count))?,
        )?;
        let time = match self.profile.time.storage {
            TimeStorage::FloatSeconds => {
                let address = host.core.at(self.profile.time.address)?;
                PickupTimeValue::Seconds(f64::from(host.core.memory.read_f32(address)?))
            }
            TimeStorage::Int64Milliseconds => {
                let address = host.core.at(self.profile.time.address)?;
                PickupTimeValue::Milliseconds(host.core.memory.read_i64(address)?)
            }
        };
        let valid = match time {
            PickupTimeValue::Seconds(value) => value.is_finite(),
            PickupTimeValue::Milliseconds(value) => value as f64 >= -9.0e15 && value as f64 <= 9.0e15,
        };
        if !valid {
            return Err(NativeHostError::Fault(
                "native pickup clock cannot be represented".to_string(),
            ));
        }
        let spawnflags = host.core.memory.read_u32(
            host.core
                .memory
                .offset(pickup, i64::from(self.profile.entity.spawnflags))?,
        )?;
        let grant = &self.profile.grants[grant_index];
        let offer = PickupOffer {
            recipient: recipient_actor,
            pickup: pickup_actor,
            source: "synthetic-primary".to_string(),
            item: item.clone(),
            default_resource: match grant.resource {
                PickupResource::Regular => OfferResource::Protection {
                    channel: ProtectionChannel::Regular,
                },
                PickupResource::Inventory => OfferResource::Inventory { item: item.clone() },
            },
            count: if count == 0 {
                PickupCount::Default
            } else {
                PickupCount::Override(count)
            },
            dropped: spawnflags & 0x30000 != 0,
            time,
        };
        self.frames.push(PickupFrame {
            pickup,
            recipient,
            offer: offer.clone(),
            grant_index,
            descriptor,
            kind: SelectionKind::Original,
            invalid: false,
            called: false,
            supply: None,
        });
        Ok(TouchDescribe::Offer {
            offer,
            grant_index,
            descriptor,
        })
    }

    /// Pop the current frame.
    pub fn end_touch(&mut self) {
        self.frames.pop();
    }

    /// Run a grant for the current frame. Blocked selections refuse; original
    /// selections must run the original (the caller does so on `Accepted`
    /// with an untouched recipient region).
    pub fn grant(
        &mut self,
        host: &mut SyntheticHost,
        pickup: GuestAddress,
        recipient: GuestAddress,
        selection: &mut PickupSelection,
    ) -> HostResult<GrantOutcome> {
        self.check_host(host)?;
        let frame_index = self.frames.len().wrapping_sub(1);
        let frame = self.frames.get(frame_index).ok_or_else(|| {
            NativeHostError::Fault("native pickup grant has no owning caller".to_string())
        })?;
        if !self.frame_current(host, frame)? {
            return Ok(GrantOutcome::Cancelled);
        }
        if pickup != frame.pickup || recipient != frame.recipient || frame.called {
            return Err(NativeHostError::Fault(
                "native pickup grant changed its owning source call".to_string(),
            ));
        }
        self.frames[frame_index].called = true;
        match selection {
            PickupSelection::Stale => Ok(GrantOutcome::Cancelled),
            PickupSelection::Blocked => {
                self.frames[frame_index].kind = SelectionKind::Blocked;
                Ok(GrantOutcome::Refused)
            }
            PickupSelection::Original => {
                self.frames[frame_index].kind = SelectionKind::Original;
                Ok(GrantOutcome::Accepted)
            }
            PickupSelection::Replacement { grant, current } => {
                self.frames[frame_index].kind = SelectionKind::Replacement;
                if !current() {
                    return Ok(GrantOutcome::Cancelled);
                }
                if self.profile.grants[self.frames[frame_index].grant_index].supply.is_some() {
                    return Err(NativeHostError::Fault(
                        "native supply grants settle through the supply path".to_string(),
                    ));
                }
                let decision = self.grant_resources(host, frame_index, grant, true)?;
                match decision {
                    PickupOutcome::Accepted => Ok(GrantOutcome::Accepted),
                    PickupOutcome::Refused => Ok(GrantOutcome::Refused),
                    PickupOutcome::Stale => Ok(GrantOutcome::Cancelled),
                }
            }
        }
    }

    fn grant_resources(
        &mut self,
        host: &mut SyntheticHost,
        frame_index: usize,
        grant: &mut dyn FnMut() -> PickupOutcome,
        consume: bool,
    ) -> HostResult<PickupOutcome> {
        let consumers = self.profile.grants[self.frames[frame_index].grant_index].consumers.clone();
        let decision = grant();
        if !self.frame_current(host, &self.frames[frame_index])? {
            return Ok(PickupOutcome::Stale);
        }
        if decision == PickupOutcome::Accepted && consume {
            for consumer in &consumers {
                let recipient_actor = self.frames[frame_index].offer.recipient;
                if let Some(hook) = self.consumer_hook.as_mut() {
                    hook(recipient_actor, consumer.protection);
                }
                let address = host.core.at(consumer.entry)?;
                let recipient = self.frames[frame_index].recipient;
                host.invoke(address, &[GuestCallValue::Pointer(Some(recipient))])?;
                if !self.frame_current(host, &self.frames[frame_index])? {
                    return Ok(PickupOutcome::Stale);
                }
            }
        }
        Ok(decision)
    }

    /// Observe the targets return; non-original frames must still be current.
    pub fn note_targets(&mut self, host: &mut SyntheticHost) -> HostResult<bool> {
        self.check_host(host)?;
        match self.frames.last() {
            Some(frame) if frame.kind != SelectionKind::Original => {
                Ok(self.frame_current(host, frame)?)
            }
            _ => Ok(true),
        }
    }

    /// Read the held supply evaluation.
    pub fn supply(&mut self, host: &mut SyntheticHost) -> HostResult<SupplyEvaluation> {
        self.check_host(host)?;
        let frame = self.frames.last().ok_or_else(|| {
            NativeHostError::Fault("native pickup supply requires its held source grant".to_string())
        })?;
        if !self.frame_current(host, frame)? {
            return Err(NativeHostError::Fault(
                "native pickup supply requires its held source grant".to_string(),
            ));
        }
        frame.supply.clone().ok_or_else(|| {
            NativeHostError::Fault("native pickup supply requires its held source grant".to_string())
        })
    }

    /// Prepare an ammo-supply evaluation for the current frame.
    pub fn prepare_supply_ammo(
        &mut self,
        host: &mut SyntheticHost,
        amount: i32,
    ) -> HostResult<SupplyEvaluation> {
        self.check_host(host)?;
        let frame_index = self.frames.len().wrapping_sub(1);
        if self.frames.get(frame_index).is_none() {
            return Err(NativeHostError::Fault(
                "native supply has no selected grant".to_string(),
            ));
        }
        let descriptor = self.frames[frame_index].descriptor;
        let item = self.frames[frame_index].offer.item.clone();
        let flags = host.core.memory.read_u32(
            host.core
                .memory
                .offset(descriptor, i64::from(self.profile.supply.flags))?,
        )?;
        let offer = if flags & self.profile.supply.weapon_flag != 0 {
            SupplyOffer::AmmoWeapon {
                item: item.clone(),
                amount,
                weapon: item.clone(),
            }
        } else {
            SupplyOffer::Ammo { item: item.clone(), amount }
        };
        let evaluation = SupplyEvaluation {
            offer,
            ammo: Some(SupplyAmmo { descriptor, amount }),
        };
        self.frames[frame_index].supply = Some(evaluation.clone());
        Ok(evaluation)
    }

    /// Prepare a weapon-supply evaluation, projecting the ownership counter.
    pub fn prepare_supply_weapon(
        &mut self,
        host: &mut SyntheticHost,
        ammo: Option<SupplyAmmo>,
    ) -> HostResult<SupplyEvaluation> {
        self.check_host(host)?;
        let frame_index = self.frames.len().wrapping_sub(1);
        if self.frames.get(frame_index).is_none() {
            return Err(NativeHostError::Fault(
                "native supply has no selected grant".to_string(),
            ));
        }
        let item = self.frames[frame_index].offer.item.clone();
        let bundled = match ammo {
            Some(ammo) => {
                let name = self.item_name(host, ammo.descriptor)?;
                vec![(name, ammo.amount)]
            }
            None => Vec::new(),
        };
        if self.supply_owner.is_some() {
            let frame = &self.frames[frame_index];
            let counter = self.counter(host, frame, frame.descriptor)?;
            let owned = self.supply_owner.as_ref().expect("owner")(
                frame.offer.recipient,
                &frame.offer.item,
                frame.offer.pickup,
            );
            host.core.memory.write_i32(counter, i32::from(owned))?;
        }
        let evaluation = SupplyEvaluation {
            offer: SupplyOffer::Weapon { item, ammo: bundled },
            ammo,
        };
        self.frames[frame_index].supply = Some(evaluation.clone());
        Ok(evaluation)
    }

    /// Project one ammo quantity through the original ammo grant.
    pub fn project_quantity(
        &mut self,
        host: &mut SyntheticHost,
        descriptor: GuestAddress,
        amount: i32,
        count: i32,
        capacity: i32,
    ) -> HostResult<PickupQuantity> {
        self.check_host(host)?;
        let frame_index = self.frames.len().wrapping_sub(1);
        let frame = self.frames.get(frame_index).ok_or_else(|| {
            NativeHostError::Fault("native pickup quantity has expired".to_string())
        })?;
        if !self.frame_current(host, frame)? || frame.supply.is_none() {
            return Err(NativeHostError::Fault("native pickup quantity has expired".to_string()));
        }
        let ammo = &self.profile.supply.ammo;
        let tag = host.core.memory.read_i32(
            host.core
                .memory
                .offset(descriptor, i64::from(ammo.tag))?,
        )?;
        let capacity_offset = usize::try_from(tag)
            .ok()
            .and_then(|tag| ammo.capacities.get(tag))
            .copied()
            .ok_or_else(|| {
                NativeHostError::Fault("native pickup ammo has no source capacity".to_string())
            })?;
        let capacity_limit = if ammo.capacity_bytes == 2 { 0x7FFF } else { i32::MAX };
        if capacity < 0 || capacity > capacity_limit {
            return Err(NativeHostError::Fault(
                "selected pickup count or capacity exceeds its original ABI".to_string(),
            ));
        }
        let frame = &self.frames[frame_index];
        let counter = self.counter(host, frame, descriptor)?;
        let client = self.client(host, frame.recipient)?;
        let cap = host.core.memory.offset(client, i64::from(capacity_offset))?;
        let previous_count = host.core.memory.read_i32(counter)?;
        let previous_cap = if ammo.capacity_bytes == 2 {
            i32::from(host.core.memory.read_i16(cap)?)
        } else {
            host.core.memory.read_i32(cap)?
        };
        host.core.memory.write_i32(counter, count)?;
        if ammo.capacity_bytes == 2 {
            host.core.memory.write_i16(cap, capacity as i16)?;
        } else {
            host.core.memory.write_i32(cap, capacity)?;
        }
        let address = host.core.at(ammo.entry)?;
        let recipient = self.frames[frame_index].recipient;
        let result = host.invoke(
            address,
            &[
                GuestCallValue::Pointer(Some(recipient)),
                GuestCallValue::Pointer(Some(descriptor)),
                GuestCallValue::Int32(amount),
            ],
        )?;
        let accepted = match result {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => value != 0,
            GuestCallResult::Value(GuestCallValue::Uint32(value)) => value != 0,
            _ => {
                return Err(NativeHostError::Fault(
                    "original ammo grant returned a non-boolean ABI value".to_string(),
                ));
            }
        };
        let granted = host.core.memory.read_i32(counter)? - count;
        host.core.memory.write_i32(counter, previous_count)?;
        if ammo.capacity_bytes == 2 {
            host.core.memory.write_i16(cap, previous_cap as i16)?;
        } else {
            host.core.memory.write_i32(cap, previous_cap)?;
        }
        Ok(PickupQuantity {
            amount: granted,
            accepted,
        })
    }

    /// Settle a prepared supply through resource grants. `accepted_ammo`
    /// aggregates the quantity projections the caller ran.
    pub fn settle_supply(
        &mut self,
        host: &mut SyntheticHost,
        selection: &mut PickupSelection,
        accepted_ammo: bool,
    ) -> HostResult<GrantOutcome> {
        self.check_host(host)?;
        let frame_index = self.frames.len().wrapping_sub(1);
        if self.frames.get(frame_index).is_none() {
            return Err(NativeHostError::Fault(
                "native supply has no selected grant".to_string(),
            ));
        }
        if !self.frame_current(host, &self.frames[frame_index])? {
            return Ok(GrantOutcome::Cancelled);
        }
        let grant = match selection {
            PickupSelection::Replacement { grant, current } => {
                if !current() {
                    return Ok(GrantOutcome::Cancelled);
                }
                grant
            }
            PickupSelection::Blocked => return Ok(GrantOutcome::Refused),
            _ => {
                return Err(NativeHostError::Fault(
                    "native supply owner changed during its source call".to_string(),
                ));
            }
        };
        let decision = self.grant_resources(host, frame_index, grant, accepted_ammo)?;
        self.frames[frame_index].supply = None;
        match decision {
            PickupOutcome::Accepted => Ok(GrantOutcome::Accepted),
            PickupOutcome::Refused => Ok(GrantOutcome::Refused),
            PickupOutcome::Stale => Ok(GrantOutcome::Cancelled),
        }
    }

    /// Invalidate frames and close the service.
    pub fn close(&mut self) {
        self.closed = true;
        for frame in &mut self.frames {
            frame.invalid = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::CLASSIC_DIGEST;
    use super::*;
    use qa_guest::core::contracts::{GuestStorage, GuestValueLayout, NativeCallAbi};

    fn signature() -> GuestCallSignature {
        GuestCallSignature {
            abi: NativeCallAbi::Cdecl,
            parameters: vec![
                GuestValueLayout::Scalar(GuestStorage::Pointer),
                GuestValueLayout::Scalar(GuestStorage::Pointer),
            ],
            result: None,
            variadic: false,
        }
    }

    fn profile() -> NativePickupProfile {
        NativePickupProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            touch: 0x100,
            grant_return: 0x110,
            targets_return: 0x120,
            touch_signature: signature(),
            grant_signature: signature(),
            grants: vec![
                NativePickupGrant {
                    entry: 0x200,
                    recipient: NativeRegion { entry: 0x210, join: 0x220 },
                    resource: PickupResource::Inventory,
                    consumers: vec![],
                    supply: Some(PickupSupply::Ammo {
                        entry: 0x230,
                        amount: GuestRegister::Rbx,
                    }),
                },
                NativePickupGrant {
                    entry: 0x300,
                    recipient: NativeRegion { entry: 0x310, join: 0x320 },
                    resource: PickupResource::Regular,
                    consumers: vec![],
                    supply: None,
                },
            ],
            items: PickupItems {
                table: 0x400,
                stride: 32,
                count: 2,
                classname: 0,
                pickup: 8,
            },
            entity: PickupEntity {
                item: 0x40,
                count: 0x44,
                spawnflags: 0x48,
                inuse: 0x4C,
                inuse_bytes: 4,
                generation: None,
            },
            time: PickupTime { address: 0x500, storage: TimeStorage::FloatSeconds },
            supply: PickupSupplyProfile {
                client: 84,
                inventory: 0x100,
                flags: 16,
                weapon_flag: 1,
                ammo: AmmoSupply {
                    entry: 0x600,
                    signature: signature(),
                    stop: None,
                    tag: 20,
                    capacities: vec![0x80],
                    capacity_bytes: 4,
                },
            },
        }
    }

    fn live(host: &mut SyntheticHost, entity: GuestAddress) {
        host.core.memory.write_i32(host.core.memory.offset(entity, 0x4C).expect("inuse"), 1).expect("live");
    }

    fn fixture() -> (SyntheticHost, GuestAddress, GuestAddress, GuestAddress) {
        let profile = profile();
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let name = host.core.allocate_string("ammo_shells").expect("name");
        let descriptor = host.core.at(0x400).expect("descriptor");
        host.core.memory.write_pointer(descriptor, Some(name)).expect("name");
        let pickup_fn = host.core.at(0x200).expect("grant");
        host.core
            .memory
            .write_pointer(host.core.memory.offset(descriptor, 8).expect("fn"), Some(pickup_fn))
            .expect("fn");
        host.core.memory.write_u32(host.core.memory.offset(descriptor, 16).expect("flags"), 0).expect("flags");
        host.core.memory.write_i32(host.core.memory.offset(descriptor, 20).expect("tag"), 0).expect("tag");
        host.core.memory.write_f32(host.core.at(0x500).expect("time"), 12.0).expect("time");
        let pickup_actor = host.core.spawn_actor(896, 512).expect("pickup");
        let recipient_actor = host.core.spawn_actor(896, 512).expect("recipient");
        let pickup = host.core.entity_of(pickup_actor).expect("pickup");
        let recipient = host.core.entity_of(recipient_actor).expect("recipient");
        let client = host.core.client_of(recipient_actor).expect("client");
        host.core.set_client(recipient, 84, client).expect("link");
        live(&mut host, pickup);
        live(&mut host, recipient);
        host.core
            .memory
            .write_pointer(host.core.memory.offset(pickup, 0x40).expect("item"), Some(descriptor))
            .expect("item");
        host.core.memory.write_i32(host.core.memory.offset(pickup, 0x44).expect("count"), 5).expect("count");
        let _ = profile;
        (host, pickup, recipient, descriptor)
    }

    #[test]
    fn describes_offers_and_passthroughs() {
        let (mut host, pickup, recipient, _) = fixture();
        let service = NativePrimaryPickups::new(profile(), None).expect("service");
        let mut service = service;
        match service.begin_touch(&mut host, pickup, recipient).expect("touch") {
            TouchDescribe::Offer { offer, grant_index, .. } => {
                assert_eq!(offer.item, "q2:ammo_shells");
                assert_eq!(grant_index, 0);
                assert_eq!(offer.count, PickupCount::Override(5));
                assert!(!offer.dropped);
            }
            _ => panic!("expected offer"),
        }
        assert!(service.note_targets(&mut host).expect("targets"));
        service.end_touch();
        service.assert_idle().expect("idle");
        let stranger = host.core.spawn_actor(896, 512).expect("stranger");
        let stranger = host.core.entity_of(stranger).expect("entity");
        live(&mut host, stranger);
        match service.begin_touch(&mut host, stranger, recipient).expect("touch") {
            TouchDescribe::PassThrough => {}
            _ => panic!("expected passthrough"),
        }
        service.assert_idle().expect("idle");
    }

    #[test]
    fn refuses_blocked_grants() {
        let (mut host, pickup, recipient, _) = fixture();
        let mut service = NativePrimaryPickups::new(profile(), None).expect("service");
        assert!(matches!(
            service.begin_touch(&mut host, pickup, recipient).expect("touch"),
            TouchDescribe::Offer { .. }
        ));
        let mut selection = PickupSelection::Blocked;
        assert_eq!(
            service.grant(&mut host, pickup, recipient, &mut selection).expect("grant"),
            GrantOutcome::Refused
        );
        service.end_touch();
    }

    #[test]
    fn settles_ammo_supply_with_quantity() {
        let (mut host, pickup, recipient, _) = fixture();
        host.on_rva(
            0x600,
            Box::new(|core, values| {
                let (recipient, descriptor) = match (&values[0], &values[1]) {
                    (GuestCallValue::Pointer(Some(recipient)), GuestCallValue::Pointer(Some(descriptor))) => {
                        (*recipient, *descriptor)
                    }
                    _ => return Err(NativeHostError::Fault("bad ammo args".to_string())),
                };
                let amount = match values[2] {
                    GuestCallValue::Int32(amount) => amount,
                    _ => return Err(NativeHostError::Fault("bad amount".to_string())),
                };
                let client = core.memory.read_pointer(core.memory.offset(recipient, 84)?)?.expect("client");
                let counter = core.memory.offset(client, 0x100)?;
                let count = core.memory.read_i32(counter)?;
                core.memory.write_i32(counter, count + amount.min(3))?;
                let _ = descriptor;
                Ok(GuestCallResult::Value(GuestCallValue::Int32(1)))
            }),
        )
        .expect("handler");
        let mut service = NativePrimaryPickups::new(profile(), None).expect("service");
        assert!(matches!(
            service.begin_touch(&mut host, pickup, recipient).expect("touch"),
            TouchDescribe::Offer { .. }
        ));
        let descriptor = host.core.at(0x400).expect("descriptor");
        let evaluation = service.prepare_supply_ammo(&mut host, 5).expect("prepare");
        assert!(matches!(evaluation.offer, SupplyOffer::Ammo { amount: 5, .. }));
        let held = service.supply(&mut host).expect("supply");
        assert_eq!(held, evaluation);
        let quantity = service.project_quantity(&mut host, descriptor, 5, 0, 200).expect("quantity");
        assert_eq!(quantity, PickupQuantity { amount: 3, accepted: true });
        let mut selection = PickupSelection::Replacement {
            grant: Box::new(|| PickupOutcome::Accepted),
            current: Box::new(|| true),
        };
        assert_eq!(
            service.settle_supply(&mut host, &mut selection, quantity.accepted).expect("settle"),
            GrantOutcome::Accepted
        );
        assert!(service.supply(&mut host).is_err());
        service.end_touch();
    }

    #[test]
    fn grants_plain_replacements_with_consumers() {
        let (mut host, pickup, recipient, descriptor) = fixture();
        host.core
            .memory
            .write_pointer(
                host.core.memory.offset(descriptor, 8).expect("fn"),
                Some(host.core.at(0x300).expect("grant")),
            )
            .expect("fn");
        host.on_rva(
            0x300,
            Box::new(|_, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(1)))),
        )
        .expect("handler");
        let mut profile = profile();
        profile.grants[1].consumers.push(PickupConsumer {
            entry: 0x700,
            signature: signature(),
            protection: ProtectionChannel::Powered,
        });
        host.on_rva(0x700, Box::new(|_, _| Ok(GuestCallResult::Void))).expect("consumer");
        let mut service = NativePrimaryPickups::new(profile, Some(Box::new(|_, _| {}))).expect("service");
        assert!(matches!(
            service.begin_touch(&mut host, pickup, recipient).expect("touch"),
            TouchDescribe::Offer { grant_index: 1, .. }
        ));
        let mut selection = PickupSelection::Replacement {
            grant: Box::new(|| PickupOutcome::Accepted),
            current: Box::new(|| true),
        };
        assert_eq!(
            service.grant(&mut host, pickup, recipient, &mut selection).expect("grant"),
            GrantOutcome::Accepted
        );
        service.end_touch();
        service.close();
    }
}
