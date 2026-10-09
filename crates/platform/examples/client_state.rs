//! Headless client arena qualification, without gameplay or network encoding.
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_core::{
        primitives::{
            ClientId, CommandIntent, ModuleId, NativeEntity, PlayerTail, RuleSetId, Vec3,
        },
        sys_events::EventTime,
    };
    use qa_platform::allocations::{begin_frame, end_frame};
    use qa_session::clients::Connection;
    use std::hint::black_box;

    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut runtime = qa_app::Runtime::load(512, std::iter::empty())?;
    let server = &mut runtime.server;
    let rules = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ];
    for slot in 0..512 {
        let connection = [Connection::Local, Connection::Remote, Connection::Bot][slot % 3];
        let id = server
            .connect(
                connection,
                ModuleId(10),
                PlayerTail::None,
                Some(NativeEntity {
                    module: ModuleId((slot / 256) as u16),
                    slot: (slot % 256) as i32,
                }),
            )
            .ok_or("client capacity")?;
        if id != ClientId(slot as u32) {
            return Err("client order".into());
        }
        let client = &mut server.clients[slot];
        client.player.movement_rules = rules[slot % rules.len()];
        client.player.trace_rules = rules[slot % rules.len()];
        client.intent = CommandIntent {
            view_angles: Vec3([0.0, 90.0, 0.0]),
            ..CommandIntent::moving([0.01, 0.02, 0.03])
        };
    }
    let inventory = server.clients[511].player.inventory.as_ptr();
    let mut measured_calls = 0;
    let mut measured_bytes = 0;
    let mut reconnects = 0;
    for frame in 0..660u64 {
        begin_frame();
        let old = server.clients[511].entity;
        if !server.disconnect(ClientId(511)) {
            return Err("disconnect".into());
        }
        let id = server
            .connect(Connection::Bot, ModuleId(42), PlayerTail::None, None)
            .ok_or("reconnect")?;
        if id != ClientId(511)
            || server.entities.resolve(old).is_some()
            || server.clients[511].player.inventory.as_ptr() != inventory
            || server.entities.columns.native_entity[server.clients[511].entity.slot as usize]
                .is_some()
        {
            return Err("lifetime or native binding".into());
        }
        server.build_bot_commands(
            EventTime(frame * 50_000_000),
            EventTime((frame + 1) * 50_000_000),
            qa_input::InputPolicy::native,
        );
        for (slot, client) in server.clients.iter_mut().enumerate() {
            client.player.inventory[1] = slot as i32;
            black_box(&client.command);
        }
        let counts = end_frame();
        if frame >= 60 {
            measured_calls += counts.allocations + counts.reallocations;
            measured_bytes += counts.requested_bytes;
            reconnects += 1;
        }
    }
    if measured_calls != 0
        || measured_bytes != 0
        || server.clients[511].player.inventory[1] != 511
        || server.clients[0].player.inventory[1] != 0
    {
        return Err("client allocation/state qualification".into());
    }
    println!(
        "{{\"scope\":\"headless app-loaded common client state; no gameplay or native protocol\",\"clients\":512,\"highest_client_id\":511,\"warmup_frames\":60,\"measured_frames\":600,\"highest_id_reconnects\":{reconnects},\"rust_calling_thread_alloc_or_realloc\":{measured_calls},\"requested_bytes\":{measured_bytes},\"allocation_positive_control\":{}}}",
        positive.allocations
    );
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    eprintln!("client_state requires --features allocation-tracking");
    std::process::exit(2);
}
