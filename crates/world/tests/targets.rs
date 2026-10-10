use qa_core::primitives::ThinkTime;
use qa_core::{
    names::{NameMatch, NameTable},
    primitives::{ModuleId, NameId},
};
use qa_world::entities::AllocationPolicy;
use qa_world::entities::EntityTable;
use qa_world::targets::TargetIndex;

#[test]
fn target_iteration_preserves_edict_order_and_rejects_freed_lifetimes() {
    let mut table = EntityTable::new(8, 1).unwrap();
    let names = NameTable::load([b"a".as_slice(), b"b", b"door", b"exit"]).unwrap();
    let first = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(0),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let other = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(0),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let last = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(0),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    table.set_targetname(first, Some(NameId(3)));
    table.set_targetname(other, Some(NameId(4)));
    table.set_targetname(last, Some(NameId(3)));
    let mut targets = TargetIndex::new(&table);
    assert!(targets.refresh(&mut table, &names));
    assert!(!targets.refresh(&mut table, &names));
    assert_eq!(
        targets
            .find(NameId(3), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [first, last]
    );
    table.release(first, ThinkTime::Seconds(0.0));
    assert!(targets.refresh(&mut table, &names));
    assert_eq!(
        targets
            .find(NameId(3), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [last]
    );
    let reused = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(0),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    table.set_targetname(reused, Some(NameId(3)));
    assert!(targets.refresh(&mut table, &names));
    assert_eq!(
        targets
            .find(NameId(3), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [reused, last]
    );
    table.set_targetname(last, Some(NameId(4)));
    assert!(targets.refresh(&mut table, &names));
    assert_eq!(
        targets
            .find(NameId(4), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [other, last]
    );
    assert!(!table.set_targetname(first, Some(NameId(3))));
}

#[test]
fn caller_selects_exact_or_folded_matches_over_one_entity_table() {
    let names = NameTable::load([b"Door".as_slice(), b"door", b"DOOR"]).unwrap();
    let mut table = EntityTable::new(8, 1).unwrap();
    let mut ids = Vec::new();
    for (module, text) in [(1, b"Door".as_slice()), (2, b"door"), (3, b"DOOR")] {
        let id = table
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(module),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        table.set_targetname(id, names.find(text));
        ids.push(id);
    }
    let mut index = TargetIndex::new(&table);
    index.refresh(&mut table, &names);
    let door = names.find(b"door").unwrap();
    assert_eq!(
        index
            .find(door, NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [ids[1]]
    );
    assert_eq!(
        index
            .find(door, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        index
            .find(names.find(b"Door").unwrap(), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [ids[0]]
    );
    assert!(table.set_targetname(ids[0], None));
    assert!(index.refresh(&mut table, &names));
    assert_eq!(
        index
            .find(door, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        ids[1..]
    );
}

#[test]
fn empty_target_strings_are_distinct_from_missing_fields() {
    let names = NameTable::load(std::iter::empty()).unwrap();
    let mut table = EntityTable::new(8, 1).unwrap();
    let missing = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let empty = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(table.columns.targetname(missing.slot as usize), None);
    table.set_targetname(empty, Some(NameId(0)));
    let mut index = TargetIndex::new(&table);
    index.refresh(&mut table, &names);
    for matching in [NameMatch::Exact, NameMatch::Folded] {
        assert_eq!(
            index.find(NameId(0), matching, &names).collect::<Vec<_>>(),
            [empty]
        );
    }
    table.set_targetname(empty, None);
    index.refresh(&mut table, &names);
    assert!(
        index
            .find(NameId(0), NameMatch::Exact, &names)
            .next()
            .is_none()
    );
}

#[test]
fn unnamed_churn_leaves_named_rows_and_refresh_counters_unchanged() -> Result<(), &'static str> {
    let names = NameTable::load([b"door".as_slice()]).map_err(|_| "names")?;
    let door = names.find(b"door").ok_or("door")?;
    let mut table = EntityTable::new(130, 2).map_err(|_| "table")?;
    let named = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .ok_or("named")?
        .id;
    assert!(table.set_targetname(named, Some(door)));
    let mut index = TargetIndex::new(&table);
    assert!(index.refresh(&mut table, &names));
    let before = index.stats();
    for _ in 0..600 {
        let rocket = table
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(2),
                AllocationPolicy::EDICT,
            )
            .ok_or("rocket")?
            .id;
        assert!(table.set_targetname(rocket, None));
        assert!(!index.refresh(&mut table, &names));
        assert!(table.release(rocket, ThinkTime::Seconds(0.0)));
        assert!(!index.refresh(&mut table, &names));
    }
    assert_eq!(index.stats(), before);
    assert_eq!(
        index
            .find(door, NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [named]
    );
    // Replacing the one load-owned index bootstraps current names even after
    // the prior index consumed their mutation bits.
    drop(index);
    let mut index = TargetIndex::new(&table);
    assert!(index.refresh(&mut table, &names));
    assert_eq!(index.stats().initial_builds, 1);
    assert_eq!(index.stats().changed_slots, 0);
    assert_eq!(
        index
            .find(door, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        [named]
    );
    Ok(())
}

#[test]
fn coalesced_names_and_lifetimes_cross_dirty_words_without_stale_rows() -> Result<(), &'static str>
{
    let names = NameTable::load([b"Door".as_slice(), b"door", b"exit"]).map_err(|_| "names")?;
    let upper = names.find(b"Door").ok_or("Door")?;
    let lower = names.find(b"door").ok_or("door")?;
    let exit = names.find(b"exit").ok_or("exit")?;
    let mut table = EntityTable::new(130, 1).map_err(|_| "table")?;
    let mut ids = Vec::new();
    for _ in 1..130 {
        ids.push(
            table
                .allocate(
                    ThinkTime::Seconds(0.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT,
                )
                .ok_or("entity")?
                .id,
        );
    }
    let middle = ids[63];
    let last = ids[128];
    assert_eq!((middle.slot, last.slot), (64, 129));
    assert!(table.set_targetname(middle, Some(upper)));
    assert!(table.set_targetname(last, Some(lower)));
    let mut index = TargetIndex::new(&table);
    assert!(index.refresh(&mut table, &names));
    let changes = index.stats().changed_slots;
    assert!(table.set_targetname(middle, Some(exit)));
    assert!(table.set_targetname(middle, None));
    assert!(table.set_targetname(middle, Some(upper)));
    assert!(index.refresh(&mut table, &names));
    assert_eq!(index.stats().changed_slots, changes + 1);
    assert_eq!(
        index
            .find(lower, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        [middle, last]
    );
    assert!(table.release(middle, ThinkTime::Seconds(0.0)));
    let reused = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .ok_or("reuse")?
        .id;
    assert_eq!(reused.slot, middle.slot);
    assert_ne!(reused.generation, middle.generation);
    assert!(table.set_targetname(reused, Some(exit)));
    assert!(table.set_targetname(reused, Some(lower)));
    assert!(table.release(last, ThinkTime::Seconds(0.0)));
    let changes = index.stats().changed_slots;
    assert!(index.refresh(&mut table, &names));
    assert_eq!(index.stats().changed_slots, changes + 2);
    assert_eq!(
        index
            .find(lower, NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [reused]
    );
    assert_eq!(
        index
            .find(upper, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        [reused]
    );
    assert!(index.find(upper, NameMatch::Exact, &names).next().is_none());
    assert!(index.find(exit, NameMatch::Exact, &names).next().is_none());
    assert!(!index.refresh(&mut table, &names));
    Ok(())
}

#[test]
fn named_client_reset_and_full_table_overwrite_remove_old_generations() -> Result<(), &'static str>
{
    let names = NameTable::load([b"door".as_slice()]).map_err(|_| "names")?;
    let door = names.find(b"door").ok_or("door")?;
    let mut table = EntityTable::new(3, 2).map_err(|_| "table")?;
    let world = table.id_at(0).ok_or("world")?;
    let client = table.id_at(1).ok_or("client")?;
    let entity = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::QUAKEWORLD,
        )
        .ok_or("entity")?
        .id;
    for id in [world, client, entity] {
        assert!(table.set_targetname(id, Some(door)));
    }
    let mut index = TargetIndex::new(&table);
    assert!(index.refresh(&mut table, &names));
    assert_eq!(
        index
            .find(door, NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [world, client, entity]
    );
    let reset = table.reset_client(client).ok_or("reset")?;
    let replacement = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::QUAKEWORLD,
        )
        .ok_or("overwrite")?;
    assert_eq!(replacement.displaced, Some(entity));
    assert_eq!(replacement.id.slot, entity.slot);
    assert_ne!(replacement.id.generation, entity.generation);
    assert!(index.refresh(&mut table, &names));
    assert_eq!(
        index
            .find(door, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        [world]
    );
    assert!(table.set_targetname(reset, Some(door)));
    assert!(table.set_targetname(replacement.id, Some(door)));
    assert!(index.refresh(&mut table, &names));
    assert_eq!(
        index
            .find(door, NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [world, reset, replacement.id]
    );
    let before = index.stats();
    assert!(table.set_never_free(replacement.id, true));
    assert!(!table.release(replacement.id, ThinkTime::Seconds(0.0)));
    assert!(
        table
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(1),
                AllocationPolicy::QUAKEWORLD
            )
            .is_none()
    );
    assert!(!index.refresh(&mut table, &names));
    assert_eq!(index.stats(), before);
    Ok(())
}
