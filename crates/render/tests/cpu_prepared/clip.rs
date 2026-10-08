use super::clip::ClipGraph;
use super::{evaluated_vertex, Camera, ClipVertex, ScreenVertex};
use crate::assets::Vertex;
use crate::assets::{MaterialSettings, Stage, TcMod};
use crate::edges::ProjectedVertex;
use crate::scene::{Refdef, Viewport};
use crate::shader::{AlphaGen, Deform, RgbGen, TexMod, WaveFunction, Waveform};
use crate::stage::DrawInputs;
use crate::stage::StageEvaluator;
use qa_core::primitives::Vec3;

fn clip_reference(
    camera: &Camera,
    input: &[ClipVertex],
    output: &mut [ClipVertex],
    plane: usize,
) -> Option<usize> {
    let mut count = 0;
    let mut previous = input[input.len() - 1];
    let mut previous_distance = camera.distance(previous, plane);
    for &current in input {
        let distance = camera.distance(current, plane);
        if !distance.is_finite() || !previous_distance.is_finite() {
            return None;
        }
        let inside = distance >= 0.0;
        let previous_inside = previous_distance >= 0.0;
        if inside != previous_inside {
            let denominator = previous_distance - distance;
            if !denominator.is_finite() || count == output.len() {
                return None;
            }
            let vertex = previous.lerp(current, previous_distance / denominator);
            if !vertex.finite() {
                return None;
            }
            output[count] = vertex;
            count += 1;
        }
        if inside {
            if count == output.len() {
                return None;
            }
            output[count] = current;
            count += 1;
        }
        previous = current;
        previous_distance = distance;
    }
    Some(count)
}

