//! Extracted original key/button comparisons; never starts SDL or a map.
use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent};
use qa_input::{Input, Target, keys};
struct Sink;
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, _: SeatId, _: EventTime, _: &str) {}
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("key names path")?;
    let text = std::fs::read_to_string(path)?;
    let mut input = Input::load();
    input
        .bind_text(82, "+forward", EventTime(0), &mut Sink)
        .map_err(|e| format!("{e:?}"))?;
    for name in text.lines() {
        let value = keys::parse(name)
            .and_then(keys::native_number)
            .map_or(-1, i32::from);
        println!("K {value}");
    }
    let mut seed = 0x42494e44u32;
    let mut time = 0;
    for _ in 0..10000 {
        let duration = 8 + next(&mut seed) % 43;
        for offset in 1..duration {
            if (next(&mut seed) >> 16) & 3 != 0 {
                continue;
            }
            let down = !next(&mut seed).is_multiple_of(3);
            let code = if (next(&mut seed) >> 16) & 1 == 0 {
                26
            } else {
                82
            };
            input.dispatch(
                SysEvent {
                    time: EventTime((time + u64::from(offset)) * 1_000_000),
                    kind: EventKind::Key {
                        device: DeviceId::Keyboard,
                        code,
                        symbol: 0,
                        down,
                        repeat: false,
                    },
                },
                &mut Sink,
            );
        }
        time += u64::from(duration);
        let command =
            input.build_frame(EventTime(time * 1_000_000), [200; 3], [0.022; 2], [None; 4])[0];
        println!("F {}", command.movement[0]);
    }
    Ok(())
}
fn next(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}
