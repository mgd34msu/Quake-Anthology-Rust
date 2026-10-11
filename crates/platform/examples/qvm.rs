//! Original-interpreter comparison and pinned VM allocation/timing probe.
use qa_compat::qvm::{SystemCalls, Trap, Vm};
use qa_core::primitives::ThinkTime;
use qa_formats::program::qvm::Image;
use qa_platform::{
    Stopwatch,
    allocations::{CountingAllocator, begin_frame, end_frame},
};
use std::{hint::black_box, io::Write};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
struct Calls;
impl SystemCalls for Calls {
    fn call(&mut self, _: &mut Vm, number: u32, args: &[i32]) -> Result<i32, Trap> {
        if number != 2 {
            return Err(Trap::Syscall);
        }
        Ok(args
            .get(1)
            .copied()
            .ok_or(Trap::Syscall)?
            .wrapping_add(args.get(2).copied().ok_or(Trap::Syscall)?))
    }
}
fn word(bytes: &[u8], at: &mut usize) -> Result<i32, Box<dyn std::error::Error>> {
    let b = bytes.get(*at..*at + 4).ok_or("fixture word")?;
    *at += 4;
    Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args()
        .nth(1)
        .ok_or("qvm --compare FIXTURE | --inspect QVM | --timings QVM")?;
    let path = std::env::args().nth(2).ok_or("path")?;
    let bytes = std::fs::read(path)?;
    if mode == "--inspect" {
        let image = Image::parse(&bytes).map_err(|e| format!("{e:?}"))?;
        println!(
            "{{\"instructions\":{},\"code_bytes\":{},\"data_bytes\":{},\"literal_bytes\":{},\"bss_bytes\":{},\"memory_bytes\":{},\"gameplay\":false}}",
            image.instructions.len(),
            image.byte_to_instruction.len(),
            image.data_length,
            image.literal_length,
            image.bss_length,
            image.memory_size
        );
        return Ok(());
    }
    if mode == "--compare" {
        let mut at = 0;
        let cases = word(&bytes, &mut at)?;
        let mut output = std::io::BufWriter::new(std::io::stdout().lock());
        for _ in 0..cases {
            let length = usize::try_from(word(&bytes, &mut at)?).map_err(|_| "fixture length")?;
            let mut args = [0; 10];
            for arg in &mut args {
                *arg = word(&bytes, &mut at)?;
            }
            let image = Image::parse(bytes.get(at..at + length).ok_or("fixture image")?)
                .map_err(|e| format!("{e:?}"))?;
            at += length;
            let mut vm = Vm::load(image).map_err(|e| format!("{e:?}"))?;
            let result = vm
                .call(&mut Calls, args, 100000, false)
                .map_err(|e| format!("{e:?}"))?;
            output.write_all(&result.to_le_bytes())?;
            output.write_all(vm.memory.read(0, 64).map_err(|e| format!("{e:?}"))?)?;
            output.write_all(
                vm.memory
                    .read((vm.memory.len() - 128) as u64, 128)
                    .map_err(|e| format!("{e:?}"))?,
            )?;
        }
        if at != bytes.len() {
            return Err("fixture tail".into());
        }
        return Ok(());
    }
    if mode == "--engine-timings" {
        return engine_timings(&bytes);
    }
    if mode != "--timings" {
        return Err("mode".into());
    }
    let mut vm = Vm::load(Image::parse(&bytes).map_err(|e| format!("{e:?}"))?)
        .map_err(|e| format!("{e:?}"))?;
    let mut samples = [0u64; 600];
    let mut checksum = 0u64;
    let mut heap = 0;
    let mut requested = 0;
    begin_frame();
    black_box(vec![0u8; 32]);
    let positive = end_frame();
    if positive.allocations != 1 {
        return Err("positive control".into());
    }
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        for i in 0..1000 {
            let mut args = [0; 10];
            args[0] = i;
            let value = vm
                .call(&mut Calls, black_box(args), 100000, false)
                .map_err(|e| format!("{e:?}"))?;
            checksum = checksum.wrapping_add(black_box(value) as u32 as u64);
        }
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            heap += counts.allocations + counts.reallocations;
            requested += counts.requested_bytes;
        }
    }
    if heap != 0 || requested != 0 {
        return Err("VM heap".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless QVM interpreted fixture; no native module gameplay\",\"warmup\":60,\"frames\":600,\"calls_per_frame\":1000,\"checksum\":{checksum},\"heap\":{heap},\"requested_bytes\":{requested},\"hook_instructions\":{},\"median_ns\":{},\"p99_ns\":{},\"positive_control\":{}}}",
        vm.hooks.instructions,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593],
        positive.allocations
    );
    Ok(())
}

