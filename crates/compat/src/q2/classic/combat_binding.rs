//! Donor: `src/compat/q2/classic/combat-binding.ts` — source damage and
//! armor stages over the native combat entries.
//!
//! Bridges shared damage requests to the original `T_Damage` and armor
//! entries: guest health, armor, and velocity are snapshotted around the
//! native call, armor intercepts run with the original entry available, and
//! reactions classify from committed health. Native entry hooks keyed on
//! the CPU stack collapse into an explicit bypass flag, and write observers
//! collapse into before/after snapshots, since no real CPU executes here.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAccess, GuestAddress, GuestCallResult, GuestCallSignature, GuestCallValue};
use qa_world::combat::{
    item_id, reaction_for_health, ArmorDamageFlags, ArmorState, CombatState, Delivery, ItemId, PoweredProtection,
    Reaction, RegularArmor,
};

use crate::userinfo::info_value_for_key;

use super::combat_profile::{
    classic_combat_profile, lower_native_combat_arguments, native_combat_signature, read_native_combat_arguments,
    validate_classic_combat_profile, ClassicCombatOperation, ClassicCombatProfile, ClassicGame,
};
use super::host::ClassicQ2GuestHost;
use super::layout::{classic_signature, q2_int, q2_pointer, ClassicQ2Error, ClassicResult};
use super::records::{read_classic_string, read_classic_vector, write_classic_vector};

/// Friendly-fire bit in classic native causes.
const FRIENDLY_FIRE: i64 = 0x8000000;
/// Canonical grapple cause.
const CAUSE_GRAPPLE: i64 = 56;

/// Q2 damage cause with its native value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicDamageCause {
    /// Canonical means of death.
    pub means_of_death: i32,
    /// Q2 damage flags.
    pub damage_flags: i32,
    /// Native cause value.
    pub native_value: i32,
}

/// Source damage request against one target.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicDamageRequest {
    /// Damage target.
    pub target: ActorId,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Damage amount.
    pub amount: i32,
    /// Knockback.
    pub knockback: i32,
    /// Damage direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Delivery class.
    pub delivery: Delivery,
    /// Damage cause.
    pub cause: ClassicDamageCause,
}

fn valid_classic(game: ClassicGame, id: i64) -> bool {
    id <= 33
        || game == ClassicGame::Xatrix && id <= 39
        || game == ClassicGame::Rogue && (40..=55).contains(&id)
        || game == ClassicGame::Ctf && id == 34
}

/// Canonicalize a classic native cause, rejecting unknown guest causes.
pub fn canonical_cause_from_native(game: ClassicGame, value: i32) -> Option<i32> {
    let value = i64::from(value);
    if value < 0 || value > FRIENDLY_FIRE + 55 {
        return None;
    }
    let friendly = value & FRIENDLY_FIRE != 0;
    let id = value & !FRIENDLY_FIRE;
    if !valid_classic(game, id) {
        return None;
    }
    let canonical = if game == ClassicGame::Ctf && id == 34 {
        CAUSE_GRAPPLE
    } else {
        id
    };
    Some((canonical + if friendly { FRIENDLY_FIRE } else { 0 }) as i32)
}

/// Lower a canonical cause to a classic native value.
pub fn native_cause_from_canonical(game: ClassicGame, canonical: i32) -> Option<i32> {
    let canonical = i64::from(canonical);
    if canonical < 0 || canonical > FRIENDLY_FIRE + 58 {
        return None;
    }
    let friendly = canonical & FRIENDLY_FIRE != 0;
    let id = canonical & !FRIENDLY_FIRE;
    let raw = if game == ClassicGame::Ctf && id == CAUSE_GRAPPLE {
        34
    } else {
        id
    };
    if game == ClassicGame::Ctf && id == 34 || !valid_classic(game, raw) {
        return None;
    }
    Some((raw + if friendly { FRIENDLY_FIRE } else { 0 }) as i32)
}

/// Decode armor flags at the original armor callsite.
#[must_use]
pub fn q2_native_armor_flags(flags: i32) -> ArmorDamageFlags {
    ArmorDamageFlags {
        stage: None,
        no_armor: flags & 2 != 0,
        no_power_armor: flags & 0x100 != 0,
        no_regular_armor: flags & 0x80 != 0,
        energy: flags & 4 != 0,
        regular_protection_scale: 1.0,
    }
}

/// Lower a damage request to native flags plus a native cause, reusing the
/// captured native value when it still canonicalizes and falling back to
/// `MOD_UNKNOWN` otherwise.
pub fn q2_native_damage_arguments(request: &ClassicDamageRequest, game: ClassicGame) -> ClassicResult<(i32, i32)> {
    let native = if canonical_cause_from_native(game, request.cause.native_value) == Some(request.cause.means_of_death)
    {
        request.cause.native_value
    } else {
        native_cause_from_canonical(game, request.cause.means_of_death)
            .or_else(|| native_cause_from_canonical(game, 0))
            .unwrap_or(0)
    };
    Ok((request.cause.damage_flags, native))
}

/// Armor protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered armor.
    Powered,
}

/// Arguments delivered to an armor intercept.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageArgs {
    /// Intercepted target.
    pub target: ActorId,
    /// Incoming damage amount.
    pub amount: i32,
    /// Damage direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Decoded armor flags.
    pub flags: ArmorDamageFlags,
}

/// Armor intercept: transform or replace the original entry result.
pub type ArmorIntercept = Box<dyn FnMut(&ArmorStageArgs, &mut dyn FnMut() -> ClassicResult<i32>) -> ClassicResult<i32>>;

