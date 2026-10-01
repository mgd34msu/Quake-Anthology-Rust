//! Quake III server cvar declarations.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-common-cvars.ts`
//! (`q3ServerCvarDefinitions`, `q3ServerCvarNames`, `registerQ3ServerCvars`). The protocol
//! version cites donor `src/network/q3/adapters.ts` (`Q3_PROTOCOL.version`, 68); no shared
//! protocol-identity port exists yet, so the value lives here. The trailing collision-map
//! rows cite donor `src/world/collision/q3/settings.ts` (`collisionMapCvarDefinitions`,
//! out of scope) and are asserted in tests to keep the name list honest.

use qa_core::cvar::{flags, CvarError, CvarRegistry};

/// Quake III network protocol version (donor `Q3_PROTOCOL.version`).
pub const Q3_PROTOCOL_VERSION: u32 = 68;

/// One server cvar declaration (donor `q3ServerCvarDefinitions` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ServerCvarDefinition {
    /// Variable name.
    pub name: &'static str,
    /// Default value.
    pub value: String,
    /// Flag word.
    pub flags: u32,
}

/// Server cvar declarations for a host (donor `q3ServerCvarDefinitions`).
#[must_use]
pub fn q3_server_cvar_definitions(max_clients: u32, map_name: &str) -> Vec<Q3ServerCvarDefinition> {
    let rows: &[(&str, String, u32)] = &[
        (
            "protocol",
            Q3_PROTOCOL_VERSION.to_string(),
            flags::SERVER_INFO | flags::READ_ONLY,
        ),
        ("sv_pure", "1".to_string(), flags::SYSTEM_INFO),
        ("sv_allowDownload", "0".to_string(), flags::SERVER_INFO),
        ("sv_maxRate", "0".to_string(), flags::SERVER_INFO),
        ("sv_fps", "20".to_string(), flags::NONE),
        ("sv_serverid", "0".to_string(), flags::SYSTEM_INFO | flags::READ_ONLY),
        ("sv_paks", String::new(), flags::SYSTEM_INFO | flags::READ_ONLY),
        ("sv_pakNames", String::new(), flags::SYSTEM_INFO | flags::READ_ONLY),
        (
            "sv_referencedPaks",
            String::new(),
            flags::SYSTEM_INFO | flags::READ_ONLY,
        ),
        (
            "sv_referencedPakNames",
            String::new(),
            flags::SYSTEM_INFO | flags::READ_ONLY,
        ),
        (
            "sv_maxclients",
            max_clients.to_string(),
            flags::SERVER_INFO | flags::LATCH,
        ),
        ("mapname", map_name.to_string(), flags::SERVER_INFO | flags::READ_ONLY),
        ("sv_mapname", String::new(), flags::SERVER_INFO | flags::READ_ONLY),
        ("sv_privateClients", "0".to_string(), flags::SERVER_INFO),
        ("sv_privatePassword", String::new(), flags::TEMPORARY),
        ("sv_reconnectlimit", "3".to_string(), flags::NONE),
        ("sv_minPing", "0".to_string(), flags::ARCHIVE | flags::SERVER_INFO),
        ("sv_maxPing", "0".to_string(), flags::ARCHIVE | flags::SERVER_INFO),
        ("sv_floodProtect", "1".to_string(), flags::ARCHIVE | flags::SERVER_INFO),
        ("sv_strictAuth", "1".to_string(), flags::ARCHIVE),
        ("bot_enable", "1".to_string(), flags::NONE),
        ("cm_noAreas", "0".to_string(), flags::CHEAT),
        ("cm_noCurves", "0".to_string(), flags::CHEAT),
        ("cm_playerCurveClip", "1".to_string(), flags::ARCHIVE | flags::CHEAT),
    ];
    rows.iter()
        .map(|(name, value, flags)| Q3ServerCvarDefinition {
            name,
            value: value.clone(),
            flags: *flags,
        })
        .collect()
}

/// Every declared server cvar name (donor `q3ServerCvarNames`).
pub const Q3_SERVER_CVAR_NAMES: &[&str] = &[
    "protocol",
    "sv_pure",
    "sv_allowDownload",
    "sv_maxRate",
    "sv_fps",
    "sv_serverid",
    "sv_paks",
    "sv_pakNames",
    "sv_referencedPaks",
    "sv_referencedPakNames",
    "sv_maxclients",
    "mapname",
    "sv_mapname",
    "sv_privateClients",
    "sv_privatePassword",
    "sv_reconnectlimit",
    "sv_minPing",
    "sv_maxPing",
    "sv_floodProtect",
    "sv_strictAuth",
    "bot_enable",
    "cm_noAreas",
    "cm_noCurves",
    "cm_playerCurveClip",
];

/// Register every server cvar (donor `registerQ3ServerCvars`).
pub fn register_q3_server_cvars(cvars: &mut CvarRegistry, max_clients: u32, map_name: &str) -> Result<(), CvarError> {
    for definition in q3_server_cvar_definitions(max_clients, map_name) {
        cvars.register(definition.name, &definition.value, definition.flags)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    #[test]
    fn names_match_definitions() {
        let definitions = q3_server_cvar_definitions(0, "");
        let names: Vec<&str> = definitions.iter().map(|definition| definition.name).collect();
        assert_eq!(names, Q3_SERVER_CVAR_NAMES);
    }

    #[test]
    fn settings_flow_into_definitions() {
        let definitions = q3_server_cvar_definitions(12, "q3dm1");
        let max = definitions
            .iter()
            .find(|definition| definition.name == "sv_maxclients")
            .unwrap();
        assert_eq!(max.value, "12");
        assert_eq!(max.flags, flags::SERVER_INFO | flags::LATCH);
        let map = definitions
            .iter()
            .find(|definition| definition.name == "mapname")
            .unwrap();
        assert_eq!(map.value, "q3dm1");
        let protocol = definitions
            .iter()
            .find(|definition| definition.name == "protocol")
            .unwrap();
        assert_eq!(protocol.value, "68");
    }

    #[test]
    fn register_declares_all_names() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_q3_server_cvars(&mut cvars, 8, "q3dm17").unwrap();
        for name in Q3_SERVER_CVAR_NAMES {
            assert!(cvars.get(name).is_some(), "missing {name}");
        }
        assert_eq!(cvars.variable_string("sv_maxclients"), "8");
        assert_eq!(cvars.variable_string("mapname"), "q3dm17");
        assert_eq!(cvars.variable_string("cm_playerCurveClip"), "1");
    }
}
