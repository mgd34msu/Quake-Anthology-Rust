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
