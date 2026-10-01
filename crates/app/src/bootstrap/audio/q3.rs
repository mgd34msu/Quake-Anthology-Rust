//! Quake III seat audio frames and voice fallback.
//!
//! Port of donor `src/app/bootstrap/audio/q3.ts`
//! (`Q3SeatAudioOperation`, `Q3SeatAudioFrame`, `q3VoiceFallback`).

use qa_client::audio::types::{LoopSound, PlaySound};
use qa_content::catalog::{CatalogError, InstalledCatalog};
use qa_content::contract::ContentId;
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::presentation::character_resources::{q3_custom_sound_fallback, CustomSoundFallback};
use qa_core::identity::{ActorId, ProviderId, SeatId};
use qa_core::math::Vec3;

/// One seat-scoped Quake III audio operation.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3SeatAudioOperation {
    /// Play a one-shot sound.
    Play {
        /// Sound to play.
        sound: PlaySound,
    },
    /// Start or refresh a looping sound.
    Loop {
        /// Sound to loop.
        sound: LoopSound,
    },
    /// Move a loop voice to a new origin.
    Position {
        /// Owning actor.
        actor: ActorId,
        /// Voice origin.
        origin: Vec3,
    },
    /// Stop the loop voice owned by an actor.
    StopLoop {
        /// Owning actor.
        actor: ActorId,
    },
    /// Clear loop voices.
    ClearLoops {
        /// Whether every voice dies, not just owned ones.
        kill_all: bool,
    },
    /// Release the seat audio owner.
    ReleaseOwner,
}

/// Seat-scoped Quake III audio frame for one content identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SeatAudioFrame {
    /// Content the frame was mixed for.
    pub content: ContentId,
    /// Receiving seat.
    pub seat: SeatId,
    /// Presenting provider, when owned.
    pub owner: Option<ProviderId>,
    /// Ordered operations for the frame.
    pub operations: Vec<Q3SeatAudioOperation>,
}

/// Character/client-code edition follows the selected provider, including
/// inherited mods.
pub fn q3_voice_fallback(
    catalog: &InstalledCatalog,
    content: &ContentId,
    team_game: bool,
) -> Result<CustomSoundFallback, CatalogError> {
    let mut product = catalog.product(content.as_str())?;
    loop {
        if product.expectation.campaign == "missionpack" {
            return Ok(q3_custom_sound_fallback(Product::Missionpack, team_game));
        }
        match product.expectation.base_product.as_deref() {
            None => return Ok(q3_custom_sound_fallback(Product::Baseq3, team_game)),
            Some(base) => {
                product = catalog.product(base)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::GameFamily;

    fn product(id: &str, campaign: &str, base_product: Option<&str>) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family: GameFamily::Q3,
                edition: "classic".to_string(),
                campaign: campaign.to_string(),
                title: String::new(),
                content_directory: String::new(),
                base_product: base_product.map(str::to_string),
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            String::new(),
            vec![
                product("q3-classic-baseq3", "baseq3", None),
                product("q3-classic-missionpack", "missionpack", Some("q3-classic-baseq3")),
                product("q3-classic-inherited", "free-for-all", Some("q3-classic-missionpack")),
                product("q3-classic-standalone", "free-for-all", Some("q3-classic-baseq3")),
            ],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
    }

    #[test]
    fn voice_follows_provider_edition() {
        let catalog = catalog();
        assert_eq!(
            q3_voice_fallback(&catalog, &ContentId("q3-classic-baseq3".to_string()), true).unwrap(),
            CustomSoundFallback::Sarge
        );
        assert_eq!(
            q3_voice_fallback(&catalog, &ContentId("q3-classic-missionpack".to_string()), true).unwrap(),
            CustomSoundFallback::James
        );
        assert_eq!(
            q3_voice_fallback(&catalog, &ContentId("q3-classic-missionpack".to_string()), false).unwrap(),
            CustomSoundFallback::Sarge
        );
    }

    #[test]
    fn voice_follows_inherited_mods() {
        let catalog = catalog();
        assert_eq!(
            q3_voice_fallback(&catalog, &ContentId("q3-classic-inherited".to_string()), true).unwrap(),
            CustomSoundFallback::James
        );
        assert_eq!(
            q3_voice_fallback(&catalog, &ContentId("q3-classic-standalone".to_string()), true).unwrap(),
            CustomSoundFallback::Sarge
        );
        assert!(q3_voice_fallback(&catalog, &ContentId("q3-classic-missing".to_string()), true).is_err());
    }
}
