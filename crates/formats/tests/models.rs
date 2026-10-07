use qa_core::primitives::Vec3;
use qa_formats::{
    FormatError,
    model::{Image, Model, ModelFormat},
};

const MDL: &[u8] = include_bytes!("fixtures/models/group.mdl");
const MD2: &[u8] = include_bytes!("fixtures/models/mesh.md2");
const MD3: &[u8] = include_bytes!("fixtures/models/mesh.md3");
const MDC: &[u8] = include_bytes!("fixtures/models/mesh.mdc");
const MD5: &[u8] = include_bytes!("fixtures/models/mesh.md5mesh");
const SPR: &[u8] = include_bytes!("fixtures/models/group.spr");
const SP2: &[u8] = include_bytes!("fixtures/models/sprite.sp2");

#[test]
fn alias_groups_and_texture_indices_keep_native_animation_data() {
    let m = Model::parse(MDL).unwrap();
    assert_eq!(m.format, ModelFormat::Mdl);
    assert_eq!(m.frames.len(), 2);
    assert_eq!(m.skin_groups[0].members, 0..2);
    assert_eq!(
        &m.intervals[m.skin_groups[0].intervals.clone()],
        &[0.25, 0.5]
    );
    assert_eq!(
        &m.intervals[m.frame_groups[0].intervals.clone()],
        &[0.5, 1.0]
    );
    assert_eq!(m.frames[0].name, b"first");
    assert_eq!(m.frames[1].name, b"second");
    let mesh = &m.meshes[0];
    assert_eq!(mesh.vertices.len(), 6);
    assert_eq!(mesh.vertices[3].position, Vec3([3.0, 2.0, 3.0]));
    assert_eq!(mesh.vertices[3].normal, Vec3([0.0, 0.0, 1.0]));
    let t = mesh.triangles[0];
    assert_eq!(t.vertex, [0, 1, 2]);
    assert_eq!(mesh.texcoords[t.texcoord[0] as usize], [0.75, 0.5]);
    assert_eq!(mesh.texcoords[t.texcoord[1] as usize], [0.75, 0.5]);
    let Image::Indexed {
        pixels,
        width,
        height,
    } = m.skins[0]
    else {
        panic!()
    };
    assert_eq!((width, height), (2, 1));
    assert_eq!(pixels, &[1, 2]);
    assert!(std::ptr::eq(pixels.as_ptr(), MDL[100..].as_ptr()));
    let mut b = MDL.to_vec();
    b[72..76].copy_from_slice(&i32::MIN.to_le_bytes());
    assert_eq!(Model::parse(&b).unwrap().sync, i32::MIN);
}

#[test]
fn md2_has_separate_uv_indices_packed_frames_and_gl_commands() {
    let m = Model::parse(MD2).unwrap();
    assert_eq!(m.format, ModelFormat::Md2);
    assert_eq!(m.frames[0].bounds.mins, Vec3([10.0, 20.0, 30.0]));
    assert_eq!(m.frames[0].bounds.maxs, Vec3([11.0, 21.0, 30.0]));
    assert_eq!(m.frames[0].packed_vertices.len(), 12);
    assert_eq!(m.gl_commands.len(), 44);
    let mut b = MD2.to_vec();
    // Reverse the UV table indices without changing geometry indices.
    b[150..156].copy_from_slice(&[2, 0, 1, 0, 0, 0]);
    let m = Model::parse(&b).unwrap();
    assert_eq!(m.meshes[0].triangles[0].vertex, [0, 1, 2]);
    assert_eq!(m.meshes[0].triangles[0].texcoord, [2, 1, 0]);
    b[248..252].copy_from_slice(&3i32.to_le_bytes());
    assert!(Model::parse(&b).is_err());
}

#[test]
fn md3_and_mdc_produce_the_same_frame_major_mesh_and_tag_shapes() {
    for bytes in [MD3, MDC] {
        let m = Model::parse(bytes).unwrap();
        assert_eq!(m.frames.len(), 2);
        assert_eq!(m.tags_per_frame, 1);
        assert_eq!(m.tags.len(), 2);
        assert_eq!(m.tags[0].name, b"tag_weapon");
        assert_eq!(m.meshes[0].vertices_per_frame, 3);
        assert_eq!(m.meshes[0].vertices.len(), 6);
        assert_eq!(m.meshes[0].triangles[0].vertex, [0, 1, 2]);
        assert_eq!(m.meshes[0].shaders[0].name, b"textures/body");
    }
    let m = Model::parse(MDC).unwrap();
    assert_eq!(m.tags[1].origin, Vec3([2.0, 0.0, 0.0]));
    assert!((m.tags[1].axes[0].0[1] - 1.0).abs() < 1e-6);
    assert_eq!(
        m.meshes[0].vertices[3].position,
        Vec3([0.0, -127.0 * 0.05, 128.0 * 0.05])
    );
    assert_eq!(m.meshes[0].vertices[3].normal, Vec3([1.0, 0.0, 0.0]));
    assert_eq!(
        m.meshes[0].vertices[4].position,
        Vec3([1.0 + 0.05, 0.0, 0.0])
    );
    let mut b = MDC.to_vec();
    b[580..582].copy_from_slice(&2i16.to_le_bytes());
    assert!(Model::parse(&b).is_err());
    b[580..582].copy_from_slice(&(-1i16).to_le_bytes());
    b[576..578].copy_from_slice(&(-1i16).to_le_bytes());
    assert!(Model::parse(&b).is_err());
}

