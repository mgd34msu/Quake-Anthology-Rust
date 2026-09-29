//! Caption overlay commands within a caller-owned safe region.
//!
//! Donor provenance: `src/ui/common/captions.ts` (`captionCommands`).
//! Captions use the same text command renderer as menus and HUD.

use qa_core::math::{vec2, vec4};

use crate::text::atlas::CapInk;
use crate::text::captions::ActiveCaption;
use crate::text::draw2d::Rect;
use crate::ui::types::{ResourceId, TextAlign, UiDrawCommand};

/// Build fill plus centered text commands for the visible caption lines.
#[must_use]
pub fn caption_commands(
    captions: &[ActiveCaption],
    area: &Rect,
    font: &ResourceId,
    scale: f32,
    measure: &dyn Fn(&str, f32) -> f32,
    cap_ink: &CapInk,
) -> Vec<UiDrawCommand> {
    if captions.is_empty() || area.width < 16.0 || area.height < 12.0 {
        return Vec::new();
    }
    let padding = 4.0;
    let width = area.width - padding * 2.0;
    let line_height = (cap_ink.height * scale).ceil() + 4.0;
    if line_height + padding * 2.0 > area.height {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for caption in captions {
        let mut text = String::new();
        if let Some(speaker) = caption.localized_speaker.as_ref() {
            text.push_str(speaker);
            text.push_str(": ");
        }
        text.push_str(&caption.localized_text);
        for paragraph in text.split('\n') {
            line.clear();
            for word in split_words(paragraph) {
                let mut candidate = line.clone();
                candidate.push_str(word);
                if measure(&candidate, scale) <= width {
                    line.push_str(word);
                    continue;
                }
                if !trim_donor(&line).is_empty() {
                    lines.push(trim_end_donor(&line).to_string());
                    line.clear();
                }
                for character in trim_start_donor(word).chars() {
                    if !line.is_empty() {
                        let mut grown = line.clone();
                        grown.push(character);
                        if measure(&grown, scale) > width {
                            lines.push(std::mem::take(&mut line));
                        }
                    }
                    line.push(character);
                }
            }
            lines.push(trim_end_donor(&line).to_string());
        }
    }
    let visible_count = (1.0f32).max(((area.height - padding * 2.0) / line_height).floor()) as usize;
    let visible_count = visible_count.min(lines.len());
    let visible = &lines[lines.len() - visible_count..];
    let height = visible.len() as f32 * line_height + padding * 2.0;
    let y = area.y + area.height - height;
    let mut commands = Vec::with_capacity(visible.len() + 1);
    commands.push(UiDrawCommand::Fill {
        rect: Rect {
            x: area.x,
            y,
            width: area.width,
            height,
        },
        color: vec4(0.0, 0.0, 0.0, 0.92),
    });
    for (index, text) in visible.iter().enumerate() {
        commands.push(UiDrawCommand::Text {
            origin: vec2(
                area.x + area.width / 2.0,
                y + padding + index as f32 * line_height - cap_ink.top * scale,
            ),
            text: text.clone(),
            font: font.clone(),
            scale,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            align: TextAlign::Center,
            shadow: true,
        });
    }
    commands
}

/// Split a paragraph into words, keeping whitespace runs as their own words.
fn split_words(paragraph: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut word_space: Option<bool> = None;
    for (index, character) in paragraph.char_indices() {
        let space = is_wrap_space(character);
        match word_space {
            None => {
                start = index;
                word_space = Some(space);
            }
            Some(current) if current == space => {}
            Some(_) => {
                words.push(&paragraph[start..index]);
                start = index;
                word_space = Some(space);
            }
        }
    }
    if word_space.is_some() {
        words.push(&paragraph[start..]);
    }
    words
}

/// Donor `\s` predicate (ECMAScript WhiteSpace plus LineTerminators).
fn is_wrap_space(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// Trim donor whitespace from both ends.
fn trim_donor(text: &str) -> &str {
    text.trim_matches(is_wrap_space)
}

/// Trim donor whitespace from the end.
fn trim_end_donor(text: &str) -> &str {
    text.trim_end_matches(is_wrap_space)
}

/// Trim donor whitespace from the start.
fn trim_start_donor(text: &str) -> &str {
    text.trim_start_matches(is_wrap_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caption(text: &str, speaker: Option<&str>) -> ActiveCaption {
        ActiveCaption {
            cue: crate::text::captions::CaptionCue {
                id: "cue".to_string(),
                start_ms: 0.0,
                duration_ms: 1000.0,
                kind: crate::text::captions::CaptionKind::Subtitle,
                speaker: None,
                text: text.to_string(),
                arguments: Vec::new(),
            },
            localized_text: text.to_string(),
            localized_speaker: speaker.map(str::to_string),
        }
    }

    fn area() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        }
    }

    fn ink() -> CapInk {
        CapInk { top: 0.0, height: 8.0 }
    }

    fn font() -> ResourceId {
        ResourceId::new("resource:test:font").unwrap()
    }

    #[test]
    fn empty_or_tiny_areas_emit_nothing() {
        let measure = |_: &str, _: f32| 0.0;
        assert!(caption_commands(&[], &area(), &font(), 1.0, &measure, &ink()).is_empty());
        let tiny = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 100.0,
        };
        let captions = [caption("hi", None)];
        assert!(caption_commands(&captions, &tiny, &font(), 1.0, &measure, &ink()).is_empty());
        let short = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 10.0,
        };
        assert!(caption_commands(&captions, &short, &font(), 1.0, &measure, &ink()).is_empty());
    }

    #[test]
    fn single_caption_matches_exact_geometry() {
        let measure = |text: &str, scale: f32| text.chars().count() as f32 * 8.0 * scale;
        let captions = [caption("hi", None)];
        let commands = caption_commands(&captions, &area(), &font(), 1.0, &measure, &ink());
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            UiDrawCommand::Fill { rect, color } => {
                assert_eq!(
                    *rect,
                    Rect {
                        x: 0.0,
                        y: 80.0,
                        width: 200.0,
                        height: 20.0
                    }
                );
                assert_eq!(*color, vec4(0.0, 0.0, 0.0, 0.92));
            }
            other => panic!("expected fill, got {other:?}"),
        }
        match &commands[1] {
            UiDrawCommand::Text {
                origin, text, align, ..
            } => {
                assert_eq!(*origin, vec2(100.0, 84.0));
                assert_eq!(text, "hi");
                assert_eq!(*align, TextAlign::Center);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn speaker_prefix_and_wrapping() {
        let measure = |text: &str, scale: f32| text.chars().count() as f32 * 8.0 * scale;
        let captions = [caption("one two three four five", Some("Bob"))];
        let narrow = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let commands = caption_commands(&captions, &narrow, &font(), 1.0, &measure, &ink());
        let texts: Vec<&str> = commands
            .iter()
            .filter_map(|command| match command {
                UiDrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["Bob: one", "two three", "four five"]);
    }

    #[test]
    fn long_words_split_by_character() {
        let measure = |text: &str, scale: f32| text.chars().count() as f32 * 8.0 * scale;
        let captions = [caption("abcdefghijklmnop", None)];
        let narrow = Rect {
            x: 0.0,
            y: 0.0,
            width: 48.0,
            height: 100.0,
        };
        let commands = caption_commands(&captions, &narrow, &font(), 1.0, &measure, &ink());
        let texts: Vec<&str> = commands
            .iter()
            .filter_map(|command| match command {
                UiDrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["abcde", "fghij", "klmno", "p"]);
    }

    #[test]
    fn visible_window_keeps_last_lines() {
        let measure = |_: &str, _: f32| 0.0;
        let captions = [caption("a\nb\nc\nd", None)];
        let short = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 40.0,
        };
        let commands = caption_commands(&captions, &short, &font(), 1.0, &measure, &ink());
        let texts: Vec<&str> = commands
            .iter()
            .filter_map(|command| match command {
                UiDrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["c", "d"]);
    }
}
