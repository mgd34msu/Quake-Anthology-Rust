//! Q2 obituaries (`src/content/q2/base/player/obituary.ts`).

use super::types::{Q2PlayerGender, Q2PlayerState};

/// Environment death messages.
const ENVIRONMENT: &[(i32, &str)] = &[
    (23, "suicides"),
    (22, "cratered"),
    (20, "was squished"),
    (17, "sank like a rock"),
    (18, "melted"),
    (19, "does a back flip into the lava"),
    (25, "blew up"),
    (26, "blew up"),
    (28, "found a way out"),
    (30, "saw the light"),
    (33, "got blasted"),
    (27, "was in the wrong place"),
    (29, "was in the wrong place"),
    (31, "was in the wrong place"),
];

/// Kill messages with attacker possessives.
const KILLS: &[(i32, &str, &str)] = &[
    (1, "was blasted by", ""),
    (2, "was gunned down by", ""),
    (3, "was blown away by", "'s super shotgun"),
    (4, "was machinegunned by", ""),
    (5, "was cut in half by", "'s chaingun"),
    (6, "was popped by", "'s grenade"),
    (7, "was shredded by", "'s shrapnel"),
    (8, "ate", "'s rocket"),
    (9, "almost dodged", "'s rocket"),
    (10, "was melted by", "'s hyperblaster"),
    (11, "was railed by", ""),
    (12, "saw the pretty lights from", "'s BFG"),
    (13, "was disintegrated by", "'s BFG blast"),
    (14, "couldn't hide from", "'s BFG"),
    (15, "caught", "'s handgrenade"),
    (16, "didn't see", "'s handgrenade"),
    (24, "feels", "'s pain"),
    (21, "tried to invade", "'s personal space"),
];

/// Obituary score recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ObituaryRecipient {
    /// Victim.
    Victim,
    /// Attacker.
    Attacker,
}

/// Format an obituary (`q2Obituary`).
pub fn q2_obituary(
    victim: &Q2PlayerState,
    attacker: Option<&Q2PlayerState>,
    suicide: bool,
    means: i32,
    deathmatch: bool,
    coop: bool,
    score: &mut dyn FnMut(Q2ObituaryRecipient, i32),
) -> String {
    let friendly = means & 0x8000000 != 0 || coop && attacker.is_some();
    let means = means & !0x8000000;
    let mut message = ENVIRONMENT
        .iter()
        .find(|(kind, _)| *kind == means)
        .map(|(_, text)| text.to_string());
    if suicide {
        let possessive = match victim.gender {
            Q2PlayerGender::Female => "her",
            Q2PlayerGender::Neutral => "its",
            Q2PlayerGender::Male => "his",
        };
        let reflexive = match victim.gender {
            Q2PlayerGender::Female => "herself",
            Q2PlayerGender::Neutral => "itself",
            Q2PlayerGender::Male => "himself",
        };
        message = Some(
            if means == 24 {
                "tried to put the pin back in".to_string()
            } else if means == 7 || means == 16 {
                format!("tripped on {possessive} own grenade")
            } else if means == 9 {
                format!("blew {reflexive} up")
            } else if means == 13 {
                "should have used a smaller gun".to_string()
            } else {
                format!("killed {reflexive}")
            },
        );
    }
    if (deathmatch || coop) && message.is_some() {
        if deathmatch {
            score(Q2ObituaryRecipient::Victim, -1);
        }
        return format!("{} {}.\n", victim.name, message.unwrap_or_default());
    }
    let kill = KILLS
        .iter()
        .find(|(kind, _, _)| *kind == means)
        .map(|(_, verb, possessive)| (verb, possessive));
    if (deathmatch || coop) && attacker.is_some() && kill.is_some() {
        if deathmatch {
            score(
                Q2ObituaryRecipient::Attacker,
                if friendly { -1 } else { 1 },
            );
        }
        let (verb, possessive) = kill.unwrap_or((&"", &""));
        return format!(
            "{} {verb} {}{possessive}\n",
            victim.name,
            attacker.map_or("", |attacker| attacker.name.as_str())
        );
    }
    if deathmatch {
        score(Q2ObituaryRecipient::Victim, -1);
    }
    format!("{} died.\n", victim.name)
}
