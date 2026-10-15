use crate::Action;
use qa_core::{primitives::CommandProducer, text::FixedText};

pub struct ActionName {
    pub name: &'static str,
    pub press: &'static str,
    pub release: &'static str,
    pub action: Action,
}
macro_rules! names { ($($name:literal => $action:ident),* $(,)?) => { &[$(ActionName { name: $name, press: concat!("+", $name), release: concat!("-", $name), action: Action::$action }),*] }; }
pub const ACTION_NAMES: &[ActionName] = names![
    "forward" => Forward, "back" => Back,
    "moveleft" => Left, "moveright" => Right,
    "moveup" => Up, "movedown" => Down,
    "attack" => Attack, "jump" => Jump,
    "use" => Use, "duck" => Crouch, "crouch" => Crouch, "holster" => Holster,
    "speed" => Walk, "walk" => Walk,
    "left" => TurnLeft, "right" => TurnRight,
    "lookup" => LookUp, "lookdown" => LookDown,
    "strafe" => Strafe, "mlook" => MouseLook,
    "klook" => KeyboardLook,
    "button0" => Attack, "button1" => Talk, "button2" => Use, "button3" => Gesture,
    "button4" => Walk, "button5" => Affirmative, "button6" => Negative,
    "button7" => GetFlag, "button8" => GuardBase, "button9" => Patrol,
    "button10" => FollowMe, "button11" => Any, "button12" => Extra12,
    "button13" => Extra13, "button14" => Extra14,
];
pub fn action_name(action: Action) -> &'static str {
    ACTION_NAMES
        .iter()
        .find(|entry| entry.action == action)
        .map_or("", |entry| entry.name)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    TooLong,
    Nul,
    Control,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Part {
    pub start: u16,
    pub end: u16,
    pub button: bool,
}
pub struct Binding {
    pub(crate) producer: Option<CommandProducer>,
    pub(crate) text: FixedText<1024>,
    pub(crate) parts: [Part; 512],
    pub(crate) count: usize,
}
impl Binding {
    pub fn for_action(action: Action) -> Self {
        let mut binding = Self {
            producer: None,
            text: FixedText::default(),
            parts: [Part::default(); 512],
            count: 1,
        };
        use std::fmt::Write;
        let _ = write!(binding.text, "+{}", action_name(action));
        binding.parts[0] = Part {
            start: 0,
            end: binding.text.as_str().len() as u16,
            button: true,
        };
        binding
    }
    pub fn parse(text: &str) -> Result<Self, BindError> {
        Self::parse_bytes(text.as_bytes())
    }
    pub fn parse_bytes(text: &[u8]) -> Result<Self, BindError> {
        if text.contains(&0) {
            return Err(BindError::Nul);
        }
        let mut binding = Self {
            producer: None,
            text: FixedText::default(),
            parts: [Part::default(); 512],
            count: 0,
        };
        binding
            .text
            .set_bytes(text)
            .map_err(|_| BindError::TooLong)?;
        let mut start = 0;
        while start < text.len() {
            let end = start + qa_core::text::command_separator(&text[start..], false, false);
            let mut first = start;
            let mut last = end;
            while first < last && text[first] <= b' ' {
                first += 1;
            }
            while last > first && text[last - 1] <= b' ' {
                last -= 1;
            }
            if first != last {
                binding.parts[binding.count] = Part {
                    start: first as u16,
                    end: last as u16,
                    button: text[first] == b'+',
                };
                binding.count += 1;
            }
            start = end + 1;
        }
        Ok(binding)
    }
    pub fn text(&self) -> &str {
        self.text.as_str()
    }
    pub fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }
    pub fn producer(&self) -> Option<CommandProducer> {
        self.producer
    }
}
