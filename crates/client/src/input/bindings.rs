//! Named inputs, default bindings, and binding commands.
//!
//! Donor provenance: `src/input/bindings.ts` (`namedPhysicalInput`,
//! `physicalInputName`, `defaultBindings`, `archivedBindings`,
//! `registerBindingCommands`, wheel commands) and the
//! `registerInputCommands` half of `src/input/seat.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::cmd::Dialect;
use qa_core::identity::SeatId;
use qa_world::client::ClientFamily;
use thiserror::Error;

use super::keycodes::keynum_to_string;
use super::keycodes::string_to_keynum;
use super::weapons::WeaponBindingItem;
use super::{parse_impulse, seat_impulse};
use super::{
    physical_mouse_button, quake_mouse_button, BindingTable, InputAction, InputBinding, InputBindingTarget,
    PhysicalInput, SourceAction,
};
use crate::input::commands::{CommandInvocation, CommandOrigin, CommandRegistry, CommandsError};

/// Named controller buttons: command, index, menu label.
const CONTROLLER_BUTTONS: [(&str, i32, &str); 21] = [
    ("gamepad_a_button", 0, "A"),
    ("gamepad_b_button", 1, "B"),
    ("gamepad_x_button", 2, "X"),
    ("gamepad_y_button", 3, "Y"),
    ("gamepad_back", 4, "Back"),
    ("gamepad_guide", 5, "Guide"),
    ("gamepad_start", 6, "Start"),
    ("gamepad_left_stick", 7, "Left stick press"),
    ("gamepad_right_stick", 8, "Right stick press"),
    ("gamepad_left_shoulder", 9, "Left shoulder"),
    ("gamepad_right_shoulder", 10, "Right shoulder"),
    ("gamepad_dpad_up", 11, "D-pad up"),
    ("gamepad_dpad_down", 12, "D-pad down"),
    ("gamepad_dpad_left", 13, "D-pad left"),
    ("gamepad_dpad_right", 14, "D-pad right"),
    ("gamepad_misc", 15, "Miscellaneous"),
    ("gamepad_paddle1", 16, "Paddle 1"),
    ("gamepad_paddle2", 17, "Paddle 2"),
    ("gamepad_paddle3", 18, "Paddle 3"),
    ("gamepad_paddle4", 19, "Paddle 4"),
    ("gamepad_touchpad", 20, "Touchpad"),
];

fn mouse_name(name: &str) -> Option<i32> {
    let digits = name.strip_prefix("mouse")?;
    if digits.len() >= 2 && digits.starts_with('0') {
        return None;
    }
    if digits.is_empty() || !digits.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    let number: i32 = digits.parse().ok()?;
    (number >= 1).then_some(number)
}

/// Parse a binding name to a physical input (`namedPhysicalInput`).
#[must_use]
pub fn named_physical_input(name: &str, device: i32) -> Option<PhysicalInput> {
    let lower = name.to_lowercase();
    if let Some(number) = mouse_name(&lower) {
        return Some(PhysicalInput::MouseButton(physical_mouse_button(number)));
    }
    let controller_name = if lower.starts_with("gamepad_") {
        lower.clone()
    } else {
        format!("gamepad_{lower}")
    };
    if let Some((_, button, _)) = CONTROLLER_BUTTONS
        .iter()
        .find(|(command, _, _)| *command == controller_name)
    {
        return Some(PhysicalInput::ControllerButton {
            device,
            button: *button,
        });
    }
    if controller_name == "gamepad_left_trigger" || controller_name == "gamepad_right_trigger" {
        return Some(PhysicalInput::ControllerAxis {
            device,
            axis: if controller_name == "gamepad_left_trigger" {
                super::ControllerAxis::LeftTrigger
            } else {
                super::ControllerAxis::RightTrigger
            },
            direction: super::AxisDirection::Positive,
        });
    }
    let code = string_to_keynum(Some(&lower));
    (code >= 0).then_some(PhysicalInput::Key(code))
}

