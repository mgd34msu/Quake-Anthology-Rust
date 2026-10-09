//! Actual scene submission fixtures for tools/check_draw_sort.py. A normal
//! test run checks sorted keys and payload ownership; it does not execute C.
use qa_render::assets::{MaterialSettings, Stage};
use qa_render::scene::DrawKind;
use qa_render::{Assets, Command, FrontEnd, Limits, MaterialId, Refdef, Vertex};
use std::fmt::Write;

fn fixtures() -> Vec<(String, Vec<usize>)> {
    let mut cases = Vec::new();
    for count in 0..=8 {
        cases.push((format!("cutoff_equal_{count}"), vec![3; count]));
        cases.push((format!("cutoff_ascending_{count}"), (0..count).collect()));
        cases.push((
            format!("cutoff_descending_{count}"),
            (0..count).rev().collect(),
        ));
        cases.push((
            format!("cutoff_mixed_{count}"),
            (0..count).map(|i| (i * 5 + 2) % 4).collect(),
        ));
    }
    cases.push(("equal_9".to_owned(), vec![5; 9]));
    cases.push(("equal_17".to_owned(), vec![5; 17]));
    cases.push((
        "mixed_9_alternating".to_owned(),
        vec![2, 1, 2, 1, 2, 1, 2, 1, 2],
    ));
    cases.push((
        "mixed_9_partition".to_owned(),
        vec![2, 1, 2, 1, 1, 2, 1, 2, 2],
    ));
    cases.push((
        "ascending_257".to_owned(),
        (0..257).map(|i| i / 9).collect(),
    ));
    cases.push((
        "descending_257".to_owned(),
        (0..257).rev().map(|i| i / 9).collect(),
    ));
    cases.push(("equal_1024".to_owned(), vec![11; 1024]));
    let lengths = [2, 7, 8, 9, 17, 31, 64, 257, 1024];
    for seed in 0..128u32 {
        let count = lengths[seed as usize % lengths.len()];
        let mut state = seed + 1;
        let keys = (0..count)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                // Alternate broad, equal-heavy and two-key partitions.
                let modulus = [32, 4, 2][seed as usize % 3];
                (state >> 16) as usize % modulus
            })
            .collect();
        cases.push((format!("seed_{seed}"), keys));
    }
    cases
}

fn sorted_items(assets: &Assets, materials: &[MaterialId], keys: &[usize]) -> Vec<(u32, u32)> {
    let count = keys.len().max(1);
    let mut front = FrontEnd::load(Limits {
        commands: 2,
        entities: 1,
        polys: count,
        vertices: count * 3,
        lights: 1,
        area_bytes: 1,
        surfaces: 1,
        draws: count,
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for (index, &key) in keys.iter().enumerate() {
        let vertices = [Vertex {
            texcoord: [index as f32, 0.0],
            ..Vertex::default()
        }; 3];
        assert!(frame.add_poly(materials[key], &vertices));
    }
    assert!(frame.render_scene(Refdef::default(), &[], assets));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    let items: Vec<_> = packet
        .draws(view.scene.draws)
        .iter()
        .map(|draw| {
            assert_eq!(draw.kind, DrawKind::Poly);
            let poly = packet.poly(draw.index);
            assert_eq!(poly.material, materials[keys[draw.index as usize]]);
            assert_eq!(
                packet.vertices(poly.vertices)[0].texcoord[0],
                draw.index as f32
            );
            (poly.material.0, draw.index)
        })
        .collect();
    assert_eq!(items.len(), keys.len());
    assert!(items.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    let mut indices: Vec<_> = items.iter().map(|item| item.1).collect();
    indices.sort_unstable();
    assert_eq!(indices, (0..keys.len() as u32).collect::<Vec<_>>());
    items
}

#[test]
fn original_draw_sort_fixture_export() {
    let mut assets = Assets::load().unwrap();
    let materials: Vec<_> = (0..32)
        .map(|key| {
            assets
                .register_material(
                    &format!("draw-sort-{key}"),
                    &[Stage::default()],
                    MaterialSettings {
                        sort: 8.0,
                        ..MaterialSettings::default()
                    },
                )
                .unwrap()
        })
        .collect();
    assert!(materials.windows(2).all(|pair| pair[0].0 < pair[1].0));
    // Every poly has the same floating sort, instance and lighting values.
    // Monotonic material handles therefore provide the complete ordering
    // relation needed by original qsortFast's single unsigned sort key.
    let mut input = String::new();
    let mut output = String::new();
    let cases = fixtures();
    for (name, keys) in &cases {
        writeln!(input, "CASE {name} {}", keys.len()).unwrap();
        for (index, &key) in keys.iter().enumerate() {
            writeln!(input, "ITEM {} {index}", materials[key].0).unwrap();
        }
        writeln!(output, "CASE {name} {}", keys.len()).unwrap();
        for (key, index) in sorted_items(&assets, &materials, keys) {
            writeln!(output, "ITEM {key} {index}").unwrap();
        }
    }
    if let Ok(path) = std::env::var("QA_DRAW_SORT_EVIDENCE") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("input.txt"), input).unwrap();
        std::fs::write(path.join("rust.txt"), output).unwrap();
        println!(
            "DRAW_SORT fixtures={} export=written original_c=NOT_RUN",
            cases.len()
        );
    } else {
        println!(
            "DRAW_SORT fixtures={} export=disabled original_c=NOT_RUN",
            cases.len()
        );
    }
}
