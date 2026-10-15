use qa_core::primitives::{CommandUnits, UserCmd, Vec3};

const SPEED_320: CommandUnits = CommandUnits::Speed {
    normalized_reference: [320.0; 3],
};
const SPEED_200: CommandUnits = CommandUnits::Speed {
    normalized_reference: [200.0; 3],
};
const NORMALIZED: CommandUnits = CommandUnits::Normalized {
    full_scale: [127.0; 3],
};

#[test]
fn speed_commands_preserve_world_units_and_raw_bits() {
    let axes = [200.25, -0.0, f32::from_bits(0x7fc0_1234)];
    assert_eq!(
        SPEED_320.project(axes, SPEED_200).map(f32::to_bits),
        axes.map(f32::to_bits)
    );
    assert_eq!(CommandUnits::default(), SPEED_320);
}

#[test]
fn native_normalized_axes_bypass_conversion_individually() {
    let destination = CommandUnits::Normalized {
        full_scale: [127.0, 255.0, 127.0],
    };
    let axes = [-128.0, 64.0, -0.0];
    assert_eq!(
        NORMALIZED.project(axes, destination).map(f32::to_bits),
        [-128.0f32, 128.0, -0.0].map(f32::to_bits)
    );
    assert_eq!(
        NORMALIZED.project(axes, NORMALIZED).map(f32::to_bits),
        axes.map(f32::to_bits)
    );
}

#[test]
fn normalized_bridges_use_fixed_references_and_destination_truncation() {
    assert_eq!(
        SPEED_320.project([200.0, -200.0, 640.0], NORMALIZED),
        [79.0, -79.0, 127.0]
    );
    assert_eq!(
        SPEED_200.project([200.0, 100.0, -400.0], NORMALIZED),
        [127.0, 63.0, -127.0]
    );
    assert_eq!(
        NORMALIZED.project([127.0, -127.0, 0.0], SPEED_320),
        [320.0, -320.0, 0.0]
    );
    assert_eq!(
        NORMALIZED.project([127.0, -127.0, 0.0], SPEED_200),
        [200.0, -200.0, 0.0]
    );
    let source = CommandUnits::Speed {
        normalized_reference: [320.0, 200.0, 100.0],
    };
    assert_eq!(
        source.project([160.0, 100.0, 25.0], NORMALIZED),
        [63.0, 63.0, 31.0]
    );
}

#[test]
fn bridge_truncation_happens_before_f32_rounding() {
    let below_full_speed = f32::from_bits(320.0f32.to_bits() - 1);
    assert_eq!(
        SPEED_320.project([below_full_speed, -below_full_speed, 0.0], NORMALIZED),
        [126.0, -126.0, 0.0]
    );
    let axis = 63.0f32;
    assert_eq!(
        NORMALIZED.project([axis; 3], SPEED_320)[0].to_bits(),
        ((f64::from(axis) * 320.0 / 127.0) as f32).to_bits()
    );
}

#[test]
fn command_conversion_changes_only_movement_and_its_units() {
    let command = UserCmd {
        movement: [200.0, 0.0, -0.0],
        units: SPEED_200,
        duration_ms: 255,
        duration_ns: 255_000_001,
        server_time_ms: 17,
        view_angles: Vec3([12.25, -90.0, -0.0]),
        buttons: 3,
        impulse: 7,
        light_level: 19,
        ..Default::default()
    };
    let converted = command.in_units(NORMALIZED);
    assert_eq!(converted.movement, [127.0, 0.0, -0.0]);
    assert_eq!(converted.units, NORMALIZED);
    assert_eq!(converted.duration_ms, command.duration_ms);
    assert_eq!(converted.duration_ns, command.duration_ns);
    assert_eq!(converted.server_time_ms, command.server_time_ms);
    assert_eq!(
        converted.view_angles.0.map(f32::to_bits),
        command.view_angles.0.map(f32::to_bits)
    );
    assert_eq!(converted.buttons, command.buttons);
    assert_eq!(converted.impulse, command.impulse);
    assert_eq!(converted.light_level, command.light_level);
    assert_eq!(command.movement, [200.0, 0.0, -0.0]);
    assert_eq!(command.units, SPEED_200);
}