/// Display name for a physical input (`physicalInputName`).
#[must_use]
pub fn physical_input_name(input: &PhysicalInput) -> String {
    match input {
        PhysicalInput::Key(code) => keynum_to_string(*code),
        PhysicalInput::MouseButton(button) => format!("MOUSE{}", quake_mouse_button(*button)),
        PhysicalInput::ControllerButton { device, button } => {
            let label = CONTROLLER_BUTTONS
                .iter()
                .find(|(_, index, _)| *index == *button)
                .map_or_else(
                    || format!("Button {}", button + 1),
                    |(_, _, label)| (*label).to_string(),
                );
            format!("Pad {} {label}", device + 1)
        }
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction,
        } => {
            let prefix = format!("Pad {}", device + 1);
            let positive = *direction == super::AxisDirection::Positive;
            match axis {
                super::ControllerAxis::LeftX => {
                    format!("{prefix} Left stick {}", if positive { "right" } else { "left" })
                }
                super::ControllerAxis::LeftY => format!("{prefix} Left stick {}", if positive { "down" } else { "up" }),
                super::ControllerAxis::RightX => {
                    format!("{prefix} Right stick {}", if positive { "right" } else { "left" })
                }
                super::ControllerAxis::RightY => {
                    format!("{prefix} Right stick {}", if positive { "down" } else { "up" })
                }
                super::ControllerAxis::LeftTrigger => {
                    format!("{prefix} Left trigger{}", if positive { "" } else { " released" })
                }
                super::ControllerAxis::RightTrigger => {
                    format!("{prefix} Right trigger{}", if positive { "" } else { " released" })
                }
            }
        }
    }
}

/// Contract name for a named action.
#[must_use]
pub const fn input_action_name(action: InputAction) -> &'static str {
    match action {
        InputAction::Attack => "attack",
        InputAction::Jump => "jump",
        InputAction::Forward => "forward",
        InputAction::Back => "back",
        InputAction::MoveLeft => "move-left",
        InputAction::MoveRight => "move-right",
        InputAction::MoveUp => "move-up",
        InputAction::MoveDown => "move-down",
        InputAction::Use => "use",
        InputAction::Crouch => "crouch",
        InputAction::Walk => "walk",
        InputAction::Scores => "scores",
        InputAction::NextWeapon => "next-weapon",
        InputAction::PreviousWeapon => "previous-weapon",
        InputAction::Menu => "menu",
    }
}

/// Default bindings for a device and dialect.
#[must_use]
pub fn default_bindings(device: i32, dialect: Dialect, items: &[WeaponBindingItem]) -> Vec<InputBinding> {
    let q1 = dialect.is_q1();
    let mut defaults: Vec<(String, String)> = [
        ("w", "+forward"),
        ("s", "+back"),
        ("a", "+moveleft"),
        ("d", "+moveright"),
        ("SPACE", if q1 { "+jump" } else { "+moveup" }),
        ("CTRL", "+movedown"),
        ("SHIFT", "+speed"),
        ("MOUSE1", "+attack"),
        ("TAB", "+scores"),
        ("MWHEELUP", "weapprev"),
        ("MWHEELDOWN", "weapnext"),
        ("q", "+weaponwheel"),
        ("GAMEPAD_RIGHT_TRIGGER", "+attack"),
        ("GAMEPAD_A_BUTTON", if q1 { "+jump" } else { "+moveup" }),
        ("GAMEPAD_B_BUTTON", "+movedown"),
        ("GAMEPAD_X_BUTTON", "+use"),
        ("GAMEPAD_LEFT_SHOULDER", "weapprev"),
        ("GAMEPAD_RIGHT_SHOULDER", "weapnext"),
        ("GAMEPAD_BACK", "+scores"),
    ]
    .iter()
    .map(|(name, text)| ((*name).to_string(), (*text).to_string()))
    .collect();
    defaults.extend(
        super::weapons::default_weapon_bindings(items)
            .iter()
            .map(|(name, text)| (name.clone(), text.clone())),
    );
    defaults
        .iter()
        .filter_map(|(name, text)| {
            named_physical_input(name, device).map(|input| InputBinding {
                input,
                target: InputBindingTarget::Command(text.clone()),
            })
        })
        .collect()
}

