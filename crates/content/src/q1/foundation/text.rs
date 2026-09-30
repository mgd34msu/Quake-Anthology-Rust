//! Classic Q1 message catalog (`src/content/q1/foundation/text.ts`).
//!
//! Classic triggers.qc, doors.qc and Rogue native messages.
//! GPL-2.0-or-later.

use super::super::base::finales::q1_finale_text;
use super::super::base::messages::classic_obituary_text;
use super::super::missionpacks::world::finale_text::mission_finale_text;
use super::types::Q1Edition;

/// Classic Q1 message table (`classicQ1Messages`), including the
/// gold/silver key, runekey, and keycard variants.
pub const CLASSIC_Q1_MESSAGES: &[(&str, &str)] = &[
    ("$qc_found_secret", "You found a secret area!"),
    ("$qc_more_go", "There are more to go..."),
    ("$qc_three_more", "Only 3 more to go..."),
    ("$qc_two_more", "Only 2 more to go..."),
    ("$qc_one_more", "Only 1 more to go..."),
    ("$qc_sequence_completed", "Sequence completed!"),
    ("$qc_need_gold_key", "You need the gold key"),
    ("$qc_need_gold_runekey", "You need the gold runekey"),
    ("$qc_need_gold_keycard", "You need the gold keycard"),
    ("$qc_need_silver_key", "You need the silver key"),
    ("$qc_need_silver_runekey", "You need the silver runekey"),
    ("$qc_need_silver_keycard", "You need the silver keycard"),
    ("$qc_already_have_rune", "You already have a rune\n"),
    ("$qc_rune_resistance", "Earth Magic\n\nRESISTANCE"),
    ("$qc_rune_strength", "Black Magic\n\nSTRENGTH"),
    ("$qc_rune_haste", "Hell Magic\n\nHASTE"),
    ("$qc_rune_regeneration", "Edler Magic\n\nRegeneration"),
    (
        "$qc_color_games",
        "You were told you can't change teams.\nGo play color games somewhere else.\n",
    ),
    ("$qc_cannot_change_teams", "You cannot change teams.\n"),
    ("$qc_ctf_disabled", "Capture the Flag is not enabled.\n"),
    ("$qc_flag_missing", "The flag is missing!\n"),
    ("$qc_flag_at_base", "The flag is at base!\n"),
    ("$qc_flag_lying_about", "The flag is lying about!\n"),
    ("$qc_you_have_flag", "You have the flag!\n"),
    ("$qc_flag_screwed_up", "The flag is screwed up!\n"),
    ("$qc_you_have_enemy_flag", "You have the enemy flag.\n"),
    ("$qc_flag_returned", "The flag has been returned!\n"),
    ("$qc_your_flag_returned_base", "Your flag has been returned to base!\n"),
    (
        "$qc_enemy_flag_returned_base",
        "Enemy flag has been returned to base!\n",
    ),
    ("$qc_your_team_captured", "Your team captured the flag!\n"),
    ("$qc_your_flag_captured", "Your flag was captured!\n"),
    ("$qc_flag_taken", "The flag has been taken!\n"),
    ("$qc_your_flag_taken", "Your flag has been taken!\n"),
    ("$qc_enemy_killed_bonus", "Enemy flag carrier killed: {0} bonus frags\n"),
    ("$qc_enemy_killed_no_bonus", "Enemy flag carrier killed, no bonus\n"),
    ("$qc_has_token", "{0} has the tag token!\n"),
    ("$qc_lost_token", "{0} lost the tag token!\n"),
    ("$qc_got_token", "{0} got the tag token!\n"),
    ("$qc_got_quad", "You got the Quad Damage\n"),
];

/// Look up a classic message format.
#[must_use]
pub fn classic_q1_message(key: &str) -> Option<&'static str> {
    CLASSIC_Q1_MESSAGES
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, text)| *text)
}

/// Format argument for [`classic_q1_text`].
#[derive(Debug, Clone, PartialEq)]
pub enum Q1TextArg {
    /// String argument.
    Text(String),
    /// Numeric argument.
    Number(f64),
}

impl Q1TextArg {
    fn donor_string(&self) -> String {
        match self {
            Q1TextArg::Text(text) => text.clone(),
            Q1TextArg::Number(value) => donor_number_string(*value),
        }
    }
}

/// JavaScript `String(number)` rendering for message arguments.
fn donor_number_string(value: f64) -> String {
    if value == f64::INFINITY {
        return String::from("Infinity");
    }
    if value == f64::NEG_INFINITY {
        return String::from("-Infinity");
    }
    if value.is_nan() {
        return String::from("NaN");
    }
    if value == 0.0 {
        return String::from("0");
    }
    // The donor formats integer-valued counts; fall back to the
    // shortest round-trip representation otherwise.
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value:?}")
    }
}

/// Substitute `{index}` placeholders, resolving nested message keys
/// like the donor.
fn substitute(format: &str, args: &[Q1TextArg]) -> String {
    let mut output = String::new();
    let mut rest = format;
    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let index_text = &after[..close];
                let replacement = index_text
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| args.get(index))
                    .map(|argument| {
                        let text = argument.donor_string();
                        classic_q1_message(&text).unwrap_or(&text).to_string()
                    });
                match replacement {
                    Some(text) => output.push_str(&text),
                    None => {
                        output.push('{');
                        output.push_str(index_text);
                        output.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                output.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    output.push_str(rest);
    output
}

/// Format a classic Q1 message (`classicQ1Text`). Unknown keys fall
/// through the obituary, mission-finale, and episode-finale tables.
#[must_use]
pub fn classic_q1_text(text: &str, args: &[Q1TextArg]) -> String {
    let strings: Vec<String> = args.iter().map(Q1TextArg::donor_string).collect();
    let borrowed: Vec<&str> = strings.iter().map(String::as_str).collect();
    let format = classic_q1_message(text).map(str::to_string).unwrap_or_else(|| {
        q1_finale_text(
            Q1Edition::Classic,
            &mission_finale_text(Q1Edition::Classic, &classic_obituary_text(text, &borrowed)),
        )
    });
    substitute(&format, args)
}

/// Decode entity string escapes once (`ED_NewString` semantics,
/// `q1EntityString`): `\n` becomes a newline and any other escaped
/// character becomes a backslash.
#[must_use]
pub fn q1_entity_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(char) = chars.next() {
        if char == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some(_) => output.push('\\'),
                None => output.push('\\'),
            }
        } else {
            output.push(char);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_nested_keys() {
        assert_eq!(classic_q1_text("$qc_found_secret", &[]), "You found a secret area!");
        assert_eq!(
            classic_q1_text("$qc_enemy_killed_bonus", &[Q1TextArg::Number(2.0)]),
            "Enemy flag carrier killed: 2 bonus frags\n"
        );
        assert_eq!(classic_q1_text("$qc_unknown_key", &[]), "$qc_unknown_key");
    }

    #[test]
    fn entity_strings_decode_escapes() {
        assert_eq!(q1_entity_string("a\\nb\\\\c\\"), "a\nb\\c\\");
    }
}