#[test]
fn shared_intersections_preserve_shader_before_clip_attribute_bits() {
    let evaluator = StageEvaluator::load();
    let camera = Camera::load(
        Refdef {
            viewport: Viewport {
                width: 8,
                height: 8,
                ..Viewport::default()
            },
            near: 0.25,
            far: 64.0,
            fov: [90.0; 2],
            ..Refdef::default()
        },
        8,
        8,
    )
    .unwrap();
    let wave = Waveform {
        function: WaveFunction::Sin,
        base: 0.0,
        amplitude: 0.1875,
        phase: 0.125,
        frequency: 0.5,
    };
    for deformation in [
        Deform::Move {
            vector: [0.0, 0.125, 0.25],
            wave,
        },
        Deform::Wave {
            spread: 0.125,
            wave,
        },
        Deform::Bulge {
            width: 3.25,
            height: 0.375,
            speed: 0.5,
        },
    ] {
        let settings = MaterialSettings {
            deforms: [Some(deformation), None, None],
            ..MaterialSettings::default()
        };
        let inputs = DrawInputs {
            time_ms: 731,
            ..DrawInputs::default()
        };
        let deforms = evaluator.prepare_deforms(&settings, &inputs).unwrap();
        let stages = [
            Stage {
                rgb_gen: RgbGen::Vertex,
                ..Stage::default()
            },
            Stage {
                rgb_gen: RgbGen::Const([0.2, 0.4, 0.7]),
                alpha_gen: AlphaGen::Vertex,
                tcmods: [
                    Some(TcMod::Script(TexMod::Turbulent {
                        base: 0.0,
                        amplitude: 0.5,
                        phase: 0.125,
                        frequency: 0.25,
                    })),
                    None,
                    None,
                    None,
                ],
                ..Stage::default()
            },
        ]
        .map(|stage| evaluator.prepare(&stage, settings, inputs).unwrap());
        let sources = [
            [0.05, 2.0, 2.0],
            [2.0, -2.0, 2.0],
            [2.0, -2.0, -2.0],
            [0.05, 2.0, -2.0],
        ]
        .into_iter()
        .enumerate()
        .map(|(index, position)| Vertex {
            position: Vec3(position),
            normal: Vec3([0.0, 0.0, 1.0]),
            texcoord: [index as f32 * 0.375 - 0.125, index as f32 * 0.125],
            color: [
                31 + index as u8 * 53,
                219 - index as u8 * 37,
                173,
                255 - index as u8 * 41,
            ],
            ..Vertex::default()
        })
        .collect::<Vec<_>>();
        let mut graph = ClipGraph::load(10).unwrap();
        graph
            .sources(sources.len())
            .unwrap()
            .copy_from_slice(&sources);
        let count = graph
            .build(&camera, sources.len(), deforms, Some(stages[0]), &evaluator)
            .unwrap();
        assert!(count >= 3);
        let mut screen = [ScreenVertex::default(); 10];
        let mut coverage = [ProjectedVertex::default(); 10];
        graph
            .project_base(&camera, &mut screen, &mut coverage)
            .unwrap();
        for (stage_index, stage) in stages.into_iter().enumerate() {
            if stage_index != 0 {
                graph.project_stage(stage, &evaluator, &mut screen).unwrap();
            }
            let mut input: Vec<_> = sources
                .iter()
                .map(|&source| {
                    let vertex = evaluated_vertex(source, deforms, Some(stage), &evaluator);
                    camera.vertex(vertex, vertex.position)
                })
                .collect();
            let mut output = [ClipVertex::default(); 10];
            for plane in 0..6 {
                let clipped = clip_reference(&camera, &input, &mut output, plane).unwrap();
                input.clear();
                input.extend_from_slice(&output[..clipped]);
                if clipped < 3 {
                    break;
                }
            }
            assert_eq!(count, input.len());
            for (index, vertex) in input.into_iter().enumerate() {
                let expected = camera.project(vertex);
                let actual = screen[index];
                assert_eq!(actual.xy.map(f32::to_bits), expected.xy.map(f32::to_bits));
                assert_eq!(
                    actual.inverse_depth.to_bits(),
                    expected.inverse_depth.to_bits()
                );
                assert_eq!(
                    actual.texcoord_over_depth.map(f32::to_bits),
                    expected.texcoord_over_depth.map(f32::to_bits)
                );
                assert_eq!(
                    actual.lightmap_over_depth.map(f32::to_bits),
                    expected.lightmap_over_depth.map(f32::to_bits)
                );
                assert_eq!(
                    actual.color_over_depth.map(f32::to_bits),
                    expected.color_over_depth.map(f32::to_bits)
                );
            }
        }
    }
}

// Frozen pre-classification path: every plane copies retained vertices through
// clip_reference, including fully accepted planes. Source validation precedes
// all clipping, and later-plane checks see only the surviving polygon.
fn old_clip(
    camera: &Camera,
    sources: &[Vertex],
    deforms: [crate::stage::DeformOp; 3],
    stage: Option<crate::stage::PreparedStage>,
    evaluator: &StageEvaluator,
    capacity: usize,
) -> Option<Vec<ClipVertex>> {
    if sources.len() > capacity {
        return None;
    }
    let mut input = Vec::with_capacity(capacity);
    for &source in sources {
        let vertex = evaluated_vertex(source, deforms, stage, evaluator);
        let vertex = camera.vertex(vertex, vertex.position);
        if !vertex.finite() {
            return None;
        }
        input.push(vertex);
    }
    let mut output = vec![ClipVertex::default(); capacity];
    for plane in 0..6 {
        if input.len() < 3 {
            input.clear();
            break;
        }
        let count = clip_reference(camera, &input, &mut output, plane)?;
        input.clear();
        input.extend_from_slice(&output[..count]);
    }
    if input.len() < 3 {
        input.clear();
    }
    Some(input)
}

fn old_project(camera: &Camera, input: &[ClipVertex]) -> Option<Vec<ScreenVertex>> {
    input
        .iter()
        .map(|&vertex| {
            let screen = camera.project(vertex);
            screen.finite().then_some(screen)
        })
        .collect()
}

