use super::*;
use crate::scene::SceneModel;
use crate::stage::EntityLighting;
use crate::world::{WorldId, WorldPresentationId};

#[test]
fn mesh_and_world_instances_share_the_same_column_major_pose_matrix() {
    let entity = SceneEntity {
        origin: Vec3([17.0, -9.0, 3.0]),
        axes: [
            Vec3([0.0, 1.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
        ],
        ..SceneEntity::default()
    };
    let expected: [f32; 16] = [
        0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 17.0, -9.0, 3.0, 1.0,
    ];
    for model in [
        entity.model,
        SceneModel::World {
            world: WorldId(3),
            presentation: WorldPresentationId(7),
            surfaces: Span { first: 7, count: 2 },
        },
    ] {
        let entity = SceneEntity { model, ..entity };
        let matrix = model_matrix(&entity);
        assert_eq!(matrix.map(f32::to_bits), expected.map(f32::to_bits));
        let projection = [
            2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 5.0, -6.0, 7.0, 1.0,
        ];
        let combined = multiply(&projection, &matrix);
        let point = [2.0, 5.0, -7.0, 1.0];
        let clip: [f32; 4] = std::array::from_fn(|row| {
            combined[row] * point[0]
                + combined[row + 4] * point[1]
                + combined[row + 8] * point[2]
                + combined[row + 12] * point[3]
        });
        // The rotated/translated point is (12,-7,-4); this synthetic affine
        // projection then maps it to (29,-27,-9). No GL context is involved.
        assert_eq!(
            clip.map(f32::to_bits),
            [29.0_f32, -27.0, -9.0, 1.0].map(f32::to_bits)
        );
    }
}

#[test]
fn entity_inputs_preserve_face_bindings_and_independent_view_data() {
    let lighting = EntityLighting {
        ambient: [11.0, 22.0, 33.0],
        directed: [44.0, 55.0, 66.0],
        direction: Vec3([0.0, 0.0, -1.0]),
    };
    let entity = SceneEntity {
        origin: Vec3([1.0, 2.0, 3.0]),
        axes: [
            Vec3([0.0, 1.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
        ],
        color: [7, 19, 31, 127],
        shader_texcoord: [0.25, -0.5],
        shader_time: 2.75,
        lighting: Some(lighting),
        ..SceneEntity::default()
    };
    let view = DrawInputs {
        time_ms: 1739,
        view_origin: Vec3([3.0, 5.0, 7.0]),
        identity_light: 0.5,
        lightmap: crate::assets::ImageId(9),
        texture_scale: [1.0 / 64.0, 1.0 / 128.0],
        specular_origin: Vec3([-960.0, 1980.0, 96.0]),
        ..DrawInputs::default()
    };
    let inputs = view.for_entity(&entity);
    assert_eq!(
        inputs.view_origin.0.map(f32::to_bits),
        [3.0_f32, -2.0, 4.0].map(f32::to_bits)
    );
    assert_eq!(inputs.entity_color, entity.color);
    assert_eq!(inputs.entity_texcoord, entity.shader_texcoord);
    assert_eq!(
        inputs.entity_shader_time.to_bits(),
        entity.shader_time.to_bits()
    );
    assert_eq!(inputs.lighting, Some(lighting));
    assert_eq!(inputs.time_ms, view.time_ms);
    assert_eq!(
        inputs.identity_light.to_bits(),
        view.identity_light.to_bits()
    );
    assert_eq!(inputs.lightmap, view.lightmap);
    assert_eq!(inputs.texture_scale, view.texture_scale);
    assert_eq!(inputs.specular_origin, view.specular_origin);
    let world_entity = SceneEntity {
        model: SceneModel::World {
            world: WorldId(3),
            presentation: WorldPresentationId(7),
            surfaces: Span { first: 7, count: 2 },
        },
        ..entity
    };
    assert_eq!(view.for_entity(&world_entity), inputs);
}
