//! THE-2882: retail plane lumps must reach the actual app collision loader.
//! Run with QA_RETAIL_ROOT set to a retail-content root and --ignored.
use qa_content::vfs::Vfs;
use qa_formats::{archive::ArchiveReader, bsp::Map};
use qa_render::{Assets, material::world_load::WorldLoadOptions};
use qa_world::collision::CollisionStore;
use std::path::PathBuf;

fn load_retail(product: &str, name: &str, family: u8) {
    let root = PathBuf::from(std::env::var_os("QA_RETAIL_ROOT").expect("QA_RETAIL_ROOT"));
    let mut vfs = Vfs::default();
    vfs.mount_product(&root.join(product), 0)
        .expect("retail product");
    let path = format!("maps/{name}.bsp");
    let file = vfs.open(path.as_bytes()).expect("retail map");
    let mut bytes =
        vec![0; usize::try_from(vfs.length(file).expect("map size")).expect("map length")];
    let read = vfs
        .read_into_reusing(file, &mut bytes, &mut ArchiveReader::default())
        .expect("retail bytes");
    assert_eq!(read, bytes.len());
    let map = Map::parse(&bytes).expect("retail BSP");
    assert_eq!(map.bsp.format.family(), family);
    let mut positive = 0;
    let mut negative = 0;
    let mut negative_tagged_axial = 0;
    for plane in &map.planes {
        assert!(plane.distance.is_finite());
        assert!(plane.normal.0.iter().all(|value| value.is_finite()));
        assert_ne!(plane.normal.0, [0.0; 3]);
        for axis in 0..3 {
            if plane
                .normal
                .0
                .iter()
                .enumerate()
                .any(|(i, value)| i != axis && *value != 0.0)
            {
                continue;
            }
            if plane.normal.0[axis] == 1.0 {
                positive += 1;
            } else if plane.normal.0[axis] == -1.0 {
                negative += 1;
                negative_tagged_axial += usize::from(plane.axis.is_some());
            }
        }
    }
    assert!(positive > 0 && negative > 0);
    if family == 2 {
        assert_eq!(negative_tagged_axial, negative);
        if name == "base1" {
            assert_eq!((positive, negative), (1928, 1928));
        }
    } else {
        // Q3 classifies only positive unit normals as axial. Keep its source
        // metadata; accepting Q2's signed axis must not retag Q3's planes.
        assert_eq!(negative_tagged_axial, 0);
    }
    let mut geometry = CollisionStore::new();
    let loaded = qa_app::map::read(&vfs, name)
        .expect("map input")
        .load(
            &vfs,
            &mut Assets::load().unwrap(),
            &mut geometry,
            WorldLoadOptions::default(),
        )
        .expect("production map/collision admission");
    assert_eq!(
        geometry.model_count(loaded.collision),
        Some(map.models.len() as u32)
    );
    for (index, model) in map.models.iter().enumerate() {
        let bounds = geometry
            .model_bounds(loaded.collision, index as u32)
            .expect("loaded model bounds");
        assert_eq!(
            bounds.mins,
            model.bounds.mins - qa_core::primitives::Vec3([1.0; 3])
        );
        assert_eq!(
            bounds.maxs,
            model.bounds.maxs + qa_core::primitives::Vec3([1.0; 3])
        );
    }
    assert_eq!(loaded.collision_brushes, map.brushes.len());
    println!(
        "{name}: {} planes, {positive} positive/{negative} negative unit normals, {negative_tagged_axial} negative axial tags, {} brushes admitted",
        map.planes.len(),
        loaded.collision_brushes
    );
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn q2_base1_plane_lump_loads() {
    load_retail("q2/baseq2", "base1", 2);
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn q2_base2_plane_lump_loads() {
    load_retail("q2/baseq2", "base2", 2);
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn q2_q2dm1_plane_lump_loads() {
    load_retail("q2/baseq2", "q2dm1", 2);
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn q3_q3dm1_plane_lump_keeps_native_axis_tags() {
    load_retail("q3a/baseq3", "q3dm1", 3);
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn q3_q3dm7_plane_lump_keeps_native_axis_tags() {
    load_retail("q3a/baseq3", "q3dm7", 3);
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn retail_families_keep_inline_models_in_one_generation_checked_store() {
    use qa_core::primitives::Vec3;
    use qa_world::collision::{Contents, EntityTraceRules, TraceQuery, TraceRules};
    let root = PathBuf::from(std::env::var_os("QA_RETAIL_ROOT").expect("QA_RETAIL_ROOT"));
    let mut vfs = Vfs::default();
    let cases = [
        (
            "q1/id1",
            "e1m1",
            TraceRules::LEGACY,
            EntityTraceRules::QUAKE,
        ),
        (
            "q2/baseq2",
            "base1",
            TraceRules::LEGACY,
            EntityTraceRules::QUAKE2,
        ),
        (
            "q3a/baseq3",
            "q3dm7",
            TraceRules::ARENA,
            EntityTraceRules::ARENA,
        ),
    ];
    for (priority, (product, _, _, _)) in cases.iter().enumerate() {
        vfs.mount_product(&root.join(product), priority as i32)
            .expect("retail product in shared VFS");
    }
    let mut geometry = CollisionStore::new();
    let mut assets = Assets::load().unwrap();
    let mut loaded = Vec::new();
    for (_, name, rules, entity_rules) in cases {
        let map = qa_app::map::read(&vfs, name)
            .expect("shared VFS map")
            .load(
                &vfs,
                &mut assets,
                &mut geometry,
                WorldLoadOptions::default(),
            )
            .expect("retail geometry admission");
        assert!(geometry.model_count(map.collision).expect("model count") > 1);
        loaded.push((map.collision, rules, entity_rules, name));
    }
    assert_ne!(loaded[0].0, loaded[1].0);
    assert_ne!(loaded[1].0, loaded[2].0);
    let mut scratch = geometry.scratch();
    let mut retained = Vec::new();
    for &(handle, rules, entity_rules, name) in &loaded {
        let bounds = geometry
            .model_bounds(handle, 1)
            .expect("native inline model");
        let center = (bounds.mins + bounds.maxs) * 0.5;
        let mut contacts = 0;
        for axis in 0..3 {
            let mut start = center;
            let mut end = center;
            start.0[axis] = bounds.maxs.0[axis] + 32.0;
            end.0[axis] = bounds.mins.0[axis] - 32.0;
            let query = TraceQuery {
                mask: Contents(u64::MAX),
                ..TraceQuery::point(start, end, rules, entity_rules)
            };
            let trace = geometry.trace_model(handle, 1, query, &mut scratch);
            contacts += usize::from(trace.fraction < 1.0);
            let words = [
                trace.fraction.to_bits(),
                trace.end.0[0].to_bits(),
                trace.end.0[1].to_bits(),
                trace.end.0[2].to_bits(),
                trace.plane.normal.0[0].to_bits(),
                trace.plane.normal.0[1].to_bits(),
                trace.plane.normal.0[2].to_bits(),
                trace.plane.distance.to_bits(),
                u32::from(trace.start_solid) | (u32::from(trace.all_solid) << 1),
            ];
            retained.push((handle, query, words));
        }
        assert!(
            contacts > 0,
            "retail inline geometry must be traceable: {name}"
        );
        println!("{name}: explicit model1 contacts={contacts}");
    }
    let removed = loaded[1].0;
    assert!(geometry.remove(removed));
    for (handle, query, words) in retained {
        let trace = geometry.trace_model(handle, 1, query, &mut scratch);
        if handle == removed {
            assert_eq!(trace.fraction, 1.0);
            assert_eq!(trace.end, query.end);
            assert_eq!(
                geometry.point_contents_model(handle, 1, Vec3::default(), query.entity_rules),
                Contents::EMPTY
            );
        } else {
            assert_eq!(
                words,
                [
                    trace.fraction.to_bits(),
                    trace.end.0[0].to_bits(),
                    trace.end.0[1].to_bits(),
                    trace.end.0[2].to_bits(),
                    trace.plane.normal.0[0].to_bits(),
                    trace.plane.normal.0[1].to_bits(),
                    trace.plane.normal.0[2].to_bits(),
                    trace.plane.distance.to_bits(),
                    u32::from(trace.start_solid) | (u32::from(trace.all_solid) << 1),
                ]
            );
        }
    }
    let replacement = qa_app::map::read(&vfs, "base1")
        .expect("reload input")
        .load(
            &vfs,
            &mut assets,
            &mut geometry,
            WorldLoadOptions::default(),
        )
        .expect("reload geometry");
    assert_eq!(replacement.collision.slot, removed.slot);
    assert_ne!(replacement.collision.generation, removed.generation);
    assert!(geometry.model_bounds(removed, 1).is_none());
    assert!(geometry.model_bounds(replacement.collision, 1).is_some());
}
