//! Q2 match selection (`src/content/composition/q2/match-selection.ts`).

use super::types::{Q2CvarSource, Q2MatchSelection};
use crate::q2::multiplayer::lmctf::types::LmctfTravel;

/// Select the Q2 match from the provider (`sourceQ2MatchSelection`).
///
/// Deathball skins and the goal limit are read eagerly; the donor holds
/// lazy cvar getters, but the port freezes them at selection time.
pub fn source_q2_match_selection(
    provider: &str,
    cvars: &Q2CvarSource,
    travel: Option<LmctfTravel>,
) -> Q2MatchSelection {
    match provider {
        "q2:lmctf" => Q2MatchSelection::Lmctf { travel },
        "q2:ctf" => Q2MatchSelection::Ctf,
        "q2:tag" => Q2MatchSelection::Tag,
        "q2:deathball" => Q2MatchSelection::Deathball {
            team1_skin: (cvars.variable_string)("dball_team1_skin"),
            team2_skin: (cvars.variable_string)("dball_team2_skin"),
            goal_limit: (cvars.variable_value)("goallimit"),
        },
        _ => Q2MatchSelection::Standard,
    }
}
