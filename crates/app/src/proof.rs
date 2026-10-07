use qa_platform::Window;
use std::time::Duration;

enum Input {
    Key { name: String, down: bool },
    Mouse(i32, i32),
    Text(String),
}

pub struct Script {
    events: Vec<(Duration, Input)>,
    next: usize,
}

impl Script {
    pub fn load(path: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut events = Vec::new();
        for line in content
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        {
            let mut fields = line.split_whitespace();
            let ms: u64 = fields
                .next()
                .ok_or("missing event time")?
                .parse()
                .map_err(|_| "invalid event time")?;
            let kind = fields.next().ok_or("missing event kind")?;
            let input = match kind {
                "key_down" | "key_up" => Input::Key {
                    name: fields.next().ok_or("missing key")?.into(),
                    down: kind == "key_down",
                },
                "mouse" => Input::Mouse(
                    fields
                        .next()
                        .ok_or("missing dx")?
                        .parse()
                        .map_err(|_| "invalid dx")?,
                    fields
                        .next()
                        .ok_or("missing dy")?
                        .parse()
                        .map_err(|_| "invalid dy")?,
                ),
                "text" => Input::Text(fields.collect::<Vec<_>>().join(" ")),
                _ => return Err("unknown script event".into()),
            };
            events.push((Duration::from_millis(ms), input));
        }
        events.sort_by_key(|event| event.0);
        Ok(Self { events, next: 0 })
    }

    pub fn inject_due(&mut self, elapsed: Duration, window: &mut Window) {
        while let Some((time, input)) = self.events.get(self.next) {
            if *time > elapsed {
                break;
            }
            let result = match input {
                Input::Key { name, down } => window.inject_key(name, *down),
                Input::Mouse(dx, dy) => window.inject_mouse(*dx, *dy),
                Input::Text(text) => window.inject_text(text),
            };
            if let Err(message) = result {
                qa_console::logger::error(&message);
            }
            self.next += 1;
        }
    }
}
