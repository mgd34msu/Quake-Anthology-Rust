use qa_compat::{memory::ModuleMemory, surfaces::NativeSurfaces};
use qa_core::primitives::{Bounds, SurfaceFlags, SurfaceId};
use qa_world::collision::{
    CollisionStore,
    brushes::BrushTree,
    surfaces::{SurfaceInput, SurfaceTable},
};

#[test]
fn native_surfaces_reserve_exact_load_rows_and_keep_both_q2_layouts() {
    for wide in [false, true] {
        let mut store = CollisionStore::new();
        let geometry = store
            .load_brushes(
                vec![],
                vec![],
                SurfaceTable {
                    records: vec![
                        SurfaceInput {
                            name: b"ABCDEFGHIJKLMNOPabcdefghijklmnop",
                            material: b"steel",
                            flags: SurfaceFlags::from_q2(0x80000083),
                            value: -127,
                            source_index: 97,
                        },
                        SurfaceInput {
                            name: b"same",
                            material: b"",
                            flags: SurfaceFlags::from_q3(4 | 0x82),
                            value: 19,
                            source_index: 3,
                        },
                    ],
                    sides: vec![],
                },
                BrushTree::direct(0).unwrap(),
                vec![Bounds::default()],
            )
            .unwrap();
        let stride = if wide { 60 } else { 24 };
        assert_eq!(NativeSurfaces::byte_length(&store, wide), Some(stride * 3));
        let base = 0x2000_0000;
        let mut memory = ModuleMemory::load(base, stride * 3, &vec![255; stride * 3]).unwrap();
        let native = NativeSurfaces::load(base, &store, wide, &mut memory).unwrap();
        assert_eq!(native.address(None).unwrap(), base);
        assert_eq!(memory.read(base, stride).unwrap(), vec![0; stride]);
        let first = native
            .address(Some(SurfaceId { geometry, index: 0 }))
            .unwrap();
        assert_eq!(first, base + stride as u64);
        assert_eq!(
            memory.cstring(first).unwrap(),
            if wide {
                &b"ABCDEFGHIJKLMNOPabcdefghijklmno"[..]
            } else {
                &b"ABCDEFGHIJKLMNO"[..]
            }
        );
        let name = if wide { 32 } else { 16 };
        assert_eq!(memory.read_word(first + name).unwrap() as u32, 0x80000083);
        assert_eq!(memory.read_word(first + name + 4).unwrap(), -127);
        if wide {
            assert_eq!(memory.read_word(first + 40).unwrap(), 97);
            assert_eq!(memory.cstring(first + 44).unwrap(), b"steel");
        }
        let second = native
            .address(Some(SurfaceId { geometry, index: 1 }))
            .unwrap();
        assert_eq!(second, first + stride as u64);
        assert_eq!(memory.cstring(second).unwrap(), b"same");
        assert_eq!(memory.read_word(second + name).unwrap() as u32, 4 | 0x82);
        assert_eq!(memory.read_word(second + name + 4).unwrap(), 19);
        assert!(
            native
                .address(Some(SurfaceId { geometry, index: 2 }))
                .is_err()
        );
        assert!(
            native
                .address(Some(SurfaceId {
                    geometry: qa_core::primitives::GeometryId {
                        generation: geometry.generation + 1,
                        ..geometry
                    },
                    index: 0
                }))
                .is_err()
        );
        // A too-small view is rejected at this external memory boundary.
        let mut short = ModuleMemory::load(base, stride * 3 - 1, &[]).unwrap();
        assert!(NativeSurfaces::load(base, &store, wide, &mut short).is_err());
    }
}
