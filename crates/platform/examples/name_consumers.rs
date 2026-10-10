//! Headless lookup/re-registration workload; no native gameplay acceptance.
use qa_console::{
    commands::{Console, Host, ScriptError},
    views::Context,
};
use qa_content::vfs::Vfs;
use qa_core::{names::NameTable, primitives::NameId, sys_events::EventTime, text::FixedText};
use qa_formats::image::RasterPolicy;
use qa_platform::{
    Stopwatch,
    allocations::{CountingAllocator, begin_frame, end_frame},
};
use qa_render::{
    Assets,
    assets::{MaterialSettings, Stage},
    material::resources::{ImageSettings, ImageUse, Images},
};
use std::{
    fmt::{Arguments, Write},
    hint::black_box,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
#[derive(Default)]
struct Capture {
    input: qa_input::Input,
    output: FixedText<8192>,
    failed: bool,
}
impl Host for Capture {
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        EventTime::default()
    }
    fn print(&mut self, text: Arguments<'_>) {
        self.failed |= self.output.write_fmt(text).is_err();
    }
    fn quit(&mut self) {
        self.failed = true;
    }
    fn read_script(&mut self, _: &str, _: &mut [u8]) -> Result<usize, ScriptError> {
        Err(ScriptError::Missing)
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let content = std::env::args()
        .nth(1)
        .ok_or("name_consumers CONTENT (with env/native.tga)")?;
    if content == "--compare" {
        let path = std::env::args().nth(2).ok_or("--compare FIXTURE")?;
        let bytes = std::fs::read(path)?;
        let mut offset = 0;
        let mut results = Vec::new();
        while offset < bytes.len() {
            let mut names = [&[][..]; 2];
            for name in &mut names {
                let prefix = bytes.get(offset..offset + 2).ok_or("name length")?;
                let length = u16::from_le_bytes([prefix[0], prefix[1]]) as usize;
                offset += 2;
                *name = bytes.get(offset..offset + length).ok_or("name bytes")?;
                offset += length;
            }
            results.push(match qa_core::names::compare_folded(names[0], names[1]) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            });
        }
        println!("{results:?}");
        return Ok(());
    }
    let mut vfs = Vfs::default();
    vfs.mount_directory(std::path::Path::new(&content), 0)
        .map_err(|e| format!("{e:?}"))?;
    let mut assets = Assets::load()?;
    let stages = [Stage::default()];
    let material =
        assets.register_material("ENV\\NATIVE.TGA", &stages, MaterialSettings::default())?;
    let material_name = assets.material(material).ok_or("material")?.name;
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RasterPolicy::Quake3,
        ImageSettings::native(qa_core::primitives::RuleSetId::Quake3),
    )
    .map_err(|e| format!("{e:?}"))?;
    let image = images
        .raster("env/native.tga", ImageUse::default())
        .map_err(|e| format!("{e:?}"))?;
    let mut console = Console::new(Context::default())?;
    let mut host = Capture::default();
    let mut names = NameTable::load_reserved([b"TARGET".as_slice(), b"target"], 1, 8)?;
    let target = names.find(b"target").ok_or("exact target")?;
    let upper = names.find(b"TARGET").ok_or("exact target")?;
    let registered = names.intern(b"later")?;
    if target == upper || registered == target || names.find(b"") != Some(NameId(0)) {
        return Err("exact identity".into());
    }
    begin_frame();
    black_box(vec![0u8; 32]);
    let positive = end_frame();
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut maximum = 0;
    let mut maximum_bytes = 0;
    for frame in 0..660 {
        host.output.clear();
        begin_frame();
        let timer = Stopwatch::start();
        for _ in 0..64 {
            if names.find(b"target") != Some(target)
                || names.find_folded(b"TaRgEt") != Some(upper)
                || names.intern(b"later")? != registered
                || names.find_folded(b"unknown").is_some()
            {
                return Err("name identity".into());
            }
            let reused = images
                .raster("ENV\\NATIVE.TGA", ImageUse::pic())
                .map_err(|e| format!("{e:?}"))?;
            if reused != image
                || images.assets.register_material(
                    "env/NATIVE.tga",
                    &stages,
                    MaterialSettings::default(),
                )? != material
            {
                return Err("asset reuse".into());
            }
        }
        console.append("alias First \"echo before\"; FIRST; alias FIRST \"echo after\"; first; unalias first; alias First \"echo stable\"; fIrSt; FOV 117; fov; NoSuchName\n", Context::default())
            .map_err(|e| format!("{e:?}"))?;
        console.execute_frame(&mut host);
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if host.failed
            || host.output.as_str()
                != "before \nafter \nstable \nfov = \"117\"\nUnknown command \"NoSuchName\"\n"
            || images.conflicts.len() != 1
            || images.conflicts[0].name != material_name
        {
            return Err(format!("consumer fidelity frame {frame}: failed={}, output={:?}, conflicts={:?}, material_name={material_name:?}", host.failed, host.output.as_str(), images.conflicts).into());
        }
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            maximum = maximum.max(counts.allocations + counts.reallocations);
            maximum_bytes = maximum_bytes.max(counts.requested_bytes);
        }
    }
    if maximum != 0 || maximum_bytes != 0 {
        return Err("lookup allocation".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless core, console alias mutation and renderer cache reuse; no gameplay\",\"warmup\":60,\"frames\":600,\"lookup_groups_per_frame\":64,\"image_conflicts\":1,\"exact_names_distinct\":true,\"maximum_allocations\":{maximum},\"maximum_requested_bytes\":{maximum_bytes},\"positive_control\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        positive.allocations,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593]
    );
    Ok(())
}
