#[path = "support/world_surfaces.rs"]
mod world_surfaces;

use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_core::primitives::Vec3;
use qa_render::{
    Assets, ShaderRegistration,
    assets::ImageId,
    material::{
        resources::{ImageSettings, Images},
        world_load::{NamedShaderLighting, compile_named_shader},
    },
    shader::{ShaderSource, context::ShaderContext, parse_sources},
    world::geometry::LightSource,
};

struct Registrations {
    assets: Assets,
    names: usize,
    page: ShaderRegistration,
    vertex: ShaderRegistration,
    folded: ShaderRegistration,
    other: ShaderRegistration,
    default_page: ShaderRegistration,
    default_vertex: ShaderRegistration,
}
fn registrations() -> Registrations {
    let catalog = parse_sources(&[ShaderSource {
        name: "context.shader",
        bytes: br#"
            b { q3map_sun 1 0 0 20 0 0
                { map $whiteimage }
            }
            c { q3map_sun 0 1 0 30 90 0
                { map $whiteimage }
            }
            failed { q3map_sun 0 0 1 40 180 0
                     bogus
            }
        "#,
    }]);
    let mut assets = Assets::load().unwrap();
    let vfs = Vfs::default();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let mut compile = |name: &[u8], source| {
        compile_named_shader(
            &catalog,
            name,
            true,
            NamedShaderLighting {
                source,
                lightmap: ImageId(0),
            },
            &mut images,
        )
        .unwrap()
    };
    let page = compile(b"b", LightSource::Page(0));
    let vertex = compile(b"b", LightSource::Vertex);
    let folded = compile(b"B.tga", LightSource::Vertex);
    let other = compile(b"c", LightSource::None);
    let default_page = compile(b"failed", LightSource::Page(0));
    let default_vertex = compile(b"failed", LightSource::Vertex);
    drop(compile);
    drop(images);
    Registrations {
        names: assets.name_capacity(),
        assets,
        page,
        vertex,
        folded,
        other,
        default_page,
        default_vertex,
    }
}

#[test]
fn native_finished_lighting_keeps_distinct_logical_instances_of_one_binding() {
    let rows = registrations();
    assert_eq!(rows.page.requested, LightSource::Page(0));
    assert_eq!(rows.page.finished, LightSource::None);
    assert_eq!(rows.vertex.finished, LightSource::Vertex);
    let mut context = ShaderContext::load(rows.names, 8).unwrap();
    let mut append = context.append();
    let first = append.admit(rows.page).unwrap();
    let vertex = append.admit(rows.vertex).unwrap();
    let repeated = append.admit(rows.page).unwrap();
    append.commit();
    assert_ne!(first, vertex);
    assert_ne!(first, repeated);
    assert_ne!(vertex, repeated);
    assert_eq!(context.len(), 3);
    assert_eq!(
        context.node(first).unwrap().shader,
        context.node(repeated).unwrap().shader
    );
    assert_eq!(
        context.find(rows.page.key, LightSource::None),
        Some(repeated)
    );
    assert_eq!(
        context.find(rows.page.key, LightSource::Vertex),
        Some(vertex)
    );
}

#[test]
fn folded_cached_hit_reuses_node_without_replaying_its_sun() {
    let rows = registrations();
    assert_eq!(rows.vertex.key, rows.folded.key);
    let mut context = ShaderContext::load(rows.names, 8).unwrap();
    let mut append = context.append();
    let first = append.admit(rows.vertex).unwrap();
    append.admit(rows.other).unwrap();
    append.commit();
    let direction = context.sun_direction();
    let light = context.sun_light();
    assert_eq!(direction, rows.other.sun.unwrap().direction());
    assert_eq!(light, Vec3([0., 30., 0.]));
    let mut append = context.append();
    assert_eq!(append.admit(rows.folded).unwrap(), first);
    append.commit();
    assert_eq!(context.len(), 2);
    assert_eq!(context.sun_direction(), direction);
    assert_eq!(context.sun_light(), light);
}

#[test]
fn named_default_matches_other_lighting_without_reapplying_its_prefix_sun() {
    let rows = registrations();
    assert!(rows.default_page.defaulted.is_some());
    assert!(rows.default_vertex.defaulted.is_some());
    let mut context = ShaderContext::load(rows.names, 8).unwrap();
    let mut append = context.append();
    let failed = append.admit(rows.default_page).unwrap();
    append.admit(rows.other).unwrap();
    append.commit();
    let direction = context.sun_direction();
    let light = context.sun_light();
    let mut append = context.append();
    assert_eq!(append.admit(rows.default_vertex).unwrap(), failed);
    append.commit();
    assert_eq!(context.len(), 2);
    assert_eq!(context.node(failed).unwrap(), &rows.default_page);
    assert_eq!(context.sun_direction(), direction);
    assert_eq!(context.sun_light(), light);
}

