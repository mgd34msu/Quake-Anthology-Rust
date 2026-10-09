//! Original-QuakeC comparison and fixed headless allocation/timing probe.
use qa_compat::quakec::{Builtins, Layout, Trap, Vm};
use qa_formats::program::quakec::Image;
use qa_platform::{
    Stopwatch,
    allocations::{CountingAllocator, begin_frame, end_frame},
};
use std::{hint::black_box, io::Write};
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
struct Calls;
impl Builtins for Calls {
    fn call(&mut self, vm: &mut Vm, number: u32, argc: usize) -> Result<(), Trap> {
        if number != 1 || argc != 2 {
            return Err(Trap::Builtin);
        }
        vm.image.globals[1] =
            (f32::from_bits(vm.image.globals[4]) + f32::from_bits(vm.image.globals[7])).to_bits();
        Ok(())
    }
}
fn load(bytes: &[u8]) -> Result<Vm, Box<dyn std::error::Error>> {
    Vm::load(
        Image::parse(bytes, None).map_err(|e| format!("{e:?}"))?,
        Layout {
            entities: 4,
            header_bytes: 16,
            extra_string_bytes: 4096,
            state_step: 0.1,
        },
    )
    .map_err(|e| format!("{e:?}").into())
}
fn word(bytes: &[u8], at: &mut usize) -> Result<u32, Box<dyn std::error::Error>> {
    let b = bytes.get(*at..*at + 4).ok_or("fixture word")?;
    *at += 4;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args()
        .nth(1)
        .ok_or("quakec --inspect PROGRAM | --compare FIXTURE | --timings PROGRAM")?;
    let bytes = std::fs::read(std::env::args().nth(2).ok_or("path")?)?;
    if mode == "--inspect" {
        let image = Image::parse(&bytes, Some(5927)).map_err(|e| format!("{e:?}"))?;
        println!(
            "{{\"header_crc\":{},\"file_crc\":{},\"statements\":{},\"functions\":{},\"globals\":{},\"entityfields\":{},\"trapped_statements\":{},\"gameplay\":false}}",
            image.header_crc,
            image.file_crc,
            image.statements.len(),
            image.functions.len(),
            image.globals.len(),
            image.entityfields,
            image.trapped_statements
        );
        return Ok(());
    }
    if mode == "--compare" {
        let mut at = 0;
        let count = word(&bytes, &mut at)?;
        let mut output = std::io::BufWriter::new(std::io::stdout().lock());
        for _ in 0..count {
            let length = word(&bytes, &mut at)? as usize;
            let program = bytes.get(at..at + length).ok_or("program")?;
            at += length;
            let mut vm = load(program)?;
            let entities = bytes.get(at..at + 192).ok_or("entities")?;
            at += 192;
            vm.entities
                .write(0, entities)
                .map_err(|e| format!("{e:?}"))?;
            vm.call(&mut Calls, 1, 100000, false)
                .map_err(|e| format!("{e:?}"))?;
            for &word in &vm.image.globals {
                output.write_all(&word.to_le_bytes())?;
            }
            output.write_all(vm.entities.read(0, 192).map_err(|e| format!("{e:?}"))?)?;
        }
        if at != bytes.len() {
            return Err("fixture tail".into());
        }
        return Ok(());
    }
    if mode != "--timings" {
        return Err("mode".into());
    }
    let mut vm = load(&bytes)?;
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
            vm.image.globals[28] = black_box((i as f32).to_bits());
            let result = vm
                .call(&mut Calls, 1, 100000, false)
                .map_err(|e| format!("{e:?}"))?;
            checksum = checksum.wrapping_add(black_box(result[0]) as u64);
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
        return Err("QC heap".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless QuakeC interpreted fixture; no retail gameplay\",\"warmup\":60,\"frames\":600,\"calls_per_frame\":1000,\"checksum\":{checksum},\"heap\":{heap},\"requested_bytes\":{requested},\"hook_instructions\":{},\"median_ns\":{},\"p99_ns\":{},\"positive_control\":{}}}",
        vm.hooks.instructions,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593],
        positive.allocations
    );
    Ok(())
}
