use qa_compat::{memory::ModuleMemory, surfaces::NativeSurfaces, traces::write_q2};
use qa_core::primitives::{Bounds, Plane, SurfaceFlags, SurfaceId, Vec3};
use qa_world::collision::{
    CollisionStore, Contents, Trace,
    brushes::BrushTree,
    surfaces::{SurfaceInput, SurfaceTable},
};

#[test]
fn native_trace_layouts_preserve_fields_padding_and_null_surface_states() {
    check::<false>();
    check::<true>();
}

fn check<const WIDE: bool>() {
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            vec![],
            vec![],
            SurfaceTable {
                records: vec![SurfaceInput::unnamed(SurfaceFlags::from_q2(0x83)); 2],
                sides: vec![],
            },
            BrushTree::direct(0).unwrap(),
            vec![Bounds::default()],
        )
        .unwrap();
    let base = 0x2000_0000;
    let mut memory = ModuleMemory::load(base, 1024, &[0xa5; 1024]).unwrap();
    let surfaces = NativeSurfaces::load(base, &store, WIDE, &mut memory).unwrap();
    let dest = base + 256;
    let (size, frac, end, normal, surface_at, contents, ent_at) = if WIDE {
        (96, 4, 8, 20, 40, 48, 56)
    } else {
        (72, 8, 12, 24, 48, 56, 64)
    };
    let mut hit = Trace::clear(Vec3([-0.0, 1.0, -2.0]));
    hit.all_solid = true;
    hit.start_solid = true;
    hit.fraction = f32::from_bits(0x3eaaaaaa);
    hit.plane = Plane {
        encoding: Some([4, 0]),
        ..Plane::oriented(Vec3([0.3, -0.4, 0.5]), -7.0)
    };
    hit.surface_id = Some(SurfaceId { geometry, index: 1 });
    hit.contents = Contents::from_q2(u32::MAX);
    hit.secondary_plane = Some(Plane {
        encoding: Some([5, 0]),
        ..Plane::oriented(Vec3([0.0, 0.0, -1.0]), 3.0)
    });
    hit.secondary_surface_id = Some(SurfaceId { geometry, index: 0 });
    let entity = 0x1122_3344_5566_7788;
    assert_eq!(
        write_q2::<WIDE>(&mut memory, dest, hit, &surfaces, entity).unwrap(),
        dest
    );
    if WIDE {
        assert_eq!(memory.read(dest, 4).unwrap(), &[1, 1, 0, 0]);
    } else {
        assert_eq!(memory.read_word(dest).unwrap(), 1);
        assert_eq!(memory.read_word(dest + 4).unwrap(), 1);
    }
    assert_eq!(
        memory.read_word(dest + frac).unwrap() as u32,
        hit.fraction.to_bits()
    );
    assert_eq!(
        memory.read_vec3(dest + end).unwrap().0.map(f32::to_bits),
        hit.end.0.map(f32::to_bits)
    );
    assert_eq!(memory.read_vec3(dest + normal).unwrap(), hit.plane.normal);
    assert_eq!(
        memory.read_word(dest + normal + 12).unwrap() as u32,
        (-7.0f32).to_bits()
    );
    assert_eq!(memory.read(dest + normal + 16, 4).unwrap(), &[4, 0, 0, 0]);
    let word = |memory: &ModuleMemory<'_>, at| {
        u64::from_le_bytes(memory.read(at, 8).unwrap().try_into().unwrap())
    };
    assert_eq!(
        word(&memory, dest + surface_at),
        surfaces.address(hit.surface_id).unwrap()
    );
    assert_eq!(memory.read_word(dest + contents).unwrap() as u32, u32::MAX);
    assert_eq!(word(&memory, dest + ent_at), entity);
    if WIDE {
        assert_eq!(memory.read(dest + 52, 4).unwrap(), &[0; 4]);
        assert_eq!(
            memory.read_vec3(dest + 64).unwrap(),
            hit.secondary_plane.unwrap().normal
        );
        assert_eq!(memory.read(dest + 80, 4).unwrap(), &[5, 0, 0, 0]);
        assert_eq!(memory.read(dest + 84, 4).unwrap(), &[0; 4]);
        assert_eq!(
            word(&memory, dest + 88),
            surfaces.address(hit.secondary_surface_id).unwrap()
        );
    }
    assert_eq!(memory.read(dest - 1, 1).unwrap(), &[0xa5]);
    assert_eq!(memory.read(dest + size, 1).unwrap(), &[0xa5]);
    // The classic ABI has no secondary contact fields. An unsupported
    // secondary surface cannot make an otherwise valid classic result fail.
    hit.secondary_surface_id = Some(SurfaceId { geometry, index: 2 });
    if WIDE {
        let preserved = memory.read(dest, size as usize).unwrap().to_vec();
        assert!(write_q2::<WIDE>(&mut memory, dest, hit, &surfaces, entity).is_err());
        assert_eq!(memory.read(dest, size as usize).unwrap(), preserved);
    } else {
        write_q2::<WIDE>(&mut memory, dest, hit, &surfaces, entity).unwrap();
    }
    // An unobstructed or stationary trace keeps nullptr; a real temporary
    // box contact uses the distinct load-built CM nullsurface address.
    let clear = Trace::clear(Vec3([2.0, 3.0, 4.0]));
    write_q2::<WIDE>(&mut memory, dest, clear, &surfaces, entity).unwrap();
    assert_eq!(word(&memory, dest + surface_at), 0);
    assert_eq!(memory.read(dest + normal, 20).unwrap(), &[0; 20]);
    if WIDE {
        assert_eq!(memory.read(dest + 64, 32).unwrap(), &[0; 32]);
    }
    hit.surface_id = None;
    hit.secondary_plane = None;
    write_q2::<WIDE>(&mut memory, dest, hit, &surfaces, entity).unwrap();
    assert_eq!(word(&memory, dest + surface_at), base);
    let preserved = memory.read(dest, size as usize).unwrap().to_vec();
    hit.surface_id = Some(SurfaceId { geometry, index: 2 });
    assert!(write_q2::<WIDE>(&mut memory, dest, hit, &surfaces, entity).is_err());
    assert_eq!(memory.read(dest, size as usize).unwrap(), preserved);
    assert!(
        write_q2::<WIDE>(
            &mut memory,
            base + 1024 - size + 1,
            clear,
            &surfaces,
            entity
        )
        .is_err()
    );
}
