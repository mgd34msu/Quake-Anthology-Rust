//! Cold retail resource probe. It initializes no display, audio or game loop.
use qa_app::map;
use qa_content::vfs::Vfs;
use qa_platform::Stopwatch;
use qa_render::{Assets, CpuPresentation, material::world_load::WorldLoadOptions};

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let product = args
        .next()
        .ok_or("expected product directory and virtual map name")?;
    let name = args.next().ok_or("expected virtual map name")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut vfs = Vfs::default();
    vfs.mount_product(std::path::Path::new(&product), 0)
        .map_err(|e| format!("mount: {e:?}"))?;
    let mut assets = Assets::load().map_err(|e| e.to_string())?;
    let mut collision = qa_world::collision::CollisionStore::new();
    let start = Stopwatch::start();
    let loaded = map::read(&vfs, &name)?.load(
        &vfs,
        &mut assets,
        &mut collision,
        WorldLoadOptions::default(),
    )?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let world = assets
        .world(loaded.render.world)
        .ok_or("missing registered world")?;
    let geometry = world.geometry();
    let stages: usize = assets
        .materials()
        .iter()
        .map(|material| material.stages.len())
        .sum();
    let skies = assets
        .materials()
        .iter()
        .filter(|material| material.settings.sky.is_some())
        .count();
    let indexed_images = assets
        .images()
        .iter()
        .filter(|image| image.indexed.is_some())
        .count();
    qa_console::logger::console(format_args!(
        "scope=cold_retail_resource_load\nmap={}\nload_ms={elapsed_ms:.3}\nworlds={}\nsurfaces={}\nvertices={}\ntriangles={}\nmaterials={}\nstages={stages}\nskies={skies}\nimages={}\nindexed_images={indexed_images}\npresentation={}\nentities_parsed={}\ncollision_brushes={}\nspawn={:?}\nspawn_fallback={}\ndiagnostics={}\nrendered=false\nwalked=false\n",
        loaded.virtual_path,
        assets.worlds().len(),
        geometry.surfaces.len(),
        geometry.vertices.len(),
        geometry.indices.len() / 3,
        assets.materials().len(),
        assets.images().len(),
        match loaded.render.presentation.cpu {
            CpuPresentation::Rgb => "rgb",
            CpuPresentation::Indexed { .. } => "indexed",
        },
        loaded.entity_count,
        loaded.collision_brushes,
        loaded.spawns[0].position,
        loaded.spawns[0].fixture_fallback,
        loaded.render.diagnostics.len()
    ));
    for diagnostic in &loaded.render.diagnostics {
        qa_console::logger::console(format_args!("diagnostic={diagnostic}\n"));
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        qa_console::logger::error(&error);
        std::process::exit(1);
    }
}
