use qa_app::output_namespace::NativeNumbers;
use qa_core::primitives::{EntityId, ModuleId, NativeEntity, ThinkTime};
use qa_network::commands::packet::Protocol;
use qa_world::entities::{AllocationPolicy, EntityTable};

const PROTOCOLS: [Protocol; 5] = [
    Protocol::NetQuake15,
    Protocol::QuakeWorld28,
    Protocol::Quake2_34,
    Protocol::Quake2Repro1038,
    Protocol::Quake3_68,
];

fn client(table: &mut EntityTable, index: usize, native: Option<NativeEntity>) -> EntityId {
    let id = table.id_at(index + 1).unwrap();
    assert_eq!(table.client_index(id), Some(index));
    table.columns.native_entity[id.slot as usize] = native;
    id
}

fn native(module: u16, slot: u32) -> NativeEntity {
    NativeEntity {
        module: ModuleId(module),
        slot: i32::try_from(slot).unwrap(),
    }
}

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
        numbers.set_native_clients(
            2,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.refresh(&table, Some(ModuleId(1)), protocol, std::iter::empty());
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
        numbers.set_native_clients(
            2,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.refresh(&table, Some(ModuleId(1)), protocol, std::iter::empty());
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
    numbers.refresh(&table, Some(ModuleId(1)), protocol, std::iter::empty());
    let first = numbers.project(&table, id).unwrap();
    assert!(table.release(id, ThinkTime::Seconds(0.0)));
    let current = entity(&mut table, ModuleId(2), 1);
    assert_eq!(current.slot, id.slot);
    assert_ne!(current.generation, id.generation);
    numbers.refresh(&table, Some(ModuleId(1)), protocol, std::iter::empty());
    assert_eq!(numbers.project(&table, id), None);
    assert_eq!(numbers.number(id), None);
    assert_eq!(numbers.project(&table, current), Some(first));
    numbers.reset();
    assert_eq!(numbers.entity(first), None);
    numbers.refresh(&table, Some(ModuleId(2)), protocol, std::iter::empty());
    assert_eq!(numbers.project(&table, current), Some(1));
    // QW's whole playerinfo namespace remains reserved, not just live seats.
    numbers.refresh(
        &table,
        Some(ModuleId(1)),
        Protocol::QuakeWorld28,
        std::iter::empty(),
    );
    assert_eq!(numbers.project(&table, current), Some(33));
}

#[test]
fn target_protocol_rejects_native_widths_instead_of_aliasing_them() {
    let mut table = EntityTable::new(8, 1).unwrap();
    let wide = entity(&mut table, ModuleId(1), 1024);
    let mut numbers = NativeNumbers::load(Protocol::Quake2_34);
    numbers.refresh(
        &table,
        Some(ModuleId(1)),
        Protocol::Quake2_34,
        std::iter::empty(),
    );
    assert_eq!(numbers.project(&table, wide), None);
    numbers.refresh(
        &table,
        Some(ModuleId(1)),
        Protocol::Quake2Repro1038,
        std::iter::empty(),
    );
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
    numbers.set_native_clients(
        1,
        qa_network::snapshots::snapshot_limits(protocol).first_entity,
    );
    numbers.refresh(&table, None, protocol, std::iter::empty());
    assert_eq!(numbers.project(&table, client), Some(1));
    assert_eq!(numbers.project(&table, foreign), Some(2));
    numbers.reset();
    assert_eq!(numbers.native_clients(), 1);
    numbers.refresh(&table, None, protocol, std::iter::empty());
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
    numbers.refresh(
        &table,
        Some(primary),
        Protocol::Quake3_68,
        std::iter::empty(),
    );
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

#[test]
fn every_protocol_projects_active_players_into_its_player_namespace() {
    use qa_app::output_namespace::native_client_entity;
    use qa_core::primitives::{ClientId, NativeClientRef, ResourceNamespace};
    use qa_network::snapshots::snapshot_limits;
    for protocol in PROTOCOLS {
        let first = snapshot_limits(protocol).first_entity;
        let mut table = EntityTable::new(16, 6).unwrap();
        // Common client order deliberately disagrees with native client order.
        let high = client(&mut table, 0, Some(native(1, first + 2)));
        let low = client(&mut table, 1, Some(native(1, first)));
        let foreign = client(&mut table, 2, Some(native(2, first)));
        let own = client(&mut table, 3, None);
        let inactive = client(&mut table, 4, Some(native(1, first + 1)));
        let monster = entity(&mut table, ModuleId(2), 77);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(
            4,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.set_self(own, 3, protocol);
        numbers.refresh(
            &table,
            Some(ModuleId(1)),
            protocol,
            [own, foreign, low, high],
        );
        assert_eq!(numbers.project(&table, low), Some(first));
        assert_eq!(numbers.project(&table, high), Some(first + 2));
        assert_eq!(
            numbers.project(&table, own),
            native_client_entity(protocol, 3)
        );
        assert_eq!(numbers.project(&table, foreign), Some(first + 1));
        let receiver = ClientId(27);
        for (id, ordinal) in [(low, 0), (high, 2), (own, 3), (foreign, 1)] {
            assert_eq!(
                numbers.client_reference(&table, id, receiver),
                Some(NativeClientRef::client(
                    ResourceNamespace::Client(receiver),
                    ordinal
                ))
            );
        }
        assert_eq!(numbers.client_reference(&table, inactive, receiver), None);
        assert_eq!(numbers.client_reference(&table, monster, receiver), None);
        assert_eq!(
            numbers.native(&table, ModuleId(2), first as i32),
            Some(first + 1)
        );
        assert_eq!(numbers.project(&table, inactive), None);
        assert_eq!(numbers.foreign(inactive), None);
        let ordinary = numbers.project(&table, monster).unwrap();
        let reserved = if protocol == Protocol::QuakeWorld28 {
            32
        } else {
            4
        };
        assert!(ordinary >= first + reserved);
        numbers.refresh(
            &table,
            Some(ModuleId(1)),
            protocol,
            [high, low, foreign, own],
        );
        assert_eq!(numbers.project(&table, foreign), Some(first + 1));
        assert_eq!(numbers.project(&table, monster), Some(ordinary));
        numbers.reset();
        numbers.refresh(
            &table,
            Some(ModuleId(1)),
            protocol,
            [high, low, foreign, own],
        );
        assert_eq!(numbers.project(&table, own), Some(first + 3));
        assert_eq!(numbers.project(&table, foreign), Some(first + 1));
    }
}

#[test]
fn projected_client_references_reject_stale_and_unadmitted_lifetimes() {
    use qa_core::primitives::{ClientId, NativeClientRef, ResourceNamespace};
    let mut table = EntityTable::new(4, 2).unwrap();
    let player = table.id_at(1).unwrap();
    let receiver = ClientId(3);
    let protocol = Protocol::Quake3_68;
    let mut numbers = NativeNumbers::load(protocol);
    numbers.refresh(&table, None, protocol, [player]);
    let expected = Some(NativeClientRef::client(
        ResourceNamespace::Client(receiver),
        0,
    ));
    assert_eq!(numbers.client_reference(&table, player, receiver), expected);
    let replacement = table.reset_client(player).unwrap();
    assert_eq!(numbers.client_reference(&table, player, receiver), None);
    assert_eq!(
        numbers.client_reference(&table, replacement, receiver),
        None
    );
    numbers.refresh(&table, None, protocol, [replacement]);
    assert_eq!(
        numbers.client_reference(&table, replacement, receiver),
        expected
    );
    numbers.refresh(&table, None, protocol, []);
    assert_eq!(
        numbers.client_reference(&table, replacement, receiver),
        None
    );
}

#[test]
fn primary_growth_relocates_a_foreign_player_without_turning_it_into_an_entity() {
    for protocol in PROTOCOLS {
        let first = qa_network::snapshots::snapshot_limits(protocol).first_entity;
        let mut table = EntityTable::new(12, 5).unwrap();
        let primary = client(&mut table, 0, Some(native(1, first)));
        let foreign = client(&mut table, 1, Some(native(2, 90)));
        let newcomer = client(&mut table, 2, None);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(
            4,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [foreign, primary]);
        assert_eq!(numbers.project(&table, foreign), Some(first + 1));
        table.columns.native_entity[newcomer.slot as usize] = Some(native(1, first + 1));
        numbers.refresh(
            &table,
            Some(ModuleId(1)),
            protocol,
            [newcomer, foreign, primary],
        );
        assert_eq!(numbers.project(&table, newcomer), Some(first + 1));
        assert_eq!(numbers.project(&table, foreign), Some(first + 2));
        assert_eq!(numbers.foreign(foreign), Some(first + 2));
        assert_eq!(numbers.entity(first + 1), Some(newcomer));
    }
}

#[test]
fn player_capacity_rejects_excess_players_but_keeps_ordinary_foreign_entities() {
    for protocol in PROTOCOLS {
        let limits = qa_network::snapshots::snapshot_limits(protocol);
        let count = if protocol == Protocol::QuakeWorld28 {
            limits.client_capacity as usize
        } else {
            1
        };
        let mut table = EntityTable::new(count + 8, count + 4).unwrap();
        let own = client(&mut table, 0, Some(native(1, limits.first_entity)));
        let players = (1..count + 3)
            .map(|index| client(&mut table, index, Some(native(2, index as u32))))
            .collect::<Vec<_>>();
        let monster = entity(&mut table, ModuleId(2), 100);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(
            1,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.set_self(own, 0, protocol);
        numbers.refresh(
            &table,
            Some(ModuleId(1)),
            protocol,
            std::iter::once(own).chain(players.iter().copied()),
        );
        assert_eq!(numbers.number(own), Some(limits.first_entity));
        for id in &players[..count - 1] {
            assert!(numbers.project(&table, *id).is_some());
        }
        for id in &players[count - 1..] {
            assert_eq!(numbers.project(&table, *id), None);
            assert_eq!(numbers.foreign(*id), None);
            let native = table.columns.native_entity[id.slot as usize].unwrap();
            assert_eq!(numbers.native(&table, native.module, native.slot), None);
        }
        assert!(numbers.project(&table, monster).unwrap() >= limits.first_entity + count as u32);
    }
}

#[test]
fn inactive_and_reconnected_player_lifetimes_release_their_player_holes() {
    let protocol = Protocol::Quake2Repro1038;
    let mut table = EntityTable::new(12, 5).unwrap();
    let primary = client(&mut table, 0, Some(native(1, 1)));
    let old = client(&mut table, 1, Some(native(2, 17)));
    let waiting = client(&mut table, 2, Some(native(3, 19)));
    let mut numbers = NativeNumbers::load(protocol);
    numbers.set_native_clients(
        2,
        qa_network::snapshots::snapshot_limits(protocol).first_entity,
    );
    numbers.refresh(&table, Some(ModuleId(1)), protocol, [primary, old]);
    assert_eq!(numbers.project(&table, old), Some(2));
    numbers.refresh(&table, Some(ModuleId(1)), protocol, [primary, waiting]);
    assert_eq!(numbers.project(&table, old), None);
    assert_eq!(numbers.foreign(old), None);
    assert_eq!(numbers.project(&table, waiting), Some(2));
    let replacement = table.reset_client(old).unwrap();
    table.columns.native_entity[replacement.slot as usize] = Some(native(2, 17));
    numbers.refresh(&table, Some(ModuleId(1)), protocol, [primary, replacement]);
    assert_eq!(numbers.project(&table, old), None);
    assert_eq!(numbers.project(&table, waiting), None);
    assert_eq!(numbers.project(&table, replacement), Some(2));
    assert_eq!(numbers.entity(2), Some(replacement));
}

#[test]
fn out_of_range_primary_player_claims_never_fall_back_or_steal_self() {
    for protocol in PROTOCOLS {
        let first = qa_network::snapshots::snapshot_limits(protocol).first_entity;
        let mut table = EntityTable::new(12, 5).unwrap();
        let own = client(&mut table, 0, Some(native(1, first + 1)));
        let invalid = client(&mut table, 1, Some(native(1, 500)));
        let collision = client(&mut table, 2, Some(native(1, first)));
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(
            2,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.set_self(own, 0, protocol);
        numbers.refresh(
            &table,
            Some(ModuleId(1)),
            protocol,
            [collision, invalid, own],
        );
        assert_eq!(numbers.number(own), Some(first));
        assert_eq!(numbers.project(&table, own), Some(first));
        assert_eq!(numbers.project(&table, invalid), None);
        assert_eq!(numbers.foreign(invalid), None);
        assert_eq!(numbers.project(&table, collision), None);
        assert_eq!(numbers.entity(first), Some(own));
    }
}

#[test]
fn an_oversized_declared_client_count_cannot_expand_the_native_player_namespace() {
    for protocol in PROTOCOLS {
        let limits = qa_network::snapshots::snapshot_limits(protocol);
        let count = limits.client_capacity as usize;
        let mut table = EntityTable::new(count + 5, count + 3).unwrap();
        let players = (0..count + 2)
            .map(|index| client(&mut table, index, Some(native(2, index as u32))))
            .collect::<Vec<_>>();
        let monster = entity(&mut table, ModuleId(3), 300);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(
            u32::MAX,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.refresh(&table, Some(ModuleId(1)), protocol, players.iter().copied());
        for (index, &id) in players[..count].iter().enumerate() {
            assert_eq!(
                numbers.project(&table, id),
                Some(limits.first_entity + index as u32)
            );
        }
        for &id in &players[count..] {
            assert_eq!(numbers.project(&table, id), None);
            assert_eq!(numbers.foreign(id), None);
        }
        assert!(numbers.project(&table, monster).unwrap() >= limits.first_entity + count as u32);
    }
}

#[test]
fn growing_the_player_range_relocates_an_ordinary_foreign_occupant_before_player_admission() {
    for protocol in PROTOCOLS
        .into_iter()
        .filter(|p| *p != Protocol::QuakeWorld28)
    {
        let first = qa_network::snapshots::snapshot_limits(protocol).first_entity;
        let mut table = EntityTable::new(12, 4).unwrap();
        let primary = client(&mut table, 0, Some(native(1, first)));
        let waiting = client(&mut table, 1, Some(native(2, 20)));
        let monster = entity(&mut table, ModuleId(3), 20);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(
            1,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [primary]);
        assert_eq!(numbers.project(&table, monster), Some(first + 1));
        numbers.set_native_clients(
            2,
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [primary, waiting]);
        assert_eq!(numbers.project(&table, waiting), Some(first + 1));
        assert_eq!(numbers.project(&table, monster), Some(first + 2));
        assert_eq!(numbers.native(&table, ModuleId(2), 20), Some(first + 1));
        assert_eq!(numbers.native(&table, ModuleId(3), 20), Some(first + 2));
    }
}

#[test]
fn primary_players_and_actor_references_rebase_between_native_client_origins() {
    use qa_core::primitives::{ClientId, NativeEntityRef, NativeReference, ResourceNamespace};
    for protocol in PROTOCOLS {
        let limits = qa_network::snapshots::snapshot_limits(protocol);
        let first = limits.first_entity;
        let source_first = u32::from(first == 0);
        let mut table = EntityTable::new(16, 4).unwrap();
        let own = client(&mut table, 0, Some(native(1, source_first)));
        let peer = client(&mut table, 1, Some(native(1, source_first + 1)));
        let actor = entity(&mut table, ModuleId(1), (source_first + 2) as i32);
        let next_actor = entity(&mut table, ModuleId(1), (source_first + 3) as i32);
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(2, source_first);
        numbers.set_self(own, 0, protocol);
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [peer, own]);
        assert_eq!(
            numbers.player_count(),
            if protocol == Protocol::QuakeWorld28 {
                32
            } else {
                2
            }
        );
        assert!(!numbers.is_identity_namespace(ModuleId(1)));
        assert_eq!(numbers.number(own), Some(first));
        assert_eq!(numbers.number(peer), Some(first + 1));
        assert_eq!(
            numbers.native(&table, ModuleId(1), (source_first + 1) as i32),
            Some(first + 1)
        );
        let actor_number = numbers.project(&table, actor).unwrap();
        let next_number = numbers.project(&table, next_actor).unwrap();
        assert_ne!(actor_number, first);
        assert_ne!(actor_number, first + 1);
        assert_ne!(actor_number, next_number);
        let reference = |ordinal| {
            NativeReference::Entity(NativeEntityRef::entity(
                ResourceNamespace::Module(ModuleId(1)),
                ordinal,
            ))
        };
        assert_eq!(
            numbers.reference(&table, ClientId(0), reference(source_first + 1)),
            Some(first + 1)
        );
        assert_eq!(
            numbers.reference(&table, ClientId(0), reference(source_first + 2)),
            Some(actor_number)
        );
        if source_first != 0 {
            assert_eq!(
                numbers.native(&table, ModuleId(1), 0),
                Some(limits.world_entity)
            );
            assert_eq!(
                numbers.reference(&table, ClientId(0), reference(0)),
                Some(limits.world_entity)
            );
        }
        // Repeating publication cannot churn a relocated actor's native identity.
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [own, peer]);
        assert_eq!(numbers.project(&table, actor), Some(actor_number));
        assert_eq!(numbers.project(&table, next_actor), Some(next_number));
        assert_eq!(
            numbers.native(&table, ModuleId(1), (source_first + 2) as i32),
            Some(actor_number)
        );
        numbers.reset();
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [own, peer]);
        assert_eq!(numbers.number(own), Some(first));
        assert_eq!(numbers.number(peer), Some(first + 1));
        // A source policy may change while the destination protocol stays fixed.
        table.columns.native_entity[own.slot as usize] = Some(native(1, first));
        table.columns.native_entity[peer.slot as usize] = Some(native(1, first + 1));
        table.columns.native_entity[actor.slot as usize] = Some(native(1, first + 2));
        table.columns.native_entity[next_actor.slot as usize] = Some(native(1, first + 3));
        numbers.set_native_clients(2, first);
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [own, peer]);
        assert!(numbers.is_identity_namespace(ModuleId(1)));
        assert_eq!(numbers.number(peer), Some(first + 1));
        numbers.set_native_clients(1, first);
        numbers.refresh(&table, Some(ModuleId(1)), protocol, [own, peer]);
        assert_eq!(numbers.project(&table, peer), None);
        assert_eq!(numbers.foreign(peer), None);
    }
}

#[test]
fn primary_weak_player_references_convert_without_a_live_client_binding() {
    use qa_core::primitives::{ClientId, NativeEntityRef, NativeReference, ResourceNamespace};
    for protocol in PROTOCOLS {
        let limits = qa_network::snapshots::snapshot_limits(protocol);
        let source_first = u32::from(limits.first_entity == 0);
        let mut table = EntityTable::new(8, 1).unwrap();
        let mut numbers = NativeNumbers::load(protocol);
        numbers.set_native_clients(3, source_first);
        numbers.refresh(&table, Some(ModuleId(1)), protocol, std::iter::empty());
        let reference = |ordinal| {
            NativeReference::Entity(NativeEntityRef::entity(
                ResourceNamespace::Module(ModuleId(1)),
                ordinal,
            ))
        };
        assert_eq!(
            numbers.reference(&table, ClientId(0), reference(source_first + 1)),
            Some(limits.first_entity + 1)
        );
        // Observed common ownership wins over the weak native reservation:
        // an ordinary actor cannot become a player just from its raw number.
        let observed = entity(&mut table, ModuleId(1), (source_first + 1) as i32);
        let observed_number = numbers.project(&table, observed).unwrap();
        assert_eq!(
            numbers.reference(&table, ClientId(0), reference(source_first + 1)),
            Some(observed_number)
        );
        // A primary ordinary actor whose raw ordinal is now a player slot must
        // resolve through the same existing map, even before any snapshot rows.
        let actor = entity(&mut table, ModuleId(1), (source_first + 3) as i32);
        let projected = numbers.project(&table, actor).unwrap();
        assert_eq!(
            numbers.reference(&table, ClientId(0), reference(source_first + 3)),
            Some(projected)
        );
        numbers.set_native_clients(1, source_first);
        numbers.refresh(&table, Some(ModuleId(1)), protocol, std::iter::empty());
        assert_eq!(
            numbers.player_count(),
            if protocol == Protocol::QuakeWorld28 {
                32
            } else {
                1
            }
        );
    }
}
