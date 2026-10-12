use super::*;

#[test]
#[expect(
    clippy::approx_constant,
    reason = "Independent original software table oracle retains qsrc's literal."
)]
fn tiled_table_and_phase_preserve_native_integer_lookup() {
    let evaluator = StageEvaluator::load();
    let table = evaluator.tiled_table();
    for (index, &actual) in table.iter().enumerate() {
        let original =
            (8.0 * 65536.0 + (index as f64 * 3.14159 * 2.0 / 128.0).sin() * (8.0 * 65536.0)) as i32;
        assert_eq!(actual, original);
    }
    // The phase pointer plus coordinate reaches the second cycle; the
    // approximate native pi makes this different from wrapping the sum.
    assert_ne!(table[128], table[0]);
    for time_f32 in [false, true] {
        for time_ms in [0, 49, 50, 6350, 6400, 917300, 16777299, 838860850] {
            let inputs = DrawInputs {
                time_ms,
                ..DrawInputs::default()
            };
            let settings = MaterialSettings {
                software_tiled: Some(SoftwareTiled {
                    turbulent: true,
                    flow_speed: None,
                    time_f32,
                }),
                ..MaterialSettings::default()
            };
            let sampling = evaluator
                .prepare(&Stage::default(), settings, inputs)
                .unwrap()
                .tiled
                .unwrap();
            let phase = if time_f32 {
                (((time_ms as f64 * 0.001) as f32 * 20.0) as i32 & 127) as usize
            } else {
                ((time_ms as f64 / 1000.0 * 20.0) as i32 & 127) as usize
            };
            assert_eq!(sampling.phase, phase);
            if time_f32 && time_ms == 16777299 {
                assert_eq!(sampling.phase, 57);
            }
            for coordinate in [
                [0, 0],
                [127 << 16, 127 << 16],
                [-1, 65 << 16],
                [511 << 16, -257 << 16],
            ] {
                let s = ((coordinate[0]
                    + i64::from(table[phase + ((coordinate[1] >> 16) & 127) as usize]))
                    >> 16)
                    & 63;
                let t = ((coordinate[1]
                    + i64::from(table[phase + ((coordinate[0] >> 16) & 127) as usize]))
                    >> 16)
                    & 63;
                assert_eq!(sampling.texel(coordinate, table), (t * 64 + s) as usize);
            }
        }
    }
}

#[test]
fn flow_keeps_the_promoted_double_fraction_until_fixed_adjustment() {
    let evaluator = StageEvaluator::load();
    for speed in [0.25, 0.77] {
        for time_ms in [0, 1, 49, 50, 999, 1000, 3999, 4000, 4001, 917300, 16777299] {
            let inputs = DrawInputs {
                time_ms,
                ..DrawInputs::default()
            };
            let settings = MaterialSettings {
                software_tiled: Some(SoftwareTiled {
                    turbulent: false,
                    flow_speed: Some(speed),
                    time_f32: true,
                }),
                ..MaterialSettings::default()
            };
            let tiled = evaluator
                .prepare(&Stage::default(), settings, inputs)
                .unwrap()
                .tiled
                .unwrap();
            let cycle = f64::from((time_ms as f64 * 0.001) as f32) * speed;
            let expected = 65536.0 * (-128.0 * (cycle - f64::from(cycle as i32)));
            assert_eq!(tiled.flow_adjust.to_bits(), expected.to_bits());
            if speed == 0.77 && time_ms == 16777299 {
                let early_narrowed_cycle = f64::from(time_ms as f32 * 0.001) * speed;
                let wrong = 65536.0
                    * (-128.0 * (early_narrowed_cycle - f64::from(early_narrowed_cycle as i32)));
                assert_ne!(tiled.flow_adjust.to_bits(), wrong.to_bits());
            }
            assert_eq!(
                tiled.texel([17 << 16, 33 << 16], evaluator.tiled_table()),
                33 * 64 + 17
            );
        }
    }
    let tiled = evaluator
        .prepare(
            &Stage::default(),
            MaterialSettings {
                software_tiled: Some(SoftwareTiled {
                    turbulent: false,
                    flow_speed: Some(0.77),
                    time_f32: true,
                }),
                ..MaterialSettings::default()
            },
            DrawInputs {
                time_ms: 1,
                ..DrawInputs::default()
            },
        )
        .unwrap()
        .tiled
        .unwrap();
    let origin = 8192_i64 << 16;
    assert_ne!(
        (origin as f64 + tiled.flow_adjust) as i64,
        origin + tiled.flow_adjust as i64
    );
}
