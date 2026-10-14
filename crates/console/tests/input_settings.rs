use qa_console::{cvars::Cvars, input_settings::InputHandles};
use qa_core::{
    primitives::RuleSetId,
    sys_events::{DeviceId, EventKind, EventTime, KeySource, SeatId, SysEvent},
};
use qa_input::{Action, Input, InputPolicy, Target};

#[test]
fn catalog_defaults_select_input_role_without_rebinding_names() {
    let vars = Cvars::new().unwrap();
    let handles = InputHandles::load(&vars).unwrap();
    let freelook = vars.find("cl_freelook").unwrap();
    let run = vars.find("cl_run").unwrap();
    vars.reset_lookup_count();
    for rules in RuleSetId::ALL {
        let policy = handles.policy(&vars, rules);
        assert_eq!(policy.rules, rules);
        assert_eq!(policy.speed[0], 200.0);
        assert_eq!(policy.back_speed, 200.0);
        assert_eq!(
            policy.sensitivity,
            if rules == RuleSetId::Quake3 { 5.0 } else { 3.0 }
        );
        assert_eq!(
            policy.always_run,
            matches!(rules, RuleSetId::Quake2Rerelease | RuleSetId::Quake3)
        );
        assert_eq!(
            policy.freelook,
            matches!(rules, RuleSetId::Quake2Rerelease | RuleSetId::Quake3)
        );
        assert_eq!(
            policy.filter,
            rules == RuleSetId::Quake3 && cfg!(target_os = "macos")
        );
        if rules == RuleSetId::Quake2 {
            assert_eq!(
                policy.mouse_side,
                if cfg!(target_os = "linux") { 0.8 } else { 1.0 }
            );
        }
        if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            assert_eq!(vars.native_number(freelook, rules), None);
            assert_eq!(vars.native_integer(run, rules), None);
        }
    }
    assert_eq!(vars.lookup_count(), 0);
}

#[test]
fn extensions_backwardspeed_and_integer_truth_use_cached_cvar_views() {
    let mut vars = Cvars::new().unwrap();
    let handles = InputHandles::load(&vars).unwrap();
    for (name, value) in [
        ("cl_forwardspeed", "444"),
        ("cl_run", "0.5"),
        ("m_filter", "0.5"),
        ("cl_freelook", "0.5"),
        ("lookspring", "0.5"),
        ("cl_mouseAccel", "0.9"),
        ("m_pitch", "-0.047"),
    ] {
        vars.set_text(vars.find(name).unwrap(), value).unwrap();
    }
    vars.reset_lookup_count();
    for rules in RuleSetId::ALL {
        let policy = handles.policy(&vars, rules);
        let quake = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
        assert_eq!(policy.back_speed, if quake { 200.0 } else { 444.0 });
        assert_eq!(policy.always_run, quake);
        assert_eq!(policy.filter, rules != RuleSetId::Quake3);
        assert_eq!(policy.freelook, rules != RuleSetId::Quake3);
        assert!(policy.lookspring);
        assert_eq!(policy.acceleration, 0.9);
        assert_eq!(policy.mouse_scale[1], -0.047);
    }
    assert_eq!(vars.lookup_count(), 0);
    let backwards = vars.find("cl_backspeed").unwrap();
    vars.set_text(backwards, "123").unwrap();
    assert_eq!(handles.policy(&vars, RuleSetId::Quake2).back_speed, 123.0);
    vars.set_text(backwards, "1e9999").unwrap();
    assert_eq!(handles.policy(&vars, RuleSetId::Quake2).back_speed, 444.0);
    vars.reset(backwards);
    assert_eq!(handles.policy(&vars, RuleSetId::Quake2).back_speed, 444.0);
}

#[test]
fn nonfinite_effective_settings_fall_back_to_cached_catalog_defaults() {
    let mut vars = Cvars::new().unwrap();
    let handles = InputHandles::load(&vars).unwrap();
    let sensitivity = vars.find("sensitivity").unwrap();
    vars.set_text(sensitivity, "1e9999").unwrap();
    assert!(!vars.value_in(sensitivity, RuleSetId::Quake3).is_finite());
    vars.reset_lookup_count();
    for _ in 0..16 {
        assert_eq!(handles.policy(&vars, RuleSetId::Quake3).sensitivity, 5.0);
    }
    assert_eq!(vars.lookup_count(), 0);
    vars.reset(sensitivity);
    assert_eq!(handles.policy(&vars, RuleSetId::Quake3).sensitivity, 5.0);
}

struct Sink;
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, _: SeatId, _: EventTime, _: Option<KeySource>, _: &str) -> bool {
        true
    }
    fn cancel_commands(&mut self, _: KeySource) {}
}
fn mouse_command(policy: InputPolicy, mlook: bool) -> qa_core::primitives::UserCmd {
    let mut input = Input::load();
    input.seed(EventTime::default());
    if mlook {
        input.button(
            SeatId::FIRST,
            Action::MouseLook,
            true,
            Some(KeySource {
                device: DeviceId::Keyboard(0),
                control: 888,
            }),
            EventTime::default(),
        );
    }
    input.dispatch(
        SysEvent {
            time: EventTime::from_milliseconds(1),
            kind: EventKind::Mouse {
                device: DeviceId::Mouse(0),
                dx: 0,
                dy: 4,
            },
        },
        &mut Sink,
    );
    input.build_frame_with_policy(EventTime::from_milliseconds(16), &[policy; SeatId::COUNT])[0]
}

#[test]
fn quake_mouse_pitch_requires_mlook_or_an_explicit_freelook_extension() {
    let mut vars = Cvars::new().unwrap();
    let handles = InputHandles::load(&vars).unwrap();
    let freelook = vars.find("freelook").unwrap();
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let policy = handles.policy(&vars, rules);
        let moving = mouse_command(policy, false);
        assert_eq!(moving.view_angles.0[0], 0.0);
        assert_eq!(moving.movement[0], -12.0);
        let looking = mouse_command(policy, true);
        assert_eq!(
            looking.view_angles.0[0].to_bits(),
            (12.0_f32 * 0.022).to_bits()
        );
        assert_eq!(looking.movement[0], 0.0);
        vars.set_text(freelook, "1").unwrap();
        assert_eq!(
            mouse_command(handles.policy(&vars, rules), false).view_angles,
            looking.view_angles
        );
        vars.reset(freelook);
        assert_eq!(
            mouse_command(handles.policy(&vars, rules), false)
                .view_angles
                .0[0],
            0.0
        );
    }
}
