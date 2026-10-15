use qa_core::primitives::{PointLeafRules, Vec3};
use qa_formats::bsp::Map;
use qa_render::world::load_visibility;
use qa_world::visibility::{
    PointVisibility, VisibilityChannel,
    VisibilityChannel::{Hearing, Sight},
    VisibilityWorld,
};

#[allow(dead_code)]
#[path = "../../formats/tests/support/brushlist.rs"]
mod q1_source;

const SIGHT: [u8; 4] = [0b0011, 0b0110, 0b1100, 0b1000];
const ROOMS: [Vec3; 4] = [
    Vec3([5.0, 0.0, 0.0]),
    Vec3([-5.0, 0.0, 0.0]),
    Vec3([-15.0, 0.0, 0.0]),
    Vec3([-25.0, 0.0, 0.0]),
];

fn word(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn bounds_f32(bytes: &mut [u8], at: usize) {
    for (axis, value) in [-128.0f32; 3].into_iter().chain([128.0; 3]).enumerate() {
        word(bytes, at + axis * 4, value.to_bits() as i32);
    }
}

fn bounds_i16(bytes: &mut [u8], at: usize) {
    for (axis, value) in [-128i16; 3].into_iter().chain([128; 3]).enumerate() {
        bytes[at + axis * 2..at + axis * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
}

fn bounds_i32(bytes: &mut [u8], at: usize) {
    for (axis, value) in [-128; 3].into_iter().chain([128; 3]).enumerate() {
        word(bytes, at + axis * 4, value);
    }
}

fn planes(stride: usize) -> Vec<u8> {
    let mut bytes = vec![0; 3 * stride];
    for (index, distance) in [0.0f32, -10.0, -20.0].into_iter().enumerate() {
        word(&mut bytes, index * stride, 1.0f32.to_bits() as i32);
        word(&mut bytes, index * stride + 12, distance.to_bits() as i32);
    }
    bytes
}

fn q1_bytes(version: u32) -> Vec<u8> {
    let wide = matches!(version, 0x32505342 | 0x42535032);
    let floats = version == 0x32505342;
    let node_stride = if floats {
        44
    } else if wide {
        32
    } else {
        24
    };
    let leaf_stride = if floats {
        44
    } else if wide {
        32
    } else {
        28
    };
    q1_source::bsp_with_lumps(version, 1, false, None, |lumps| {
        lumps[1] = planes(20);
        lumps[4] = SIGHT.to_vec();
        lumps[5] = vec![0; 3 * node_stride];
        for (index, children) in [[-2, 1], [-3, 2], [-4, -5]].into_iter().enumerate() {
            let at = index * node_stride;
            word(&mut lumps[5], at, index as i32);
            if wide {
                word(&mut lumps[5], at + 4, children[0]);
                word(&mut lumps[5], at + 8, children[1]);
                if floats {
                    bounds_f32(&mut lumps[5], at + 12);
                } else {
                    bounds_i16(&mut lumps[5], at + 12);
                }
            } else {
                for (side, child) in children.into_iter().enumerate() {
                    lumps[5][at + 4 + side * 2..at + 6 + side * 2]
                        .copy_from_slice(&(child as i16).to_le_bytes());
                }
                bounds_i16(&mut lumps[5], at + 8);
            }
        }
        lumps[10] = vec![0; 5 * leaf_stride];
        for index in 0..5 {
            let at = index * leaf_stride;
            word(&mut lumps[10], at, if index == 0 { -2 } else { -1 });
            word(&mut lumps[10], at + 4, index as i32 - 1);
            if floats {
                bounds_f32(&mut lumps[10], at + 8);
            } else {
                bounds_i16(&mut lumps[10], at + 8);
            }
        }
        word(&mut lumps[14], 52, 4);
    })
}

fn ibsp(version: u32, lumps: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0; 8 + lumps.len() * 8];
    bytes[..4].copy_from_slice(b"IBSP");
    word(&mut bytes, 4, version as i32);
    for (index, lump) in lumps.iter().enumerate() {
        let offset = bytes.len() as i32;
        word(&mut bytes, 8 + index * 8, offset);
        word(&mut bytes, 12 + index * 8, lump.len() as i32);
        bytes.extend(lump);
    }
    bytes
}

fn q3_bytes(version: u32, visibility: bool) -> Vec<u8> {
    let mut lumps = vec![vec![]; if version == 47 { 18 } else { 17 }];
    lumps[2] = planes(16);
    lumps[3] = vec![0; 3 * 36];
    for (index, children) in [[-1, 1], [-2, 2], [-3, -4]].into_iter().enumerate() {
        let at = index * 36;
        word(&mut lumps[3], at, index as i32);
        word(&mut lumps[3], at + 4, children[0]);
        word(&mut lumps[3], at + 8, children[1]);
        bounds_i32(&mut lumps[3], at + 12);
    }
    lumps[4] = vec![0; 4 * 48];
    for index in 0..4 {
        word(&mut lumps[4], index * 48, index as i32);
        bounds_i32(&mut lumps[4], index * 48 + 8);
    }
    lumps[7] = vec![0; 40];
    bounds_f32(&mut lumps[7], 0);
    if visibility {
        lumps[16] = vec![0; 8];
        word(&mut lumps[16], 0, 4);
        word(&mut lumps[16], 4, 4);
        for row in SIGHT {
            // These bytes pad the native row stride, not the four-selector PVS.
            lumps[16].extend([row, 255, 255, 255]);
        }
    }
    ibsp(version, &lumps)
}

fn q2_bytes() -> Vec<u8> {
    let mut lumps = vec![vec![]; 19];
    lumps[1] = planes(20);
    lumps[4] = vec![0; 3 * 28];
    for (index, children) in [[-2, 1], [-3, 2], [-4, -5]].into_iter().enumerate() {
        let at = index * 28;
        word(&mut lumps[4], at, index as i32);
        word(&mut lumps[4], at + 4, children[0]);
        word(&mut lumps[4], at + 8, children[1]);
        bounds_i16(&mut lumps[4], at + 12);
    }
    lumps[8] = vec![0; 5 * 28];
    word(&mut lumps[8], 0, 1);
    for index in 0..5 {
        let at = index * 28;
        lumps[8][at + 4..at + 6].copy_from_slice(&(index as i16 - 1).to_le_bytes());
        bounds_i16(&mut lumps[8], at + 8);
    }
    lumps[13] = vec![0; 48];
    bounds_f32(&mut lumps[13], 0);
    lumps[17] = vec![0; 8];
    lumps[3] = vec![0; 36];
    word(&mut lumps[3], 0, 4);
    for index in 0..4 {
        word(&mut lumps[3], 4 + index * 8, 36 + index as i32);
        word(&mut lumps[3], 8 + index * 8, 40 + index as i32);
    }
    lumps[3].extend(SIGHT);
    lumps[3].extend([1, 2, 4, 8]);
    ibsp(38, &lumps)
}

fn visible(
    world: &VisibilityWorld,
    target: usize,
    channel: VisibilityChannel,
    rules: PointLeafRules,
) -> bool {
    world
        .test_points(
            ROOMS[0],
            ROOMS[target],
            &mut [0],
            PointVisibility {
                leaf_rules: rules,
                channel,
                missing_visible: false,
                area_zero: true,
            },
            None,
        )
        .unwrap()
}

fn assert_one_hop(world: &VisibilityWorld) {
    assert_eq!(world.pvs().selector_count(), 4);
    assert_eq!(world.pvs().row_bytes(), 1);
    for rules in [PointLeafRules::RENDER, PointLeafRules::CLIP] {
        assert!(visible(world, 1, Sight, rules));
        assert!(!visible(world, 2, Sight, rules));
        assert!(visible(world, 2, Hearing, rules));
        assert!(!visible(world, 3, Hearing, rules));
    }
    let mut row = [0];
    for (selector, expected) in SIGHT.into_iter().enumerate() {
        world
            .pvs()
            .read_channel(Sight, Some(selector as u32), &mut row)
            .unwrap();
        assert_eq!(row, [expected]);
    }
}

#[test]
fn parsed_q1_leaf_zero_is_not_a_pvs_selector_and_native_leaf_bits_expand_once() {
    for version in [29, 30, 0x32505342, 0x42535032, 0x51363420] {
        let bytes = q1_bytes(version);
        let map = Map::parse(&bytes).unwrap();
        assert_eq!(map.leaves.len(), 5);
        let world = load_visibility(&map).unwrap();
        assert!(world.leaf(0).unwrap().solid);
        assert_eq!(world.leaf(0).unwrap().selector, None);
        for (index, point) in ROOMS.into_iter().enumerate() {
            assert_eq!(
                world.point_in_leaf(point, PointLeafRules::RENDER),
                Some(index as u32 + 1)
            );
            assert_eq!(
                world.leaf(index as u32 + 1).unwrap().selector,
                Some(index as u32)
            );
        }
        assert_one_hop(&world);
    }
}

#[test]
fn parsed_q3_dense_rows_honor_native_stride_without_treating_padding_as_neighbors() {
    for version in [46, 47] {
        let bytes = q3_bytes(version, true);
        let map = Map::parse(&bytes).unwrap();
        let world = load_visibility(&map).unwrap();
        assert_eq!(world.leaf(0).unwrap().selector, Some(0));
        for (index, point) in ROOMS.into_iter().enumerate() {
            assert_eq!(
                world.point_in_leaf(point, PointLeafRules::CLIP),
                Some(index as u32)
            );
        }
        assert_one_hop(&world);
        let mut row = [0];
        world
            .pvs()
            .read_channel(Hearing, Some(0), &mut row)
            .unwrap();
        assert_eq!(row, [0b0111]);
    }
}

#[test]
fn parsed_q2_compiled_phs_remains_authoritative_for_any_selected_leaf_policy() {
    let bytes = q2_bytes();
    let map = Map::parse(&bytes).unwrap();
    let world = load_visibility(&map).unwrap();
    for rules in [PointLeafRules::RENDER, PointLeafRules::CLIP] {
        assert!(visible(&world, 1, Sight, rules));
        assert!(!visible(&world, 1, Hearing, rules));
        assert!(!visible(&world, 2, Hearing, rules));
    }
    let mut row = [0];
    for selector in 0..4 {
        world
            .pvs()
            .read_channel(Hearing, Some(selector), &mut row)
            .unwrap();
        assert_eq!(row, [1 << selector]);
        world
            .pvs()
            .read_channel(Sight, Some(selector), &mut row)
            .unwrap();
        assert_eq!(row, [SIGHT[selector as usize]]);
    }
}

#[test]
fn parsed_missing_q3_visibility_keeps_both_channels_all_visible() {
    let bytes = q3_bytes(46, false);
    let map = Map::parse(&bytes).unwrap();
    let world = load_visibility(&map).unwrap();
    assert_eq!(world.pvs().selector_count(), 4);
    for channel in [Sight, Hearing] {
        for target in 0..4 {
            assert!(visible(&world, target, channel, PointLeafRules::RENDER));
        }
        let mut row = [0];
        world.pvs().read_channel(channel, None, &mut row).unwrap();
        assert_eq!(row, [255]);
    }
}