fn assert_screen_bits(actual: ScreenVertex, expected: ScreenVertex) {
    assert_eq!(actual.xy.map(f32::to_bits), expected.xy.map(f32::to_bits));
    assert_eq!(
        actual.inverse_depth.to_bits(),
        expected.inverse_depth.to_bits()
    );
    assert_eq!(
        actual.texcoord_over_depth.map(f32::to_bits),
        expected.texcoord_over_depth.map(f32::to_bits),
    );
    assert_eq!(
        actual.lightmap_over_depth.map(f32::to_bits),
        expected.lightmap_over_depth.map(f32::to_bits),
    );
    assert_eq!(
        actual.color_over_depth.map(f32::to_bits),
        expected.color_over_depth.map(f32::to_bits),
    );
}

fn compare_old_graph(
    graph: &mut ClipGraph,
    camera: &Camera,
    sources: &[Vertex],
    base: Option<crate::stage::PreparedStage>,
    later_stages: &[crate::stage::PreparedStage],
    evaluator: &StageEvaluator,
    capacity: usize,
) {
    let deforms = [crate::stage::DeformOp::None; 3];
    let expected = old_clip(camera, sources, deforms, base, evaluator, capacity);
    graph
        .sources(sources.len())
        .unwrap()
        .copy_from_slice(sources);
    let count = graph.build(camera, sources.len(), deforms, base, evaluator);
    assert_eq!(count, expected.as_ref().map(Vec::len));
    let Some(expected) = expected else {
        return;
    };
    let expected = old_project(camera, &expected);
    let mut screen = vec![ScreenVertex::default(); capacity];
    let mut coverage = vec![ProjectedVertex::default(); capacity];
    let projected = graph.project_base(camera, &mut screen, &mut coverage);
    assert_eq!(projected, expected.as_ref().map(Vec::len));
    let Some(expected) = expected else {
        return;
    };
    for (index, &expected) in expected.iter().enumerate() {
        assert_screen_bits(screen[index], expected);
        assert_eq!(
            coverage[index].xy.map(f32::to_bits),
            expected.xy.map(|value| (value - 0.5).to_bits()),
        );
        assert_eq!(
            coverage[index].inverse_depth.to_bits(),
            expected.inverse_depth.to_bits()
        );
        assert_eq!(
            coverage[index].texcoord_over_depth.map(f32::to_bits),
            expected.texcoord_over_depth.map(f32::to_bits),
        );
    }
    for &stage in later_stages {
        let expected = old_clip(camera, sources, deforms, Some(stage), evaluator, capacity)
            .and_then(|vertices| old_project(camera, &vertices));
        let actual = graph.project_stage(stage, evaluator, &mut screen);
        assert_eq!(actual, expected.as_ref().map(Vec::len));
        if let Some(expected) = expected {
            for (index, expected) in expected.into_iter().enumerate() {
                assert_screen_bits(screen[index], expected);
            }
        }
    }
}

fn clip_camera() -> Camera {
    Camera::load(
        Refdef {
            viewport: Viewport {
                x: 3,
                y: 2,
                width: 16,
                height: 12,
            },
            near: 0.25,
            far: 64.0,
            fov: [90.0; 2],
            ..Refdef::default()
        },
        24,
        18,
    )
    .unwrap()
}

fn camera_sources(points: &[[f32; 3]]) -> Vec<Vertex> {
    points
        .iter()
        .enumerate()
        .map(|(index, &[x, y, z])| Vertex {
            position: Vec3([z, -x, y]),
            normal: Vec3([0.0, 0.0, 1.0]),
            texcoord: [index as f32 * 0.375 - 0.25, index as f32 * 0.125],
            lightmap_coord: [index as f32 * 0.0625, 0.875 - index as f32 * 0.125],
            color: [
                31 + index as u8 * 53,
                219 - index as u8 * 37,
                173,
                255 - index as u8 * 41,
            ],
        })
        .collect()
}

