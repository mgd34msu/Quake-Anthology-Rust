use qa_core::primitives::{
    ModuleId, MovementMagnitude, NativeEntity, NativePlayerTail, PlayerState, ResourceNamespace,
    RuleSetId,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};

#[test]
fn native_metadata_follows_the_bound_lifetime_and_clears_on_rebind_or_disconnect() {
    let mut server = Server::load(2, 8, 1, 1, 1, 0).unwrap();
    let native = NativeEntity {
        module: ModuleId(7),
        slot: 0,
    };
    let id = server
        .connect(Connection::Local, native.module, Some(native))
        .unwrap();
    let occupied = NativeEntity {
        module: ModuleId(8),
        slot: 0,
    };
    server
        .connect(Connection::Remote, occupied.module, Some(occupied))
        .unwrap();
    let mut tail =
        NativePlayerTail::new(RuleSetId::Quake3, ResourceNamespace::Module(native.module));
    tail.gravity = Some(16_777_217);
    tail.speed = Some(-1);
    tail.client_number = Some(0);
    tail.weapon_number = Some(0);
    tail.flags = 0x8000_3163;
    server.clients[id.0 as usize].player.native = Some(tail);
    server.clients[id.0 as usize].player.prediction =
        qa_core::primitives::PredictionPolicy::DISABLED;
    assert!(server.bind_client_native(id, native));
    assert_eq!(server.clients[id.0 as usize].player.native, Some(tail));
    assert_eq!(
        server.clients[id.0 as usize].player.prediction,
        qa_core::primitives::PredictionPolicy::DISABLED
    );
    assert!(!server.bind_client_native(id, occupied));
    assert_eq!(server.clients[id.0 as usize].player.native, Some(tail));
    let changed = NativeEntity { slot: 1, ..native };
    assert!(server.bind_client_native(id, changed));
    assert_eq!(server.clients[id.0 as usize].player.native, None);
    assert_eq!(
        server.clients[id.0 as usize].player.prediction,
        Default::default()
    );
    server.clients[id.0 as usize].player.native = Some(tail);
    assert!(server.disconnect(id));
    let replacement = server
        .connect(Connection::Bot, native.module, Some(native))
        .unwrap();
    assert_eq!(replacement, id);
    assert_eq!(server.clients[id.0 as usize].player.native, None);
}

#[test]
fn prediction_copies_metadata_and_effective_hot_inputs_without_copying_inventory() {
    let mut source = PlayerState::with_capacity(2, 1, 1);
    source.movement_rules = RuleSetId::QuakeWorld;
    source.trace_rules = RuleSetId::Quake2;
    let mut tail = NativePlayerTail::new(RuleSetId::Quake3, ResourceNamespace::Module(ModuleId(7)));
    tail.gravity = Some(16_777_217);
    tail.speed = Some(250);
    tail.client_number = Some(1);
    source.native = Some(tail);
    source.prediction = qa_core::primitives::PredictionPolicy {
        position: false,
        angles: true,
    };
    source.movement.tuning.gravity_multiplier = 0.0;
    source.movement.tuning.gravity = Some(MovementMagnitude::native(1200.0, 0.0));
    source.inventory.item_counts[0] = 99;
    let mut prediction = Prediction {
        player: PlayerState::with_capacity(2, 1, 1),
    };
    prediction.player.inventory.item_counts[0] = 3;
    let inventory = prediction.player.inventory.item_counts.as_ptr();
    prediction.apply_snapshot(&source);
    assert_eq!(prediction.player.native, source.native);
    assert_eq!(prediction.player.prediction, source.prediction);
    assert_eq!(prediction.player.movement, source.movement);
    assert_eq!(
        (
            prediction.player.movement_rules,
            prediction.player.trace_rules
        ),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    assert_eq!(prediction.player.inventory.item_counts[0], 3);
    assert_eq!(prediction.player.inventory.item_counts.as_ptr(), inventory);
    source.reset();
    prediction.apply_snapshot(&source);
    assert_eq!(prediction.player.native, None);
    assert_eq!(prediction.player.prediction, Default::default());
}
