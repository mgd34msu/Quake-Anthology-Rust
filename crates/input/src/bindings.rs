use crate::Action;
use qa_core::text::FixedText;

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
pub fn action(name: &str) -> Option<Action> {
    ACTION_NAMES
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(name))
        .map(|entry| entry.action)
}
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
    pub action: Option<Action>,
    pub button: bool,
}
pub struct Binding {
    pub(crate) text: FixedText<1024>,
    pub(crate) parts: [Part; 512],
    pub(crate) count: usize,
}
impl Binding {
    pub fn for_action(action: Action) -> Self {
        let mut binding = Self {
            text: FixedText::default(),
            parts: [Part::default(); 512],
            count: 1,
        };
        use std::fmt::Write;
        let _ = write!(binding.text, "+{}", action_name(action));
        binding.parts[0] = Part {
            start: 0,
            end: binding.text.as_str().len() as u16,
            action: Some(action),
            button: true,
        };
        binding
    }
    pub fn parse(text: &str) -> Result<Self, BindError> {
        if text.contains('\0') {
            return Err(BindError::Nul);
        }
        let mut binding = Self {
            text: FixedText::default(),
            parts: [Part::default(); 512],
            count: 0,
        };
        binding.text.set(text).map_err(|_| BindError::TooLong)?;
        let mut start = 0;
        let mut quoted = false;
        for (at, byte) in text
            .bytes()
            .enumerate()
            .chain(std::iter::once((text.len(), b';')))
        {
            if byte == b'"' {
                quoted = !quoted;
            }
            if byte == b';' && !quoted || at == text.len() {
                let clause = text[start..at].trim_matches(|c: char| c.is_ascii() && c <= ' ');
                if !clause.is_empty() {
                    let first = clause.as_ptr() as usize - text.as_ptr() as usize;
                    let button = clause.starts_with('+');
                    binding.parts[binding.count] = Part {
                        start: first as u16,
                        end: (first + clause.len()) as u16,
                        action: clause.strip_prefix('+').and_then(action),
                        button,
                    };
                    binding.count += 1;
                }
                start = at + 1;
            }
        }
        Ok(binding)
    }
    pub fn text(&self) -> &str {
        self.text.as_str()
    }
}
