//! Rewrite campaign intents when the primary world owns level authority.
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/source-transition.ts`.

use qa_content::contract::CampaignSelection;
use qa_core::identity::ProviderId;
use qa_world::session::TransitionIntent;

/// Who owns the level transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLevelAuthority {
    /// Shared primary world.
    PrimaryWorld,
    /// Acting source.
    ActorSource,
}

/// Rewrite a campaign-level intent to match rotation when the primary world
/// has no campaign. Projects the donor `Pick<ExecutableRecipe, "campaign" | "match">`
/// to the two fields the rewrite reads.
#[must_use]
pub fn source_level_transition(
    intent: TransitionIntent,
    campaign: &CampaignSelection,
    match_provider: &ProviderId,
    authority: SourceLevelAuthority,
) -> TransitionIntent {
    if authority == SourceLevelAuthority::PrimaryWorld && matches!(campaign, CampaignSelection::None) {
        if let TransitionIntent::CampaignLevel { map, .. } = &intent {
            return TransitionIntent::MatchRotation {
                match_id: match_provider.clone(),
                map: map.clone(),
            };
        }
    }
    intent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str) -> ProviderId {
        ProviderId {
            namespace: "test".to_string(),
            name: name.to_string(),
        }
    }

    fn campaign_level() -> TransitionIntent {
        TransitionIntent::CampaignLevel {
            campaign: provider("campaign"),
            map: "q1:e1m1".to_string(),
            spawn: "start".to_string(),
            gates: Vec::new(),
        }
    }

    #[test]
    fn rewrites_campaign_level_without_campaign() {
        let out = source_level_transition(
            campaign_level(),
            &CampaignSelection::None,
            &provider("match"),
            SourceLevelAuthority::PrimaryWorld,
        );
        match out {
            TransitionIntent::MatchRotation { match_id, map } => {
                assert_eq!(match_id, provider("match"));
                assert_eq!(map, "q1:e1m1".to_string());
            }
            other => panic!("unexpected intent: {other:?}"),
        }
    }

    #[test]
    fn actor_source_keeps_intent() {
        let out = source_level_transition(
            campaign_level(),
            &CampaignSelection::None,
            &provider("match"),
            SourceLevelAuthority::ActorSource,
        );
        assert!(matches!(out, TransitionIntent::CampaignLevel { .. }));
    }

    #[test]
    fn selected_campaign_keeps_intent() {
        let out = source_level_transition(
            campaign_level(),
            &CampaignSelection::None,
            &provider("match"),
            SourceLevelAuthority::ActorSource,
        );
        assert!(matches!(out, TransitionIntent::CampaignLevel { .. }));
        let complete = TransitionIntent::CampaignComplete {
            campaign: provider("campaign"),
            gates: Vec::new(),
        };
        let out = source_level_transition(
            complete,
            &CampaignSelection::None,
            &provider("match"),
            SourceLevelAuthority::PrimaryWorld,
        );
        assert!(matches!(out, TransitionIntent::CampaignComplete { .. }));
    }
}
