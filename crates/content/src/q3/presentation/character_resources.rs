//! Quake III presentation: character resources.
//!
//! Donor provenance: `src/content/q3/presentation/character-resources.ts`.

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::Product;

// ---------------------------------------------------------------------------
// character-resources.ts
// ---------------------------------------------------------------------------

/// Character media paths (`Q3_CHARACTER_SOUNDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CharacterSounds {
    /// Weapon change.
    pub select_sound: &'static str,
    /// Gib splash.
    pub gib_sound: &'static str,
    /// Teleport in.
    pub tele_in_sound: &'static str,
    /// Teleport out.
    pub tele_out_sound: &'static str,
    /// Respawn.
    pub respawn_sound: &'static str,
    /// Land.
    pub land_sound: &'static str,
    /// Water in.
    pub watr_in_sound: &'static str,
    /// Water out.
    pub watr_out_sound: &'static str,
    /// Water under.
    pub watr_un_sound: &'static str,
    /// Jump pad.
    pub jump_pad_sound: &'static str,
}

/// Character media paths.
pub const Q3_CHARACTER_SOUNDS: Q3CharacterSounds = Q3CharacterSounds {
    select_sound: "sound/weapons/change.wav",
    gib_sound: "sound/player/gibsplt1.wav",
    tele_in_sound: "sound/world/telein.wav",
    tele_out_sound: "sound/world/teleout.wav",
    respawn_sound: "sound/items/respawn1.wav",
    land_sound: "sound/player/land1.wav",
    watr_in_sound: "sound/player/watr_in.wav",
    watr_out_sound: "sound/player/watr_out.wav",
    watr_un_sound: "sound/player/watr_un.wav",
    jump_pad_sound: "sound/world/jumppad.wav",
};

/// Footstep kind (`keyof ClientMedia["footsteps"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FootstepKind {
    /// Normal.
    Normal,
    /// Boot.
    Boot,
    /// Flesh.
    Flesh,
    /// Mech.
    Mech,
    /// Energy.
    Energy,
    /// Splash.
    Splash,
    /// Metal.
    Metal,
}

/// Footstep path table (`Q3_FOOTSTEP_PATHS`).
pub const Q3_FOOTSTEP_PATHS: [(FootstepKind, &str); 7] = [
    (FootstepKind::Normal, "step"),
    (FootstepKind::Boot, "boot"),
    (FootstepKind::Flesh, "flesh"),
    (FootstepKind::Mech, "mech"),
    (FootstepKind::Energy, "energy"),
    (FootstepKind::Splash, "splash"),
    (FootstepKind::Metal, "clank"),
];

/// Custom sound fallback model (`q3CustomSoundFallback` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomSoundFallback {
    /// Sarge.
    Sarge,
    /// James.
    James,
}

impl CustomSoundFallback {
    /// Model name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sarge => "sarge",
            Self::James => "james",
        }
    }
}

/// Custom sound fallback (`q3CustomSoundFallback`).
#[must_use]
pub const fn q3_custom_sound_fallback(product: Product, team_game: bool) -> CustomSoundFallback {
    match (product, team_game) {
        (Product::Missionpack, true) => CustomSoundFallback::James,
        _ => CustomSoundFallback::Sarge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_resources() {
        assert_eq!(Q3_CHARACTER_SOUNDS.select_sound, "sound/weapons/change.wav");
        assert_eq!(Q3_FOOTSTEP_PATHS.len(), 7);
        assert_eq!(
            q3_custom_sound_fallback(Product::Missionpack, true),
            CustomSoundFallback::James
        );
        assert_eq!(
            q3_custom_sound_fallback(Product::Baseq3, true),
            CustomSoundFallback::Sarge
        );
        assert_eq!(CustomSoundFallback::Sarge.name(), "sarge");
    }
}
