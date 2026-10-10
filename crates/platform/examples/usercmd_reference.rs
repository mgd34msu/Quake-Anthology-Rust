//! Original command arithmetic and mixed human/bot builder qualification.
use qa_core::{
    primitives::{CommandIntent, RuleSetId},
    sys_events::EventTime,
};
use qa_input::{InputPolicy, UserCmdBuilder};
use std::{hint::black_box, time::Duration};
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

fn next(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}
fn intent(seed: &mut u32) -> CommandIntent {
    let fractions: [f32; 8] = std::array::from_fn(|_| (next(seed) % 1025) as f32 / 1024.0);
    CommandIntent {
        movement: [
            [fractions[6], fractions[7]],
            [fractions[2], fractions[3]],
            [fractions[4], fractions[5]],
        ],
        strafe: [fractions[0], fractions[1]],
        speed_modifier: next(seed) & 1 != 0,
        ..Default::default()
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).ok_or("native rule id or bench")?;
    if arg == "bench" {
        return bench();
    }
    if arg == "bench-view" {
        return bench_view();
    }
    if arg == "heap-input" {
        return heap_input();
    }
    if arg == "view-q1" || arg == "view-qw" {
        return view(arg == "view-qw");
    }
    let index: usize = arg.parse()?;
    let rules = *RuleSetId::ALL.get(index).ok_or("native rule id")?;
    let policy = InputPolicy::native(rules);
    let mut seed = 0x434d4449;
    for _ in 0..10000 {
        let command = UserCmdBuilder::build(
            Duration::from_millis(16),
            EventTime(16_000_000),
            intent(&mut seed),
            policy,
        );
        println!(
            "{:08x} {:08x} {:08x} {}",
            command.movement[0].to_bits(),
            command.movement[1].to_bits(),
            command.movement[2].to_bits(),
            command.buttons
        );
    }
    Ok(())
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn heap_input() -> Result<(), Box<dyn std::error::Error>> {
    use qa_core::{
        primitives::buttons,
        sys_events::{DeviceId, EventKind, SeatId, SysEvent},
    };
    use qa_input::{Input, Target};
    use qa_platform::allocations::{begin_frame, end_frame};
    struct Sink;
    impl Target for Sink {
        fn key(&mut self, _: SeatId, control: u16, _: bool, _: bool) -> bool {
            control == 900
        }
        fn character(&mut self, _: SeatId, _: char) {}
        fn command(&mut self, _: SeatId, _: EventTime, _: &str) {}
    }
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink;
    let devices = [
        DeviceId::Keyboard,
        DeviceId::Controller(1),
        DeviceId::Controller(2),
        DeviceId::Controller(3),
    ];
    let seats = [
        SeatId::FIRST,
        SeatId::new(1).ok_or("seat")?,
        SeatId::new(2).ok_or("seat")?,
        SeatId::new(3).ok_or("seat")?,
    ];
    for (&device, &seat) in devices.iter().zip(&seats) {
        if !input.assign(device, seat) {
            return Err("input device assignment".into());
        }
    }
    let policies = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake3,
    ]
    .map(InputPolicy::native);
    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut allocations = 0;
    let mut reallocations = 0;
    let mut bytes = 0;
    let mut checked_commands = 0;
    for frame in 0..660 {
        let time = EventTime((frame + 1) * 16_666_667);
        begin_frame();
        let phase = frame % 8;
        for &device in &devices {
            let edges: &[(u16, bool, bool)] = match phase {
                0 => &[(1023, true, false), (1023, true, true), (1023, true, false)],
                1 => &[(900, true, false)],
                2 => &[(1023, false, false), (1023, false, false)],
                4 => &[(900, false, false)],
                5 => &[(224, true, false), (228, true, false)],
                6 => &[(224, false, false)],
                _ => &[],
            };
            for &(code, down, repeat) in edges {
                input.dispatch(
                    SysEvent {
                        time,
                        kind: EventKind::Key {
                            device,
                            code,
                            symbol: 0,
                            down,
                            repeat,
                        },
                    },
                    &mut sink,
                );
            }
        }
        if phase == 3 {
            input.dispatch(
                SysEvent {
                    time,
                    kind: EventKind::DeviceRemoved(devices[1]),
                },
                &mut sink,
            );
            if !input.assign(devices[1], seats[1]) {
                return Err("removed device assignment".into());
            }
        }
        if phase == 6 && !input.bind(224, None, time, &mut sink) {
            return Err("held modifier replacement".into());
        }
        if phase == 7 {
            input.dispatch(
                SysEvent {
                    time,
                    kind: EventKind::Focus(false),
                },
                &mut sink,
            );
        }
        let commands = black_box(input.build_frame_with_policy(time, &policies));
        for (seat, command) in commands.iter().enumerate() {
            let expected = !matches!(phase, 4 | 7) && !(phase == 3 && seat == 1);
            if (command.buttons & buttons::ANY != 0) != expected {
                return Err("physical held-key state".into());
            }
        }
        let counts = end_frame();
        if frame >= 60 {
            allocations += counts.allocations;
            reallocations += counts.reallocations;
            bytes += counts.requested_bytes;
            checked_commands += commands.len();
        }
    }
    if allocations != 0 || reallocations != 0 || bytes != 0 {
        return Err("held input allocation gate".into());
    }
    println!(
        "{{\"scope\":\"four-seat physical key transitions and native command construction; no OS input or gameplay\",\"warmup\":60,\"frames\":600,\"checked_commands\":{checked_commands},\"allocations\":{allocations},\"reallocations\":{reallocations},\"requested_bytes\":{bytes},\"allocation_positive_control\":1}}"
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn heap_input() -> Result<(), Box<dyn std::error::Error>> {
    Err("heap-input requires allocation-tracking".into())
}
fn view(qw: bool) -> Result<(), Box<dyn std::error::Error>> {
    use qa_core::{
        primitives::{MovementMode, PlayerState, Vec3},
        sys_events::SeatId,
    };
    use qa_input::{Action, Input};
    let mut input = Input::load();
    input.seed(EventTime(0));
    input.set_view_angles(SeatId::FIRST, Vec3([40.0, 5.0, 0.0]));
    let mut player = PlayerState::default();
    let policy = InputPolicy::native(if qw {
        RuleSetId::QuakeWorld
    } else {
        RuleSetId::Quake
    });
    let mut ms = 0;
    for frame in 0..10000 {
        ms += [1, 7, 16, 33, 100][frame % 5];
        let time = EventTime(ms * 1_000_000);
        if frame % 137 == 0 {
            input.center_view(SeatId::FIRST);
        }
        input.button(
            SeatId::FIRST,
            Action::MouseLook,
            (3..6).contains(&(frame % 83)),
            Some(1),
            time,
        );
        input.build_frame_with_policy(time, &[policy; SeatId::COUNT]);
        player.movement.grounded = frame % 17 != 0;
        player.movement.mode = if frame % 53 == 0 {
            MovementMode::Noclip
        } else {
            MovementMode::Walk
        };
        player.ideal_pitch = (frame % 23) as f32 - 11.0;
        let forward = if frame % 41 < 20 { 200.0 } else { 0.0 };
        let angles = input.drift_view(SeatId::FIRST, time, policy, &player, forward);
        println!("{:08x}", angles.0[0].to_bits());
    }
    Ok(())
}
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::{
        Stopwatch,
        allocations::{begin_frame, end_frame},
    };
    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut seed = 0x434d4449;
    let intents: [CommandIntent; 64] = std::array::from_fn(|_| intent(&mut seed));
    let policies: [InputPolicy; 64] =
        std::array::from_fn(|index| InputPolicy::native(RuleSetId::ALL[index % 5]));
    let mut samples = [0u64; 600];
    let mut allocations = 0;
    let mut bytes = 0;
    let mut checksum = 0u64;
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        for index in 0..64 {
            let command = black_box(UserCmdBuilder::build(
                Duration::from_nanos(16_666_667),
                EventTime(frame * 16_666_667),
                black_box(intents[index]),
                policies[index],
            ));
            checksum = checksum.wrapping_add(u64::from(command.movement[0].to_bits()));
        }
        let ns = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[(frame - 60) as usize] = ns;
            allocations += counts.allocations + counts.reallocations;
            bytes += counts.requested_bytes;
        }
    }
    if allocations != 0 || bytes != 0 {
        return Err("mixed command allocation gate".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"one mixed-rule intent builder; no gameplay or wire framing\",\"clients\":64,\"warmup\":60,\"frames\":600,\"allocations_or_reallocations\":{allocations},\"requested_bytes\":{bytes},\"checksum\":{checksum},\"median_ns\":{},\"p99_ns\":{}}}",
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    Err("bench requires allocation-tracking".into())
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn bench_view() -> Result<(), Box<dyn std::error::Error>> {
    use qa_core::{
        primitives::{PlayerState, Vec3},
        sys_events::SeatId,
    };
    use qa_input::Input;
    use qa_platform::{
        Stopwatch,
        allocations::{begin_frame, end_frame},
    };
    let mut input = Input::load();
    input.seed(EventTime(0));
    let policies = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake3,
    ]
    .map(InputPolicy::native);
    let mut player = PlayerState::default();
    player.movement.grounded = true;
    for seat in 0..SeatId::COUNT {
        input.set_view_angles(
            SeatId::new(seat as u8).ok_or("seat")?,
            Vec3([40.0, 0.0, 0.0]),
        );
    }
    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut allocations = 0;
    let mut bytes = 0;
    let mut checksum = 0u64;
    for frame in 0..660 {
        let time = EventTime((frame + 1) * 16_666_667);
        begin_frame();
        let timer = Stopwatch::start();
        if frame % 137 == 0 {
            for seat in 0..SeatId::COUNT {
                input.center_view(SeatId::new(seat as u8).ok_or("seat")?);
            }
        }
        let commands = black_box(input.build_frame_with_policy(time, &policies));
        for seat in 0..SeatId::COUNT {
            let view = black_box(input.drift_view(
                SeatId::new(seat as u8).ok_or("seat")?,
                time,
                policies[seat],
                &player,
                commands[seat].movement[0],
            ));
            checksum = checksum.wrapping_add(u64::from(view.0[0].to_bits()));
        }
        let ns = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[frame as usize - 60] = ns;
            allocations += counts.allocations + counts.reallocations;
            bytes += counts.requested_bytes;
        }
    }
    if allocations != 0 || bytes != 0 {
        return Err("view allocation gate".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"four-seat native command and view centering; no gameplay or native heap\",\"warmup\":60,\"frames\":600,\"allocations_or_reallocations\":{allocations},\"requested_bytes\":{bytes},\"allocation_positive_control\":1,\"checksum\":{checksum},\"median_ns\":{},\"p99_ns\":{}}}",
        (samples[299] + samples[300]) / 2,
        samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn bench_view() -> Result<(), Box<dyn std::error::Error>> {
    Err("bench-view requires allocation-tracking".into())
}
