use qa_render::surface_cache::{
    BuildState, CacheLayout, IndexedTexture, PaletteLighting, RgbaBuildState, SurfaceCache,
    SurfaceSource,
};
use std::cell::Cell;

fn indexed_source(extents: [u32; 2]) -> SurfaceSource {
    SurfaceSource::load([0; 2], extents, None, false).unwrap()
}

fn indexed_resources() -> (IndexedTexture, PaletteLighting) {
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![37; (16 >> mip) * (16 >> mip)]);
    let texture = IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), false).unwrap();
    let rgb: Vec<_> = (0..256u16).flat_map(|i| [i as u8; 3]).collect();
    let colormap: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    let palette = PaletteLighting::load(&rgb, &colormap, None, 256).unwrap();
    (texture, palette)
}

fn state() -> RgbaBuildState {
    RgbaBuildState {
        material_id: 17,
        base_image_id: 29,
        lightmap_image_id: Some(41),
        ..RgbaBuildState::default()
    }
}

#[test]
fn reservation_budget_includes_rover_alignment() {
    let source = SurfaceSource::load_rgba_texels([-1; 2], [3; 2], 1).unwrap();
    assert_eq!(source.reservation_bytes(0), Some(40));
    assert_eq!(source.reservation_bytes(1), None);
    let mut cache = SurfaceCache::load(vec![source], 40).unwrap();
    let block = cache
        .prepare_rgba(0, 0, state(), |out| {
            assert_eq!(out.len(), 36);
            out.fill(19);
        })
        .unwrap();
    assert_eq!(cache.rgba_pixels(block).unwrap(), &[[19; 4]; 9]);
}

#[test]
fn texel_grid_mips_align_negative_origins_and_odd_rectangular_tails() {
    let source = SurfaceSource::load_rgba_texels([-3, 1], [5, 3], 4).unwrap();
    let expected = [
        ([-3, 1], [5, 3]),
        ([-4, 0], [3, 2]),
        ([-4, 0], [2, 1]),
        ([-8, 0], [2, 1]),
    ];
    for (mip, layout) in expected.into_iter().enumerate() {
        assert_eq!(source.mip_layout(mip as u8), Some(layout));
    }
    assert_eq!(source.mip_layout(4), None);
    let mut cache = SurfaceCache::load(vec![source], 1024).unwrap();
    let block = cache
        .prepare_rgba(0, 1, state(), |out| {
            assert_eq!(out.len(), 3 * 2 * 4);
            out.fill(73);
        })
        .unwrap();
    assert_eq!(block.texture_mins, [-4, 0]);
    assert_eq!([block.width, block.height], [3, 2]);
    assert_eq!(cache.rgba_pixels(block).unwrap(), &[[73; 4]; 6]);
    let mut tampered = block;
    tampered.texture_mins[0] += 1;
    assert!(cache.rgba_pixels(tampered).is_none());
}

#[test]
fn explicit_stage_bytes_and_identity_light_invalidate_rgba_blocks() {
    let mut cache = SurfaceCache::load(
        vec![SurfaceSource::load_rgba([0; 2], [2; 2], 1).unwrap()],
        128,
    )
    .unwrap();
    let first = cache
        .prepare_rgba(0, 0, state(), |out| out.fill(1))
        .unwrap();
    let changed = RgbaBuildState {
        stage_colors: [[37; 4], [255; 4]],
        ..state()
    };
    let second = cache
        .prepare_rgba(0, 0, changed, |out| out.fill(2))
        .unwrap();
    assert!(cache.rgba_pixels(first).is_none());
    assert_eq!(cache.rgba_pixels(second).unwrap(), &[[2; 4]; 4]);
    let third = cache
        .prepare_rgba(
            0,
            0,
            RgbaBuildState {
                identity_light: 0.5,
                ..changed
            },
            |out| out.fill(3),
        )
        .unwrap();
    assert!(cache.rgba_pixels(second).is_none());
    assert_eq!(cache.rgba_pixels(third).unwrap(), &[[3; 4]; 4]);
    assert_eq!(cache.stats().fills, 3);
}

