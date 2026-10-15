use qa_core::primitives::{Plane, Vec3};
use qa_formats::{FormatError, bsp::Map};
#[path = "support/brushlist.rs"]
mod support;
use support::{SourceBrush, bsp, bsp_with_lumps, chunk};

fn word(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn brushlist_presence_is_typed_and_preserves_absent_empty_and_omitted_hulls() {
    for version in [29, 30, 0x32505342, 0x42535032, 0x51363420] {
        let bytes = bsp(version, 3, false, None);
        let original = Map::parse(&bytes).unwrap();
        assert!(!original.authored_brushes);
        assert!(original.brushes.is_empty());
        let bytes = bsp(version, 3, true, Some(&[]));
        let empty = Map::parse(&bytes).unwrap();
        assert!(empty.authored_brushes);
        assert!(empty.brushes.is_empty());
        assert!(empty.models.iter().all(|model| model.brushes.count == 0));
        assert!(empty.clipnodes.is_empty());
        assert_eq!(empty.models[0].headnodes, [0; 4]);
        assert_eq!(empty.planes.len(), original.planes.len());
        assert!(Map::parse(&bsp(version, 3, true, None)).is_err());
    }
}

#[test]
fn complete_repeated_model_chunks_replace_and_flatten_once_in_existing_records() {
    let mut brush = SourceBrush::cube(20.0, -3);
    brush.planes = vec![[1.0, 1.0, 0.0, 24.0], [2.0, 0.0, 0.0, 42.0]];
    let payload = [
        chunk(1, &[SourceBrush::cube(10.0, -8)]),
        chunk(0, &[brush]),
        chunk(1, &[SourceBrush::cube(30.0, -16)]),
    ]
    .concat();
    let bytes = bsp(29, 3, true, Some(&payload));
    let map = Map::parse(&bytes).unwrap();
    assert!(map.authored_brushes);
    assert_eq!(map.brushes.len(), 2);
    assert_eq!(
        map.brushes
            .iter()
            .map(|brush| brush.contents)
            .collect::<Vec<_>>(),
        [-3, -16]
    );
    assert_eq!(
        map.models
            .iter()
            .map(|model| (model.brushes.first, model.brushes.count))
            .collect::<Vec<_>>(),
        [(0, 1), (1, 1), (2, 0)]
    );
    assert_eq!(map.planes[0].normal, Vec3([1.0, 0.0, 0.0]));
    assert_eq!(map.planes[1], Plane::oriented(Vec3([1.0, 1.0, 0.0]), 24.0));
    assert_eq!(map.planes[2], Plane::oriented(Vec3([2.0, 0.0, 0.0]), 42.0));
    let expected = [
        ([1.0, 0.0, 0.0], 21.0),
        ([0.0, 1.0, 0.0], 1.0),
        ([0.0, 0.0, 1.0], 1.0),
        ([-1.0, 0.0, 0.0], -19.0),
        ([0.0, -1.0, 0.0], 1.0),
        ([0.0, 0.0, -1.0], 1.0),
    ];
    for (plane, (normal, distance)) in map.planes[3..9].iter().zip(expected) {
        assert_eq!(*plane, Plane::oriented(Vec3(normal), distance));
    }
    assert!(
        map.brush_sides
            .iter()
            .enumerate()
            .all(|(index, side)| side.plane as usize == index + 1
                && side.texture_info.is_none()
                && side.shader.is_none())
    );
    let payload = [payload, chunk(1, &[])].concat();
    let bytes = bsp(29, 3, true, Some(&payload));
    let cleared = Map::parse(&bytes).unwrap();
    assert_eq!(cleared.brushes.len(), 1);
    assert_eq!(cleared.models[1].brushes.count, 0);
    assert_eq!(cleared.models[2].brushes.count, 0);
    assert_eq!(cleared.planes.len(), 9);
}

#[test]
fn brushlist_rejects_malformed_recognized_data_without_native_fallback() {
    let valid = chunk(0, &[SourceBrush::cube(0.0, -2)]);
    let mut invalid = Vec::new();
    for (name, offset, value) in [
        ("version", 0, 2),
        ("model equals count", 4, 3),
        ("brush count", 8, u32::MAX),
        ("plane total", 12, u32::MAX),
        ("nonfinite bounds", 16, f32::INFINITY.to_bits()),
        ("reversed bounds", 16, 2.0f32.to_bits()),
    ] {
        let mut row = valid.clone();
        word(&mut row, offset, value);
        invalid.push((name, row));
    }
    let mut excess = valid.clone();
    excess[42..44].copy_from_slice(&1u16.to_le_bytes());
    invalid.push(("brush plane excess", excess));
    for (name, payload) in [
        ("truncated header", valid[..15].to_vec()),
        ("truncated brush", valid[..43].to_vec()),
        ("trailing", [valid.clone(), vec![0]].concat()),
        ("unconsumed planes", {
            let mut payload = [chunk(0, &[]), vec![0; 16]].concat();
            word(&mut payload, 12, 1);
            payload
        }),
    ] {
        invalid.push((name, payload));
    }
    let mut plane = SourceBrush::cube(0.0, -2);
    plane.planes.push([1.0, 1.0, 0.0, 0.0]);
    let with_plane = chunk(0, &[plane]);
    for (name, offset, value) in [
        ("plane infinity", 44, f32::INFINITY.to_bits()),
        ("plane nan distance", 56, f32::NAN.to_bits()),
    ] {
        let mut row = with_plane.clone();
        word(&mut row, offset, value);
        invalid.push((name, row));
    }
    let mut zero = with_plane.clone();
    zero[44..56].fill(0);
    invalid.push(("zero normal", zero));
    let mut mismatch = with_plane.clone();
    word(&mut mismatch, 12, 0);
    invalid.push(("underreported planes", mismatch));
    for (name, payload) in invalid {
        assert!(
            Map::parse(&bsp(29, 3, true, Some(&payload))).is_err(),
            "{name}"
        );
    }
}

#[test]
fn authored_models_can_retain_only_a_prefix_of_compiled_clip_hulls() {
    for version in [29, 30, 0x32505342, 0x42535032, 0x51363420] {
        let payload = chunk(1, &[SourceBrush::cube(10.0, -2)]);
        let bytes = bsp_with_lumps(version, 3, true, Some(&payload), |lumps| {
            compiled_clip(lumps, version);
            for at in [40, 44, 48] {
                word(&mut lumps[14], 64 + at, 123);
            }
        });
        let map = Map::parse(&bytes).unwrap();
        assert_eq!(map.clipnodes.len(), 1);
        assert_eq!(map.models[0].headnodes[1..], [0; 3]);
        assert_eq!(map.models[1].headnodes[1..], [123; 3]);
        assert_eq!(map.models[1].brushes.count, 1);
        let stock = bsp_with_lumps(version, 3, true, None, |lumps| {
            compiled_clip(lumps, version);
            word(&mut lumps[14], 64 + 40, 123);
        });
        assert!(matches!(
            Map::parse(&stock),
            Err(FormatError::InvalidReference("model clip root", 1))
        ));
    }
}

fn compiled_clip(lumps: &mut [Vec<u8>], version: u32) {
    if matches!(version, 0x32505342 | 0x42535032) {
        lumps[9] = vec![0; 12];
        word(&mut lumps[9], 4, (-1i32) as u32);
        word(&mut lumps[9], 8, (-2i32) as u32);
    } else {
        lumps[9] = vec![0; 8];
        lumps[9][4..6].copy_from_slice(&(-1i16).to_le_bytes());
        lumps[9][6..8].copy_from_slice(&(-2i16).to_le_bytes());
    }
}

fn native_face(lumps: &mut [Vec<u8>], wide: bool) {
    lumps[3] = vec![0; 12];
    lumps[6] = vec![0; 40];
    lumps[7] = vec![0; if wide { 28 } else { 20 }];
    if wide {
        word(&mut lumps[7], 12, 3);
        word(&mut lumps[7], 24, (-1i32) as u32);
    } else {
        lumps[7][8..10].copy_from_slice(&3u16.to_le_bytes());
        word(&mut lumps[7], 16, (-1i32) as u32);
    }
    lumps[12] = vec![0; if wide { 8 } else { 4 }];
    lumps[13] = vec![0; 12];
}

#[test]
fn authored_planes_cannot_validate_out_of_range_native_plane_references() {
    let payload = chunk(0, &[SourceBrush::cube(10.0, -2)]);
    for version in [29, 30, 0x32505342, 0x42535032, 0x51363420] {
        let wide = matches!(version, 0x32505342 | 0x42535032);
        for (lump, field) in [(5, "node plane"), (9, "clipnode plane"), (7, "face plane")] {
            let make = |bad| {
                bsp_with_lumps(version, 1, true, Some(&payload), |lumps| {
                    compiled_clip(lumps, version);
                    native_face(lumps, wide);
                    if bad {
                        if lump == 7 && !wide {
                            lumps[lump][..2].copy_from_slice(&1u16.to_le_bytes());
                        } else {
                            word(&mut lumps[lump], 0, 1);
                        }
                    }
                })
            };
            let valid = make(false);
            let map = Map::parse(&valid).unwrap();
            assert_eq!(map.planes.len(), 7);
            assert!(map.brush_sides.iter().all(|side| side.plane >= 1));
            assert!(
                matches!(
                    Map::parse(&make(true)),
                    Err(FormatError::InvalidReference(actual, 0)) if actual == field
                ),
                "{version:#x} {field}"
            );
        }
    }
}