/// Binding archive error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BindingsError {
    /// A binding cannot be represented in a source cfg.
    #[error("Source cfg cannot represent a binding containing quotes or newlines; use the seat settings document")]
    Unrepresentable,
}

fn quoted(value: &str) -> Result<String, BindingsError> {
    if value.chars().any(|char| matches!(char, '"' | '\r' | '\n' | '\0')) {
        return Err(BindingsError::Unrepresentable);
    }
    Ok(format!("\"{value}\""))
}

/// Archive bindings as `bind` commands (`archivedBindings`).
pub fn archived_bindings(
    bindings: &[InputBinding],
    include_controller_bindings: bool,
) -> Result<Vec<String>, BindingsError> {
    let mut commands = vec!["unbindall".to_string()];
    for binding in bindings {
        let InputBindingTarget::Command(text) = &binding.target else {
            continue;
        };
        let mut name = match &binding.input {
            PhysicalInput::Key(_) | PhysicalInput::MouseButton(_) => Some(physical_input_name(&binding.input)),
            _ => None,
        };
        if include_controller_bindings {
            match &binding.input {
                PhysicalInput::ControllerButton { device: 0, button } => {
                    name = CONTROLLER_BUTTONS
                        .iter()
                        .find(|(_, index, _)| index == button)
                        .map(|(command, _, _)| command.to_uppercase());
                }
                PhysicalInput::ControllerAxis {
                    device: 0,
                    axis,
                    direction: super::AxisDirection::Positive,
                } => {
                    name = match axis {
                        super::ControllerAxis::LeftTrigger => Some("GAMEPAD_LEFT_TRIGGER".to_string()),
                        super::ControllerAxis::RightTrigger => Some("GAMEPAD_RIGHT_TRIGGER".to_string()),
                        _ => None,
                    };
                }
                _ => {}
            }
        }
        if let Some(name) = name {
            commands.push(format!("bind {} {}", quoted(&name)?, quoted(text)?));
        }
    }
    Ok(commands)
}

/// Wheel menu mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WheelMode {
    /// Weapon wheel.
    Weapons,
    /// Powerup wheel.
    Powerups,
}

const WHEEL_COMMANDS: [(&str, WheelMode); 4] = [
    ("weaponwheel", WheelMode::Weapons),
    ("wheel", WheelMode::Weapons),
    ("powerupwheel", WheelMode::Powerups),
    ("wheel2", WheelMode::Powerups),
];

/// Canonical `+weaponwheel`/`+powerupwheel` spelling.
#[must_use]
pub fn canonical_wheel_command(text: &str) -> String {
    let command = text.trim().to_lowercase();
    let prefix = command.chars().next().unwrap_or('\0');
    if prefix != '+' && prefix != '-' {
        return text.to_string();
    }
    let mode = WHEEL_COMMANDS
        .iter()
        .find(|(name, _)| command[1..] == **name)
        .map(|(_, mode)| *mode);
    match mode {
        None => text.to_string(),
        Some(WheelMode::Weapons) => format!("{prefix}weaponwheel"),
        Some(WheelMode::Powerups) => format!("{prefix}powerupwheel"),
    }
}

/// Seat binding-table lookup for binding commands.
pub type BindingLookup = Rc<dyn Fn(&SeatId) -> Option<Rc<RefCell<BindingTable>>>>;
/// Print sink for binding commands.
pub type PrintFn = Rc<dyn Fn(&str)>;

fn local_seat(
    invocation: &CommandInvocation,
    lookup: &BindingLookup,
    console: &Option<Rc<RefCell<BindingTable>>>,
) -> Option<Rc<RefCell<BindingTable>>> {
    match invocation.root_origin() {
        CommandOrigin::LocalSeat(seat) => lookup(seat),
        CommandOrigin::LocalConsole | CommandOrigin::ServerConsole => console.clone(),
        _ => None,
    }
}