/// Exclusive armor stage lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArmorStageLease {
    /// Bound slot.
    pub slot: u32,
    /// Bound channel.
    pub channel: ProtectionChannel,
}

/// One armor stage invocation: the intercepted call's values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorStrike {
    /// Incoming damage amount.
    pub amount: i32,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Spark count (regular armor only).
    pub sparks: i32,
    /// Damage flags.
    pub flags: i32,
}

#[derive(Debug, Clone)]
struct DamageFrame {
    request: ClassicDamageRequest,
    slot: u32,
}

/// One committed damage store observed around the native call.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredObservation {
    /// Health store.
    Health {
        /// Health before.
        before: i32,
        /// Health after.
        after: i32,
    },
    /// Armor store.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Velocity store.
    Velocity {
        /// Velocity before.
        before: Vec3,
        /// Velocity after.
        after: Vec3,
    },
}

/// Damage outcome: applied damage plus the classified reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageOutcome {
    /// Health damage applied.
    pub applied_damage: i32,
    /// Classified reaction.
    pub reaction: Reaction,
    /// Committed stores.
    pub observations: Vec<StoredObservation>,
}

/// Projected foreign body used to back a temporary damage edict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectedBody {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Hull minimums.
    pub mins: Vec3,
    /// Hull maximums.
    pub maxs: Vec3,
    /// Velocity.
    pub velocity: Vec3,
}

#[derive(Debug, Clone)]
struct ArmorDefinition {
    item: ItemId,
    normal_protection: f32,
    energy_protection: f32,
}

/// Source ABI damage bindings over the native combat entries.
pub struct ClassicCombatBindings {
    image: GuestAddress,
    profile: ClassicCombatProfile,
    damage_signature: GuestCallSignature,
    regular_signature: GuestCallSignature,
    power_signature: GuestCallSignature,
    frames: Vec<DamageFrame>,
    armor_intercepts: HashMap<(u32, ProtectionChannel), ArmorIntercept>,
    bypass: Option<ProtectionChannel>,
    closed: bool,
    projected_bodies: HashMap<ActorId, ProjectedBody>,
}

impl ClassicCombatBindings {
    /// Bind the combat entries at `image`, unless the digest is unadmitted.
    pub fn create(
        host: &mut ClassicQ2GuestHost,
        image: GuestAddress,
        declared: Option<ClassicCombatProfile>,
    ) -> ClassicResult<Option<Self>> {
        let profile = declared.or_else(|| classic_combat_profile(&host.memory.module().digest));
        let Some(profile) = profile else {
            return Ok(None);
        };
        if profile.digest != host.memory.module().digest {
            return Err(ClassicQ2Error::invalid(
                "Classic combat profile belongs to another original artifact",
            ));
        }
        validate_classic_combat_profile(&profile)?;
        for entry in [
            profile.entries.damage,
            profile.entries.power_armor,
            profile.entries.regular_armor,
            profile.entries.spawn,
            profile.entries.free,
        ] {
            host.memory
                .check(host.memory.offset(image, i64::from(entry))?, 1, GuestAccess::Execute)?;
        }
        Ok(Some(Self {
            image,
            damage_signature: native_combat_signature(&profile.calls.damage, ClassicCombatOperation::Damage)?,
            regular_signature: native_combat_signature(
                &profile.calls.regular_armor,
                ClassicCombatOperation::RegularArmor,
            )?,
            power_signature: native_combat_signature(&profile.calls.power_armor, ClassicCombatOperation::PowerArmor)?,
            profile,
            frames: Vec::new(),
            armor_intercepts: HashMap::new(),
            bypass: None,
            closed: false,
            projected_bodies: HashMap::new(),
        }))
    }

    /// Admit a projected body for a foreign damage actor.
    pub fn project_body(&mut self, actor: ActorId, body: ProjectedBody) {
        self.projected_bodies.insert(actor, body);
    }

    /// Release all intercepts and frames.
    pub fn close(&mut self) {
        self.closed = true;
        self.frames.clear();
        self.armor_intercepts.clear();
    }

    fn check_open(&self) -> ClassicResult<()> {
        if self.closed {
            return Err(ClassicQ2Error::invalid("Classic combat bindings are closed"));
        }
        Ok(())
    }

    fn entry(&self, host: &ClassicQ2GuestHost, offset: u32) -> ClassicResult<GuestAddress> {
        Ok(host.memory.offset(self.image, i64::from(offset))?)
    }