#[test]
fn active_plane_classification_matches_old_inside_and_each_crossing_plane() {
    let camera = clip_camera();
    let evaluator = StageEvaluator::load();
    let stages = [
        Stage {
            rgb_gen: RgbGen::ExactVertex,
            ..Stage::default()
        },
        Stage {
            rgb_gen: RgbGen::Const([0.2, 0.4, 0.7]),
            alpha_gen: AlphaGen::Vertex,
            tcmods: [
                Some(TcMod::Script(TexMod::Transform {
                    matrix: [[1.5, -0.25], [0.5, 0.75]],
                    translate: [-0.125, 0.25],
                })),
                None,
                None,
                None,
            ],
            ..Stage::default()
        },
    ]
    .map(|stage| {
        evaluator
            .prepare(&stage, MaterialSettings::default(), DrawInputs::default())
            .unwrap()
    });
    let mut graph = ClipGraph::load(16).unwrap();
    for points in [
        [[-0.5, -0.5, 4.0], [0.5, -0.5, 4.0], [0.0, 0.5, 4.0]],
        [[0.0, 0.0, 0.125], [-0.5, -0.5, 1.0], [0.5, 0.5, 1.0]],
        [[0.0, 0.0, 128.0], [-0.5, -0.5, 32.0], [0.5, 0.5, 32.0]],
        [[-5.0, 0.0, 4.0], [0.0, -0.5, 4.0], [0.0, 0.5, 4.0]],
        [[5.0, 0.0, 4.0], [0.0, -0.5, 4.0], [0.0, 0.5, 4.0]],
        [[0.0, -5.0, 4.0], [-0.5, 0.0, 4.0], [0.5, 0.0, 4.0]],
        [[0.0, 5.0, 4.0], [-0.5, 0.0, 4.0], [0.5, 0.0, 4.0]],
    ] {
        let sources = camera_sources(&points);
        compare_old_graph(
            &mut graph,
            &camera,
            &sources,
            Some(stages[0]),
            &stages[1..],
            &evaluator,
            16,
        );
    }
}

#[test]
fn active_planes_ignore_later_overflow_after_earlier_removal() {
    let mut camera = clip_camera();
    camera.refdef.fov = [178.0; 2];
    camera = Camera::load(camera.refdef, 24, 18).unwrap();
    let evaluator = StageEvaluator::load();
    let sources = camera_sources(&[
        [0.0, 0.0, f32::MAX],
        [1.0, 0.0, f32::MAX],
        [0.0, 1.0, f32::MAX],
    ]);
    let clip = camera.vertex(sources[0], sources[0].position);
    assert!(clip.finite());
    assert!(!camera.distance(clip, 2).is_finite());
    let mut graph = ClipGraph::load(16).unwrap();
    compare_old_graph(&mut graph, &camera, &sources, None, &[], &evaluator, 16);
    assert_eq!(
        graph.build(
            &camera,
            sources.len(),
            [crate::stage::DeformOp::None; 3],
            None,
            &evaluator
        ),
        Some(0)
    );
}

#[test]
fn active_planes_keep_finite_polygon_after_removing_one_later_overflow_source() {
    let mut refdef = clip_camera().refdef;
    refdef.fov = [178.0; 2];
    let camera = Camera::load(refdef, 24, 18).unwrap();
    let evaluator = StageEvaluator::load();
    let sources = camera_sources(&[
        [1.0, 1.0, f32::MAX],
        [1.0, -1.0, 4.0],
        [-1.0, -1.0, 4.0],
        [-1.0, 1.0, 4.0],
    ]);
    let removed = camera.vertex(sources[0], sources[0].position);
    assert!(removed.finite());
    assert!(camera.distance(removed, 1) < 0.0);
    assert!(!camera.distance(removed, 2).is_finite());
    for &source in &sources[1..] {
        let vertex = camera.vertex(source, source.position);
        assert!(vertex.finite());
        assert!((0..6).all(|plane| camera.distance(vertex, plane).is_finite()));
    }
    let survivors = old_clip(
        &camera,
        &sources,
        [crate::stage::DeformOp::None; 3],
        None,
        &evaluator,
        16,
    )
    .unwrap();
    assert!(survivors.len() >= 3);
    assert!(old_project(&camera, &survivors).is_some());
    let mut graph = ClipGraph::load(16).unwrap();
    compare_old_graph(&mut graph, &camera, &sources, None, &[], &evaluator, 16);
}

