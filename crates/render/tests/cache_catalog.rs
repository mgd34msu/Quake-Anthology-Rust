use qa_render::surface_cache::{
    BuildState, CacheLayout, IndexedTexture, LightGrid, PaletteLighting, RgbaBuildState,
    SurfaceCache, SurfaceCatalog, SurfaceSource,
};
use std::sync::Arc;

fn resources() -> (IndexedTexture, PaletteLighting) {
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![17; (16 >> mip) * (16 >> mip)]);
    let texture = IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), false).unwrap();
    let rgb: Vec<_> = (0..256u16).flat_map(|index| [index as u8; 3]).collect();
    let colormap: Vec<_> = (0..64u8).flat_map(|grade| [grade; 256]).collect();
    let palette = PaletteLighting::load(&rgb, &colormap, None, 256).unwrap();
    (texture, palette)
}

fn lit_source() -> SurfaceSource {
    SurfaceSource::load(
        [0; 2],
        [16; 2],
        Some(LightGrid::gray(2, 2, [0, 255, 255, 255], &[64; 4]).unwrap()),
        true,
    )
    .unwrap()
}

#[test]
fn native_sources_and_samples_are_shared_while_fills_and_stamps_are_private() {
    let source = lit_source();
    let samples = source.lightmap().unwrap().samples().as_ptr();
    let catalog = SurfaceCatalog::load(vec![source]).unwrap();
    let mut first = SurfaceCache::load_shared(Arc::clone(&catalog), 256).unwrap();
    let mut second = SurfaceCache::load_shared(Arc::clone(&catalog), 256).unwrap();
    assert!(Arc::ptr_eq(&first.catalog(), &second.catalog()));
    assert!(std::ptr::eq(
        first.surface(0).unwrap(),
        second.surface(0).unwrap()
    ));
    assert_eq!(
        first
            .surface(0)
            .unwrap()
            .lightmap()
            .unwrap()
            .samples()
            .as_ptr(),
        samples
    );
    assert_eq!(
        second
            .surface(0)
            .unwrap()
            .lightmap()
            .unwrap()
            .samples()
            .as_ptr(),
        samples
    );

    let (texture, palette) = resources();
    let original = first
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let other = second
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    // Native 64*256 light becomes colormap grade 47 after six-bit inversion.
    assert_eq!(first.indexed_pixels(original).unwrap(), &[47; 256]);
    assert_eq!(second.indexed_pixels(other).unwrap(), &[47; 256]);
    assert!(first.pixels(other).is_none());
    assert!(second.pixels(original).is_none());
    let hit = first
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert_eq!(first.indexed_pixels(hit).unwrap(), &[47; 256]);
    let half = second
        .prepare(
            0,
            0,
            &texture,
            &palette,
            BuildState {
                style_scales: [128, 256, 256, 256],
                ..BuildState::default()
            },
        )
        .unwrap();
    assert_eq!(second.indexed_pixels(half).unwrap(), &[55; 256]);
    assert!(second.indexed_pixels(other).is_none());
    assert_eq!(first.indexed_pixels(original).unwrap(), &[47; 256]);
    assert_eq!(first.stats().fills, 1);
    assert_eq!(first.stats().hits, 1);
    assert_eq!(second.stats().fills, 2);
    assert_eq!(second.stats().hits, 0);
}

#[test]
#[expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "Keep the independent packed-pixel or triangle oracle and incomplete-tail expectations unchanged"
)]
fn mixed_payloads_have_independent_pins_eviction_and_cross_cache_ownership() {
    let catalog = SurfaceCatalog::load(vec![
        lit_source(),
        SurfaceSource::load_rgba([0; 2], [2, 1], 3).unwrap(),
        SurfaceSource::load_rgba([0; 2], [1; 2], 1).unwrap(),
    ])
    .unwrap();
    let mut first = SurfaceCache::load_shared(Arc::clone(&catalog), 264).unwrap();
    let mut second = SurfaceCache::load_shared(catalog, 264).unwrap();
    let (texture, palette) = resources();
    let stamp = RgbaBuildState {
        material_id: 7,
        base_image_id: 11,
        lightmap_image_id: Some(13),
        ..RgbaBuildState::default()
    };
    assert!(first.begin_batch());
    let native = first
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let product = first
        .prepare_rgba(1, 0, stamp, |out| {
            let base = [[200u16, 100, 50], [128, 64, 32]];
            let light = [[128u16, 64, 256], [128, 256, 256]];
            for ((pixel, base), light) in out.chunks_exact_mut(4).zip(base).zip(light) {
                for channel in 0..3 {
                    pixel[channel] = (base[channel] * light[channel] / 256) as u8;
                }
                pixel[3] = 255;
            }
        })
        .unwrap();
    let golden = [[100, 25, 50, 255], [64, 64, 32, 255]];
    assert_eq!(first.rgba_pixels(product).unwrap(), &golden);
    assert!(first.prepare_rgba(2, 0, stamp, |out| out.fill(1)).is_none());
    assert_eq!(first.indexed_pixels(native).unwrap(), &[47; 256]);
    assert_eq!(first.rgba_pixels(product).unwrap(), &golden);

    let second_native = second
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let second_product = second
        .prepare_rgba(1, 0, stamp, |out| out.fill(19))
        .unwrap();
    let replacement = second
        .prepare_rgba(2, 0, stamp, |out| out.copy_from_slice(&[3, 5, 7, 255]))
        .unwrap();
    assert!(second.indexed_pixels(second_native).is_none());
    assert_eq!(second.rgba_pixels(second_product).unwrap(), &[[19; 4]; 2]);
    assert_eq!(second.rgba_pixels(replacement).unwrap(), &[[3, 5, 7, 255]]);
    assert!(second.rgba_pixels(product).is_none());
    assert!(first.rgba_pixels(replacement).is_none());
    assert_eq!(first.indexed_pixels(native).unwrap(), &[47; 256]);
    assert_eq!(first.rgba_pixels(product).unwrap(), &golden);
    assert_eq!(first.stats().rejected, 1);
    assert_eq!(first.stats().evictions, 0);
    assert_eq!(second.stats().rejected, 0);
    assert_eq!(second.stats().evictions, 1);
    first.end_batch();
    assert!(
        first
            .prepare_rgba(2, 0, stamp, |out| out.fill(23))
            .is_some()
    );
}

