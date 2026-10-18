use super::*;
use qa_render::material::world_load::{
    WorldPresentationRequest, append_world_presentation, reload_world_presentation,
};

fn image(color: [u8; 3]) -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&1u16.to_le_bytes());
    bytes[16] = 24;
    bytes.extend_from_slice(&[color[2], color[1], color[0]]);
    bytes
}

#[test]
fn world_sources_keep_scripts_images_reloads_and_cached_presentations_isolated()
-> Result<(), String> {
    for family in [1, 2, 3] {
        let low_root = resources(&format!("source-low-{family}"))?;
        let high_root = resources(&format!("source-high-{family}"))?;
        let name = if family == 3 {
            "textures/modern"
        } else {
            "textures/wall"
        };
        for (root, texture, first, second) in [
            (&low_root, "first", [211, 0, 0], [0, 211, 0]),
            (&high_root, "second", [0, 211, 0], [0, 0, 211]),
        ] {
            std::fs::write(
                root.join("scripts/test.shader"),
                format!("{name} {{\n cull none\n {{ map textures/{texture}.tga }}\n }}"),
            )
            .map_err(|e| e.to_string())?;
            std::fs::write(root.join("textures/first.tga"), image(first))
                .map_err(|e| e.to_string())?;
            std::fs::write(root.join("textures/second.tga"), image(second))
                .map_err(|e| e.to_string())?;
        }
        let mut vfs = Vfs::default();
        let low_mount = vfs.mount_directory(&low_root, 0).unwrap();
        let high_mount = vfs.mount_directory(&high_root, 10).unwrap();
        let low = vfs.search_view([low_mount]).unwrap();
        let high = vfs.search_view([high_mount]).unwrap();
        let equivalent_low = vfs.search_view([low_mount, low_mount]).unwrap();
        let bytes = map_bytes(family);
        let mut map = if family == 3 {
            modern_map(&bytes)?
        } else {
            legacy_map(&bytes, family)?
        };
        if family != 3 {
            map.surface_edges.reverse();
        }
        let mut assets = Assets::load().unwrap();
        let source_palette = PaletteSource::native(map.bsp.format.rule_set())
            .map(|source| load_palette(vfs.read_view(None), &mut assets, source))
            .transpose()
            .map_err(|e| format!("{e:?}"))?;
        let profile = WorldPresentationProfile::native(RuleSetId::Quake3, None);
        let request = |search| WorldPresentationRequest {
            profile,
            native_default: None,
            search,
        };
        let loaded = load_world(
            &vfs,
            &map,
            &mut assets,
            source_palette,
            &[
                request(Some(&low)),
                request(Some(&high)),
                request(Some(&equivalent_low)),
                request(None),
            ],
            Default::default(),
            false,
        )
        .map_err(|e| format!("family {family}: {e:?}"))?;
        let [low_row, high_row, same_low, default_row] = *loaded.presentations else {
            panic!("four requests")
        };
        assert_eq!(low_row, same_low);
        assert_ne!(low_row.id, high_row.id);
        assert_ne!(high_row.id, default_row.id);
        assert_eq!(assets.world(loaded.world).unwrap().presentations().len(), 3);
        let mesh = assets.world(loaded.world).unwrap().mesh();
        let check = |assets: &Assets, row: LoadedWorldPresentation, color: [u8; 3]| {
            let texture = assets.image(surface_image(assets, row.id)).unwrap();
            assert_eq!(&texture.rgba[..3], &color);
            let pixels = draw_registered_profiles(assets, loaded.world, &[row]).unwrap();
            assert!(
                pixels.iter().any(|p| {
                    let rgba = p.to_le_bytes();
                    if color[0] != 0 {
                        rgba[0] > 100 && rgba[1] == 0 && rgba[2] == 0
                    } else {
                        rgba[2] > 100 && rgba[0] == 0 && rgba[1] == 0
                    }
                }),
                "visible selected world, family {family}"
            );
            pixels
        };
        let red = check(&assets, low_row, [211, 0, 0]);
        let blue = check(&assets, high_row, [0, 0, 211]);
        assert_eq!(blue, check(&assets, default_row, [0, 0, 211]));
        let counts = assets.counts();
        vfs.take_lookup_count();
        for (view, expected) in [
            (Some(&equivalent_low), low_row),
            (Some(&high), high_row),
            (None, default_row),
        ] {
            let cached =
                append_world_presentation(&vfs, &mut assets, loaded.world, request(view), false)
                    .map_err(|e| format!("{e:?}"))?;
            assert_eq!(cached.presentations[0], expected);
        }
        assert_eq!(assets.counts(), counts);
        assert_eq!(vfs.take_lookup_count(), 0);
        let foreign = Vfs::default();
        let foreign_view = foreign.default_view();
        assert!(
            append_world_presentation(
                &vfs,
                &mut assets,
                loaded.world,
                request(Some(&foreign_view)),
                false
            )
            .is_err()
        );
        assert!(
            append_world_presentation(&foreign, &mut assets, loaded.world, request(None), false)
                .is_err()
        );
        assert_eq!(assets.counts(), counts);
        assert!(
            assets
                .register_world_presentation_metadata(loaded.world, &[low_row], Some(&high))
                .is_err()
        );
        let rejected = assets.begin_world_image_reload().unwrap();
        rejected
            .replay(&vfs, &mut assets)
            .map_err(|e| format!("{e:?}"))?;
        let refreshed = reload_world_presentation(
            &vfs,
            &mut assets,
            loaded.world,
            low_row.id,
            request(Some(&equivalent_low)),
            false,
        )
        .map_err(|e| format!("{e:?}"))?
        .presentations[0];
        assert_eq!(check(&assets, refreshed, [211, 0, 0]), red);
        rejected.restore(&mut assets).unwrap();
        assets.discard_registration_suffix(counts).unwrap();
        assert_eq!(assets.counts(), counts);
        assert_eq!(check(&assets, low_row, [211, 0, 0]), red);
        assert_eq!(check(&assets, high_row, [0, 0, 211]), blue);
        let accepted = assets.begin_world_image_reload().unwrap();
        accepted
            .replay(&vfs, &mut assets)
            .map_err(|e| format!("{e:?}"))?;
        for (view, old, pixels, color) in [
            (&low, low_row, &red, [211, 0, 0]),
            (&high, high_row, &blue, [0, 0, 211]),
        ] {
            let next = reload_world_presentation(
                &vfs,
                &mut assets,
                loaded.world,
                old.id,
                request(Some(view)),
                false,
            )
            .map_err(|e| format!("{e:?}"))?
            .presentations[0];
            assert_eq!(&check(&assets, next, color), pixels);
            assert!(
                assets
                    .world_presentation(next.id)
                    .unwrap()
                    .search()
                    .unwrap()
                    .same_selection(view)
            );
        }
        accepted.commit(&mut assets);
        assert_eq!(assets.world(loaded.world).unwrap().mesh(), mesh);
        std::fs::remove_dir_all(low_root).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(high_root).map_err(|e| e.to_string())?;
    }
    Ok(())
}
