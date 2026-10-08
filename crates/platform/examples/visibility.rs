//! THE-862 retail load/query verification. No window, input or gameplay host.
use qa_content::vfs::Vfs;
use qa_core::primitives::Vec3;
use qa_formats::{
    archive::Archive,
    bsp::{Lump, Map},
    entities::{EntityLump, EntitySyntax},
};
use qa_platform::Stopwatch;
use qa_world::visibility::{ViewVisibility, VisibilityWorld};
use std::{fmt::Write, fs::File, path::PathBuf, sync::Arc};

const SEED: u32 = 0x5155414b;
const SEEDED_POINTS: usize = 10_000;
const QUERIES_PER_MAP: usize = 32;
const WARMUP: usize = 60;
const FRAMES: usize = 600;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Spec {
    archive: PathBuf,
    map: String,
}
struct NativeRows {
    rows: Vec<Vec<u8>>,
    missing: Vec<bool>,
    all: Vec<u8>,
    source_stride: usize,
}
impl NativeRows {
    fn read(&self, selector: Option<u32>) -> &[u8] {
        selector.map_or(&self.all, |index| &self.rows[index as usize])
    }
    fn missing(&self, selector: Option<u32>) -> bool {
        selector.is_none_or(|index| self.missing[index as usize])
    }
}
fn word(bytes: &[u8], at: usize) -> Result<u32, &'static str> {
    let value = bytes.get(at..at + 4).ok_or("native visibility header")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}