#[test]
fn mixed_layouts_and_variable_mip_prefixes_share_one_cache() {
    let sources = vec![
        indexed_source([16; 2]),
        SurfaceSource::load_rgba([-7, 11], [7, 3], 3).unwrap(),
        SurfaceSource::load_rgba([4, -13], [1, 5], 1).unwrap(),
        indexed_source([16; 2]),
    ];
    let mut cache = SurfaceCache::load(sources, 1024).unwrap();
    let (texture, palette) = indexed_resources();
    assert_eq!(cache.surface(0).unwrap().layout(), CacheLayout::Indexed8);
    assert_eq!(cache.surface(0).unwrap().mip_count(), 4);
    assert_eq!(cache.surface(1).unwrap().layout(), CacheLayout::Rgba8);
    assert_eq!(cache.surface(1).unwrap().mip_count(), 3);
    let native = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let rgb = cache
        .prepare_rgba(1, 1, state(), |out| {
            assert_eq!(out.len(), 3 * 4);
            out.copy_from_slice(&[9, 19, 29, 255, 8, 18, 28, 255, 7, 17, 27, 255]);
        })
        .unwrap();
    let narrow = cache
        .prepare_rgba(2, 0, state(), |out| out.fill(91))
        .unwrap();
    let last_native = cache
        .prepare(3, 3, &texture, &palette, BuildState::default())
        .unwrap();
    assert_eq!(cache.indexed_pixels(native).unwrap(), &[37; 256]);
    assert_eq!(cache.indexed_pixels(last_native).unwrap(), &[37; 4]);
    assert_eq!([rgb.width, rgb.height], [3, 1]);
    assert_eq!(rgb.texture_mins, [-7, 11]);
    assert_eq!(
        cache.rgba_pixels(rgb).unwrap(),
        &[[9, 19, 29, 255], [8, 18, 28, 255], [7, 17, 27, 255]]
    );
    assert_eq!(cache.pixels(narrow).unwrap(), &[91; 20]);
    assert!(cache.rgba_pixels(native).is_none());
    assert!(cache.indexed_pixels(rgb).is_none());
    assert_eq!(cache.stats().fills, 4);
    assert_eq!(cache.stats().evictions, 0);
}

#[test]
fn variable_mips_keep_one_dimensional_images_at_least_one_pixel_wide() {
    let mut cache = SurfaceCache::load(
        vec![
            SurfaceSource::load_rgba([-9, 37], [1, 17], 6).unwrap(),
            SurfaceSource::load_rgba([0; 2], [13, 1], 5).unwrap(),
            SurfaceSource::load_rgba([0; 2], [1, 1], 32).unwrap(),
        ],
        512,
    )
    .unwrap();
    for (mip, height) in [17, 8, 4, 2, 1, 1].into_iter().enumerate() {
        let span = cache
            .prepare_rgba(0, mip as u8, state(), |out| out.fill(mip as u8))
            .unwrap();
        assert_eq!([span.width, span.height], [1, height]);
        assert_eq!(span.texture_mins, [-9, 37]);
        assert_eq!(cache.rgba_pixels(span).unwrap().len(), height as usize);
    }
    for (mip, width) in [13, 6, 3, 1, 1].into_iter().enumerate() {
        let span = cache
            .prepare_rgba(1, mip as u8, state(), |out| out.fill(70 + mip as u8))
            .unwrap();
        assert_eq!([span.width, span.height], [width, 1]);
        assert_eq!(cache.rgba_pixels(span).unwrap().len(), width as usize);
    }
    let smallest = cache
        .prepare_rgba(2, 31, state(), |out| out.copy_from_slice(&[1, 2, 3, 255]))
        .unwrap();
    assert_eq!([smallest.width, smallest.height], [1, 1]);
    assert_eq!(cache.rgba_pixels(smallest).unwrap(), &[[1, 2, 3, 255]]);
}