    fn record_address(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<GuestAddress> {
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        Ok(edicts.at(&mut host.memory, slot)?.address)
    }

    fn slot_of(&self, host: &ClassicQ2GuestHost, actor: &ActorId) -> ClassicResult<u32> {
        let slot = host
            .registry
            .source_of(actor)
            .filter(|(provider, _)| provider == &host.provider)
            .map(|(_, slot)| slot);
        match slot {
            Some(slot) => Ok(slot),
            None => Err(ClassicQ2Error::invalid("Damage actor is not a live source actor")),
        }
    }

    /// Whether the notarget flag is set.
    pub fn notarget(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<bool> {
        let address = self.record_address(host, slot)?;
        let flags = host
            .memory
            .read_i32(host.memory.offset(address, self.profile.fields.flags as i64)?)?;
        Ok(flags as u32 & self.profile.flags.notarget != 0)
    }

    fn client(&self, host: &mut ClassicQ2GuestHost, address: GuestAddress) -> ClassicResult<Option<GuestAddress>> {
        Ok(host.memory.read_pointer(host.memory.offset(address, 84)?)?)
    }

    fn count(&self, host: &mut ClassicQ2GuestHost, client: GuestAddress, index: usize) -> ClassicResult<i32> {
        Ok(host.memory.read_i32(
            host.memory
                .offset(client, (self.profile.client.inventory + index * 4) as i64)?,
        )?)
    }

    fn armor_definition(&self, host: &mut ClassicQ2GuestHost, index: usize) -> ClassicResult<ArmorDefinition> {
        let record = host.memory.offset(
            self.image,
            (self.profile.globals.item_list as usize + index * self.profile.globals.item_bytes) as i64,
        )?;
        let class_name_at = host.memory.offset(record, self.profile.item_fields.class_name as i64)?;
        let class_name = host.memory.read_pointer(class_name_at)?;
        let name = read_classic_string(&mut host.memory, class_name, 65536)?;
        let info = host
            .memory
            .read_pointer(host.memory.offset(record, self.profile.item_fields.armor_info as i64)?)?;
        let Some(info) = info else {
            return Err(ClassicQ2Error::invalid(
                "Original native armor item lacks its name or protection information",
            ));
        };
        if name.is_empty() {
            return Err(ClassicQ2Error::invalid(
                "Original native armor item lacks its name or protection information",
            ));
        }
        Ok(ArmorDefinition {
            item: item_id("q2", &name),
            normal_protection: host.memory.read_f32(
                host.memory
                    .offset(info, self.profile.armor_info.normal_protection as i64)?,
            )?,
            energy_protection: host.memory.read_f32(
                host.memory
                    .offset(info, self.profile.armor_info.energy_protection as i64)?,
            )?,
        })
    }

    /// Read live armor from the client inventory.
    pub fn armor_state(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<ArmorState> {
        let address = self.record_address(host, slot)?;
        let Some(client) = self.client(host, address)? else {
            return Ok(ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::None,
            });
        };
        let mut found = None;
        for index in &self.profile.armor.regular {
            if self.count(host, client, *index)? > 0 {
                found = Some(*index);
                break;
            }
        }
        let flags = host
            .memory
            .read_i32(host.memory.offset(address, self.profile.fields.flags as i64)?)?;
        let powered = flags as u32 & self.profile.flags.power_armor != 0;
        let power = if !powered {
            None
        } else if self.count(host, client, self.profile.items.shield)? > 0 {
            Some(PoweredProtection::Shield {
                cells: self.count(host, client, self.profile.items.cells)?,
            })
        } else if self.count(host, client, self.profile.items.screen)? > 0 {
            Some(PoweredProtection::Screen {
                cells: self.count(host, client, self.profile.items.cells)?,
            })
        } else {
            None
        };
        let regular = match found {
            Some(index) => {
                let definition = self.armor_definition(host, index)?;
                RegularArmor::Q2 {
                    points: f64::from(self.count(host, client, index)?),
                    normal_protection: f64::from(definition.normal_protection),
                    energy_protection: f64::from(definition.energy_protection),
                    item: definition.item,
                }
            }
            None => RegularArmor::None,
        };
        Ok(ArmorState {
            regular,
            powered: power.unwrap_or(PoweredProtection::None),
        })
    }

    /// Validate armor values against the declared native inventory.
    pub fn validate_armor(&self, host: &mut ClassicQ2GuestHost, slot: u32, armor: &ArmorState) -> ClassicResult<()> {
        match &armor.regular {
            RegularArmor::None => {}
            RegularArmor::Q2 { item, .. } => {
                let mut admitted = false;
                for index in &self.profile.armor.regular {
                    if &self.armor_definition(host, *index)?.item == item {
                        admitted = true;
                        break;
                    }
                }
                if !admitted {
                    return Err(ClassicQ2Error::invalid(
                        "Source armor item is outside its declared native inventory",
                    ));
                }
            }
            _ => return Err(ClassicQ2Error::invalid("Source armor requires Q2 armor values")),
        }
        if powered_kind(&armor.powered) != powered_kind(&self.armor_state(host, slot)?.powered) {
            return Err(ClassicQ2Error::invalid(
                "Native power activation requires its original source equipment operation",
            ));
        }
        Ok(())
    }

    /// Empty-tier regular armor definition.
    pub fn empty_regular_armor(&self, host: &mut ClassicQ2GuestHost) -> ClassicResult<RegularArmor> {
        let definition = self.armor_definition(host, self.profile.armor.empty)?;
        Ok(RegularArmor::Q2 {
            points: 0.0,
            normal_protection: f64::from(definition.normal_protection),
            energy_protection: f64::from(definition.energy_protection),
            item: definition.item,
        })
    }

    /// Read live combat state: health, damageability, mass, armor, team, and
    /// protection flags.
    pub fn read_state(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<CombatState> {
        let address = self.record_address(host, slot)?;
        let client = self.client(host, address)?;
        let flags = host
            .memory
            .read_i32(host.memory.offset(address, self.profile.fields.flags as i64)?)?;
        let team = match client {
            Some(client) => {
                let userinfo_at = host.memory.offset(client, self.profile.client.userinfo as i64)?;
                let info =
                    read_classic_string(&mut host.memory, Some(userinfo_at), self.profile.client.userinfo_bytes)?;
                let skin = info_value_for_key(&info, "skin", self.profile.client.userinfo_bytes + 1)
                    .map_err(|error| ClassicQ2Error::invalid(error.to_string()))?;
                let rules = host.cvars.registry().variable_value("dmflags") as i32;
                if host.cvars.registry().variable_value("coop") != 0.0 {
                    Some("q2:coop".to_string())
                } else if rules as u32 & self.profile.teams.model != 0 {
                    Some({
                        let model = skin.split('/').next().unwrap_or("");
                        format!("q2:model:{model}")
                    })
                } else if rules as u32 & self.profile.teams.skin != 0 {
                    Some({
                        let face = skin.split('/').nth(1).unwrap_or("");
                        format!("q2:skin:{face}")
                    })
                } else {
                    None
                }
            }
            None => None,
        };
        let invulnerable = flags as u32 & self.profile.flags.invulnerable != 0
            || match client {
                Some(client) => {
                    host.memory.read_f32(
                        host.memory
                            .offset(client, self.profile.client.invincible_frame as i64)?,
                    )? > host
                        .memory
                        .read_i32(self.entry(host, self.profile.globals.level_frame)?)? as f32
                }
                None => false,
            };
        Ok(CombatState {
            health: f64::from(
                host.memory
                    .read_i32(host.memory.offset(address, self.profile.fields.health as i64)?)?,
            ),
            armor: self.armor_state(host, slot)?,
            mass: f64::from(
                host.memory
                    .read_i32(host.memory.offset(address, self.profile.fields.mass as i64)?)?,
            ),
            can_take_damage: host
                .memory
                .read_i32(host.memory.offset(address, self.profile.fields.damageable as i64)?)?
                != 0,
            invulnerable,
            no_knockback: flags as u32 & self.profile.flags.no_knockback != 0,
            team,
        })
    }

    /// Write health.
    pub fn write_health(&self, host: &mut ClassicQ2GuestHost, slot: u32, health: i32) -> ClassicResult<()> {
        let address = self.record_address(host, slot)?;
        host.memory
            .write_i32(host.memory.offset(address, self.profile.fields.health as i64)?, health)?;
        Ok(())
    }

    /// Write armor inventories, keeping untouched tiers stable.
    pub fn write_armor(&self, host: &mut ClassicQ2GuestHost, slot: u32, armor: &ArmorState) -> ClassicResult<()> {
        self.validate_armor(host, slot, armor)?;
        let address = self.record_address(host, slot)?;
        let Some(client) = self.client(host, address)? else {
            if armor.regular != RegularArmor::None || armor.powered != PoweredProtection::None {
                return Err(ClassicQ2Error::invalid("Source non-client has no inventory armor"));
            }
            return Ok(());
        };
        let current = self.armor_state(host, slot)?.regular;
        for index in &self.profile.armor.regular {
            let name = self.armor_definition(host, *index)?.item;
            if let (RegularArmor::Q2 { item: next, .. }, RegularArmor::Q2 { item: previous, .. }) =
                (&armor.regular, &current)
            {
                if next == previous && name != *previous {
                    continue;
                }
            }
            let points = if let RegularArmor::Q2 { item, points, .. } = &armor.regular {
                if item == &name {
                    *points as i32
                } else {
                    0
                }
            } else {
                0
            };
            let at = host
                .memory
                .offset(client, (self.profile.client.inventory + index * 4) as i64)?;
            if host.memory.read_i32(at)? != points {
                host.memory.write_i32(at, points)?;
            }
        }
        if armor.powered != PoweredProtection::None {
            let cells = match armor.powered {
                PoweredProtection::Shield { cells } | PoweredProtection::Screen { cells } => cells,
                PoweredProtection::None => 0,
            };
            host.memory.write_i32(
                host.memory.offset(
                    client,
                    (self.profile.client.inventory + self.profile.items.cells * 4) as i64,
                )?,
                cells,
            )?;
        }
        Ok(())
    }

    /// Bind an armor intercept for one slot and channel.
    pub fn bind_armor_stage(
        &mut self,
        slot: u32,
        channel: ProtectionChannel,
        intercept: ArmorIntercept,
    ) -> ClassicResult<ArmorStageLease> {
        self.check_open()?;
        match self.armor_intercepts.entry((slot, channel)) {
            Entry::Occupied(_) => {
                return Err(ClassicQ2Error::invalid(
                    "Classic source armor stage already has an owner",
                ));
            }
            Entry::Vacant(entry) => {
                entry.insert(intercept);
            }
        }
        Ok(ArmorStageLease { slot, channel })
    }

    /// Release an armor intercept.
    pub fn release_armor_stage(&mut self, lease: &ArmorStageLease) {
        self.armor_intercepts.remove(&(lease.slot, lease.channel));
    }

    /// Push a source damage frame. `apply_damage` manages frames itself;
    /// direct armor-stage drivers bracket `run_armor_stage` with these.
    pub fn begin_frame(&mut self, request: ClassicDamageRequest, slot: u32) -> ClassicResult<()> {
        self.check_open()?;
        self.frames.push(DamageFrame { request, slot });
        Ok(())
    }

    /// Pop the current source damage frame.
    pub fn end_frame(&mut self) {
        self.frames.pop();
    }

    /// Run one armor stage through its intercept, if bound.
    pub fn run_armor_stage(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        slot: u32,
        channel: ProtectionChannel,
        strike: &ArmorStrike,
    ) -> ClassicResult<i32> {
        self.check_open()?;
        let frame = self
            .frames
            .last()
            .ok_or_else(|| ClassicQ2Error::invalid("Classic armor stage has no active source damage request"))?;
        if frame.slot != slot {
            return Err(ClassicQ2Error::invalid(
                "Classic armor stage has no active source damage request",
            ));
        }
        let occupant = host.registry.at_source(&host.provider, slot);
        if !occupant
            .as_ref()
            .is_some_and(|actor| *actor.id() == frame.request.target)
        {
            return Err(ClassicQ2Error::invalid(
                "Classic armor stage has no active source damage request",
            ));
        }
        let args = ArmorStageArgs {
            target: frame.request.target.clone(),
            amount: strike.amount,
            direction: frame.request.direction,
            point: strike.point,
            normal: strike.normal,
            flags: q2_native_armor_flags(strike.flags),
        };
        let operation = if channel == ProtectionChannel::Regular {
            ClassicCombatOperation::RegularArmor
        } else {
            ClassicCombatOperation::PowerArmor
        };
        let address = self.record_address(host, slot)?;
        let strike = *strike;
        let run_original = |bindings: &mut Self, host: &mut ClassicQ2GuestHost| -> ClassicResult<i32> {
            let scratch = host
                .memory
                .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(24))?;
            write_classic_vector(&mut host.memory, scratch, strike.point)?;
            let normal_at = host.memory.offset(scratch, 12)?;
            write_classic_vector(&mut host.memory, normal_at, strike.normal)?;
            let mut semantic = vec![
                GuestCallValue::Pointer(Some(address)),
                GuestCallValue::Pointer(Some(scratch)),
                GuestCallValue::Pointer(Some(normal_at)),
                GuestCallValue::Int32(strike.amount),
            ];
            if channel == ProtectionChannel::Regular {
                semantic.push(GuestCallValue::Int32(strike.sparks));
            }
            semantic.push(GuestCallValue::Int32(strike.flags));
            let call = if channel == ProtectionChannel::Regular {
                &bindings.profile.calls.regular_armor
            } else {
                &bindings.profile.calls.power_armor
            };
            let lowered = lower_native_combat_arguments(
                call,
                operation,
                &semantic,
                &mut host.memory,
                Some(bindings.image),
                None,
            )?;
            let entry = bindings.entry(
                host,
                if channel == ProtectionChannel::Regular {
                    bindings.profile.entries.regular_armor
                } else {
                    bindings.profile.entries.power_armor
                },
            )?;
            let signature = if channel == ProtectionChannel::Regular {
                bindings.regular_signature.clone()
            } else {
                bindings.power_signature.clone()
            };
            let previous = bindings.bypass;
            bindings.bypass = Some(channel);
            let result = host.invoke(entry, &signature, &lowered, host.instruction_budget);
            bindings.bypass = previous;
            host.memory.unmap(scratch, 24)?;
            let GuestCallResult::Value(GuestCallValue::Int32(saved)) = result? else {
                return Err(ClassicQ2Error::invalid(
                    "Classic armor stage returned a non-integer result",
                ));
            };
            Ok(saved)
        };
        if self.bypass == Some(channel) {
            return run_original(&mut *self, &mut *host);
        }
        let key = (slot, channel);
        let Some(mut intercept) = self.armor_intercepts.remove(&key) else {
            return run_original(&mut *self, &mut *host);
        };
        let mut original = || run_original(&mut *self, &mut *host);
        let result = intercept(&args, &mut original);
        self.armor_intercepts.insert(key, intercept);
        let saved = result?;
        if !host.registry.is_live(&args.target) {
            return Err(ClassicQ2Error::TargetRemoved);
        }
        Ok(saved)
    }

    fn party_address(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        actor: Option<&ActorId>,
        temporary: &mut Vec<GuestAddress>,
    ) -> ClassicResult<GuestAddress> {
        let Some(actor) = actor else {
            return self.record_address(host, 0);
        };
        if let Some((provider, slot)) = host.registry.source_of(actor) {
            if provider == host.provider {
                return self.record_address(host, slot);
            }
        }
        let body = self
            .projected_bodies
            .get(actor)
            .copied()
            .ok_or_else(|| ClassicQ2Error::invalid("Foreign damage actor has no body to project"))?;
        let spawn = self.entry(host, self.profile.entries.spawn)?;
        let signature = classic_signature(vec![], Some(q2_pointer()), false);
        let result = host.invoke(spawn, &signature, &[], host.instruction_budget)?;
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(value))) = result else {
            return Err(ClassicQ2Error::invalid("Source G_Spawn returned no damage projection"));
        };
        temporary.push(value);
        for (offset, vector) in [
            (4i64, body.origin),
            (16, body.angles),
            (188, body.mins),
            (200, body.maxs),
            (self.profile.fields.velocity as i64, body.velocity),
        ] {
            let at = host.memory.offset(value, offset)?;
            write_classic_vector(&mut host.memory, at, vector)?;
        }
        Ok(value)
    }