fn native_rows(map: &Map<'_>) -> Result<NativeRows, &'static str> {
    let bytes = map.bsp.bytes(Lump::Visibility);
    let family = map.bsp.format.family();
    let count = if family == 1 {
        usize::try_from(map.models[0].visible_leaves).map_err(|_| "native leaf count")?
    } else if bytes.is_empty() {
        map.leaves
            .iter()
            .filter_map(|leaf| usize::try_from(leaf.cluster).ok())
            .max()
            .map_or(0, |cluster| cluster + 1)
    } else {
        word(bytes, 0)? as usize
    };
    let length = count.div_ceil(8);
    let source_stride = if family == 3 && !bytes.is_empty() {
        word(bytes, 4)? as usize
    } else {
        length
    };
    let mut rows = Vec::with_capacity(count);
    let mut missing = Vec::with_capacity(count);
    for selector in 0..count {
        let offset = if bytes.is_empty() {
            None
        } else if family == 1 {
            usize::try_from(map.leaves[selector + 1].visibility_offset).ok()
        } else if family == 2 {
            usize::try_from(word(bytes, 4 + selector * 8)? as i32).ok()
        } else {
            Some(8 + selector * source_stride)
        };
        let mut row = vec![255; length];
        if let Some(mut at) = offset {
            if family == 3 {
                row.copy_from_slice(bytes.get(at..at + length).ok_or("native dense PVS row")?);
            } else {
                // qsrc Q1/Q2 Mod_DecompressVis: literal, or 0 followed by zero count.
                let mut written = 0;
                while written < length {
                    let byte = *bytes.get(at).ok_or("native compressed PVS row")?;
                    at += 1;
                    if byte != 0 {
                        row[written] = byte;
                        written += 1;
                    } else {
                        let count = *bytes.get(at).ok_or("native PVS zero run")? as usize;
                        at += 1;
                        if count == 0 || count > length - written {
                            return Err("native PVS zero run length");
                        }
                        row[written..written + count].fill(0);
                        written += count;
                    }
                }
            }
        }
        missing.push(offset.is_none());
        rows.push(row);
    }
    Ok(NativeRows {
        rows,
        missing,
        all: vec![255; length],
        source_stride,
    })
}
fn native_leaf(map: &Map<'_>, point: Vec3) -> u32 {
    let mut child = map.models[0].headnodes[0];
    while child >= 0 {
        let node = &map.nodes[child as usize];
        let plane = map.planes[node.plane as usize];
        // qsrc Mod_PointInLeaf/R_PointInLeaf: a zero distance takes child 1.
        let distance = point.0[0] * plane.normal.0[0]
            + point.0[1] * plane.normal.0[1]
            + point.0[2] * plane.normal.0[2]
            - plane.distance;
        child = node.children[if distance > 0.0 { 0 } else { 1 }];
    }
    (-1 - i64::from(child)) as u32
}
fn selector(map: &Map<'_>, leaf: u32, count: usize) -> Option<u32> {
    if map.bsp.format.family() == 1 {
        (leaf > 0 && leaf as usize <= count).then_some(leaf.saturating_sub(1))
    } else {
        u32::try_from(map.leaves[leaf as usize].cluster).ok()
    }
}
fn reachable_leaves(map: &Map<'_>) -> Vec<bool> {
    let mut seen_nodes = vec![false; map.nodes.len()];
    let mut leaves = vec![false; map.leaves.len()];
    let mut pending = vec![map.models[0].headnodes[0]];
    while let Some(child) = pending.pop() {
        if child < 0 {
            leaves[(-1 - i64::from(child)) as usize] = true;
        } else if !seen_nodes[child as usize] {
            seen_nodes[child as usize] = true;
            pending.extend(map.nodes[child as usize].children);
        }
    }
    leaves
}
fn native_faces(
    map: &Map<'_>,
    rows: &NativeRows,
    reachable: &[bool],
    primary: Option<u32>,
    secondary: Option<u32>,
    marked: &mut [bool],
) {
    marked.fill(false);
    let primary_row = rows.read(primary);
    let secondary_row = secondary.map(|id| rows.read(Some(id)));
    let all = rows.missing(primary) || secondary.is_some_and(|id| rows.missing(Some(id)));
    for (index, leaf) in map.leaves.iter().enumerate() {
        let solid = match map.bsp.format.family() {
            1 => leaf.contents == -2,
            2 => leaf.contents & 1 != 0,
            _ => false,
        };
        if !reachable[index] || solid {
            continue;
        }
        let visible = all
            || selector(map, index as u32, rows.rows.len()).is_some_and(|id| {
                let byte = id as usize / 8;
                let mask = 1 << (id & 7);
                primary_row[byte] & mask != 0
                    || secondary_row.is_some_and(|row| row[byte] & mask != 0)
            });
        if visible {
            for &face in &map.leaf_faces[leaf.faces.indices()] {
                marked[face as usize] = true;
            }
        }
    }
}
fn spawn_origins(map: &Map<'_>) -> Result<Vec<Vec3>, &'static str> {
    let syntax = match map.bsp.format.family() {
        1 => EntitySyntax::Quake,
        2 => EntitySyntax::Quake2,
        _ => EntitySyntax::Quake3,
    };
    let entities = EntityLump::parse(map.entity_text(), syntax).map_err(|_| "entity lump parse")?;
    let find = |name: &[u8]| {
        if syntax == EntitySyntax::Quake {
            entities.names.find(name)
        } else {
            entities.names.find_folded(name)
        }
    };
    let origin = find(b"origin");
    let classname = find(b"classname");
    let mut points = Vec::new();
    for range in &entities.records {
        let fields = &entities.fields[range.clone()];
        if !fields
            .iter()
            .any(|field| Some(field.key) == classname && field.value.starts_with(b"info_player"))
        {
            continue;
        }
        let mut position = Vec3::default();
        if let Some(field) = fields.iter().find(|field| Some(field.key) == origin) {
            let mut coordinates = std::str::from_utf8(field.value)
                .map_err(|_| "spawn origin text")?
                .split_ascii_whitespace();
            for axis in &mut position.0 {
                *axis = coordinates
                    .next()
                    .ok_or("spawn origin coordinates")?
                    .parse()
                    .map_err(|_| "spawn origin number")?;
            }
            if coordinates.next().is_some() || position.0.iter().any(|value| !value.is_finite()) {
                return Err("spawn origin dimensions");
            }
        }
        points.push(position);
    }
    Ok(points)
}

struct Case {
    origin: Vec3,
    leaf: u32,
    primary: Option<u32>,
    secondary: Option<u32>,
    surfaces: usize,
    surface_id_sum: u64,
}
struct Loaded {
    spec: Spec,
    family: u8,
    world: VisibilityWorld,
    view: ViewVisibility,
    cases: Vec<Case>,
    spawn_count: usize,
    row_count: usize,
    row_bytes: usize,
    source_stride: usize,
    missing_rows: usize,
    bits_compared: u64,
    secondary_row_cases: usize,
    union_expansion_cases: usize,
    membership_queries: usize,
    membership_surface_checks: u64,
    verified_surface_count: u64,
    verified_surface_id_sum: u64,
    measured_surface_count: u64,
    measured_surface_id_sum: u64,
    last_leaf: u32,
}
fn load(spec: Spec) -> Result<Loaded, Box<dyn std::error::Error>> {
    let file = Arc::new(File::open(&spec.archive)?);
    let archive = Arc::new(Archive::parse(file).map_err(|e| format!("archive: {e:?}"))?);
    let mut vfs = Vfs::default();
    vfs.mount_archive(&spec.archive, 0, archive)
        .map_err(|e| format!("VFS mount: {e:?}"))?;
    let reference = vfs
        .open(spec.map.as_bytes())
        .ok_or("retail map absent in explicit archive")?;
    let length = usize::try_from(
        vfs.length(reference)
            .map_err(|e| format!("map length: {e:?}"))?,
    )?;
    let mut bytes = vec![0; length];
    if vfs
        .read_at(reference, 0, &mut bytes)
        .map_err(|e| format!("map read: {e:?}"))?
        != length
    {
        return Err("incomplete retail map read".into());
    }
    let map = Map::parse(&bytes).map_err(|e| format!("BSP parse: {e:?}"))?;
    let world =
        qa_render::world::load_visibility(&map).map_err(|e| format!("visibility load: {e:?}"))?;
    let rows = native_rows(&map)?;
    if rows.rows.len() != world.pvs().selector_count() || rows.all.len() != world.pvs().row_bytes()
    {
        return Err("normalized PVS dimensions differ".into());
    }
    let mut normalized = vec![0; rows.all.len()];
    let mut bits_compared = 0;
    for (selector, expected) in rows.rows.iter().enumerate() {
        world
            .pvs()
            .read_into(Some(selector as u32), &mut normalized)
            .map_err(|e| format!("PVS read: {e:?}"))?;
        for bit in 0..normalized.len() * 8 {
            if normalized[bit / 8] & (1 << (bit & 7)) != expected[bit / 8] & (1 << (bit & 7)) {
                return Err(
                    format!("{} PVS selector {selector} bit {bit} differs", spec.map).into(),
                );
            }
            bits_compared += 1;
        }
    }
    world
        .pvs()
        .read_into(None, &mut normalized)
        .map_err(|e| format!("fallback PVS: {e:?}"))?;
    if normalized != rows.all {
        return Err("all-visible fallback differs".into());
    }
    for (index, _) in map.leaves.iter().enumerate() {
        let native = selector(&map, index as u32, rows.rows.len());
        if world
            .leaf(index as u32)
            .is_none_or(|leaf| leaf.selector != native)
        {
            return Err("normalized leaf selector differs".into());
        }
    }
    let mut points = spawn_origins(&map)?;
    let spawn_count = points.len();
    let bounds = map.models[0].bounds;
    let mut state = SEED;
    for _ in 0..SEEDED_POINTS {
        points.push(Vec3(std::array::from_fn(|axis| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let fraction = (state >> 8) as f32 / 16_777_216.0;
            bounds.mins.0[axis] + (bounds.maxs.0[axis] - bounds.mins.0[axis]) * fraction
        })));
    }
    let reachable = reachable_leaves(&map);
    let mut view = ViewVisibility::new(&world);
    let mut expected_faces = vec![false; world.surface_count()];
    let mut actual_faces = vec![false; world.surface_count()];
    let mut cases = Vec::with_capacity(points.len());
    let (mut verified_surface_count, mut verified_surface_id_sum) = (0u64, 0u64);
    let (mut secondary_row_cases, mut union_expansion_cases) = (0, 0);
    for (index, &origin) in points.iter().enumerate() {
        let leaf = native_leaf(&map, origin);
        if world.point_in_leaf(origin) != Some(leaf) {
            return Err(format!("{} point {index} native leaf differs", spec.map).into());
        }
        let primary = selector(&map, leaf, rows.rows.len());
        let secondary = selector(
            &map,
            native_leaf(&map, points[(index + 97) % points.len()]),
            rows.rows.len(),
        );
        let mut primary_surface_count = 0;
        for (query_index, other) in [None, secondary].into_iter().enumerate() {
            native_faces(&map, &rows, &reachable, primary, other, &mut expected_faces);
            view.query(&world, origin, primary, other, &[], &[])
                .map_err(|e| format!("visibility query: {e:?}"))?;
            actual_faces.fill(false);
            for &face in view.visible_surfaces() {
                if actual_faces[face as usize] {
                    return Err("duplicate visible face".into());
                }
                actual_faces[face as usize] = true;
            }
            if actual_faces != expected_faces {
                return Err(
                    format!("{} point {index} visible face membership differs", spec.map).into(),
                );
            }
            if query_index == 0 {
                primary_surface_count = view.visible_surfaces().len();
            }
        }
        let surfaces = view.visible_surfaces().len();
        secondary_row_cases += usize::from(secondary.is_some() && secondary != primary);
        union_expansion_cases += usize::from(surfaces > primary_surface_count);
        let surface_id_sum = view
            .visible_surfaces()
            .iter()
            .map(|&face| u64::from(face) + 1)
            .sum();
        verified_surface_count += surfaces as u64;
        verified_surface_id_sum += surface_id_sum;
        cases.push(Case {
            origin,
            leaf,
            primary,
            secondary,
            surfaces,
            surface_id_sum,
        });
    }
    let membership_queries = points.len() * 2;
    let membership_surface_checks = membership_queries as u64 * world.surface_count() as u64;
    Ok(Loaded {
        family: map.bsp.format.family(),
        spec,
        world,
        view,
        cases,
        spawn_count,
        row_count: rows.rows.len(),
        row_bytes: rows.all.len(),
        source_stride: rows.source_stride,
        missing_rows: rows.missing.iter().filter(|&&missing| missing).count(),
        bits_compared,
        secondary_row_cases,
        union_expansion_cases,
        membership_queries,
        membership_surface_checks,
        verified_surface_count,
        verified_surface_id_sum,
        measured_surface_count: 0,
        measured_surface_id_sum: 0,
        last_leaf: 0,
    })
}

fn string(out: &mut String, value: &str) -> std::fmt::Result {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => write!(out, "\\u{:04x}", c as u32)?,
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(())
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let mut specs = Vec::new();
    let mut pending_archive = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(option) = args.next() {
        match option.as_str() {
            "--archive" if pending_archive.is_none() => {
                pending_archive = Some(PathBuf::from(args.next().ok_or("missing archive")?))
            }
            "--map" => specs.push(Spec {
                archive: pending_archive
                    .take()
                    .ok_or("each map needs an explicit archive")?,
                map: args.next().ok_or("missing map")?,
            }),
            "--output" => output = Some(PathBuf::from(args.next().ok_or("missing output")?)),
            _ => return Err("unknown visibility option".into()),
        }
    }
    if specs.len() != 3 || pending_archive.is_some() {
        return Err("provide three archive/map pairs for Q1, Q2 and Q3".into());
    }
    let mut maps: Vec<_> = specs.into_iter().map(load).collect::<Result<_, _>>()?;
    let mut families: Vec<_> = maps.iter().map(|map| map.family).collect();
    families.sort_unstable();
    if families != [1, 2, 3] {
        return Err("visibility workload requires one map from each BSP family".into());
    }
    let mut ns = [0u64; FRAMES];
    let (
        mut allocations,
        mut requested_bytes,
        mut maximum_allocations,
        mut maximum_requested_bytes,
    ) = (0u64, 0u64, 0u64, 0u64);
    let mut queries = 0usize;
    for frame in 0..WARMUP + FRAMES {
        begin_frame();
        let timer = Stopwatch::start();
        let mut matched = true;
        for map in &mut maps {
            for slot in 0..QUERIES_PER_MAP {
                let case = &map.cases[(frame * QUERIES_PER_MAP + slot) % map.cases.len()];
                matched &= map.world.point_in_leaf(case.origin) == Some(case.leaf);
                matched &= map
                    .view
                    .query(
                        &map.world,
                        case.origin,
                        case.primary,
                        case.secondary,
                        &[],
                        &[],
                    )
                    .is_ok();
                let surfaces = map.view.visible_surfaces();
                let sum: u64 = surfaces.iter().map(|&id| u64::from(id) + 1).sum();
                matched &= surfaces.len() == case.surfaces && sum == case.surface_id_sum;
                if frame >= WARMUP {
                    map.measured_surface_count += surfaces.len() as u64;
                    map.measured_surface_id_sum += sum;
                    map.last_leaf = case.leaf;
                }
            }
        }
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if !matched {
            return Err("timed visibility result differs from verified retail cases".into());
        }
        if frame >= WARMUP {
            ns[frame - WARMUP] = elapsed;
            queries += maps.len() * QUERIES_PER_MAP;
            let count = counts.allocations + counts.reallocations;
            allocations += count;
            requested_bytes += counts.requested_bytes;
            maximum_allocations = maximum_allocations.max(count);
            maximum_requested_bytes = maximum_requested_bytes.max(counts.requested_bytes);
        }
    }
    let mut sorted = ns;
    sorted.sort_unstable();
    let mut report = String::new();
    write!(
        report,
        "{{\"scope\":\"retail BSP load and visibility queries; no rendering or gameplay\",\"gameplay_qualified\":false,\"window_opened\":false,\"seed\":{SEED},\"warmup\":{WARMUP},\"frames\":{FRAMES},\"queries_per_map_per_frame\":{QUERIES_PER_MAP},\"measured_queries\":{queries},\"state_match\":true,\"allocations\":{allocations},\"requested_bytes\":{requested_bytes},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_requested_bytes},\"median_ns\":{},\"p99_ns\":{},\"samples_ns\":[",
        sorted[299] as f64 * 0.5 + sorted[300] as f64 * 0.5,
        sorted[593]
    )?;
    for (index, sample) in ns.iter().enumerate() {
        if index != 0 {
            report.push(',');
        }
        write!(report, "{sample}")?;
    }
    report.push_str("],\"maps\":[");
    for (index, map) in maps.iter().enumerate() {
        if index != 0 {
            report.push(',');
        }
        report.push_str("{\"archive\":");
        string(&mut report, &map.spec.archive.to_string_lossy())?;
        report.push_str(",\"map\":");
        string(&mut report, &map.spec.map)?;
        write!(
            report,
            ",\"family\":{},\"nodes\":{},\"leaves\":{},\"surfaces\":{},\"pvs_rows\":{},\"row_bytes\":{},\"source_stride\":{},\"missing_rows\":{},\"pvs_bits_compared\":{},\"pvs_rows_matched\":true,\"missing_pvs_all_visible\":true,\"leaf_selectors_matched\":true,\"seeded_points\":{SEEDED_POINTS},\"spawn_origins\":{},\"point_cases\":{},\"point_leaves_matched\":true,\"secondary_row_cases\":{},\"union_expansion_cases\":{},\"membership_queries\":{},\"membership_surface_checks\":{},\"face_membership_matched\":true,\"verified_surface_count\":{},\"verified_surface_id_sum\":{},\"measured_queries\":{},\"measured_surface_count\":{},\"measured_surface_id_sum\":{},\"last_leaf\":{}}}",
            map.family,
            map.world.node_count(),
            map.world.leaf_count(),
            map.world.surface_count(),
            map.row_count,
            map.row_bytes,
            map.source_stride,
            map.missing_rows,
            map.bits_compared,
            map.spawn_count,
            map.cases.len(),
            map.secondary_row_cases,
            map.union_expansion_cases,
            map.membership_queries,
            map.membership_surface_checks,
            map.verified_surface_count,
            map.verified_surface_id_sum,
            FRAMES * QUERIES_PER_MAP,
            map.measured_surface_count,
            map.measured_surface_id_sum,
            map.last_leaf
        )?;
    }
    report.push_str("],\"limits\":\"Unfrustumed static compiled PVS and primary/secondary union membership. Reference uses parsed native BSP nodes and leaf face spans; no extracted C executable, area mask, image, dlight or gameplay proof. Rust allocations cover the calling thread. Surface id sums are arithmetic workload results, never identity or cache keys.\"}\n");
    if let Some(path) = output {
        std::fs::write(path, &report)?;
    }
    print!("{report}");
    if allocations != 0 || requested_bytes != 0 {
        return Err("visibility allocation gate".into());
    }
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() -> Result<(), &'static str> {
    Err("build this developer visibility check with allocation-tracking")
}