#[test]
fn precombined_texture_times_lightmap_bytes_are_reused_and_revised() {
    let mut cache = SurfaceCache::load(
        vec![SurfaceSource::load_rgba([0; 2], [2, 1], 2).unwrap()],
        32,
    )
    .unwrap();
    let base = [[200u8, 100, 50, 255], [10, 250, 64, 255]];
    let light = [128u8, 255, 64];
    let fills = Cell::new(0);
    let first = cache
        .prepare_rgba(0, 0, state(), |out| {
            fills.set(fills.get() + 1);
            for (pixel, base) in out.as_chunks_mut::<4>().0.iter_mut().zip(base) {
                for channel in 0..3 {
                    pixel[channel] =
                        (u16::from(base[channel]) * u16::from(light[channel]) / 255) as u8;
                }
                pixel[3] = base[3];
            }
        })
        .unwrap();
    assert_eq!(
        cache.rgba_pixels(first).unwrap(),
        &[[100, 100, 12, 255], [5, 250, 16, 255]]
    );
    let hit = cache
        .prepare_rgba(0, 0, state(), |_| fills.set(fills.get() + 1))
        .unwrap();
    assert_eq!(fills.get(), 1);
    assert_eq!(cache.pixels(hit), cache.pixels(first));
    let changed = RgbaBuildState {
        lightmap_revision: 1,
        ..state()
    };
    let replacement = cache
        .prepare_rgba(0, 0, changed, |out| {
            out.copy_from_slice(base.as_flattened())
        })
        .unwrap();
    assert!(cache.pixels(first).is_none());
    assert!(cache.pixels(hit).is_none());
    assert_eq!(cache.rgba_pixels(replacement).unwrap(), &base);
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().fills, 2);
}

#[test]
fn numeric_id_revisions_and_exact_rgb_style_values_invalidate_precombined_data() {
    let mut cache = SurfaceCache::load(
        vec![SurfaceSource::load_rgba([0; 2], [1, 1], 1).unwrap()],
        8,
    )
    .unwrap();
    let initial = state();
    let mut changes = [initial; 10];
    changes[0].material_id += 1;
    changes[1] = changes[0];
    changes[1].material_revision += 1;
    changes[2] = changes[1];
    changes[2].base_image_id += 1;
    changes[3] = changes[2];
    changes[3].base_revision += 1;
    changes[4] = changes[3];
    changes[4].lightmap_image_id = None;
    changes[5] = changes[4];
    changes[5].lightmap_revision += 1;
    changes[6] = changes[5];
    changes[6].lighting_revision += 1;
    changes[7] = changes[6];
    changes[7].dynamic_revision += 1;
    changes[8] = changes[7];
    changes[8].style_scales[0][0] = f32::from_bits(1.0f32.to_bits() + 1);
    changes[9] = changes[8];
    changes[9].fullbright = true;
    let mut previous = cache
        .prepare_rgba(0, 0, initial, |out| out.fill(0))
        .unwrap();
    for (index, changed) in changes.into_iter().enumerate() {
        let next = cache
            .prepare_rgba(0, 0, changed, |out| out.fill(index as u8 + 1))
            .unwrap();
        assert!(cache.pixels(previous).is_none());
        assert_eq!(cache.pixels(next).unwrap(), &[index as u8 + 1; 4]);
        previous = next;
    }
    assert_eq!(cache.stats().hits, 0);
    assert_eq!(cache.stats().fills, 11);
}

#[test]
fn batch_pins_protect_both_layouts_and_changed_stamp_refills() {
    let (texture, palette) = indexed_resources();
    let mut cache = SurfaceCache::load(
        vec![
            indexed_source([16; 2]),
            SurfaceSource::load_rgba([0; 2], [2, 1], 1).unwrap(),
        ],
        256,
    )
    .unwrap();
    assert!(cache.begin_batch());
    let native = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let fills = Cell::new(0);
    assert!(
        cache
            .prepare_rgba(1, 0, state(), |_| fills.set(fills.get() + 1))
            .is_none()
    );
    assert_eq!(fills.get(), 0);
    assert_eq!(cache.indexed_pixels(native).unwrap(), &[37; 256]);
    cache.end_batch();
    let rgb = cache
        .prepare_rgba(1, 0, state(), |out| out.fill(83))
        .unwrap();
    assert!(cache.pixels(native).is_none());
    assert!(cache.begin_batch());
    let hit = cache.prepare_rgba(1, 0, state(), |_| {}).unwrap();
    assert!(
        cache
            .prepare(0, 0, &texture, &palette, BuildState::default())
            .is_none()
    );
    assert!(
        cache
            .prepare_rgba(
                1,
                0,
                RgbaBuildState {
                    base_revision: 1,
                    ..state()
                },
                |_| fills.set(fills.get() + 1)
            )
            .is_none()
    );
    assert_eq!(fills.get(), 0);
    assert_eq!(cache.pixels(rgb).unwrap(), &[83; 8]);
    assert_eq!(cache.pixels(hit).unwrap(), &[83; 8]);
    cache.end_batch();
    assert!(
        cache
            .prepare(0, 0, &texture, &palette, BuildState::default())
            .is_some()
    );
    assert!(cache.pixels(rgb).is_none());
    assert_eq!(cache.stats().rejected, 3);
}

