use qa_core::primitives::{
    MovementMode, MovementTimer, NumericValue, PlayerState, RuleSetId, ValueBinding, ValueId,
    ValueReset, ValueWidth, Vec3,
};
use qa_network::{
    commands::{QwCmd, packet::Protocol},
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerProjection},
    states,
};

fn context() -> PlayerContext {
    PlayerContext {
        client_number: Some(17),
        ground_number: Some(1022),
        weapon_number: Some(5),
        weapon_model: Some(8),
        gravity: 800.,
        speed: 300.,
        player_info_flags: 28,
        command_age_ms: 999,
        body_yaw: 75.,
    }
}
fn player() -> PlayerState {
    let mut p = PlayerState::with_capacity(2, 2, 4);
    p.movement_rules = RuleSetId::Quake2;
    p.trace_rules = RuleSetId::QuakeWorld;
    p.body.position = Vec3([12.3, -47.17, -0.]);
    p.body.velocity = Vec3([16.12, -32.7, 1.01]);
    p.view_angles = Vec3([15., 300., -30.]);
    p.movement.delta_angles = Vec3([90., 180., -90.]);
    p.view_offset = Vec3([0., 0., 22.]);
    p.health = -12;
    p.armor = 55;
    p.score = 123;
    p.frags = 7;
    p.movement.command_time_ms = 1200;
    p.movement.remaining_ms = 250;
    p.movement.ducked = true;
    p.movement.jump_held = true;
    p.movement.grounded = true;
    p.movement.timer = MovementTimer(MovementTimer::LAND.0 | MovementTimer::KNOCKBACK.0);
    p.movement.water_level = 2;
    p
}

