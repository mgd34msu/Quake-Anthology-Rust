//! Developer inventory through the engine's actual VFS/catalog load path.
use qa_content::vfs::Vfs;
use qa_formats::{
    archive::{Archive, ArchiveReader},
    bsp::Map,
};
use qa_render::world::geometry::{GeometryKind, GeometryOptions, load_geometry};
use qa_render::{
    material::load_catalog,
    shader::{Severity, canonical_path},
};
use std::{fmt::Write, fs::File, path::PathBuf, sync::Arc};

#[expect(
    clippy::write_with_newline,
    reason = "Retain the existing exact report serialization format including its trailing newline"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let archive_path = PathBuf::from(args.next().ok_or("archive path required")?);
    let map_name = args.next().ok_or("virtual map name required")?;
    let output = PathBuf::from(args.next().ok_or("output path required")?);
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let archive = Arc::new(
        Archive::parse(Arc::new(File::open(&archive_path)?))
            .map_err(|error| format!("archive: {error:?}"))?,
    );
    let mut vfs = Vfs::default();
    vfs.mount_archive(&archive_path, 0, archive)
        .map_err(|error| format!("mount: {error:?}"))?;
    let catalog = load_catalog(&vfs).map_err(|error| format!("catalog: {error:?}"))?;
    let reference = vfs.open(map_name.as_bytes()).ok_or("missing map")?;
    let mut bytes = vec![
        0;
        usize::try_from(
            vfs.length(reference)
                .map_err(|e| format!("length: {e:?}"))?
        )?
    ];
    if vfs
        .read_into_reusing(reference, &mut bytes, &mut ArchiveReader::default())
        .map_err(|e| format!("map read: {e:?}"))?
        != bytes.len()
    {
        return Err("short map read".into());
    }
    let map = Map::parse(&bytes).map_err(|error| format!("map: {error:?}"))?;
    let geometry = load_geometry(&map, GeometryOptions::default())
        .map_err(|error| format!("geometry: {error:?}"))?;
    let scripts = vfs
        .files()
        .filter(|(_, name)| name.starts_with(b"scripts/") && name.ends_with(b".shader"))
        .count();
    let mut report = format!(
        "{{\"map\":{map_name:?},\"scripts\":{scripts},\"geometry_surfaces\":{},\"geometry_vertices\":{},\"geometry_boundaries\":{},\"light_samples\":{},\"patch_surfaces\":{},\"definitions\":{},\"invalid_definitions\":{},\"diagnostics\":{},\"error_diagnostics\":{},\"referenced_shaders\":{},\"materials\":[",
        geometry.surfaces.len(),
        geometry.vertices.len(),
        geometry.boundaries.len(),
        geometry.light_samples.len(),
        geometry
            .surfaces
            .iter()
            .filter(|s| s.kind == GeometryKind::Patch)
            .count(),
        catalog.definitions.len(),
        catalog.definitions.iter().filter(|s| !s.valid).count(),
        catalog.diagnostics.len(),
        catalog
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count(),
        map.shaders.len()
    );
    let (mut scripted, mut invalid, mut unsupported, mut stages) = (0, 0, 0, 0);
    for (index, shader) in map.shaders.iter().enumerate() {
        if index != 0 {
            report.push(',');
        }
        let name = canonical_path(std::str::from_utf8(shader.name)?);
        if let Some(definition) = catalog.find_canonical(&name) {
            scripted += 1;
            invalid += usize::from(!definition.valid);
            unsupported += usize::from(definition.has_unsupported_runtime());
            stages += definition.stages.len();
            write!(
                report,
                "{{\"name\":{name:?},\"scripted\":true,\"valid\":{},\"stages\":{},\"deforms\":{},\"no_draw\":{},\"unsupported_runtime\":{},\"declarations\":[",
                definition.valid,
                definition.stages.len(),
                definition.deforms.len(),
                shader.surface_flags & 128 != 0,
                definition.has_unsupported_runtime()
            )?;
            let mut first = true;
            for declaration in definition
                .unsupported
                .iter()
                .chain(definition.stages.iter().flat_map(|s| s.unsupported.iter()))
            {
                if !first {
                    report.push(',');
                }
                first = false;
                write!(report, "{:?}", declaration.keyword)?;
            }
            report.push_str("]}");
        } else {
            write!(
                report,
                "{{\"name\":{name:?},\"scripted\":false,\"default_material_required\":true}}"
            )?;
        }
    }
    write!(
        report,
        "],\"scripted_references\":{scripted},\"invalid_references\":{invalid},\"unsupported_runtime_references\":{unsupported},\"scripted_stages\":{stages},\"rendering_qualified\":false}}\n"
    )?;
    std::fs::write(output, report)?;
    println!(
        "scripts={scripts} definitions={} map_shaders={} scripted={scripted} invalid={invalid} unsupported={unsupported} stages={stages}",
        catalog.definitions.len(),
        map.shaders.len()
    );
    for diagnostic in &catalog.diagnostics {
        if diagnostic.severity == Severity::Error
            && diagnostic.shader.as_deref().is_some_and(|name| {
                map.shaders
                    .iter()
                    .any(|s| s.name == name.as_bytes() && s.surface_flags & 128 == 0)
            })
        {
            println!("parser_error: {diagnostic:?}");
        }
    }
    Ok(())
}