#[test]
fn rejected_nested_batches_preserve_pins_after_block_metadata_reuse() {
    let mut cache = SurfaceCache::load(
        vec![
            SurfaceSource::load_rgba([0; 2], [2, 1], 1).unwrap(),
            SurfaceSource::load_rgba([0; 2], [2, 1], 1).unwrap(),
            SurfaceSource::load_rgba([0; 2], [4, 1], 1).unwrap(),
        ],
        16,
    )
    .unwrap();
    let rejected_fills = Cell::new(0);

    assert!(cache.begin_batch());
    let first = cache
        .prepare_rgba(0, 0, state(), |out| out.fill(17))
        .unwrap();
    let second = cache
        .prepare_rgba(1, 0, state(), |out| out.fill(29))
        .unwrap();
    assert!(!cache.begin_batch());
    assert!(
        cache
            .prepare_rgba(2, 0, state(), |_| {
                rejected_fills.set(rejected_fills.get() + 1)
            })
            .is_none()
    );
    assert_eq!(cache.pixels(first).unwrap(), &[17; 8]);
    assert_eq!(cache.pixels(second).unwrap(), &[29; 8]);
    cache.end_batch();

    // A new batch permits both old owners to be coalesced. Their second
    // metadata record returns to the free list, then is reused by a split.
    assert!(cache.begin_batch());
    let combined = cache
        .prepare_rgba(2, 0, state(), |out| out.fill(41))
        .unwrap();
    assert!(cache.pixels(first).is_none());
    assert!(cache.pixels(second).is_none());
    assert_eq!(cache.pixels(combined).unwrap(), &[41; 16]);
    cache.end_batch();

    assert!(cache.begin_batch());
    let reused_first = cache
        .prepare_rgba(0, 0, state(), |out| out.fill(53))
        .unwrap();
    let reused_second = cache
        .prepare_rgba(1, 0, state(), |out| out.fill(67))
        .unwrap();
    assert!(cache.pixels(combined).is_none());
    assert!(!cache.begin_batch());
    assert!(
        cache
            .prepare_rgba(2, 0, state(), |_| {
                rejected_fills.set(rejected_fills.get() + 1)
            })
            .is_none()
    );
    assert_eq!(cache.pixels(reused_first).unwrap(), &[53; 8]);
    assert_eq!(cache.pixels(reused_second).unwrap(), &[67; 8]);
    assert_eq!(rejected_fills.get(), 0);
    cache.end_batch();

    assert!(cache.begin_batch());
    let final_combined = cache
        .prepare_rgba(2, 0, state(), |out| out.fill(79))
        .unwrap();
    assert!(cache.pixels(reused_first).is_none());
    assert!(cache.pixels(reused_second).is_none());
    assert_eq!(cache.pixels(final_combined).unwrap(), &[79; 16]);
    assert_eq!(cache.stats().rejected, 4);
    assert_eq!(cache.stats().fills, 6);
    cache.end_batch();
}

#[test]
fn mixed_payload_eviction_coalesces_and_invalidates_every_owner() {
    let (texture, palette) = indexed_resources();
    let mut cache = SurfaceCache::load(
        vec![
            indexed_source([16; 2]),
            SurfaceSource::load_rgba([0; 2], [8, 8], 1).unwrap(),
            SurfaceSource::load_rgba([0; 2], [8, 16], 1).unwrap(),
        ],
        512,
    )
    .unwrap();
    let native = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let small = cache
        .prepare_rgba(1, 0, state(), |out| out.fill(29))
        .unwrap();
    let large = cache
        .prepare_rgba(2, 0, state(), |out| out.fill(57))
        .unwrap();
    assert!(cache.pixels(native).is_none());
    assert!(cache.pixels(small).is_none());
    assert_eq!(cache.pixels(large).unwrap(), &[57; 512]);
    assert_eq!(cache.stats().evictions, 2);
}

