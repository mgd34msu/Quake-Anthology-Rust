use qa_core::{
    events::{FrameEvent, OutputSubmission, TextStore, dispatch_consumer},
    primitives::*,
};
use qa_session::clients::{Connection, Server};

#[test]
fn hud_publication_keeps_float_bits_and_resets_for_the_next_client_lifetime() {
    let mut server = Server::load(1, 8, 2, 2, 2, 0).unwrap();
    let id = server
        .connect(Connection::Local, ModuleId(0), PlayerTail::None, None)
        .unwrap();
    let client = &mut server.clients[0];
    client.player.inventory.item_acquired_at[1] = -0.0;
    client.player.inventory.powerup_until[1] = f64::from_bits(0x7ff8_0000_0000_0001);
    for _ in 0..10 {
        client
            .hud_publication
            .publish_changes(&client.player, &mut server.events, id);
    }
    assert_eq!(server.events.len(), 2);
    let consumer = client.output.unwrap();
    let mut changes = Vec::new();
    dispatch_consumer(
        &mut server.events,
        consumer,
        &mut |record: qa_core::events::OutputRecord, _: &mut TextStore| {
            let FrameEvent::Hud(event) = record.event else {
                panic!("HUD event");
            };
            changes.push(event.change);
            OutputSubmission::BestEffort
        },
    );
    assert!(
        matches!(changes[0], HudChange::Item { item: ItemId(1), count: 0, acquired_at } if acquired_at.to_bits() == (-0.0f64).to_bits())
    );
    assert!(
        matches!(changes[1], HudChange::Powerup { slot: 1, until } if until.to_bits() == 0x7ff8_0000_0000_0001)
    );
    assert!(server.disconnect(id));
    assert_eq!(
        server.connect(Connection::Local, ModuleId(0), PlayerTail::None, None),
        Some(id)
    );
    let client = &mut server.clients[0];
    client.player.inventory.item_acquired_at[1] = -0.0;
    client.player.inventory.powerup_until[1] = f64::from_bits(0x7ff8_0000_0000_0001);
    client
        .hud_publication
        .publish_changes(&client.player, &mut server.events, id);
    assert_eq!(server.events.len(), 2);
    assert_eq!(server.events.rejected_publications(), 0);
}