#[test]
fn active_planes_preserve_signed_zero_subnormal_and_boundary_attributes() {
    let mut refdef = clip_camera().refdef;
    refdef.near = f32::MIN_POSITIVE * 4.0;
    refdef.far = 1.0;
    let camera = Camera::load(refdef, 24, 18).unwrap();
    let edge = refdef.near * camera.tangent[0];
    let outside = f32::from_bits(edge.to_bits() + 1);
    let subnormal = f32::from_bits(1);
    let evaluator = StageEvaluator::load();
    let mut graph = ClipGraph::load(16).unwrap();
    for x in [edge, outside] {
        let mut sources = camera_sources(&[
            [x, 0.0, refdef.near],
            [-edge, subnormal, refdef.near],
            [0.0, -subnormal, refdef.near],
        ]);
        sources[0].texcoord = [-0.0, subnormal];
        sources[1].texcoord = [0.0, -subnormal];
        compare_old_graph(&mut graph, &camera, &sources, None, &[], &evaluator, 16);
    }
}

#[test]
fn active_planes_reject_capacity_and_interpolation_overflow_like_old_clip() {
    let camera = clip_camera();
    let evaluator = StageEvaluator::load();
    let mut small = ClipGraph::load(3).unwrap();
    let crossing = camera_sources(&[[0.0, 0.0, 0.125], [-0.5, -0.5, 1.0], [0.5, 0.5, 1.0]]);
    compare_old_graph(&mut small, &camera, &crossing, None, &[], &evaluator, 3);
    let inside = camera_sources(&[[-0.5, -0.5, 4.0], [0.5, -0.5, 4.0], [0.0, 0.5, 4.0]]);
    compare_old_graph(&mut small, &camera, &inside, None, &[], &evaluator, 3);
    let mut graph = ClipGraph::load(16).unwrap();
    let denominator = camera_sources(&[
        [0.0, 0.0, -f32::MAX],
        [0.0, 0.0, f32::MAX],
        [1.0, 0.0, f32::MAX],
    ]);
    compare_old_graph(&mut graph, &camera, &denominator, None, &[], &evaluator, 16);
    let mut attributes = crossing;
    attributes[0].texcoord = [f32::MAX; 2];
    attributes[1].texcoord = [-f32::MAX; 2];
    compare_old_graph(&mut graph, &camera, &attributes, None, &[], &evaluator, 16);
    let mut invalid = inside;
    invalid[0].lightmap_coord = [f32::INFINITY, 0.0];
    compare_old_graph(&mut graph, &camera, &invalid, None, &[], &evaluator, 16);
}

#[test]
fn active_plane_scratch_resets_across_motion_and_polygon_sizes() {
    let evaluator = StageEvaluator::load();
    let mut graph = ClipGraph::load(16).unwrap();
    let cases = [
        camera_sources(&[[-0.5, -0.5, 4.0], [0.5, -0.5, 4.0], [0.0, 0.5, 4.0]]),
        camera_sources(&[
            [-10.0, -10.0, 2.0],
            [10.0, -10.0, 2.0],
            [10.0, 10.0, 2.0],
            [-10.0, 10.0, 2.0],
        ]),
        camera_sources(&[[0.0, 0.0, 128.0], [1.0, 0.0, 128.0], [0.0, 1.0, 128.0]]),
    ];
    for frame in 0..12 {
        let mut refdef = clip_camera().refdef;
        refdef.origin = Vec3([frame as f32 * 0.125, frame as f32 * -0.0625, 0.03125]);
        let camera = Camera::load(refdef, 24, 18).unwrap();
        compare_old_graph(
            &mut graph,
            &camera,
            &cases[frame % cases.len()],
            None,
            &[],
            &evaluator,
            16,
        );
    }
}
