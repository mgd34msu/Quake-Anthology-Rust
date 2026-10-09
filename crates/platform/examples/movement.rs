//! Pinned developer timing of the shared primitive path, not a gameplay run.
use qa_core::{
    primitives::{
        Bounds, ClientId, CommandIntent, GeometryId, ModuleId, Plane, PlayerTail, RuleSetId,
        SurfaceFlags, Vec3,
    },
    sys_events::EventTime,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, WorldTrace,
        brushes::{Brush, BrushTree},
    },
};

fn box_brush(planes: &mut Vec<Plane>, brushes: &mut Vec<Brush>, mins: Vec3, maxs: Vec3) {
    let first_plane = planes.len() as u32;
    for axis in 0..3 {
        for sign in [1.0, -1.0] {
            let mut normal = Vec3::default();
            normal.0[axis] = sign;
            planes.push(Plane {
                normal,
                distance: if sign > 0.0 {
                    maxs.0[axis]
                } else {
                    -mins.0[axis]
                },
                axis: None,
            });
        }
    }
    brushes.push(Brush {
        first_plane,
        plane_count: 6,
        contents: Contents::SOLID,
    });
}

fn scene(store: &mut CollisionStore) -> Result<GeometryId, String> {
    let mut planes = vec![Plane {
        normal: Vec3([0.0, 0.0, 1.0]),
        distance: 0.0,
        axis: None,
    }];
    let mut brushes = vec![Brush {
        first_plane: 0,
        plane_count: 1,
        contents: Contents::SOLID,
    }];
    for room in 0..64 {
        let x = (room % 8) as f32 * 320.0;
        let y = (room / 8) as f32 * 320.0;
        for (mins, maxs) in [
            ([-144.0, -120.0, 0.0], [-128.0, 120.0, 256.0]),
            ([240.0, -120.0, 0.0], [256.0, 120.0, 256.0]),
            ([-144.0, -136.0, 0.0], [256.0, -120.0, 256.0]),
            ([-144.0, 120.0, 0.0], [256.0, 136.0, 256.0]),
            ([96.0, -120.0, 0.0], [160.0, 120.0, 16.0]),
        ] {
            box_brush(
                &mut planes,
                &mut brushes,
                Vec3([mins[0] + x, mins[1] + y, mins[2]]),
                Vec3([maxs[0] + x, maxs[1] + y, maxs[2]]),
            );
        }
    }
    let surfaces = vec![SurfaceFlags(0); planes.len()];
    let tree = BrushTree::direct(brushes.len())
        .map_err(|error| format!("analytic membership: {error:?}"))?;
    store
        .load_brushes(
            planes,
            brushes,
            surfaces,
            tree,
            vec![Bounds {
                mins: Vec3([-160.0, -160.0, -512.0]),
                maxs: Vec3([2560.0, 2560.0, 512.0]),
            }],
        )
        .map_err(|error| format!("analytic scene: {error:?}"))
}
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
    let mut server = Server::load(64, 128, 1, 0, 0).map_err(|e| format!("{e:?}"))?;
    server.area = AreaGrid::load(
        128,
        Bounds {
            mins: Vec3([-160.0, -160.0, -512.0]),
            maxs: Vec3([2560.0, 2560.0, 512.0]),
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    let mut predictions: [Prediction; 64] = std::array::from_fn(|_| Prediction::default());
    let mut store = CollisionStore::new();
    let geometry = scene(&mut store)?;
    let mut scratch = store.scratch();
    let rules = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ];
    for slot in 0..64 {
        server
            .connect(
                [Connection::Local, Connection::Remote, Connection::Bot][slot % 3],
                ModuleId((slot % 3) as u16),
                PlayerTail::Q2 { weapon_frame: 0 },
                None,
            )
            .ok_or("client capacity")?;
        let client = &mut server.clients[slot];
        client.player.movement_rules = rules[slot % 5];
        client.player.trace_rules = rules[slot % 5];
        qa_movement::set_bounds(&mut client.player);
        // All rooms stay inside native Q2's signed eighth-unit origin range.
        client.player.body.position =
            Vec3([(slot % 8) as f32 * 320.0, (slot / 8) as f32 * 320.0, 24.125]);
        client.player.movement.grounded = true;
        server
            .entities
            .columns
            .set_body(client.entity.slot as usize, client.player.body);
        if !server.area.link(
            &server.entities,
            client.entity,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit,
        ) {
            return Err("initial client link".into());
        }
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
                view_angles: Vec3([0.0, (frame / 96 % 4) as f32 * 90.0, 0.0]),
                ..CommandIntent::moving([
                    1.0,
                    if frame / 48 % 2 == 0 {
                        64.0 / 127.0
                    } else {
                        -64.0 / 127.0
                    },
                    if frame % 96 < 8 { 1.0 } else { 0.0 },
                ])
            };
            client.intent = intent;
            prediction.apply_snapshot(&client.player);
            if client.connection != Some(Connection::Bot) {
                let command = qa_input::UserCmdBuilder::build(
                    std::time::Duration::from_nanos(end.since(start)),
                    end,
                    intent,
                    qa_input::InputPolicy::native(client.player.movement_rules),
                );
                server.submit_command(ClientId(slot as u32), command);
            }
        }
        server.build_bot_commands(start, end, qa_input::InputPolicy::native);
        let count = server.move_pending_clients(&store, geometry, 0, &mut scratch);
        for (client, prediction) in server.clients.iter().zip(&mut predictions) {
            let mut trace = WorldTrace::new(
                &store,
                geometry,
                0,
                &server.entities,
                &server.area,
                &mut scratch,
                Some(client.entity),
            );
            prediction.advance(client.command, &mut trace);
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
        for (slot, (client, prediction)) in server.clients.iter().zip(&predictions).enumerate() {
            if client.player.body.position != prediction.player.body.position
                || client.player.body.velocity != prediction.player.body.velocity
                || client.player.movement != prediction.player.movement
            {
                let nearby = server
                    .clients
                    .iter()
                    .enumerate()
                    .find(|(other_slot, other)| {
                        *other_slot != slot
                            && (0..3).all(|axis| {
                                (other.player.body.position.0[axis]
                                    - prediction.player.body.position.0[axis])
                                    .abs()
                                    < 64.0
                            })
                    })
                    .map(|(slot, other)| (slot, other.player.body.position));
                return Err(format!("authoritative/prediction mismatch frame={frame} client={slot} nearby={nearby:?} rules={:?} server_position={:?} prediction_position={:?} server_velocity={:?} prediction_velocity={:?} server_movement={:?} prediction_movement={:?}", client.player.movement_rules, client.player.body.position, prediction.player.body.position, client.player.body.velocity, prediction.player.body.velocity, client.player.movement, prediction.player.movement).into());
            }
        }
    }
    ns.sort_unstable();
    println!(
        "{{\"scope\":\"64 mixed-rule clients with authoritative and prediction WorldTrace movement on loaded brush stairs/walls in 8x8 native-range rooms\",\"workload\":\"linked_scene_64_native_range_rooms\",\"matched_previous_workload\":false,\"warmup\":60,\"frames\":600,\"server_steps\":{steps},\"median_ns\":{},\"p99_ns\":{},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_bytes},\"state_match\":true}}",
        (ns[299] + ns[300]) / 2,
        ns[593]
    );
    if maximum_allocations != 0 || maximum_bytes != 0 {
        return Err("allocation gate".into());
    }
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    eprintln!("build developer probe with allocation-tracking");
}