#[test]
fn catalog_metadata_and_lifetime_survive_independent_rover_drop() {
    let catalog = SurfaceCatalog::load(vec![
        SurfaceSource::load([0; 2], [16, 32], None, false).unwrap(),
        SurfaceSource::load_rgba_texels([-1; 2], [3; 2], 3).unwrap(),
        SurfaceSource::load_rgba([0; 2], [1, 11], 32).unwrap(),
    ])
    .unwrap();
    assert_eq!(catalog.surfaces().len(), 3);
    assert_eq!(catalog.max_reservation_bytes(CacheLayout::Indexed8), 512);
    assert_eq!(catalog.max_reservation_bytes(CacheLayout::Rgba8), 48);
    assert_eq!(catalog.surface(1).unwrap().reservation_bytes(0), Some(40));
    assert_eq!(
        catalog.surface(2).unwrap().mip_layout(31),
        Some(([0; 2], [1; 2]))
    );
    assert!(catalog.surface(3).is_none());
    let weak = Arc::downgrade(&catalog);
    let first = SurfaceCache::load_shared(Arc::clone(&catalog), 1024).unwrap();
    let mut second = SurfaceCache::load_shared(Arc::clone(&catalog), 1024).unwrap();
    let retained = first.catalog();
    assert!(Arc::ptr_eq(&catalog, &retained));
    drop(catalog);
    drop(first);
    let block = second
        .prepare_rgba(2, 31, RgbaBuildState::default(), |out| {
            out.copy_from_slice(&[17, 23, 31, 255]);
        })
        .unwrap();
    assert_eq!(second.rgba_pixels(block).unwrap(), &[[17, 23, 31, 255]]);
    drop(second);
    assert!(weak.upgrade().is_some());
    drop(retained);
    assert!(weak.upgrade().is_none());
}

#[test]
fn empty_catalog_and_invalid_arena_preserve_the_shared_owner() {
    let catalog = SurfaceCatalog::load(Vec::new()).unwrap();
    assert!(catalog.surfaces().is_empty());
    assert_eq!(catalog.max_reservation_bytes(CacheLayout::Indexed8), 0);
    assert_eq!(catalog.max_reservation_bytes(CacheLayout::Rgba8), 0);
    assert!(SurfaceCache::load_shared(Arc::clone(&catalog), 0).is_err());
    assert_eq!(Arc::strong_count(&catalog), 1);
    let cache = SurfaceCache::load_shared(Arc::clone(&catalog), 1).unwrap();
    assert!(Arc::ptr_eq(&catalog, &cache.catalog()));
    assert!(cache.surface(0).is_none());
    assert_eq!(cache.stats().fills, 0);
}

// The pre-catalog arithmetic is an independent oracle for stored mip layouts.
fn original_layout(
    minima: [i32; 2],
    extents: [u32; 2],
    mip: u8,
    texel_grid: bool,
) -> ([i32; 2], [u32; 2]) {
    if !texel_grid {
        return (minima, extents.map(|extent| (extent >> mip).max(1)));
    }
    let step = 1i64 << mip;
    let lower = minima.map(|value| i64::from(value).div_euclid(step));
    let upper: [i64; 2] = std::array::from_fn(|axis| {
        (i64::from(minima[axis]) + i64::from(extents[axis]) + step - 1).div_euclid(step)
    });
    (
        lower.map(|value| (value * step) as i32),
        std::array::from_fn(|axis| (upper[axis] - lower[axis]) as u32),
    )
}

