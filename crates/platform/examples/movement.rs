//! Pinned developer timing of the shared primitive path, not a gameplay run.
#[path = "../../movement/tests/support/mod.rs"]
mod support;
use qa_core::{
    primitives::{ClientId, CommandIntent, ModuleId, MovementRules, PlayerTail, Vec3},
    sys_events::EventTime,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::{
        Stopwatch,
        allocations::{begin_frame, end_frame},
    };
    let mut server = Server::load(64, 128, 1, 0).map_err(|e| format!("{e:?}"))?;
    let mut predictions: [Prediction; 64] = std::array::from_fn(|_| Prediction::default());
    let mut world = support::FixtureWorld {
        step: true,
        water: false,
    };
    let rules = [
        MovementRules::Quake,
        MovementRules::QuakeWorld,
        MovementRules::Quake2,
        MovementRules::Quake2Rerelease,
        MovementRules::Quake3,
    ];
    for slot in 0..64 {
        server
            .connect(
                [Connection::Local, Connection::Remote, Connection::Bot][slot % 3],
                ModuleId((slot % 3) as u16),
                PlayerTail::Q2 { weapon_frame: 0 },
            )
            .ok_or("client capacity")?;
        let player = &mut server.clients[slot].player;
        player.movement_rules = rules[slot % 5];
        qa_movement::set_bounds(player);
        player.body.position = Vec3([0.0, 0.0, 24.0]);
        player.movement.grounded = true;
    }
    let mut ns = [0u64; 600];
    let mut maximum_allocations = 0;
    let mut maximum_bytes = 0;
    let mut steps = 0u64;
    for frame in 0..660 {
        let end = EventTime((frame + 1) * 11_764_705);
        let start = EventTime(frame * 11_764_705);
        begin_frame();
        let timer = Stopwatch::start();
        for (slot, prediction) in predictions.iter_mut().enumerate() {
            let client = &mut server.clients[slot];
            let intent = CommandIntent {
                movement: [
                    127,
                    if frame / 48 % 2 == 0 { 64 } else { -64 },
                    if frame % 96 < 8 { 127 } else { 0 },
                ],
                view_angles: Vec3([0.0, (frame / 96 % 4) as f32 * 90.0, 0.0]),
                ..Default::default()
            };
            client.intent = intent;
            prediction.apply_snapshot(&client.player);
            if client.connection != Some(Connection::Bot) {
                server.submit_command(
                    ClientId(slot as u8),
                    qa_input::UserCmdBuilder::build(
                        std::time::Duration::from_nanos(end.since(start)),
                        end,
                        intent,
                    ),
                );
            }
        }
        server.build_bot_commands(start, end);
        let count = server.move_pending_clients(&mut world);
        for (client, prediction) in server.clients.iter().zip(&mut predictions) {
            prediction.advance(client.command, &mut world);
        }
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            ns[(frame - 60) as usize] = elapsed;
            steps += u64::from(count);
            maximum_allocations =
                maximum_allocations.max(counts.allocations + counts.reallocations);
            maximum_bytes = maximum_bytes.max(counts.requested_bytes);
        }
        for (client, prediction) in server.clients.iter().zip(&predictions) {
            if client.player.body.position != prediction.player.body.position
                || client.player.body.velocity != prediction.player.body.velocity
                || client.player.movement != prediction.player.movement
            {
                return Err("authoritative/prediction mismatch".into());
            }
        }
    }
    ns.sort_unstable();
    println!(
        "{{\"scope\":\"64 mixed-rule clients with authoritative and prediction movement on analytic stairs/walls\",\"warmup\":60,\"frames\":600,\"server_steps\":{steps},\"median_ns\":{},\"p99_ns\":{},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_bytes},\"state_match\":true}}",
        (ns[299] + ns[300]) / 2,
        ns[593]
    );
    for (slot, client) in server.clients.iter().enumerate() {
        println!(
            "client {slot} {:?} {:?}",
            client.player.body.position, client.player.body.velocity
        );
    }
    if maximum_allocations != 0 || maximum_bytes != 0 {
        return Err("allocation gate".into());
    }
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    eprintln!("build developer probe with allocation-tracking");
}