/// Register `bind`, `unbind`, `unbindall`, and `bindlist`.
///
/// Returns the registered names so the caller can unregister them.
pub fn register_binding_commands(
    registry: &mut dyn CommandRegistry,
    lookup: BindingLookup,
    print: PrintFn,
    console: Option<Rc<RefCell<BindingTable>>>,
) -> Vec<String> {
    let mut registered = Vec::new();
    let bind_lookup = lookup.clone();
    let bind_print = print.clone();
    let bind_console = console.clone();
    if registry.register_engine(
        "bind",
        Box::new(move |invocation| {
            let name = invocation.argv.get(1).cloned();
            let seat = local_seat(invocation, &bind_lookup, &bind_console);
            let (Some(seat), Some(name)) = (seat, name) else {
                bind_print("bind <key> [command]\n");
                return Ok(());
            };
            let table = seat.borrow();
            let device = table
                .bindings()
                .iter()
                .find(|binding| {
                    matches!(
                        binding.input,
                        PhysicalInput::ControllerButton { .. } | PhysicalInput::ControllerAxis { .. }
                    )
                })
                .map(|binding| match &binding.input {
                    PhysicalInput::ControllerButton { device, .. } | PhysicalInput::ControllerAxis { device, .. } => {
                        *device
                    }
                    _ => 0,
                })
                .unwrap_or(0);
            let Some(input) = named_physical_input(&name, device) else {
                bind_print(&format!("Unknown key {name}\n"));
                return Ok(());
            };
            if invocation.argv.len() == 2 {
                match table.binding(&input) {
                    None => bind_print(&format!("{} = Unbound\n", physical_input_name(&input))),
                    Some(InputBindingTarget::Command(text)) => {
                        bind_print(&format!("{} = {text}\n", physical_input_name(&input)))
                    }
                    Some(InputBindingTarget::Action(action)) => {
                        bind_print(&format!(
                            "{} = {}\n",
                            physical_input_name(&input),
                            input_action_name(*action)
                        ));
                    }
                }
                return Ok(());
            }
            let text = invocation.argv[2..].join(" ");
            drop(table);
            seat.borrow_mut().bind(InputBinding {
                input,
                target: InputBindingTarget::Command(text),
            });
            Ok(())
        }),
    ) {
        registered.push("bind".to_string());
    }
    let unbind_lookup = lookup.clone();
    let unbind_print = print.clone();
    let unbind_console = console.clone();
    if registry.register_engine(
        "unbind",
        Box::new(move |invocation| {
            let name = invocation.argv.get(1).cloned();
            if invocation.argv.len() != 2 || name.is_none() {
                unbind_print("unbind <key> : remove commands from a key\n");
                return Ok(());
            }
            let name = name.unwrap_or_default();
            let Some(seat) = local_seat(invocation, &unbind_lookup, &unbind_console) else {
                unbind_print("unbind requires a local binding owner.\n");
                return Ok(());
            };
            let Some(input) = named_physical_input(&name, 0) else {
                unbind_print(&format!("Unknown key {name}\n"));
                return Ok(());
            };
            let mut table = seat.borrow_mut();
            match &input {
                PhysicalInput::ControllerButton { button, .. } => {
                    let owned: Vec<PhysicalInput> = table
                        .bindings()
                        .iter()
                        .filter_map(|binding| match &binding.input {
                            PhysicalInput::ControllerButton { button: candidate, .. } if candidate == button => {
                                Some(binding.input.clone())
                            }
                            _ => None,
                        })
                        .collect();
                    for candidate in owned {
                        table.unbind(&candidate);
                    }
                }
                PhysicalInput::ControllerAxis { axis, direction, .. } => {
                    let owned: Vec<PhysicalInput> = table
                        .bindings()
                        .iter()
                        .filter_map(|binding| match &binding.input {
                            PhysicalInput::ControllerAxis {
                                axis: candidate_axis,
                                direction: candidate_direction,
                                ..
                            } if candidate_axis == axis && candidate_direction == direction => {
                                Some(binding.input.clone())
                            }
                            _ => None,
                        })
                        .collect();
                    for candidate in owned {
                        table.unbind(&candidate);
                    }
                }
                _ => table.unbind(&input),
            }
            Ok(())
        }),
    ) {
        registered.push("unbind".to_string());
    }
    let unbindall_lookup = lookup.clone();
    let unbindall_console = console.clone();
    if registry.register_engine(
        "unbindall",
        Box::new(move |invocation| {
            if let Some(seat) = local_seat(invocation, &unbindall_lookup, &unbindall_console) {
                seat.borrow_mut().unbind_all();
            }
            Ok(())
        }),
    ) {
        registered.push("unbindall".to_string());
    }
    let list_lookup = lookup.clone();
    let list_print = print.clone();
    let list_console = console;
    if registry.register_engine(
        "bindlist",
        Box::new(move |invocation| {
            if let Some(seat) = local_seat(invocation, &list_lookup, &list_console) {
                for binding in seat.borrow().bindings() {
                    let target = match &binding.target {
                        InputBindingTarget::Command(text) => text.clone(),
                        InputBindingTarget::Action(action) => input_action_name(*action).to_string(),
                    };
                    list_print(&format!("{} = {target}\n", physical_input_name(&binding.input)));
                }
            }
            Ok(())
        }),
    ) {
        registered.push("bindlist".to_string());
    }
    registered
}