#[test]
fn catalog_rgba_layouts_match_original_formulas_for_all_levels_and_origins() {
    for (minima, extents) in [
        ([-3, 1], [5, 3]),
        ([-1, 37], [1, 17]),
        ([i32::MIN + 1, i32::MAX], [13, 7]),
        ([i32::MAX, i32::MIN], [8191, 1]),
    ] {
        for texel_grid in [false, true] {
            let source = if texel_grid {
                SurfaceSource::load_rgba_texels(minima, extents, 32)
            } else {
                SurfaceSource::load_rgba(minima, extents, 32)
            }
            .unwrap();
            let expected: [_; 32] =
                std::array::from_fn(|mip| original_layout(minima, extents, mip as u8, texel_grid));
            let max_reservation = expected
                .iter()
                .map(|(_, [width, height])| {
                    (usize::try_from(*width).unwrap() * usize::try_from(*height).unwrap() * 4 + 7)
                        & !7
                })
                .max()
                .unwrap();
            let catalog = SurfaceCatalog::load(vec![source]).unwrap();
            assert_eq!(
                catalog.max_reservation_bytes(CacheLayout::Rgba8),
                max_reservation
            );
            assert_eq!(catalog.max_reservation_bytes(CacheLayout::Indexed8), 0);
            let mut cache = SurfaceCache::load_shared(catalog, max_reservation).unwrap();
            for (mip, (origin, dimensions)) in expected.into_iter().enumerate() {
                let [width, height] = dimensions;
                let bytes = width as usize * height as usize * 4;
                assert_eq!(
                    cache.surface(0).unwrap().mip_layout(mip as u8),
                    Some((origin, dimensions))
                );
                assert_eq!(
                    cache.surface(0).unwrap().reservation_bytes(mip as u8),
                    Some((bytes + 7) & !7)
                );
                let marker = mip as u8;
                let span = cache
                    .prepare_rgba(0, marker, RgbaBuildState::default(), |out| {
                        assert_eq!(out.len(), bytes);
                        out.fill(marker);
                    })
                    .unwrap();
                assert_eq!(span.texture_mins, origin);
                assert_eq!([span.width, span.height], dimensions);
                assert_eq!(cache.pixels(span).unwrap().len(), bytes);
                assert!(
                    cache
                        .pixels(span)
                        .unwrap()
                        .iter()
                        .all(|&byte| byte == marker)
                );
                let hit = cache
                    .prepare_rgba(0, marker, RgbaBuildState::default(), |_| {
                        panic!("warm RGBA cache hit must not refill");
                    })
                    .unwrap();
                assert_eq!(cache.rgba_pixels(hit), cache.rgba_pixels(span));
            }
            assert_eq!(cache.surface(0).unwrap().mip_layout(32), None);
            assert_eq!(cache.surface(0).unwrap().reservation_bytes(32), None);
            assert_eq!(cache.stats().fills, 32);
            assert_eq!(cache.stats().hits, 32);
        }
    }
}

#[test]
fn catalog_indexed_layouts_match_original_formulas_at_extreme_origins() {
    let (texture, palette) = resources();
    for (minima, extents) in [
        ([-3, 1], [16, 32]),
        ([i32::MIN, i32::MAX], [32, 16]),
        ([i32::MAX, i32::MIN + 1], [16, 16]),
    ] {
        let catalog = SurfaceCatalog::load(vec![
            SurfaceSource::load(minima, extents, None, false).unwrap(),
        ])
        .unwrap();
        assert_eq!(
            catalog.max_reservation_bytes(CacheLayout::Indexed8),
            extents[0] as usize * extents[1] as usize
        );
        assert_eq!(catalog.max_reservation_bytes(CacheLayout::Rgba8), 0);
        let mut cache = SurfaceCache::load_shared(catalog, 1024).unwrap();
        for mip in 0..4 {
            let (origin, dimensions) = original_layout(minima, extents, mip, false);
            let bytes = dimensions[0] as usize * dimensions[1] as usize;
            let span = cache
                .prepare(0, mip, &texture, &palette, BuildState::default())
                .unwrap();
            assert_eq!(span.texture_mins, origin);
            assert_eq!([span.width, span.height], dimensions);
            assert_eq!(cache.indexed_pixels(span).unwrap().len(), bytes);
            let hit = cache
                .prepare(0, mip, &texture, &palette, BuildState::default())
                .unwrap();
            assert_eq!(cache.indexed_pixels(hit), cache.indexed_pixels(span));
            let mut altered = span;
            altered.height += 1;
            assert!(cache.pixels(altered).is_none());
            altered = span;
            altered.mip = (mip + 1) % 4;
            assert!(cache.pixels(altered).is_none());
            altered = span;
            altered.texture_mins[0] ^= 1;
            assert!(cache.pixels(altered).is_none());
            altered = span;
            altered.cutout = true;
            assert!(cache.pixels(altered).is_none());
        }
        for mip in 4..32 {
            assert_eq!(cache.surface(0).unwrap().mip_layout(mip), None);
            assert_eq!(cache.surface(0).unwrap().reservation_bytes(mip), None);
            assert!(
                cache
                    .prepare(0, mip, &texture, &palette, BuildState::default())
                    .is_none()
            );
        }
        assert_eq!(cache.stats().fills, 4);
        assert_eq!(cache.stats().hits, 4);
        assert_eq!(cache.stats().rejected, 28);
    }
}