    /// Run source damage through the native entry, snapshotting stores.
    pub fn apply_damage(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        request: &ClassicDamageRequest,
    ) -> ClassicResult<DamageOutcome> {
        self.check_open()?;
        let slot = self.slot_of(host, &request.target)?;
        let address = self.record_address(host, slot)?;
        {
            let edicts = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
            if edicts.at(&mut host.memory, slot)?.stride_bytes != self.profile.entity_bytes {
                return Err(ClassicQ2Error::invalid(
                    "Native combat declaration differs from the source edict stride",
                ));
            }
        }
        self.begin_frame(request.clone(), slot)?;
        let outcome = self.apply_framed(host, request, slot, address);
        self.end_frame();
        host.reconcile()?;
        outcome
    }

    fn apply_framed(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        request: &ClassicDamageRequest,
        slot: u32,
        address: GuestAddress,
    ) -> ClassicResult<DamageOutcome> {
        let before_health = host
            .memory
            .read_i32(host.memory.offset(address, self.profile.fields.health as i64)?)?;
        let before_armor = self.armor_state(host, slot)?;
        let velocity_at = host.memory.offset(address, self.profile.fields.velocity as i64)?;
        let before_velocity = read_classic_vector(&mut host.memory, velocity_at)?;
        let mut temporary = Vec::new();
        let attacker = self.party_address(host, request.attacker.as_ref(), &mut temporary)?;
        let inflictor = if request.attacker.is_some() && request.inflictor == request.attacker {
            attacker
        } else {
            self.party_address(host, request.inflictor.as_ref(), &mut temporary)?
        };
        let vectors = host
            .memory
            .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(36))?;
        write_classic_vector(&mut host.memory, vectors, request.direction)?;
        let point_at = host.memory.offset(vectors, 12)?;
        write_classic_vector(&mut host.memory, point_at, request.point)?;
        let normal_at = host.memory.offset(vectors, 24)?;
        write_classic_vector(&mut host.memory, normal_at, request.normal)?;
        let (damage_flags, native) = q2_native_damage_arguments(request, self.profile.game)?;
        let semantic = vec![
            GuestCallValue::Pointer(Some(address)),
            GuestCallValue::Pointer(Some(inflictor)),
            GuestCallValue::Pointer(Some(attacker)),
            GuestCallValue::Pointer(Some(vectors)),
            GuestCallValue::Pointer(Some(host.memory.offset(vectors, 12)?)),
            GuestCallValue::Pointer(Some(host.memory.offset(vectors, 24)?)),
            GuestCallValue::Int32(request.amount),
            GuestCallValue::Int32(request.knockback),
            GuestCallValue::Int32(damage_flags),
            GuestCallValue::Int32(native),
        ];
        let lowered = lower_native_combat_arguments(
            &self.profile.calls.damage,
            ClassicCombatOperation::Damage,
            &semantic,
            &mut host.memory,
            Some(self.image),
            None,
        )?;
        let entry = self.entry(host, self.profile.entries.damage)?;
        let signature = self.damage_signature.clone();
        let budget = host.instruction_budget;
        let invoked = host.invoke(entry, &signature, &lowered, budget);
        host.memory.unmap(vectors, 36)?;
        let free = self.entry(host, self.profile.entries.free)?;
        let free_signature = classic_signature(vec![q2_pointer()], None, false);
        for value in temporary {
            host.invoke(free, &free_signature, &[GuestCallValue::Pointer(Some(value))], budget)?;
        }
        invoked?;
        if !host.registry.is_live(&request.target) {
            return Err(ClassicQ2Error::TargetRemoved);
        }
        let after_health = host
            .memory
            .read_i32(host.memory.offset(address, self.profile.fields.health as i64)?)?;
        let after_armor = self.armor_state(host, slot)?;
        let velocity_at = host.memory.offset(address, self.profile.fields.velocity as i64)?;
        let after_velocity = read_classic_vector(&mut host.memory, velocity_at)?;
        let mut observations = Vec::new();
        if after_health != before_health {
            observations.push(StoredObservation::Health {
                before: before_health,
                after: after_health,
            });
        }
        if after_velocity != before_velocity {
            observations.push(StoredObservation::Velocity {
                before: before_velocity,
                after: after_velocity,
            });
        }
        if after_armor != before_armor {
            observations.push(StoredObservation::Armor {
                before: before_armor,
                after: after_armor,
            });
        }
        let applied_damage = before_health - after_health;
        Ok(DamageOutcome {
            applied_damage,
            reaction: reaction_for_health(f64::from(after_health), applied_damage == 0),
            observations,
        })
    }

    /// Handle a guest-initiated native damage call.
    pub fn incoming_damage(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        args: &[GuestCallValue],
    ) -> ClassicResult<DamageOutcome> {
        self.check_open()?;
        let semantic = read_native_combat_arguments(&self.profile.calls.damage, ClassicCombatOperation::Damage, args)?;
        let mut actors = Vec::with_capacity(3);
        for (position, value) in semantic.iter().take(3).enumerate() {
            let GuestCallValue::Pointer(Some(address)) = value else {
                return Err(ClassicQ2Error::invalid(format!(
                    "API 3 argument {position} cannot be null"
                )));
            };
            actors.push(
                host.observe_edict(*address)?
                    .ok_or_else(|| ClassicQ2Error::invalid("Classic damage references a retired source actor"))?,
            );
        }
        let mut vectors = Vec::with_capacity(3);
        for (offset, value) in semantic.iter().skip(3).take(3).enumerate() {
            let GuestCallValue::Pointer(Some(address)) = value else {
                let index = offset + 3;
                return Err(ClassicQ2Error::invalid(format!(
                    "API 3 argument {index} cannot be null"
                )));
            };
            vectors.push(read_classic_vector(&mut host.memory, *address)?);
        }
        let number = |index: usize| {
            if let Some(GuestCallValue::Int32(value)) = semantic.get(index) {
                Ok(*value)
            } else {
                Err(ClassicQ2Error::invalid(format!(
                    "API 3 argument {index} must be numeric"
                )))
            }
        };
        let flags = number(8)?;
        let native = number(9)?;
        let means_of_death = canonical_cause_from_native(self.profile.game, native)
            .ok_or_else(|| ClassicQ2Error::invalid("Unclassified classic damage cause"))?;
        let request = ClassicDamageRequest {
            target: actors[0].id().clone(),
            attacker: Some(actors[2].id().clone()),
            inflictor: Some(actors[1].id().clone()),
            amount: number(6)?,
            knockback: number(7)?,
            direction: vectors[0],
            point: vectors[1],
            normal: vectors[2],
            delivery: if flags & 1 != 0 {
                Delivery::Radius
            } else {
                Delivery::Direct
            },
            cause: ClassicDamageCause {
                means_of_death,
                damage_flags: flags,
                native_value: native,
            },
        };
        self.apply_damage(host, &request)
    }
}

