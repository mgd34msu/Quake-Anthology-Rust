//! Port of `src/compat/q2/native-primary-validation.ts`.
//! Bridges admission checks: exact source records and executable addresses.

use qa_guest::core::contracts::GuestLayout;

use super::native_combat_call::CombatArgument;
use super::native_primary::{NativePrimaryProfile, PrimaryEdition};
use super::native_primary_commands::ItemAmmo;
use super::native_pickups::{PickupSupply, TimeStorage};
use super::native_primary_reader::{NativeItemField, NativeItemTest, NativeRegion, RecordKind};
use super::native_primary_weapons::{DelayEvaluate, WeaponDamage};

/// One synthetic image section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeSection {
    /// Section RVA.
    pub rva: u32,
    /// Mapped length in bytes.
    pub mapped_size: u32,
    /// Whether the section is executable.
    pub executable: bool,
}

/// Synthetic PE image standing in for the loaded module. Sections are
/// readable by construction; only executability varies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticPeImage {
    /// Image size in bytes.
    pub image_size: u32,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
    /// Image sections.
    pub sections: Vec<PeSection>,
}

/// Field offset within a record layout, if declared.
#[must_use]
pub fn layout_field_offset(layout: &GuestLayout, name: &str) -> Option<u32> {
    layout
        .fields
        .iter()
        .find(|field| field.name == name)
        .and_then(|field| u32::try_from(field.byte_offset).ok())
}