#[test]
fn one_player_projects_into_all_native_layouts_without_changing_role_choices()
-> Result<(), qa_network::message::Error> {
    let p = player();
    let c = context();
    let mut bytes = [0; 1400];
    let nq = PlayerProjection::load(Protocol::NetQuake15, &[]);
    let mut words = [0; states::NQ_PLAYER_WORDS];
    assert!(nq.reduce(&p, &c, &mut words));
    assert_eq!(words[12], (-12.0f32).to_bits());
    assert_eq!(words[10], 55.0f32.to_bits());
    assert_eq!((words[19], words[20]), (1, 1));
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_nq_player(&mut writer, &words)?;
    let n = writer.size();
    let decoded = states::read_nq_player(&mut Reader::new(&bytes[..n], Encoding::Bytes), false)?;
    assert_eq!(decoded[12] as i32, -12);
    assert_eq!(decoded[10], 55);
    assert_eq!(f32::from_bits(decoded[3]), 16.);
    assert_eq!(f32::from_bits(decoded[5]), -32.);

    let qw = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let mut words = [0; states::QW_PLAYER_WORDS];
    assert!(qw.reduce(&p, &c, &mut words));
    let command = QwCmd {
        msec: 0,
        view_angles: Vec3::default(),
        movement: [0; 3],
        buttons: 0,
        impulse: 0,
    };
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert!(states::write_qw_player(&mut writer, 17, &words, command)?);
    let n = writer.size();
    let decoded =
        states::read_qw_player(&mut Reader::new(&bytes[..n], Encoding::Bytes), 0, command)?;
    assert_eq!(decoded.number, 17);
    assert_eq!(f32::from_bits(decoded.words[0]), 12.25);
    assert_eq!(f32::from_bits(decoded.words[6]), -32.);

    let q2 = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut words = [0; states::Q2_PLAYER_WORDS];
    assert!(q2.reduce(&p, &c, &mut words));
    assert_eq!(words[1] as i32, 98);
    assert_eq!(words[2] as i32, -377);
    assert_eq!(words[7], 31);
    assert_eq!(words[8], 23);
    assert_eq!(words[11] as i32, -32768);
    assert_eq!(words[12] as i32, -16384);
    assert_eq!(words[37] as i32, -12);
    assert_eq!(words[41], 55);
    assert_eq!(words[50], 7);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_player(&mut writer, &[0; states::Q2_PLAYER_WORDS], &words)?;
    let n = writer.size();
    let decoded = states::read_q2_player(
        &mut Reader::new(&bytes[..n], Encoding::Bytes),
        &[0; states::Q2_PLAYER_WORDS],
    )?;
    assert_eq!(decoded[1..13], words[1..13]);
    assert_eq!(decoded[37], words[37]);
    assert_eq!(decoded[41], words[41]);
    assert_eq!(decoded[50], words[50]);

    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    assert!(q3.reduce(&p, &c, &mut words));
    assert_eq!(words[1], p.body.position.0[0].to_bits());
    assert_eq!(words[19], 99);
    assert_eq!(words[20], 1022);
    assert_eq!(words[40], 17);
    assert_eq!(words[26], 32768);
    assert_eq!(words[36], 49152);
    assert_eq!(words[48] as i32, -12);
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words)?;
    let n = writer.size();
    let decoded = states::read_q3_player(
        &mut Reader::new(&bytes[..n], Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    assert_eq!(decoded[19], 99);
    assert_eq!(decoded[1], words[1]);
    assert_eq!(decoded[48] as i32, -12);
    assert_eq!(decoded[26], words[26]);
    assert_eq!(decoded[36], words[36]);
    assert_eq!(p.movement_rules, RuleSetId::Quake2);
    assert_eq!(p.trace_rules, RuleSetId::QuakeWorld);
    Ok(())
}

#[test]
fn module_bindings_use_common_values_and_drop_only_unrepresentable_destinations() {
    let mut p = player();
    let binding = ValueBinding {
        id: ValueId(0),
        width: ValueWidth::Signed16,
        reset: ValueReset::Life,
    };
    assert!(binding.import(&mut p.values, 65534));
    let absent = ValueBinding {
        id: ValueId(999),
        ..binding
    };
    let projection = PlayerProjection::load(
        Protocol::Quake3_68,
        &[(48, binding), (49, absent), (1000, binding)],
    );
    assert_eq!(projection.dropped_bindings, 1);
    let mut words = [123; states::PLAYER_WORDS];
    assert!(projection.reduce(&p, &context(), &mut words));
    assert_eq!(words[48] as i32, -2);
    assert_eq!(words[49], 0);
    assert_eq!(p.values.get(ValueId(0)), Some(NumericValue::integer(-2)));
    let mut short = [123; 8];
    assert!(!projection.reduce(&p, &context(), &mut short));
    assert_eq!(short, [123; 8]);
}

#[test]
fn native_ordinals_and_modes_remain_boundary_data() {
    let mut p = player();
    let mut c = context();
    c.client_number = Some(9999);
    c.ground_number = Some(9999);
    c.weapon_number = None;
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    for (mode, native) in [
        (MovementMode::Walk, 0),
        (MovementMode::Fly, 1),
        (MovementMode::Noclip, 1),
        (MovementMode::Spectator, 2),
        (MovementMode::Dead, 3),
        (MovementMode::Gib, 3),
        (MovementMode::Frozen, 4),
    ] {
        p.movement.mode = mode;
        assert!(projection.reduce(&p, &c, &mut words));
        assert_eq!(words[34], native);
        assert_eq!(words[40], 0);
        assert_eq!(words[20], 1023);
        assert_eq!(words[41], 0);
    }
}

#[test]
fn delta_angles_keep_the_native_macro_order_and_signedness() {
    let mut p = player();
    let q2 = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut q2_words = [0; states::Q2_PLAYER_WORDS];
    let mut q3_words = [0; states::PLAYER_WORDS];
    let mut seed = 860u32;
    for _ in 0..4096 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let angle = (seed as i32 as f32) / 4096.;
        p.movement.delta_angles = Vec3([angle; 3]);
        assert!(q2.reduce(&p, &context(), &mut q2_words));
        assert!(q3.reduce(&p, &context(), &mut q3_words));
        let native = (angle * 65536. / 360.) as i32;
        for word in &q2_words[10..13] {
            assert_eq!(*word, native as i16 as i32 as u32);
        }
        for index in [26, 35, 36] {
            assert_eq!(q3_words[index], (native & 65535) as u32);
        }
    }
}
