use qa_app::output_namespace::NativeNumbers;
use qa_core::primitives::{EntityId, ModuleId, NativeEntity, ThinkTime};
use qa_network::commands::packet::Protocol;
use qa_world::entities::{AllocationPolicy, EntityTable};

fn entity(table: &mut EntityTable, module: ModuleId, slot: i32) -> EntityId {
    let id = table
        .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    table.columns.native_entity[id.slot as usize] = Some(NativeEntity { module, slot });
    id
}

#[test]
fn snapshots_and_cues_share_primary_and_secondary_entity_numbers() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let mut table = EntityTable::new(16, 1).unwrap();
        let primary = entity(&mut table, ModuleId(1), 1);
        let secondary = entity(&mut table, ModuleId(2), 1);
        let foreign_world = entity(&mut table, ModuleId(2), 0);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(2);
        numbers.refresh(&table, Some(ModuleId(1)), protocol);
        assert_eq!(numbers.project(&table, primary), Some(1));
        assert_eq!(numbers.native(&table, ModuleId(1), 0), Some(0));
        let foreign = numbers.project(&table, secondary).unwrap();
        assert!(foreign > 2, "vacant native client2 remains reserved");
        assert_eq!(numbers.native(&table, ModuleId(2), 1), Some(foreign));
        assert_eq!(numbers.project(&table, secondary), Some(foreign));
        assert_eq!(numbers.entity(foreign), Some(secondary));
        let world = numbers.native(&table, ModuleId(2), 0).unwrap();
        assert_ne!(world, 0);
        assert_ne!(world, foreign);
        assert_eq!(numbers.project(&table, foreign_world), Some(world));
        // A newly loaded primary entity claims its exact native ordinal.
        let grown = entity(&mut table, ModuleId(1), foreign as i32);
        numbers.set_native_clients(2);
        numbers.refresh(&table, Some(ModuleId(1)), protocol);
        assert_eq!(numbers.project(&table, grown), Some(foreign));
        let relocated = numbers.project(&table, secondary).unwrap();
        assert_ne!(relocated, foreign);
        assert_eq!(numbers.entity(foreign), Some(grown));
        assert_eq!(numbers.entity(relocated), Some(secondary));
        assert_eq!(numbers.native(&table, ModuleId(2), 1), Some(relocated));
        assert_eq!(numbers.project(&table, foreign_world), Some(world));
    }
}

#[test]
fn namespace_release_reuse_and_reconnect_never_retain_an_old_lifetime() {
    let protocol = Protocol::Quake2_34;
    let mut table = EntityTable::new(4, 1).unwrap();
    let id = entity(&mut table, ModuleId(2), 1);
    let mut numbers = NativeNumbers::load(protocol);
    numbers.refresh(&table, Some(ModuleId(1)), protocol);
    let first = numbers.project(&table, id).unwrap();
    assert!(table.release(id, ThinkTime::Seconds(0.0)));
    let current = entity(&mut table, ModuleId(2), 1);
    assert_eq!(current.slot, id.slot);
    assert_ne!(current.generation, id.generation);
    numbers.refresh(&table, Some(ModuleId(1)), protocol);
    assert_eq!(numbers.project(&table, id), None);
    assert_eq!(numbers.number(id), None);
    assert_eq!(numbers.project(&table, current), Some(first));
    numbers.reset();
    assert_eq!(numbers.entity(first), None);
    numbers.refresh(&table, Some(ModuleId(2)), protocol);
    assert_eq!(numbers.project(&table, current), Some(1));
    // QW's whole playerinfo namespace remains reserved, not just live seats.
    numbers.refresh(&table, Some(ModuleId(1)), Protocol::QuakeWorld28);
    assert_eq!(numbers.project(&table, current), Some(33));
}

#[test]
fn target_protocol_rejects_native_widths_instead_of_aliasing_them() {
    let mut table = EntityTable::new(8, 1).unwrap();
    let wide = entity(&mut table, ModuleId(1), 1024);
    let mut numbers = NativeNumbers::load(Protocol::Quake2_34);
    numbers.refresh(&table, Some(ModuleId(1)), Protocol::Quake2_34);
    assert_eq!(numbers.project(&table, wide), None);
    numbers.refresh(&table, Some(ModuleId(1)), Protocol::Quake2Repro1038);
    assert_eq!(numbers.project(&table, wide), Some(1024));
    let invalid = entity(&mut table, ModuleId(1), -1);
    assert_eq!(numbers.project(&table, invalid), None);
    assert_eq!(numbers.native(&table, ModuleId(1), -1), None);
}

#[test]
fn common_self_claim_and_native_reservations_survive_output_resync() {
    let protocol = Protocol::Quake2_34;
    let mut table = EntityTable::new(8, 1).unwrap();
    let client = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let foreign = entity(&mut table, ModuleId(2), 1);
    let mut numbers = NativeNumbers::load(protocol);
    numbers.set_self(client, 0, protocol);
    numbers.set_native_clients(1);
    numbers.refresh(&table, None, protocol);
    assert_eq!(numbers.project(&table, client), Some(1));
    assert_eq!(numbers.project(&table, foreign), Some(2));
    numbers.reset();
    assert_eq!(numbers.native_clients(), 1);
    numbers.refresh(&table, None, protocol);
    assert_eq!(numbers.project(&table, client), Some(1));
    assert_eq!(numbers.project(&table, foreign), Some(2));
}

#[test]
fn weak_references_preserve_primary_ordinals_and_rebase_only_foreign_entities() {
    use qa_core::primitives::{
        ClientId, NativeClientRef, NativeEntityRef, NativeReference, ResourceNamespace,
    };
    let primary = ModuleId(1);
    let secondary = ModuleId(2);
    let receiver = ClientId(3);
    let mut table = EntityTable::new(8, 1).unwrap();
    let foreign = entity(&mut table, secondary, 12);
    let mut numbers = NativeNumbers::load(Protocol::Quake3_68);
    numbers.refresh(&table, Some(primary), Protocol::Quake3_68);
    let reference = |module, ordinal| {
        NativeReference::Entity(NativeEntityRef::entity(
            ResourceNamespace::Module(module),
            ordinal,
        ))
    };
    // Public references can name primary slots before those slots are active.
    assert_eq!(
        numbers.reference(&table, receiver, reference(primary, 0)),
        Some(0)
    );
    assert_eq!(
        numbers.reference(&table, receiver, reference(primary, 12)),
        Some(12)
    );
    let projected = numbers
        .reference(&table, receiver, reference(secondary, 12))
        .unwrap();
    assert_eq!(numbers.entity(projected), Some(foreign));
    assert_eq!(
        numbers.reference(&table, receiver, reference(secondary, 13)),
        None
    );
    assert_eq!(
        numbers.reference(
            &table,
            receiver,
            NativeReference::Client(NativeClientRef::client(
                ResourceNamespace::Module(secondary),
                12
            ))
        ),
        None
    );
    assert_eq!(
        numbers.reference(
            &table,
            receiver,
            NativeReference::Client(NativeClientRef::client(
                ResourceNamespace::Module(primary),
                0
            ))
        ),
        Some(0)
    );
    assert_eq!(
        numbers.reference(
            &table,
            receiver,
            NativeReference::Entity(NativeEntityRef::entity(
                ResourceNamespace::Client(receiver),
                0
            ))
        ),
        Some(0)
    );
    assert_eq!(
        numbers.reference(
            &table,
            receiver,
            NativeReference::Entity(NativeEntityRef::entity(
                ResourceNamespace::Client(ClientId(4)),
                0
            ))
        ),
        None
    );
}