fn engine_timings(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use qa_compat::{
        abi::{Q3_SERVER, QvmCalls, UnknownCalls},
        services::{CallContext, ServiceStorage},
    };
    use qa_core::{events::OutputSubmission, primitives::ModuleId, sys_events::EventTime};
    use qa_world::{area::LinkOrder, entities::AllocationPolicy};
    let mut vm = Vm::load(Image::parse(bytes).map_err(|e| format!("{e:?}"))?)
        .map_err(|e| format!("{e:?}"))?;
    let mut runtime = qa_app::Runtime::load(4, std::iter::empty())?;
    let mut console = qa_console::commands::Console::new(qa_console::views::Context::default())?;
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 16)], 8, &console.cvars)
        .map_err(|e| format!("{e:?}"))?;
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(64)?;
    let mut samples = [0u64; 600];
    let mut checksum = 0u64;
    let mut heap = 0;
    let mut requested = 0;
    let mut publications = 0;
    begin_frame();
    black_box(vec![0u8; 32]);
    let positive = end_frame();
    if positive.allocations != 1 {
        return Err("positive control".into());
    }
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        {
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let context = CallContext {
                module: ModuleId(1),
                clock: ThinkTime::Milliseconds(frame),
                console: qa_console::views::Context::default(),
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Head,
            };
            let mut calls = QvmCalls {
                services: &mut services,
                table: &Q3_SERVER,
                context,
                platform_time: EventTime(frame as u64 * 1_000_000),
                command: &[],
                unknown: &mut unknown,
            };
            for _ in 0..64 {
                checksum = checksum.wrapping_add(
                    vm.call(&mut calls, black_box([0; 10]), 1000, false)
                        .map_err(|e| format!("{e:?}"))? as u32 as u64,
                );
            }
            let consumer = services.server.presentation;
            let mut batch = services.server.events.batch(consumer).ok_or("batch")?;
            while let Some(record) = services.server.events.next(&mut batch) {
                if !services.server.events.submit(
                    consumer,
                    record.sequence,
                    OutputSubmission::BestEffort,
                ) {
                    return Err("submission".into());
                }
                publications += 1;
            }
        }
        console.execute_frame(&mut runtime);
        let elapsed = timer.elapsed().as_nanos() as u64;
        let count = end_frame();
        if frame >= 60 {
            samples[frame as usize - 60] = elapsed;
            heap += count.allocations + count.reallocations;
            requested += count.requested_bytes;
        }
    }
    if heap != 0
        || requested != 0
        || unknown.calls != 0
        || checksum != 4_435_200
        || publications != 42240
    {
        return Err("engine VM fixture/heap".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless QVM numbered engine calls, cvars, print retirement and console; no retail gameplay\",\"warmup\":60,\"frames\":600,\"calls_per_frame\":64,\"checksum\":{checksum},\"publications\":{publications},\"unknown_calls\":{},\"heap\":{heap},\"requested_bytes\":{requested},\"hook_instructions\":{},\"median_ns\":{},\"p99_ns\":{},\"positive_control\":{}}}",
        unknown.calls,
        vm.hooks.instructions,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593],
        positive.allocations
    );
    Ok(())
}
