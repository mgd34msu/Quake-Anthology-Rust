use qa_core::primitives::{Bounds, Plane, PointLeafRules, Vec3};
use qa_world::portals::{AreaPortals, Portal};
use qa_world::visibility::{
    PointVisibility, PvsRows, SurfaceSpan, VisLeaf, VisNode,
    VisibilityChannel::{All, Hearing, Sight},
    VisibilityError, VisibilityQueryError, VisibilityWorld,
};

#[test]
fn hearing_uses_one_original_pvs_hop_without_reaching_a_third_neighbor() {
    // A sees A/B, B sees B/C, C sees C/D, D sees D. Hearing from A includes
    // C, but it must never read the generated B hearing row and include D.
    let rows = PvsRows::load(
        vec![Some(0), Some(1), Some(2), Some(3)],
        vec![0b0011, 0b0110, 0b1100, 0b1000],
        None,
    )
    .unwrap();
    let mut row = [0];
    rows.read_channel(Sight, Some(0), &mut row).unwrap();
    assert_eq!(row, [0b0011]);
    rows.read_channel(Hearing, Some(0), &mut row).unwrap();
    assert_eq!(row, [0b0111]);
    rows.read_channel(Sight, Some(0), &mut row).unwrap();
    assert_eq!(row, [0b0011]);
}

#[test]
fn hearing_does_not_insert_self_or_make_visibility_symmetric() {
    let rows = PvsRows::load(
        vec![Some(0), Some(1), Some(2)],
        vec![0b010, 0b100, 0, 1],
        None,
    )
    .unwrap();
    for _ in 0..3 {
        for (selector, sight, hearing) in [(0, 0b010, 0b110), (1, 0b100, 0b100), (2, 0, 0)] {
            let mut row = [255];
            rows.read_channel(Hearing, Some(selector), &mut row)
                .unwrap();
            assert_eq!(row, [hearing]);
            rows.read_into(Some(selector), &mut row).unwrap();
            assert_eq!(row, [sight]);
        }
    }
}

#[test]
fn compiled_hearing_is_authoritative_including_individual_missing_rows() {
    let rows = PvsRows::load(
        vec![Some(0), Some(1), Some(2), Some(3)],
        vec![0b0011, 0b0110, 0b1100, 0b1000, 0b0001, 0b0010, 0b1000],
        Some(vec![Some(4), Some(5), None, Some(6)]),
    )
    .unwrap();
    for (selector, sight, hearing) in [
        (0, 0b0011, 0b0001),
        (1, 0b0110, 0b0010),
        (2, 0b1100, 255),
        (3, 0b1000, 0b1000),
    ] {
        let mut row = [0];
        rows.read_channel(Hearing, Some(selector), &mut row)
            .unwrap();
        assert_eq!(row, [hearing]);
        rows.read_channel(Sight, Some(selector), &mut row).unwrap();
        assert_eq!(row, [sight]);
    }
}

#[test]
fn only_selected_missing_sight_rows_expand_hearing_to_all_visible() {
    for (first, expected) in [(0b001, 0b001), (0b010, 255)] {
        let rows = PvsRows::load(vec![Some(0), None, Some(1)], vec![first, 0b100], None).unwrap();
        let mut row = [0];
        rows.read_channel(Hearing, Some(0), &mut row).unwrap();
        assert_eq!(row, [expected]);
        rows.read_channel(Hearing, Some(1), &mut row).unwrap();
        assert_eq!(row, [255]);
        rows.read_channel(Hearing, Some(2), &mut row).unwrap();
        assert_eq!(row, [0b100]);
        rows.read_channel(Sight, Some(0), &mut row).unwrap();
        assert_eq!(row, [first]);
    }
}

#[test]
fn one_hop_crosses_byte_boundaries_and_ignores_padding_as_selectors() {
    let mut encoded = Vec::new();
    let mut offsets = Vec::new();
    for selector in 0..10 {
        offsets.push(Some(encoded.len() as u32));
        let row = match selector {
            0 => [0, 0b1111_1101], // Selector 8 plus six unused padding bits.
            8 => [0, 0b0000_0010], // Selector 9.
            9 => [0b0000_0010, 0], // A third hop to selector 1 is forbidden.
            _ => [0, 0],
        };
        qa_core::rle::encode_pvs(&row, &mut encoded);
    }
    let rows = PvsRows::load(offsets, encoded, None).unwrap();
    assert_eq!(rows.row_bytes(), 2);
    let mut row = [0; 2];
    rows.read_channel(Hearing, Some(0), &mut row).unwrap();
    assert_eq!(row, [0, 255]);
    assert!(PvsRows::row_contains(&row, 8));
    assert!(PvsRows::row_contains(&row, 9));
    assert!(!PvsRows::row_contains(&row, 1));
    rows.read_channel(Sight, Some(0), &mut row).unwrap();
    assert_eq!(row, [0, 0b1111_1101]);
}

