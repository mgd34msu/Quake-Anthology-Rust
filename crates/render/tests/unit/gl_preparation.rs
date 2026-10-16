use super::MeshLayout;
use crate::assets::Vertex;
use std::mem::size_of;

#[test]
fn appended_meshes_keep_native_model_order_and_old_static_offsets() {
    let old = MeshLayout::prepare([(4, 6), (8, 9)]).unwrap();
    let extended = MeshLayout::prepare([(4, 6), (8, 9), (12, 18)]).unwrap();
    for (old, new) in old.meshes.iter().zip(extended.meshes.iter()) {
        assert_eq!(
            (old.first_index, old.count, old.base_vertex),
            (new.first_index, new.count, new.base_vertex),
        );
    }
    let appended = &extended.meshes[2];
    assert_eq!(
        (appended.first_index, appended.count, appended.base_vertex),
        (15, 18, 12)
    );
    assert_eq!(extended.vertex_bytes, (24 * size_of::<Vertex>()) as isize);
    assert_eq!(extended.index_bytes, (33 * size_of::<u32>()) as isize);
}

#[test]
fn mesh_layout_rejects_gl_scalar_overflow_before_flattening_or_upload() {
    assert!(MeshLayout::prepare([(0, i32::MAX as usize + 1)]).is_err());
    assert!(MeshLayout::prepare([(i32::MAX as usize + 1, 0), (0, 0)]).is_err());
    assert!(MeshLayout::prepare([(usize::MAX, 0)]).is_err());
    assert!(MeshLayout::prepare([(0, 0)]).is_ok());
}
