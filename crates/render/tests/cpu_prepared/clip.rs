use super::clip::ClipGraph;
use super::{Camera, ClipVertex, ScreenVertex, evaluated_vertex};
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