#[test]
fn md5_indexed_records_weights_and_bind_pose_are_ready_for_shared_upload() {
    let m = Model::parse(MD5).unwrap();
    assert_eq!(m.format, ModelFormat::Md5Mesh);
    assert_eq!(m.bones[0].name, b"origin");
    assert_eq!(m.bones[0].parent, None);
    let mesh = &m.meshes[0];
    assert_eq!(mesh.weights.len(), 3);
    assert_eq!(mesh.vertex_weights.len(), 3);
    assert_eq!(mesh.vertices[0].position, Vec3([1.0, 2.0, 3.0]));
    assert_eq!(mesh.vertices[1].position, Vec3([2.0, 2.0, 3.0]));
    assert_eq!(mesh.vertices[2].position, Vec3([1.0, 3.0, 3.0]));
    assert_eq!(mesh.vertices[0].normal, Vec3([0.0, 0.0, -1.0]));
    assert_eq!(mesh.texcoords[2], [0.0, 1.0]);
    assert_eq!(mesh.bind_normals.len(), 3);
    let text = std::str::from_utf8(MD5).unwrap();
    assert!(Model::parse(text.replace("vert 2", "vert 0").as_bytes()).is_err());
    assert!(Model::parse(text.replace("\"origin\" -1", "\"origin\" 0").as_bytes()).is_err());
    assert!(Model::parse(text.replace("weight 2 0 1", "weight 2 0 1.5").as_bytes()).is_err());
    assert!(Model::parse(text.replace("2 1\nvert 0", "999999 1\nvert 0").as_bytes()).is_err());
    let commented = format!(" /* lead */\n // line\n{text} // tail\n");
    assert!(Model::parse(commented.as_bytes()).is_ok());
}

#[test]
fn sprites_share_group_ranges_and_borrow_pixels_or_image_names() {
    let m = Model::parse(SPR).unwrap();
    assert_eq!(m.sprites.len(), 2);
    assert_eq!(m.frame_groups[0].members, 0..2);
    assert_eq!(
        &m.intervals[m.frame_groups[0].intervals.clone()],
        &[0.25, 0.5]
    );
    assert_eq!(m.sprites[0].origin, [-1, 1]);
    let Image::Indexed { pixels, .. } = m.sprites[1].image else {
        panic!()
    };
    assert_eq!(pixels, &[1, 2, 3, 5]);
    let m = Model::parse(SP2).unwrap();
    let Image::External(name) = m.sprites[0].image else {
        panic!()
    };
    assert_eq!(name, b"sprites/test.pcx");
    assert_eq!((m.sprites[0].width, m.sprites[0].height), (4, 2));
    assert_eq!(m.sprites[0].origin, [1, 1]);
}

#[test]
fn tag_only_md3_accepts_native_empty_bounds_but_active_meshes_require_bounds() {
    let mut b = MD3[..444].to_vec();
    b[84..88].copy_from_slice(&0i32.to_le_bytes());
    b[104..108].copy_from_slice(&444i32.to_le_bytes());
    for i in 0..2 {
        for a in 0..3 {
            b[108 + i * 56 + a * 4..112 + i * 56 + a * 4]
                .copy_from_slice(&99999.0f32.to_le_bytes());
            b[120 + i * 56 + a * 4..124 + i * 56 + a * 4]
                .copy_from_slice(&(-99999.0f32).to_le_bytes());
        }
    }
    let m = Model::parse(&b).unwrap();
    assert!(m.meshes.is_empty());
    assert_eq!(m.tags.len(), 2);
    assert_eq!(m.bounds, qa_core::primitives::Bounds::default());
    let mut b = MD3.to_vec();
    b[108..112].copy_from_slice(&99999.0f32.to_le_bytes());
    assert!(Model::parse(&b).is_err());
}

#[test]
fn all_truncations_and_seeded_mutations_stay_inside_the_format_boundary() {
    let fixtures = [MDL, MD2, MD3, MDC, MD5, SPR, SP2];
    for fixture in fixtures {
        // MD5's trailing whitespace is valid; these fixtures end in a token.
        for end in 0..fixture.len() {
            assert!(Model::parse(&fixture[..end]).is_err());
        }
    }
    let mut seed = 0x4d4f444cu32;
    for i in 0..10000 {
        let mut b = fixtures[i % fixtures.len()].to_vec();
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let at = seed as usize % b.len();
        b[at] ^= (seed >> 24) as u8;
        let _scoped_result = Model::parse(&b);
    }
    let mut b = MD2.to_vec();
    b[24..28].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(matches!(
        Model::parse(&b),
        Err(FormatError::InvalidRange) | Err(FormatError::InvalidRecordSize)
    ));
    b = MD3.to_vec();
    b[108..112].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(matches!(Model::parse(&b), Err(FormatError::InvalidValue)));
}