#[test]
fn empty_and_all_visible_tables_keep_channel_and_query_boundaries() {
    for rows in [
        PvsRows::load(vec![], vec![], None).unwrap(),
        PvsRows::load(vec![], vec![], Some(vec![])).unwrap(),
        PvsRows::all_visible(0),
    ] {
        assert_eq!(rows.selector_count(), 0);
        assert_eq!(rows.row_bytes(), 0);
        for channel in [Sight, Hearing] {
            rows.read_channel(channel, None, &mut []).unwrap();
            assert_eq!(
                rows.read_channel(channel, Some(0), &mut []),
                Err(VisibilityQueryError::Selector(0))
            );
            assert_eq!(
                rows.read_channel(channel, None, &mut [0]),
                Err(VisibilityQueryError::ScratchSize)
            );
        }
    }
    let rows = PvsRows::all_visible(9);
    for channel in [Sight, Hearing] {
        let mut row = [0; 2];
        rows.read_channel(channel, Some(8), &mut row).unwrap();
        assert_eq!(row, [255; 2]);
        rows.read_channel(channel, None, &mut row).unwrap();
        assert_eq!(row, [255; 2]);
        assert_eq!(
            rows.read_channel(channel, Some(9), &mut row),
            Err(VisibilityQueryError::Selector(9))
        );
    }
}

#[test]
fn both_sight_and_compiled_hearing_validate_offsets_and_native_rle() {
    for (encoded, expected) in [
        (vec![], VisibilityError::PvsOffset(0)),
        (vec![0], VisibilityError::PvsEncoding(0)),
        (vec![0, 0], VisibilityError::PvsEncoding(0)),
        (vec![0, 2], VisibilityError::PvsEncoding(0)),
    ] {
        assert_eq!(
            PvsRows::load(vec![Some(0)], encoded.clone(), None).unwrap_err(),
            expected
        );
        assert_eq!(
            PvsRows::load(vec![None], encoded, Some(vec![Some(0)])).unwrap_err(),
            expected
        );
    }
    assert_eq!(
        PvsRows::load(vec![None], vec![1], Some(vec![])).unwrap_err(),
        VisibilityError::Size
    );
    assert_eq!(
        PvsRows::load(vec![Some(1)], vec![1], None).unwrap_err(),
        VisibilityError::PvsOffset(1)
    );
    assert_eq!(
        PvsRows::load(vec![Some(0)], vec![1], Some(vec![Some(1)])).unwrap_err(),
        VisibilityError::PvsOffset(1)
    );
}

#[test]
fn all_channel_bypasses_pvs_but_retains_origin_and_portal_policy() {
    let rows = PvsRows::load(vec![Some(0)], vec![0, 1], None).unwrap();
    let mut row = [0];
    rows.read_channel(All, Some(u32::MAX), &mut row).unwrap();
    assert_eq!(row, [255]);
    assert_eq!(
        rows.read_channel(All, None, &mut []),
        Err(VisibilityQueryError::ScratchSize)
    );
    let bounds = Bounds {
        mins: Vec3([-2.0; 3]),
        maxs: Vec3([2.0; 3]),
    };
    let world = VisibilityWorld::load(
        vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds,
            surfaces: SurfaceSpan::default(),
        }],
        vec![
            VisLeaf {
                selector: None,
                area: Some(0),
                solid: false,
                bounds,
                surfaces: SurfaceSpan::default(),
            },
            VisLeaf {
                selector: Some(0),
                area: Some(1),
                solid: false,
                bounds,
                surfaces: SurfaceSpan::default(),
            },
        ],
        vec![],
        0,
        0,
        rows,
    )
    .unwrap();
    let mut portals = AreaPortals::load(
        2,
        1,
        vec![Portal {
            number: 0,
            first: 0,
            second: 1,
        }],
    )
    .unwrap();
    let first = Vec3([1.0, 0.0, 0.0]);
    let second = Vec3([-1.0, 0.0, 0.0]);
    let policy = PointVisibility {
        leaf_rules: PointLeafRules::RENDER,
        channel: All,
        missing_visible: false,
        area_zero: true,
    };
    assert_eq!(
        world.test_points(first, second, &mut row, policy, None),
        Ok(true)
    );
    assert_eq!(
        world.test_points(
            first,
            second,
            &mut row,
            PointVisibility {
                channel: Sight,
                ..policy
            },
            None,
        ),
        Ok(false)
    );
    assert_eq!(
        world.test_points(first, second, &mut row, policy, Some(&portals)),
        Ok(false)
    );
    portals.set(0, true).unwrap();
    assert_eq!(
        world.test_points(first, second, &mut row, policy, Some(&portals)),
        Ok(true)
    );
    assert_eq!(
        world.test_points(
            first,
            second,
            &mut row,
            PointVisibility {
                area_zero: false,
                ..policy
            },
            Some(&portals),
        ),
        Ok(false)
    );
    for (first, second) in [
        (Vec3([f32::NAN, 0.0, 0.0]), second),
        (first, Vec3([f32::INFINITY, 0.0, 0.0])),
    ] {
        assert_eq!(
            world.test_points(first, second, &mut row, policy, None),
            Err(VisibilityQueryError::Origin)
        );
    }
}