#[test]
fn abandoned_append_restores_heads_nodes_and_both_sun_values() {
    let rows = registrations();
    let mut context = ShaderContext::load(rows.names, 8).unwrap();
    let mut append = context.append();
    let retained = append.admit(rows.vertex).unwrap();
    append.commit();
    let direction = context.sun_direction();
    let light = context.sun_light();
    {
        let mut append = context.append();
        append.admit(rows.other).unwrap();
        append.admit(rows.page).unwrap();
        append.admit(rows.page).unwrap();
        append.reset_sun_direction();
    }
    assert_eq!(context.len(), 1);
    assert_eq!(
        context.find(rows.vertex.key, LightSource::Vertex),
        Some(retained)
    );
    assert!(context.find(rows.page.key, LightSource::None).is_none());
    assert!(context.find(rows.other.key, LightSource::None).is_none());
    assert_eq!(context.sun_direction(), direction);
    assert_eq!(context.sun_light(), light);
    let mut retry = context.append();
    let recovered = retry.admit(rows.page).unwrap();
    retry.commit();
    assert_ne!(retained, recovered);
    assert_eq!(context.len(), 2);
}

#[test]
fn capacity_or_name_failure_cannot_publish_an_earlier_partial_append() {
    let rows = registrations();
    let mut capacity = ShaderContext::load(rows.names, 1).unwrap();
    {
        let mut append = capacity.append();
        append.admit(rows.page).unwrap();
        assert!(append.admit(rows.vertex).is_err());
    }
    assert!(capacity.is_empty());
    assert!(capacity.find(rows.page.key, LightSource::None).is_none());
    assert_eq!(
        capacity.sun_direction(),
        qa_render::lighting::default_sun_direction()
    );
    assert_eq!(capacity.sun_light(), Vec3::default());

    let mut names = ShaderContext::load(0, 2).unwrap();
    {
        let mut append = names.append();
        assert!(append.admit(rows.page).is_err());
    }
    assert!(names.is_empty());
    assert_eq!(names.sun_light(), Vec3::default());
}

#[test]
fn world_direction_reset_preserves_sun_light_and_full_reset_clears_the_context() {
    let rows = registrations();
    let mut context = ShaderContext::load(rows.names, 8).unwrap();
    let mut append = context.append();
    append.admit(rows.other).unwrap();
    append.commit();
    let light = context.sun_light();
    let mut world_load = context.append();
    world_load.reset_sun_direction();
    world_load.commit();
    assert_eq!(
        context.sun_direction(),
        qa_render::lighting::default_sun_direction()
    );
    assert_eq!(context.sun_light(), light);
    assert_eq!(context.len(), 1);
    context.reset();
    assert!(context.is_empty());
    assert!(context.find(rows.other.key, LightSource::None).is_none());
    assert_eq!(
        context.sun_direction(),
        qa_render::lighting::default_sun_direction()
    );
    assert_eq!(context.sun_light(), Vec3::default());
}

#[test]
fn world_presentations_keep_immutable_initial_suns_using_the_same_creation_rules() {
    use qa_render::world::{PreparedShader, SurfaceMaterial, WorldPresentationInput};
    let mut rows = registrations();
    let a = *rows.assets.shader_binding(rows.vertex.shader).unwrap();
    let b = *rows.assets.shader_binding(rows.other.shader).unwrap();
    let first_registrations = [
        rows.vertex.map_shader(|_| PreparedShader::Surface(0)),
        rows.other.map_shader(|_| PreparedShader::Binding {
            shader: b,
            surface: None,
        }),
        rows.vertex.map_shader(|_| PreparedShader::Surface(0)),
    ];
    let second_registrations = [rows.vertex.map_shader(|_| PreparedShader::Surface(0))];
    let surface = [SurfaceMaterial {
        shader: a,
        texture_scale: [1.; 2],
    }];
    let (geometry, visibility) = world_surfaces::inputs(1);
    let world = rows
        .assets
        .register_world(
            geometry,
            visibility,
            &[
                WorldPresentationInput {
                    surfaces: &surface,
                    registrations: &first_registrations,
                    native_page0: None,
                    native_default: None,
                },
                WorldPresentationInput {
                    surfaces: &surface,
                    registrations: &second_registrations,
                    native_page0: None,
                    native_default: None,
                },
            ],
        )
        .unwrap();
    let presentations = rows.assets.world(world).unwrap().presentations().to_vec();
    let first = rows.assets.world_presentation(presentations[0]).unwrap();
    assert_eq!(
        first.sun_direction(),
        rows.other.sun.unwrap().direction(),
        "cached A after B must not replay A's sun"
    );
    assert_eq!(first.sun_light(), Vec3([0., 30., 0.]));
    let second = rows.assets.world_presentation(presentations[1]).unwrap();
    assert_eq!(second.sun_direction(), rows.vertex.sun.unwrap().direction());
    assert_eq!(second.sun_light(), Vec3([20., 0., 0.]));
}