#[test]
fn failed_coalescing_probe_preserves_unpinned_native_spans_before_pinned_barrier() {
    let (texture, palette) = indexed_resources();
    let mut cache = SurfaceCache::load(
        vec![
            indexed_source([16; 2]),
            indexed_source([32, 16]),
            SurfaceSource::load_rgba([0; 2], [8, 4], 1).unwrap(),
            SurfaceSource::load_rgba([0; 2], [14, 8], 1).unwrap(),
        ],
        512,
    )
    .unwrap();
    let first = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let second = cache
        .prepare(1, 1, &texture, &palette, BuildState::default())
        .unwrap();
    let rgb = cache
        .prepare_rgba(2, 0, state(), |out| out.fill(63))
        .unwrap();
    assert!(cache.begin_batch());
    assert!(cache.prepare_rgba(2, 0, state(), |_| {}).is_some());
    let fills = Cell::new(0);
    assert!(
        cache
            .prepare_rgba(3, 0, state(), |_| fills.set(fills.get() + 1))
            .is_none()
    );
    assert_eq!(fills.get(), 0);
    assert_eq!(cache.indexed_pixels(first).unwrap(), &[37; 256]);
    assert_eq!(cache.indexed_pixels(second).unwrap(), &[37; 128]);
    assert_eq!(cache.pixels(rgb).unwrap(), &[63; 128]);
    assert_eq!(cache.stats().evictions, 0);
    cache.end_batch();
}

#[test]
fn invalid_layout_mip_and_capacity_requests_do_not_fill_or_evict_native_spans() {
    let (texture, palette) = indexed_resources();
    let mut cache = SurfaceCache::load(
        vec![
            indexed_source([16; 2]),
            SurfaceSource::load_rgba([0; 2], [64, 64], 2).unwrap(),
        ],
        256,
    )
    .unwrap();
    let native = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let fills = Cell::new(0);
    for (surface, mip) in [(0, 0), (1, 0), (1, 2), (2, 0), (1, 32)] {
        assert!(
            cache
                .prepare_rgba(surface, mip, state(), |_| fills.set(fills.get() + 1))
                .is_none()
        );
    }
    assert!(
        cache
            .prepare(1, 0, &texture, &palette, BuildState::default())
            .is_none()
    );
    assert_eq!(fills.get(), 0);
    assert_eq!(cache.indexed_pixels(native).unwrap(), &[37; 256]);
    assert_eq!(cache.stats().evictions, 0);
    assert_eq!(cache.stats().rejected, 6);
}

#[test]
fn pixel_borrows_reject_foreign_cache_ids_and_changed_layout_shape_or_mip() {
    let source = || SurfaceSource::load_rgba([-4, 6], [2, 1], 2).unwrap();
    let mut a = SurfaceCache::load(vec![source()], 32).unwrap();
    let mut b = SurfaceCache::load(vec![source()], 32).unwrap();
    let span_a = a.prepare_rgba(0, 0, state(), |out| out.fill(3)).unwrap();
    let span_b = b.prepare_rgba(0, 0, state(), |out| out.fill(5)).unwrap();
    assert!(a.pixels(span_b).is_none());
    assert!(b.pixels(span_a).is_none());
    let mut changed = span_a;
    changed.layout = CacheLayout::Indexed8;
    assert!(a.pixels(changed).is_none());
    changed = span_a;
    changed.width = u32::MAX;
    assert!(a.pixels(changed).is_none());
    changed = span_a;
    changed.mip = 1;
    assert!(a.pixels(changed).is_none());
    changed = span_a;
    changed.mip = 32;
    assert!(a.pixels(changed).is_none());
    changed = span_a;
    changed.texture_mins[0] += 1;
    assert!(a.pixels(changed).is_none());
    changed = span_a;
    changed.transparent_index = Some(0);
    changed.cutout = true;
    assert!(a.pixels(changed).is_none());
    assert_eq!(a.pixels(span_a).unwrap(), &[3; 8]);
    assert_eq!(b.pixels(span_b).unwrap(), &[5; 8]);
}

#[test]
fn malformed_rgba_load_shapes_and_mip_counts_are_scoped_errors() {
    for extents in [[0, 1], [1, 0], [8193, 1], [1, u32::MAX]] {
        assert!(SurfaceSource::load_rgba([0; 2], extents, 1).is_err());
    }
    for mip_count in [0, 33, u8::MAX] {
        assert!(SurfaceSource::load_rgba([0; 2], [1, 1], mip_count).is_err());
    }
    assert!(SurfaceSource::load_rgba([i32::MIN, i32::MAX], [8192, 1], 14).is_ok());
    assert!(SurfaceCache::load(vec![], 0).is_err());
}