/// Normalize a legacy power-only armor view that fabricated an empty
/// regular tier from the placeholder item.
#[must_use]
pub fn normalize_legacy_armor(legacy: &ArmorState, current: &ArmorState) -> ArmorState {
    let placeholder = item_id("q2", "none");
    let fabricated = matches!(
        &legacy.regular,
        RegularArmor::Q2 { item, points, normal_protection, energy_protection }
            if *item == placeholder && *points == 0.0 && *normal_protection == 0.0 && *energy_protection == 0.0
    );
    if current.regular == RegularArmor::None
        && fabricated
        && powered_kind(&legacy.powered) == powered_kind(&current.powered)
        && legacy.powered != PoweredProtection::None
        && legacy.powered == current.powered
    {
        ArmorState {
            regular: RegularArmor::None,
            powered: legacy.powered.clone(),
        }
    } else {
        legacy.clone()
    }
}

fn powered_kind(powered: &PoweredProtection) -> u8 {
    match powered {
        PoweredProtection::None => 0,
        PoweredProtection::Screen { .. } => 1,
        PoweredProtection::Shield { .. } => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::super::combat_profile::{xatrix_combat_profile, XATRIX_DIGEST_VALUE};
    use super::super::records::{allocate_classic_string, ClassicQ2Edicts};
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{
        ContentDigest, GuestAllocationOptions, GuestMapOptions, GuestPermissions, ModuleIdentity,
    };

    const CODE_BASE: u64 = 0x20000000;

    fn test_host() -> ClassicQ2GuestHost {
        ClassicQ2GuestHost::new(
            ProviderId::new("q2", "classic"),
            ModuleIdentity::new(
                ProviderId::new("q2", "classic"),
                "gamex86.dll",
                ContentDigest::new("sha256", XATRIX_DIGEST_VALUE),
                "test",
            ),
            100000,
        )
        .unwrap()
    }

    fn fixture(host: &mut ClassicQ2GuestHost) -> GuestAddress {
        host.memory
            .map(&GuestMapOptions {
                base: CODE_BASE,
                byte_length: 0x80000,
                permissions: GuestPermissions::ReadWriteExecute,
                label: "combat code".to_string(),
                bytes: None,
            })
            .unwrap();
        let image = GuestAddress::new(host.memory.address_space(), CODE_BASE);
        let profile = xatrix_combat_profile();
        let item_base = GuestAddress::new(
            host.memory.address_space(),
            image.offset + u64::from(profile.globals.item_list),
        );
        for (index, name, normal, energy) in [
            (1, "item_armor_body", 0.6f32, 0.6f32),
            (2, "item_armor_combat", 0.5, 0.5),
            (3, "item_armor_jacket", 0.3, 0.0),
        ] {
            let record = host.memory.offset(item_base, index * 76).unwrap();
            let classname = allocate_classic_string(&mut host.memory, name).unwrap();
            let info = host.memory.allocate(&GuestAllocationOptions::bytes(16)).unwrap();
            host.memory.write_pointer(record, Some(classname)).unwrap();
            host.memory
                .write_pointer(host.memory.offset(record, 64).unwrap(), Some(info))
                .unwrap();
            host.memory
                .write_f32(host.memory.offset(info, 8).unwrap(), normal)
                .unwrap();
            host.memory
                .write_f32(host.memory.offset(info, 12).unwrap(), energy)
                .unwrap();
        }
        let client = host.memory.allocate(&GuestAllocationOptions::bytes(4096)).unwrap();
        host.memory
            .write_i32(host.memory.offset(client, 740 + 3 * 4).unwrap(), 50)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(client, 740 + 23 * 4).unwrap(), 0)
            .unwrap();
        let userinfo_at = host.memory.offset(client, 188).unwrap();
        super::super::records::write_classic_string(&mut host.memory, userinfo_at, "\\skin\\male/grunt", 512).unwrap();
        let edicts = host.memory.allocate(&GuestAllocationOptions::bytes(896 * 4)).unwrap();
        let exports = host.memory.allocate(&GuestAllocationOptions::bytes(80)).unwrap();
        host.memory.write_i32(exports, 3).unwrap();
        host.memory
            .write_pointer(host.memory.offset(exports, 64).unwrap(), Some(edicts))
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(exports, 68).unwrap(), 896)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(exports, 72).unwrap(), 4)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(exports, 76).unwrap(), 4)
            .unwrap();
        let one = host.memory.offset(edicts, 896).unwrap();
        host.memory.write_i32(host.memory.offset(one, 88).unwrap(), 1).unwrap();
        host.memory
            .write_pointer(host.memory.offset(one, 84).unwrap(), Some(client))
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(one, 480).unwrap(), 100)
            .unwrap();
        host.memory.write_i32(host.memory.offset(one, 512).unwrap(), 1).unwrap();
        host.memory
            .write_i32(host.memory.offset(one, 400).unwrap(), 200)
            .unwrap();
        host.edicts =
            Some(ClassicQ2Edicts::new(&mut host.memory, exports, ProviderId::new("q2", "classic"), None).unwrap());
        host.observe_edict(one).unwrap();
        image
    }

    #[test]
    fn armor_causes_and_state_read_live_words() {
        let mut host = test_host();
        let image = fixture(&mut host);
        let bindings = ClassicCombatBindings::create(&mut host, image, None).unwrap().unwrap();
        assert_eq!(canonical_cause_from_native(ClassicGame::Xatrix, 9), Some(9));
        assert_eq!(canonical_cause_from_native(ClassicGame::Xatrix, 90), None);
        assert_eq!(native_cause_from_canonical(ClassicGame::Xatrix, 9), Some(9));
        assert!(!bindings.notarget(&mut host, 1).unwrap());
        let armor = bindings.armor_state(&mut host, 1).unwrap();
        assert!(matches!(armor.regular, RegularArmor::Q2 { points: 50.0, .. }));
        assert_eq!(armor.powered, PoweredProtection::None);
        bindings.validate_armor(&mut host, 1, &armor).unwrap();
        let state = bindings.read_state(&mut host, 1).unwrap();
        assert_eq!(state.health, 100.0);
        assert!(state.can_take_damage);
        assert_eq!(state.mass, 200.0);
        assert!(!state.invulnerable);
        assert!(!state.no_knockback);
        let flags = q2_native_armor_flags(0x104);
        assert!(flags.no_power_armor && flags.energy && !flags.no_armor);
    }

    #[test]
    fn damage_applies_through_native_entry_with_observations() {
        let mut host = test_host();
        let image = fixture(&mut host);
        let mut bindings = ClassicCombatBindings::create(&mut host, image, None).unwrap().unwrap();
        let profile = xatrix_combat_profile();
        let damage = image.offset + u64::from(profile.entries.damage);
        let free = image.offset + u64::from(profile.entries.free);
        host.register_guest_handler(
            damage,
            Box::new(|memory, args| {
                let GuestCallValue::Pointer(Some(target)) = &args[0] else {
                    panic!("damage target must be a pointer");
                };
                let health_at = memory.offset(*target, 480).unwrap();
                let health = memory.read_i32(health_at).unwrap();
                let GuestCallValue::Int32(amount) = &args[6] else {
                    panic!("damage amount must be int");
                };
                memory.write_i32(health_at, health - amount).unwrap();
                Ok(GuestCallResult::Void)
            }),
        );
        host.register_guest_handler(free, Box::new(|_, _| Ok(GuestCallResult::Void)));
        let target = host.registry.at_source(&ProviderId::new("q2", "classic"), 1).unwrap();
        let request = ClassicDamageRequest {
            target: target.id().clone(),
            attacker: None,
            inflictor: None,
            amount: 30,
            knockback: 0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            delivery: Delivery::Direct,
            cause: ClassicDamageCause {
                means_of_death: 9,
                damage_flags: 0,
                native_value: 9,
            },
        };
        let outcome = bindings.apply_damage(&mut host, &request).unwrap();
        assert_eq!(outcome.applied_damage, 30);
        assert_eq!(outcome.reaction, Reaction::Pain);
        assert_eq!(outcome.observations.len(), 1);
        assert!(matches!(
            outcome.observations[0],
            StoredObservation::Health { before: 100, after: 70 }
        ));
        let state = bindings.read_state(&mut host, 1).unwrap();
        assert_eq!(state.health, 70.0);
        bindings
            .write_armor(
                &mut host,
                1,
                &ArmorState {
                    regular: RegularArmor::Q2 {
                        points: 25.0,
                        normal_protection: 0.3,
                        energy_protection: 0.0,
                        item: "q2:item_armor_jacket".to_string(),
                    },
                    powered: PoweredProtection::None,
                },
            )
            .unwrap();
        let armor = bindings.armor_state(&mut host, 1).unwrap();
        assert!(matches!(armor.regular, RegularArmor::Q2 { points: 25.0, .. }));
    }

    #[test]
    fn armor_stages_intercept_with_original_fallback() {
        let mut host = test_host();
        let image = fixture(&mut host);
        let mut bindings = ClassicCombatBindings::create(&mut host, image, None).unwrap().unwrap();
        let profile = xatrix_combat_profile();
        let regular = image.offset + u64::from(profile.entries.regular_armor);
        host.register_guest_handler(
            regular,
            Box::new(|_, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(12)))),
        );
        let target = host.registry.at_source(&ProviderId::new("q2", "classic"), 1).unwrap();
        let request = ClassicDamageRequest {
            target: target.id().clone(),
            attacker: None,
            inflictor: None,
            amount: 30,
            knockback: 0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            delivery: Delivery::Direct,
            cause: ClassicDamageCause {
                means_of_death: 9,
                damage_flags: 0,
                native_value: 9,
            },
        };
        bindings.begin_frame(request.clone(), 1).unwrap();
        let strike = ArmorStrike {
            amount: 30,
            point: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            sparks: 1,
            flags: 0,
        };
        assert_eq!(
            bindings
                .run_armor_stage(&mut host, 1, ProtectionChannel::Regular, &strike)
                .unwrap(),
            12
        );
        let lease = bindings
            .bind_armor_stage(
                1,
                ProtectionChannel::Regular,
                Box::new(|args, original| {
                    assert_eq!(args.amount, 30);
                    Ok(original()? * 2)
                }),
            )
            .unwrap();
        assert!(bindings
            .bind_armor_stage(1, ProtectionChannel::Regular, Box::new(|_, _| Ok(0)))
            .is_err());
        assert_eq!(
            bindings
                .run_armor_stage(&mut host, 1, ProtectionChannel::Regular, &strike)
                .unwrap(),
            24
        );
        bindings.release_armor_stage(&lease);
        assert_eq!(
            bindings
                .run_armor_stage(&mut host, 1, ProtectionChannel::Regular, &strike)
                .unwrap(),
            12
        );
        bindings.end_frame();
        assert!(bindings
            .run_armor_stage(&mut host, 1, ProtectionChannel::Regular, &strike)
            .is_err());
        let legacy = ArmorState {
            regular: RegularArmor::Q2 {
                points: 0.0,
                normal_protection: 0.0,
                energy_protection: 0.0,
                item: "q2:none".to_string(),
            },
            powered: PoweredProtection::Shield { cells: 10 },
        };
        let current = ArmorState {
            regular: RegularArmor::None,
            powered: PoweredProtection::Shield { cells: 10 },
        };
        assert_eq!(normalize_legacy_armor(&legacy, &current).regular, RegularArmor::None);
        assert_eq!(normalize_legacy_armor(&current, &current), current);
    }
}