/// Admission checks the exact source records and executable addresses before
/// init can run.
pub fn validate_native_primary(profile: &NativePrimaryProfile, pe: &SyntheticPeImage) -> Result<(), String> {
    let weapons = profile.weapons();
    let player = profile.player();
    let commands = profile.commands();
    let inventory = profile.inventory();
    let drop = profile.drop();
    let pickups = profile.pickups();
    let edition = profile.edition();
    let entity_bytes = match profile {
        NativePrimaryProfile::Classic { world, .. } => world.entity_bytes,
        NativePrimaryProfile::Rerelease { world, .. } => world.edict.byte_length as u32,
    };
    let client_bytes = weapons.client.byte_length;
    let pointer_bytes = pe.pointer_bytes as u32;

    let bound = |offset: u32, bytes: u32, limit: u32| -> Result<(), String> {
        if bytes < 1 || u64::from(offset) + u64::from(bytes) > u64::from(limit) {
            return Err("native primary field exceeds its declared source record".to_string());
        }
        Ok(())
    };
    let image = |offset: u32, bytes: u32, execute: bool| -> Result<(), String> {
        bound(offset, bytes, pe.image_size)?;
        let covered = pe.sections.iter().any(|section| {
            u64::from(offset) >= u64::from(section.rva)
                && u64::from(offset) + u64::from(bytes) <= u64::from(section.rva) + u64::from(section.mapped_size)
                && (!execute || section.executable)
        });
        if covered {
            Ok(())
        } else {
            Err("native primary address has no matching image section".to_string())
        }
    };
    let record = |kind: RecordKind, offset: u32, bytes: u32| -> Result<(), String> {
        match kind {
            RecordKind::Image => image(offset, bytes, false),
            RecordKind::Entity => bound(offset, bytes, entity_bytes),
            RecordKind::Client => bound(offset, bytes, client_bytes),
        }
    };
    let field = |value: &NativeItemField| -> Result<(), String> {
        record(value.record, value.offset, value.encoding.width() as u32)
    };
    let test = |value: &NativeItemTest| -> Result<(), String> {
        match value {
            NativeItemTest::Scalar { field: item_field, .. } => field(item_field),
            NativeItemTest::Pointer {
                record: kind,
                offset,
                value,
            } => {
                record(*kind, *offset, pointer_bytes)?;
                if let Some(expected) = value {
                    image(expected.rva, pointer_bytes, false)?;
                }
                Ok(())
            }
        }
    };

    let world_addresses: Vec<(u32, u32)> = match profile {
        NativePrimaryProfile::Classic { world, .. } => [
            &world.calls.pain,
            &world.calls.death,
            &world.calls.damage,
            &world.calls.regular_armor,
            &world.calls.power_armor,
        ]
        .into_iter()
        .flat_map(|call| call.arguments.iter())
        .filter_map(|argument| match argument {
            CombatArgument::Address(Some(address)) => {
                let base = u64::from(address.rva) + u64::from(address.indirections.first().copied().unwrap_or(0));
                let bytes = if address.indirections.is_empty() {
                    1
                } else {
                    pointer_bytes
                };
                u32::try_from(base).ok().map(|base| (base, bytes))
            }
            _ => None,
        })
        .collect(),
        NativePrimaryProfile::Rerelease { world, .. } => [
            &world.calls.pain,
            &world.calls.death,
            &world.calls.process_pain,
            &world.calls.damage,
            &world.calls.power_armor,
        ]
        .into_iter()
        .flat_map(|call| call.arguments.iter())
        .filter_map(|argument| match argument {
            CombatArgument::Address(Some(address)) => {
                let base = u64::from(address.rva) + u64::from(address.indirections.first().copied().unwrap_or(0));
                let bytes = if address.indirections.is_empty() {
                    1
                } else {
                    pointer_bytes
                };
                u32::try_from(base).ok().map(|base| (base, bytes))
            }
            _ => None,
        })
        .collect(),
    };
    for (base, bytes) in world_addresses {
        image(base, bytes, false)?;
    }

    let mut entries = vec![
        weapons.spawn.entry,
        weapons.attack_animation.entry,
        player.spawn,
        commands.give.entry,
        commands.give.weapons,
        commands.give.ammo,
        commands.drop.entry,
        inventory.next.entry,
        inventory.next.scan,
        inventory.previous.entry,
        inventory.previous.scan,
        inventory.validate.entry,
        inventory.use_profile.entry,
        inventory.use_profile.call,
        inventory.named_use.entry,
        inventory.named_use.call,
        inventory.named_use.lookup_call,
        inventory.named_use.lookup_return,
        drop.named,
        drop.inventory.entry,
        drop.inventory.admitted,
        drop.find,
        drop.lookup_return,
        drop.allocate,
        drop.free,
        pickups.touch,
        pickups.grant_return,
        pickups.targets_return,
        pickups.supply.ammo.entry,
        // The dispatcher entry is an RVA by construction.
        weapons.dispatcher.entry_rva,
    ];
    let mut regions: Vec<NativeRegion> = Vec::new();
    regions.extend(weapons.decisions.iter().map(|decision| NativeRegion {
        entry: decision.entry,
        join: decision.join,
    }));
    regions.extend(weapons.attack_animation.skip.iter().copied());
    regions.push(weapons.delay.region);
    regions.push(commands.give.unknown);
    regions.extend(commands.give.ammo_grants.iter().map(|grant| NativeRegion {
        entry: grant.entry,
        join: grant.join,
    }));
    regions.push(commands.drop.eligibility);
    regions.push(NativeRegion {
        entry: inventory.next.entry,
        join: inventory.next.join,
    });
    regions.push(NativeRegion {
        entry: inventory.previous.entry,
        join: inventory.previous.join,
    });
    regions.push(NativeRegion {
        entry: inventory.use_profile.entry,
        join: inventory.use_profile.join,
    });
    regions.push(NativeRegion {
        entry: inventory.named_use.entry,
        join: inventory.named_use.join,
    });
    regions.extend(drop.callbacks.iter().copied());
    regions.extend(drop.debits.iter().copied());
    if let Some(scan) = inventory.validate.scan {
        regions.push(scan);
    }
    if let Some(consumer) = drop.consumer {
        regions.push(consumer);
    }
    if let super::native_primary_player::PlayerObjectives::Entry(entry) = player.objectives {
        entries.push(entry);
    }
    match &weapons.damage {
        WeaponDamage::SourceResult { entry, .. } => entries.push(*entry),
        WeaponDamage::SourceFlag {
            address,
            encoding,
            region,
            ..
        } => {
            image(*address, encoding.width() as u32, false)?;
            regions.push(*region);
        }
    }
    if let DelayEvaluate::SourceAnimation {
        entry,
        baseline_milliseconds,
        projection,
        writes,
    } = &weapons.delay.evaluate
    {
        entries.push(*entry);
        for write in writes {
            field(write)?;
        }
        for write in projection {
            field(&write.field)?;
        }
        if *baseline_milliseconds <= 0.0 {
            return Err("original weapon animation interval must be positive".to_string());
        }
    }
    for grant in &pickups.grants {
        entries.push(grant.entry);
        regions.push(grant.recipient);
        for consumer in &grant.consumers {
            entries.push(consumer.entry);
        }
        match &grant.supply {
            Some(PickupSupply::Ammo { entry, .. }) => entries.push(*entry),
            Some(PickupSupply::Weapon {
                ammo_return,
                settle,
                autoswitch,
            }) => {
                entries.push(*ammo_return);
                entries.push(*settle);
                regions.push(*autoswitch);
            }
            None => {}
        }
    }
    if let Some(stop) = pickups.supply.ammo.stop {
        entries.push(stop);
    }
    for region in &regions {
        entries.push(region.entry);
        entries.push(region.join);
        if region.entry == region.join {
            return Err("native source region is empty".to_string());
        }
    }
    for entry in &entries {
        image(*entry, 1, true)?;
    }

    for value in [
        weapons.entity.water_level,
        weapons.entity.view_height,
        weapons.entity.max_health,
        weapons.client.buttons,
        weapons.client.latched_buttons,
        weapons.delay.flag,
        weapons.animation.frame,
        weapons.animation.end,
        weapons.animation.priority,
        weapons.animation.duck,
        weapons.animation.run,
    ] {
        field(&value)?;
    }
    for decision in &weapons.decisions {
        for value in &decision.fields {
            field(&value.field)?;
            if value.clear_mask < 1 {
                return Err("native input mask is outside uint32".to_string());
            }
        }
    }
    for value in weapons
        .spawn
        .accepted
        .iter()
        .chain(weapons.active.iter())
        .chain(weapons.committed_input.iter().flatten())
        .chain(weapons.continuations.iter().flatten())
    {
        test(value)?;
    }
    if weapons.entity.max_health.record != RecordKind::Entity
        || weapons.entity.max_health.encoding != super::native_primary_reader::NativeScalar::Int32
    {
        return Err("native source max health requires its declared int32 entity field".to_string());
    }
    bound(weapons.client.view_angles, 12, client_bytes)?;
    bound(player.command_angles, 12, client_bytes)?;
    if let Some(match_profile) = &player.match_profile {
        bound(match_profile.score, 4, client_bytes)?;
        if match_profile.score % 4 != 0 {
            return Err("native score field requires int32 alignment".to_string());
        }
    }
    if let Some(forward) = player.forward {
        bound(forward, 12, client_bytes)?;
    }
    bound(player.velocity, 12, entity_bytes)?;
    if inventory.client != if edition == PrimaryEdition::Classic { 84 } else { 120 } {
        return Err("native client pointer differs from the selected public game API".to_string());
    }
    bound(inventory.inventory, inventory.count * 4, client_bytes)?;
    bound(inventory.cursor, 4, client_bytes)?;
    for span in &inventory.selection_writes {
        bound(span.offset, span.bytes, client_bytes)?;
    }
    for offset in [commands.client.weapon, drop.client.weapon, drop.client.pending] {
        bound(offset, pointer_bytes, client_bytes)?;
    }
    if let Some(ammo_index) = commands.client.ammo_index {
        bound(ammo_index, 4, client_bytes)?;
    }
    for offset in &pickups.supply.ammo.capacities {
        bound(*offset, u32::from(pickups.supply.ammo.capacity_bytes), client_bytes)?;
    }
    image(commands.give.argc, pointer_bytes, false)?;
    image(commands.give.argv, pointer_bytes, false)?;
    let stride = commands.items.stride;
    bound(commands.items.classname, pointer_bytes, stride)?;
    bound(commands.items.icon, pointer_bytes, stride)?;
    bound(pickups.items.pickup, pointer_bytes, stride)?;
    bound(commands.items.flags, 4, stride)?;
    bound(pickups.supply.flags, 4, stride)?;
    bound(pickups.supply.ammo.tag, 4, stride)?;
    match commands.items.ammo {
        ItemAmmo::Name { offset, label } => {
            bound(offset, pointer_bytes, stride)?;
            bound(label, pointer_bytes, stride)?;
        }
        ItemAmmo::Index { offset } => {
            bound(offset, 4, stride)?;
        }
    }
    image(commands.items.table, commands.items.count * stride, false)?;
    image(weapons.time.address, weapons.time.encoding.width() as u32, false)?;
    image(
        pickups.time.address,
        if pickups.time.storage == TimeStorage::FloatSeconds {
            4
        } else {
            8
        },
        false,
    )?;
    bound(pickups.entity.count, 4, entity_bytes)?;
    bound(pickups.entity.spawnflags, 4, entity_bytes)?;
    bound(pickups.entity.item, pointer_bytes, entity_bytes)?;
    bound(
        pickups.entity.inuse,
        u32::from(pickups.entity.inuse_bytes),
        entity_bytes,
    )?;
    if let Some(generation) = pickups.entity.generation {
        bound(generation, 4, entity_bytes)?;
    }
    if commands.items.weapon_flag != pickups.supply.weapon_flag
        || commands.items.classname != pickups.items.classname
        || commands.items.flags != pickups.supply.flags
        || inventory.cursor != drop.client.cursor
        || commands.client.weapon != drop.client.weapon
    {
        return Err("native primary interfaces disagree about original source fields".to_string());
    }

    match profile {
        NativePrimaryProfile::Classic { world, .. } => {
            if world.client.inventory != inventory.inventory
                || world.client.inventory_count != inventory.count
                || world.globals.item_list != commands.items.table
                || world.globals.item_bytes != stride
                || world.inventory_table.count != commands.items.count
                || world.inventory_table.class_name != commands.items.classname
                || world.inventory_table.flags != commands.items.flags
                || world.inventory_table.ammo_flag != commands.items.ammunition_flag
            {
                return Err("classic source world and item interfaces disagree".to_string());
            }
            for entry in [
                world.entries.damage,
                world.entries.power_armor,
                world.entries.regular_armor,
                world.entries.spawn,
                world.entries.free,
            ] {
                image(entry, 1, true)?;
            }
            image(world.globals.level_frame, 4, false)?;
            bound(world.client.inventory, 4, client_bytes)?;
            bound(world.client.max_grenades, 4, client_bytes)?;
            bound(world.client.invincible_frame, 4, client_bytes)?;
            bound(world.client.userinfo, world.client.userinfo_bytes, client_bytes)?;
            bound(world.client.view_angles, 12, client_bytes)?;
            for offset in &world.inventory_table.capacities {
                bound(*offset, 4, client_bytes)?;
            }
        }
        NativePrimaryProfile::Rerelease { world, .. } => {
            if world.client.layout.byte_length as u32 != client_bytes
                || world.client.inventory_count != inventory.count
                || layout_field_offset(&world.client.layout, "pers.inventory") != Some(inventory.inventory)
                || layout_field_offset(&world.client.layout, "pers.weapon") != Some(commands.client.weapon)
                || layout_field_offset(&world.client.layout, "pers.selected_item") != Some(inventory.cursor)
            {
                return Err("rerelease source world and item interfaces disagree".to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::native_primary::{builtin_native_primary, PrimaryEdition};
    use super::super::native_primary_reader::{CLASSIC_DIGEST, RETAIL_DIGEST};
    use super::*;

    fn classic_image() -> SyntheticPeImage {
        SyntheticPeImage {
            image_size: 0x80000,
            pointer_bytes: 4,
            sections: vec![PeSection {
                rva: 0,
                mapped_size: 0x80000,
                executable: true,
            }],
        }
    }

    fn retail_image() -> SyntheticPeImage {
        SyntheticPeImage {
            image_size: 0x250000,
            pointer_bytes: 8,
            sections: vec![PeSection {
                rva: 0,
                mapped_size: 0x250000,
                executable: true,
            }],
        }
    }

    #[test]
    fn admits_classic_builtins() {
        let profile = builtin_native_primary(CLASSIC_DIGEST, PrimaryEdition::Classic).expect("classic");
        validate_native_primary(&profile, &classic_image()).expect("valid");
    }

    #[test]
    fn admits_retail_builtins() {
        let profile = builtin_native_primary(RETAIL_DIGEST, PrimaryEdition::Rerelease).expect("retail");
        validate_native_primary(&profile, &retail_image()).expect("valid");
    }

    #[test]
    fn rejects_addresses_outside_executable_sections() {
        let mut profile = builtin_native_primary(CLASSIC_DIGEST, PrimaryEdition::Classic).expect("classic");
        match &mut profile {
            NativePrimaryProfile::Classic { commands, .. } => {
                commands.give.entry = 0x90000;
            }
            NativePrimaryProfile::Rerelease { .. } => panic!("expected classic"),
        }
        assert!(validate_native_primary(&profile, &classic_image()).is_err());
        let profile = builtin_native_primary(CLASSIC_DIGEST, PrimaryEdition::Classic).expect("classic");
        let data_only = SyntheticPeImage {
            image_size: 0x80000,
            pointer_bytes: 4,
            sections: vec![PeSection {
                rva: 0,
                mapped_size: 0x80000,
                executable: false,
            }],
        };
        assert!(validate_native_primary(&profile, &data_only).is_err());
    }
}