/// Register `+weaponwheel`/`-wheel` style commands.
pub fn register_wheel_commands(
    registry: &mut dyn CommandRegistry,
    wheel: Rc<dyn Fn(SeatId, WheelMode, bool)>,
) -> Vec<String> {
    let mut registered = Vec::new();
    for (name, mode) in WHEEL_COMMANDS {
        for down in [true, false] {
            let command = format!("{}{name}", if down { "+" } else { "-" });
            let wheel = wheel.clone();
            if registry.register_engine(
                &command,
                Box::new(move |invocation| {
                    if let CommandOrigin::LocalSeat(seat) = invocation.root_origin() {
                        wheel(seat.clone(), mode, down);
                    }
                    Ok(())
                }),
            ) {
                registered.push(command);
            }
        }
    }
    registered
}

/// Seat surface for `+`/`-` button commands.
pub trait ButtonSeat {
    /// Press or release a named button from a key.
    fn command_button(&mut self, action: SourceAction, key: &str, down: bool, time_ms: i64);
    /// Release a named button from every key.
    fn release_button(&mut self, action: SourceAction, time_ms: i64);
    /// Set the pending impulse byte.
    fn set_impulse_byte(&mut self, value: u8);
}

/// Seat lookup for button commands.
pub type ButtonSeatLookup = Rc<dyn Fn(&SeatId) -> Option<Rc<RefCell<dyn ButtonSeat>>>>;
/// Client-scores interceptor for the `+scores` command.
pub type ClientScoresFn = Rc<dyn Fn(&CommandInvocation) -> bool>;

fn action_commands() -> Vec<(String, SourceAction)> {
    let named = [
        ("attack", SourceAction::Action(InputAction::Attack)),
        ("jump", SourceAction::Action(InputAction::Jump)),
        ("forward", SourceAction::Action(InputAction::Forward)),
        ("back", SourceAction::Action(InputAction::Back)),
        ("moveleft", SourceAction::Action(InputAction::MoveLeft)),
        ("moveright", SourceAction::Action(InputAction::MoveRight)),
        ("moveup", SourceAction::Action(InputAction::MoveUp)),
        ("movedown", SourceAction::Action(InputAction::MoveDown)),
        ("use", SourceAction::Action(InputAction::Use)),
        ("crouch", SourceAction::Action(InputAction::Crouch)),
        ("speed", SourceAction::Action(InputAction::Walk)),
        ("scores", SourceAction::Action(InputAction::Scores)),
        ("showscores", SourceAction::Action(InputAction::Scores)),
        ("left", SourceAction::TurnLeft),
        ("right", SourceAction::TurnRight),
        ("lookup", SourceAction::LookUp),
        ("lookdown", SourceAction::LookDown),
        ("strafe", SourceAction::Strafe),
        ("mlook", SourceAction::MouseLook),
        ("klook", SourceAction::KeyLook),
        ("holster", SourceAction::Holster),
    ];
    let mut commands: Vec<(String, SourceAction)> = named
        .iter()
        .map(|(name, action)| ((*name).to_string(), *action))
        .collect();
    for index in 0..15u8 {
        commands.push((format!("button{index}"), SourceAction::Button(index)));
    }
    commands
}

