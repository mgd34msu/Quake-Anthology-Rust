//! Quake II configstring application layout.
//!
//! Port of `src/app/bootstrap/network/q2-layout.ts` (`q2ApplicationLayout`).
//! The donor takes a `Q2ProtocolIdentity`; the shared Rust identity is the
//! wider [`ProtocolIdentity`](qa_net::protocol::ProtocolIdentity), so
//! non-Quake-II identities are rejected with [`Q2LayoutError::NotQuake2`].
//! Every Quake II identity maps exactly as the donor's if/else does,
//! including protocol 4038 (`Q2PrivateClassic`) using the classic layout.

use qa_net::protocol::ProtocolIdentity;
use thiserror::Error;

/// Quake II configstring application layout (`Q2ApplicationLayout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ApplicationLayout {
    /// First model configstring.
    pub models: u32,
    /// First sound configstring.
    pub sounds: u32,
    /// First image configstring.
    pub images: u32,
    /// First light configstring.
    pub lights: u32,
    /// First item configstring.
    pub items: u32,
    /// First player-skin configstring.
    pub player_skins: u32,
    /// Maximum models.
    pub max_models: u32,
    /// Maximum sounds.
    pub max_sounds: u32,
    /// Maximum images.
    pub max_images: u32,
    /// Maximum configstrings.
    pub max_config_strings: u32,
    /// Map checksum configstring.
    pub map_checksum: u32,
    /// Max clients configstring.
    pub max_clients: u32,
    /// Air accelerate configstring.
    pub air_accelerate: u32,
    /// N64 physics configstring, when the layout has one.
    pub n64_physics: Option<u32>,
}

/// Error for Quake II application layout selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Q2LayoutError {
    /// A non-Quake-II protocol identity was supplied.
    #[error("Q2 application layout requires a Quake II protocol identity")]
    NotQuake2,
}

/// Configstring layout follows game API/limits; protocol 4038 intentionally
/// uses the classic game layout.
pub fn q2_application_layout(protocol: ProtocolIdentity) -> Result<Q2ApplicationLayout, Q2LayoutError> {
    match protocol {
        ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2Kex | ProtocolIdentity::Q2KexDemo => {
            Ok(Q2ApplicationLayout {
                models: 62,
                sounds: 8254,
                images: 10302,
                lights: 10814,
                items: 11326,
                player_skins: 11582,
                max_models: 8192,
                max_sounds: 2048,
                max_images: 512,
                max_config_strings: 12448,
                map_checksum: 61,
                max_clients: 60,
                air_accelerate: 59,
                n64_physics: Some(12103),
            })
        }
        ProtocolIdentity::Q2Classic
        | ProtocolIdentity::Q2R1q2 { .. }
        | ProtocolIdentity::Q2Q2pro { .. }
        | ProtocolIdentity::Q2PrivateClassic => Ok(Q2ApplicationLayout {
            models: 32,
            sounds: 288,
            images: 544,
            lights: 800,
            items: 1056,
            player_skins: 1312,
            max_models: 256,
            max_sounds: 256,
            max_images: 256,
            max_config_strings: 2080,
            map_checksum: 31,
            max_clients: 30,
            air_accelerate: 29,
            n64_physics: None,
        }),
        _ => Err(Q2LayoutError::NotQuake2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_layout_matches_donor_values() {
        let layout = q2_application_layout(ProtocolIdentity::Q2Classic).unwrap();
        assert_eq!(
            layout,
            Q2ApplicationLayout {
                models: 32,
                sounds: 288,
                images: 544,
                lights: 800,
                items: 1056,
                player_skins: 1312,
                max_models: 256,
                max_sounds: 256,
                max_images: 256,
                max_config_strings: 2080,
                map_checksum: 31,
                max_clients: 30,
                air_accelerate: 29,
                n64_physics: None,
            }
        );
    }

    #[test]
    fn rerelease_family_uses_extended_layout() {
        for protocol in [
            ProtocolIdentity::Q2Rerelease,
            ProtocolIdentity::Q2Kex,
            ProtocolIdentity::Q2KexDemo,
        ] {
            let layout = q2_application_layout(protocol).unwrap();
            assert_eq!(layout.models, 62);
            assert_eq!(layout.sounds, 8254);
            assert_eq!(layout.images, 10302);
            assert_eq!(layout.lights, 10814);
            assert_eq!(layout.items, 11326);
            assert_eq!(layout.player_skins, 11582);
            assert_eq!(layout.max_models, 8192);
            assert_eq!(layout.max_sounds, 2048);
            assert_eq!(layout.max_images, 512);
            assert_eq!(layout.max_config_strings, 12448);
            assert_eq!(layout.map_checksum, 61);
            assert_eq!(layout.max_clients, 60);
            assert_eq!(layout.air_accelerate, 59);
            assert_eq!(layout.n64_physics, Some(12103));
        }
    }

    #[test]
    fn protocol_4038_uses_classic_layout() {
        let layout = q2_application_layout(ProtocolIdentity::Q2PrivateClassic).unwrap();
        assert_eq!(layout.max_config_strings, 2080);
        assert_eq!(layout.n64_physics, None);
    }

    #[test]
    fn r1q2_and_q2pro_use_classic_layout() {
        for protocol in [
            ProtocolIdentity::Q2R1q2 { revision: 1904 },
            ProtocolIdentity::Q2Q2pro { revision: 1026 },
        ] {
            let layout = q2_application_layout(protocol).unwrap();
            assert_eq!(layout.models, 32);
            assert_eq!(layout.max_config_strings, 2080);
        }
    }

    #[test]
    fn non_q2_identities_are_rejected() {
        for protocol in [
            ProtocolIdentity::Q1Netquake,
            ProtocolIdentity::Q1Fitzquake,
            ProtocolIdentity::Q1Rmq { flags: 0 },
            ProtocolIdentity::Q1Quakeworld,
            ProtocolIdentity::Q1QuakeworldWide { flags: 0 },
            ProtocolIdentity::Q3,
        ] {
            assert_eq!(q2_application_layout(protocol), Err(Q2LayoutError::NotQuake2));
        }
        assert_eq!(
            Q2LayoutError::NotQuake2.to_string(),
            "Q2 application layout requires a Quake II protocol identity"
        );
    }
}
