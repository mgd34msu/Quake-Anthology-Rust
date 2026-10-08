//! THE-2882: retail plane lumps must reach the actual app collision loader.
//! Run with QA_RETAIL_ROOT set to a retail-content root and --ignored.
use qa_content::vfs::Vfs;
use qa_formats::{archive::ArchiveReader, bsp::Map};
use qa_render::{Assets, material::world_load::WorldLoadOptions};
use qa_world::collision::CollisionWorld;
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
    let loaded = qa_app::map::read(&vfs, name)
        .expect("map input")
        .load(&vfs, &mut Assets::load(), WorldLoadOptions::default())
        .expect("production map/collision admission");
    assert!(matches!(loaded.collision, CollisionWorld::Brushes(_)));
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
