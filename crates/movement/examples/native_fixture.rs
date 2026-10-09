#[path = "../tests/support/mod.rs"]
mod support;
use qa_core::primitives::{PlayerState, RuleSetId, UserCmd, Vec3};
fn main() {
    let arena = std::env::args().nth(1).as_deref() == Some("q3");
    let rules = if arena {
        RuleSetId::Quake3
    } else {
        RuleSetId::Quake2
    };
    for scenario in 0..6 {
        let mut player = PlayerState {
            movement_rules: rules,
            trace_rules: rules,
            ..Default::default()
        };
        qa_movement::set_bounds(&mut player);
        player.body.position = Vec3([0.0, 0.0, 24.0]);
        let mut world = support::FixtureWorld {
            step: scenario == 1,
            water: scenario == 5,
        };
        let mut seed = 0x12345678u32;
        let mut time = 0;
        for frame in 0..192 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let mut command = UserCmd {
                duration_ms: 8 + (seed % 17) as u16,
                movement: [
                    if scenario < 2 {
                        300.0
                    } else {
                        ((seed >> 8) % 601) as f32 - 300.0
                    },
                    if scenario < 2 {
                        0.0
                    } else {
                        ((seed >> 18) % 401) as f32 - 200.0
                    },
                    if scenario == 4 && frame % 64 < 32 {
                        -200.0
                    } else if scenario == 3 && frame % 48 < 8 {
                        200.0
                    } else {
                        0.0
                    },
                ],
                view_angles: Vec3([
                    if scenario == 4 {
                        if frame % 64 < 32 { 45.0 } else { -45.0 }
                    } else {
                        0.0
                    },
                    if scenario < 2 {
                        0.0
                    } else {
                        (frame / 48) as f32 * 90.0
                    },
                    0.0,
                ]),
                ..Default::default()
            };
            if arena {
                time += i32::from(command.duration_ms);
                command.server_time_ms = time;
                command.movement[0] = if scenario < 2 {
                    127.0
                } else {
                    ((seed >> 8) % 255) as f32 - 127.0
                };
                command.movement[1] = if scenario < 2 {
                    0.0
                } else {
                    ((seed >> 18) % 255) as f32 - 127.0
                };
                command.movement[2] = if scenario == 4 && frame % 64 < 32 {
                    -127.0
                } else if scenario == 3 && frame % 48 < 8 {
                    127.0
                } else {
                    0.0
                };
            }
            qa_movement::pmove(command, &mut player, &mut world);
            if arena {
                let p = player.body.position.0;
                let v = player.body.velocity.0;
                println!(
                    "{scenario} {frame} {} {} {} {} {} {} {} {} {}",
                    p[0],
                    p[1],
                    p[2],
                    v[0],
                    v[1],
                    v[2],
                    u8::from(player.movement.grounded),
                    u8::from(player.movement.jump_held),
                    player.movement.remaining_ms
                );
                continue;
            }
            let p = player.body.position.0.map(|v| (v * 8.0) as i32);
            let v = player.body.velocity.0.map(|v| (v * 8.0) as i32);
            println!(
                "{scenario} {frame} {} {} {} {} {} {} {} {} {}",
                p[0],
                p[1],
                p[2],
                v[0],
                v[1],
                v[2],
                u8::from(player.movement.grounded),
                u8::from(player.movement.jump_held),
                player.movement.remaining_ms
            );
        }
    }
}