fn dialect_family(dialect: Dialect) -> ClientFamily {
    match dialect {
        Dialect::Q1Netquake => ClientFamily::Q1Netquake,
        Dialect::Q1Quakeworld => ClientFamily::Q1Quakeworld,
        Dialect::Q2Classic => ClientFamily::Q2Classic,
        Dialect::Q2Rerelease => ClientFamily::Q2Rerelease,
        Dialect::Q3 => ClientFamily::Q3,
    }
}

/// Register `+`/`-` button commands and `impulse`.
pub fn register_input_commands(
    registry: &mut dyn CommandRegistry,
    lookup: ButtonSeatLookup,
    client_scores: Option<ClientScoresFn>,
) -> Vec<String> {
    let mut registered = Vec::new();
    for (name, action) in action_commands() {
        for down in [true, false] {
            let command = format!("{}{name}", if down { "+" } else { "-" });
            let lookup = lookup.clone();
            let client_scores = client_scores.clone();
            let name_clone = name.clone();
            if registry.register_engine(
                &command,
                Box::new(move |invocation| {
                    if name_clone == "scores" && client_scores.as_ref().is_some_and(|scores| scores(invocation)) {
                        return Ok(());
                    }
                    let CommandOrigin::LocalSeat(seat) = invocation.root_origin() else {
                        return Ok(());
                    };
                    let Some(target) = lookup(seat) else {
                        return Ok(());
                    };
                    let time = invocation
                        .argv
                        .get(2)
                        .map_or(0.0, |text| text.parse::<f64>().unwrap_or(f64::NAN));
                    if !time.is_finite() {
                        return Err(CommandsError::BadTimestamp);
                    }
                    let time_ms = time.trunc() as i64;
                    if !down && invocation.argv.get(1).is_none() {
                        target.borrow_mut().release_button(action, time_ms);
                    } else {
                        let key = invocation.argv.get(1).cloned().unwrap_or_else(|| "console".to_string());
                        target.borrow_mut().command_button(action, &key, down, time_ms);
                    }
                    Ok(())
                }),
            ) {
                registered.push(command);
            }
        }
    }
    let impulse_lookup = lookup;
    if registry.register(
        "impulse",
        Box::new(move |invocation| {
            let CommandOrigin::LocalSeat(seat) = invocation.root_origin() else {
                return Ok(());
            };
            let Some(target) = impulse_lookup(seat) else {
                return Ok(());
            };
            let text = invocation.argv.get(1).cloned().unwrap_or_default();
            let value = parse_impulse(&text, dialect_family(invocation.dialect));
            let byte = seat_impulse(value).map_err(|_| CommandsError::BadImpulse)?;
            target.borrow_mut().set_impulse_byte(byte);
            Ok(())
        }),
    ) {
        registered.push("impulse".to_string());
    }
    registered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::commands::CommandHandler;
    use qa_core::identity::IdentityOwner;

    struct FakeRegistry {
        commands: std::collections::HashMap<String, CommandHandler>,
    }

    impl FakeRegistry {
        fn invoke(&mut self, name: &str, invocation: &CommandInvocation) -> Result<(), CommandsError> {
            let mut handler = self.commands.remove(name).expect("command");
            let result = handler(invocation);
            self.commands.insert(name.to_string(), handler);
            result
        }
    }

    impl CommandRegistry for FakeRegistry {
        fn register_engine(&mut self, name: &str, handler: CommandHandler) -> bool {
            if self.commands.contains_key(name) {
                return false;
            }
            self.commands.insert(name.to_string(), handler);
            true
        }

        fn register(&mut self, name: &str, handler: CommandHandler) -> bool {
            self.register_engine(name, handler)
        }

        fn unregister(&mut self, name: &str) {
            self.commands.remove(name);
        }

        fn exists(&self, name: &str) -> bool {
            self.commands.contains_key(name)
        }

        fn append(&mut self, _text: &str, _seat: &SeatId) {}
    }

    #[test]
    fn names_map_both_directions() {
        assert_eq!(
            named_physical_input("MOUSE2", 0),
            Some(PhysicalInput::MouseButton(physical_mouse_button(2)))
        );
        assert_eq!(
            named_physical_input("GAMEPAD_A_BUTTON", 1),
            Some(PhysicalInput::ControllerButton { device: 1, button: 0 })
        );
        assert_eq!(named_physical_input("SPACE", 0), Some(PhysicalInput::Key(32)));
        assert_eq!(named_physical_input("q", 0), Some(PhysicalInput::Key(113)));
        assert_eq!(named_physical_input("bogus-key", 0), None);
        assert_eq!(physical_input_name(&PhysicalInput::Key(32)), "SPACE");
        assert_eq!(physical_input_name(&PhysicalInput::MouseButton(1)), "MOUSE1");
        assert_eq!(
            physical_input_name(&PhysicalInput::ControllerButton { device: 0, button: 0 }),
            "Pad 1 A"
        );
        assert_eq!(
            physical_input_name(&PhysicalInput::ControllerAxis {
                device: 1,
                axis: super::super::ControllerAxis::LeftX,
                direction: super::super::AxisDirection::Positive,
            }),
            "Pad 2 Left stick right"
        );
    }

    #[test]
    fn defaults_and_archives_cover_dialects() {
        let items =
            super::super::weapons::base_weapon_binding_items(super::super::weapons::WeaponFamily::Q1, "", "classic");
        let q1 = default_bindings(0, Dialect::Q1Netquake, &items);
        assert!(q1
            .iter()
            .any(|binding| binding.target == InputBindingTarget::Command("+jump".to_string())));
        let q3 = default_bindings(0, Dialect::Q3, &items);
        assert!(q3
            .iter()
            .any(|binding| binding.target == InputBindingTarget::Command("+moveup".to_string())));
        assert!(q3
            .iter()
            .any(|binding| binding.target == InputBindingTarget::Command("use q1:weapon/axe".to_string())));
        let archived = archived_bindings(&q1, true).unwrap();
        assert_eq!(archived[0], "unbindall");
        assert!(archived.iter().any(|line| line.starts_with("bind \"SPACE\"")));
        assert!(archived.iter().any(|line| line.contains("GAMEPAD_A_BUTTON")));
        let dropped = archived_bindings(&q1, false).unwrap();
        assert!(!dropped.iter().any(|line| line.contains("GAMEPAD")));
        let bad = [InputBinding {
            input: PhysicalInput::Key(65),
            target: InputBindingTarget::Command("say \"hi\"".to_string()),
        }];
        assert_eq!(archived_bindings(&bad, false), Err(BindingsError::Unrepresentable));
        assert_eq!(canonical_wheel_command("+wheel"), "+weaponwheel");
        assert_eq!(canonical_wheel_command("-wheel2"), "-powerupwheel");
        assert_eq!(canonical_wheel_command("weaponwheel"), "weaponwheel");
    }

    #[test]
    fn bind_commands_read_write_and_clear() {
        let owner = IdentityOwner::create("test").unwrap();
        let seat = owner.seat(0);
        let table = Rc::new(RefCell::new(BindingTable::new()));
        let lookup_table = table.clone();
        let lookup: BindingLookup = Rc::new(move |_| Some(lookup_table.clone()));
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = printed.clone();
        let print: PrintFn = Rc::new(move |text| sink.borrow_mut().push(text.to_string()));
        let mut registry = FakeRegistry {
            commands: std::collections::HashMap::new(),
        };
        let names = register_binding_commands(&mut registry, lookup, print, None);
        assert_eq!(names.len(), 4);
        let invoke = |registry: &mut FakeRegistry, argv: &[&str]| {
            registry
                .invoke(
                    argv[0],
                    &CommandInvocation::new(
                        argv.iter().map(|value| (*value).to_string()).collect(),
                        CommandOrigin::LocalSeat(seat.clone()),
                        Dialect::Q3,
                    ),
                )
                .unwrap();
        };
        invoke(&mut registry, &["bind", "SPACE", "+jump"]);
        invoke(&mut registry, &["bind", "SPACE"]);
        assert!(printed.borrow().iter().any(|line| line == "SPACE = +jump\n"));
        invoke(&mut registry, &["bindlist"]);
        assert!(printed.borrow().iter().any(|line| line == "SPACE = +jump\n"));
        invoke(&mut registry, &["unbind", "SPACE"]);
        invoke(&mut registry, &["bind", "SPACE"]);
        assert!(printed.borrow().iter().any(|line| line == "SPACE = Unbound\n"));
        invoke(&mut registry, &["bind", "BOGUS"]);
        assert!(printed.borrow().iter().any(|line| line == "Unknown key BOGUS\n"));
        invoke(&mut registry, &["bind", "SPACE", "+jump"]);
        invoke(&mut registry, &["unbindall"]);
        assert!(table.borrow().bindings().is_empty());
        let wheels = register_wheel_commands(&mut registry, Rc::new(|_, _, _| {}));
        assert_eq!(wheels.len(), 8);
    }

    #[test]
    fn button_commands_drive_seats() {
        struct Seat {
            buttons: Vec<(SourceAction, String, bool)>,
            impulse: u8,
        }
        impl ButtonSeat for Seat {
            fn command_button(&mut self, action: SourceAction, key: &str, down: bool, _time_ms: i64) {
                self.buttons.push((action, key.to_string(), down));
            }
            fn release_button(&mut self, action: SourceAction, _time_ms: i64) {
                self.buttons.push((action, String::new(), false));
            }
            fn set_impulse_byte(&mut self, value: u8) {
                self.impulse = value;
            }
        }
        let owner = IdentityOwner::create("test").unwrap();
        let seat = owner.seat(0);
        let target: Rc<RefCell<dyn ButtonSeat>> = Rc::new(RefCell::new(Seat {
            buttons: Vec::new(),
            impulse: 0,
        }));
        let lookup: ButtonSeatLookup = Rc::new(move |_| Some(target.clone()));
        let mut registry = FakeRegistry {
            commands: std::collections::HashMap::new(),
        };
        let names = register_input_commands(&mut registry, lookup, None);
        assert!(names.contains(&"+attack".to_string()));
        assert!(names.contains(&"impulse".to_string()));
        registry
            .invoke(
                "+attack",
                &CommandInvocation::new(
                    vec!["+attack".to_string(), "32".to_string(), "40".to_string()],
                    CommandOrigin::LocalSeat(seat.clone()),
                    Dialect::Q3,
                ),
            )
            .unwrap();
        registry
            .invoke(
                "impulse",
                &CommandInvocation::new(
                    vec!["impulse".to_string(), "12".to_string()],
                    CommandOrigin::LocalSeat(seat.clone()),
                    Dialect::Q3,
                ),
            )
            .unwrap();
        assert!(registry
            .invoke(
                "impulse",
                &CommandInvocation::new(
                    vec!["impulse".to_string(), "999".to_string()],
                    CommandOrigin::LocalSeat(seat.clone()),
                    Dialect::Q3,
                ),
            )
            .is_err());
    }
}
