//! Quake III server cvar declarations.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-common-cvars.ts`
//! (`q3ServerCvarDefinitions`, `q3ServerCvarNames`, `registerQ3ServerCvars`). The protocol
//! version cites donor `src/network/q3/adapters.ts` (`Q3_PROTOCOL.version`, 68); no shared
//! protocol-identity port exists yet, so the value lives here. The trailing collision-map
//! rows are the canonical [`COLLISION_MAP_CVAR_DEFINITIONS`] port of donor
//! `src/world/collision/q3/settings.ts`, and are asserted in tests to keep the name list
//! honest.

use qa_core::cvar::{flags, CvarError, CvarRegistry};
use qa_world::collision::q3::settings::COLLISION_MAP_CVAR_DEFINITIONS;

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
///
/// This is the one engine table: `SV_Init` serverinfo/systeminfo/server
/// vars plus the bot cvars `SV_Init` pulls in through `SV_BotInitCvars`,
/// the client capture cvars, and the `common.c` filesystem/build rows the
/// game re-registers. Administration (`rconPassword`, `sv_master*`) stays
/// in [`super::server_administration`]; game tables re-register and merge
/// flags exactly like the engine-then-VM order.
#[must_use]
pub fn q3_server_cvar_definitions(max_clients: u32, map_name: &str) -> Vec<Q3ServerCvarDefinition> {
    let server_info = flags::SERVER_INFO;
    let read_only = flags::READ_ONLY;
    let system_info = flags::SYSTEM_INFO;
    let archive = flags::ARCHIVE;
    let cheat = flags::CHEAT;
    let temporary = flags::TEMPORARY;
    let rows: &[(&str, String, u32)] = &[
        ("protocol", Q3_PROTOCOL_VERSION.to_string(), server_info | read_only),
        ("dmflags", "0".to_string(), server_info),
        ("fraglimit", "20".to_string(), server_info),
        ("timelimit", "0".to_string(), server_info),
        ("g_gametype", "0".to_string(), server_info | flags::LATCH),
        ("sv_keywords", String::new(), server_info),
        ("sv_pure", "1".to_string(), system_info),
        ("sv_allowDownload", "0".to_string(), server_info),
        ("sv_maxRate", "0".to_string(), archive | server_info),
        ("sv_fps", "20".to_string(), temporary),
        ("sv_timeout", "200".to_string(), temporary),
        ("sv_zombietime", "2".to_string(), temporary),
        ("nextmap", String::new(), temporary),
        ("sv_serverid", "0".to_string(), system_info | read_only),
        ("sv_cheats", "1".to_string(), system_info | read_only),
        ("sv_paks", String::new(), system_info | read_only),
        ("sv_pakNames", String::new(), system_info | read_only),
        ("sv_referencedPaks", String::new(), system_info | read_only),
        ("sv_referencedPakNames", String::new(), system_info | read_only),
        ("sv_maxclients", max_clients.to_string(), server_info | flags::LATCH),
        ("mapname", map_name.to_string(), server_info | read_only),
        ("sv_mapname", String::new(), server_info | read_only),
        ("sv_privateClients", "0".to_string(), server_info),
        ("sv_privatePassword", String::new(), temporary),
        ("sv_hostname", "noname".to_string(), server_info | archive),
        ("sv_reconnectlimit", "3".to_string(), flags::NONE),
        ("sv_showloss", "0".to_string(), flags::NONE),
        ("sv_padPackets", "0".to_string(), flags::NONE),
        ("sv_killserver", "0".to_string(), flags::NONE),
        ("sv_mapChecksum", String::new(), read_only),
        ("sv_lanForceRate", "1".to_string(), archive),
        ("sv_minPing", "0".to_string(), archive | server_info),
        ("sv_maxPing", "0".to_string(), archive | server_info),
        ("sv_floodProtect", "1".to_string(), archive | server_info),
        ("sv_strictAuth", "1".to_string(), archive),
        ("bot_enable", "1".to_string(), flags::NONE),
        ("bot_developer", "0".to_string(), cheat),
        ("bot_debug", "0".to_string(), cheat),
        ("bot_maxdebugpolys", "2".to_string(), flags::NONE),
        ("bot_groundonly", "1".to_string(), flags::NONE),
        ("bot_reachability", "0".to_string(), flags::NONE),
        ("bot_highlightarea", "0".to_string(), flags::NONE),
        ("bot_visualizejumppads", "0".to_string(), cheat),
        ("bot_forceclustering", "0".to_string(), flags::NONE),
        ("bot_forcereachability", "0".to_string(), flags::NONE),
        ("bot_forcewrite", "0".to_string(), flags::NONE),
        ("bot_aasoptimize", "0".to_string(), flags::NONE),
        ("bot_saveroutingcache", "0".to_string(), flags::NONE),
        ("bot_thinktime", "100".to_string(), cheat),
        ("bot_reloadcharacters", "0".to_string(), flags::NONE),
        ("bot_testichat", "0".to_string(), flags::NONE),
        ("bot_testrchat", "0".to_string(), flags::NONE),
        ("bot_testsolid", "0".to_string(), cheat),
        ("bot_testclusters", "0".to_string(), cheat),
        ("bot_fastchat", "0".to_string(), flags::NONE),
        ("bot_nochat", "0".to_string(), flags::NONE),
        ("bot_pause", "0".to_string(), cheat),
        ("bot_report", "0".to_string(), cheat),
        ("bot_grapple", "0".to_string(), flags::NONE),
        ("bot_rocketjump", "1".to_string(), flags::NONE),
        ("bot_challenge", "0".to_string(), flags::NONE),
        ("bot_minplayers", "0".to_string(), flags::NONE),
        ("bot_interbreedchar", String::new(), cheat),
        ("bot_interbreedbots", "10".to_string(), cheat),
        ("bot_interbreedcycle", "20".to_string(), cheat),
        ("bot_interbreedwrite", String::new(), cheat),
        ("bot_predictobstacles", "1".to_string(), flags::NONE),
        ("bot_memorydump", "0".to_string(), cheat),
        ("g_spSkill", "2".to_string(), flags::NONE),
        ("g_arenasFile", String::new(), flags::INIT | read_only),
        ("g_botsFile", String::new(), flags::INIT | read_only),
        ("fs_game", String::new(), flags::INIT | system_info),
        ("com_buildScript", "0".to_string(), flags::NONE),
        ("com_blood", "1".to_string(), archive),
        ("cl_avidemo", "0".to_string(), flags::NONE),
        ("cl_forceavidemo", "0".to_string(), flags::NONE),
    ];
    rows.iter()
        .map(|(name, value, flags)| Q3ServerCvarDefinition {
            name,
            value: value.clone(),
            flags: *flags,
        })
        .chain(
            COLLISION_MAP_CVAR_DEFINITIONS
                .iter()
                .map(|(name, value, flags)| Q3ServerCvarDefinition {
                    name,
                    value: value.to_string(),
                    flags: *flags,
                }),
        )
        .collect()
}

/// Every declared server cvar name (donor `q3ServerCvarNames`).
pub const Q3_SERVER_CVAR_NAMES: &[&str] = &[
    "protocol",
    "dmflags",
    "fraglimit",
    "timelimit",
    "g_gametype",
    "sv_keywords",
    "sv_pure",
    "sv_allowDownload",
    "sv_maxRate",
    "sv_fps",
    "sv_timeout",
    "sv_zombietime",
    "nextmap",
    "sv_serverid",
    "sv_cheats",
    "sv_paks",
    "sv_pakNames",
    "sv_referencedPaks",
    "sv_referencedPakNames",
    "sv_maxclients",
    "mapname",
    "sv_mapname",
    "sv_privateClients",
    "sv_privatePassword",
    "sv_hostname",
    "sv_reconnectlimit",
    "sv_showloss",
    "sv_padPackets",
    "sv_killserver",
    "sv_mapChecksum",
    "sv_lanForceRate",
    "sv_minPing",
    "sv_maxPing",
    "sv_floodProtect",
    "sv_strictAuth",
    "bot_enable",
    "bot_developer",
    "bot_debug",
    "bot_maxdebugpolys",
    "bot_groundonly",
    "bot_reachability",
    "bot_highlightarea",
    "bot_visualizejumppads",
    "bot_forceclustering",
    "bot_forcereachability",
    "bot_forcewrite",
    "bot_aasoptimize",
    "bot_saveroutingcache",
    "bot_thinktime",
    "bot_reloadcharacters",
    "bot_testichat",
    "bot_testrchat",
    "bot_testsolid",
    "bot_testclusters",
    "bot_fastchat",
    "bot_nochat",
    "bot_pause",
    "bot_report",
    "bot_grapple",
    "bot_rocketjump",
    "bot_challenge",
    "bot_minplayers",
    "bot_interbreedchar",
    "bot_interbreedbots",
    "bot_interbreedcycle",
    "bot_interbreedwrite",
    "bot_predictobstacles",
    "bot_memorydump",
    "g_spSkill",
    "g_arenasFile",
    "g_botsFile",
    "fs_game",
    "com_buildScript",
    "com_blood",
    "cl_avidemo",
    "cl_forceavidemo",
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
    fn engine_rows_match_source() {
        let definitions = q3_server_cvar_definitions(8, "q3dm17");
        let find = |name| definitions.iter().find(|definition| definition.name == name).unwrap();
        let cheats = find("sv_cheats");
        assert_eq!(cheats.value, "1");
        assert_eq!(cheats.flags, flags::SYSTEM_INFO | flags::READ_ONLY);
        assert_eq!(find("sv_maxRate").flags, flags::ARCHIVE | flags::SERVER_INFO);
        assert_eq!(find("sv_fps").flags, flags::TEMPORARY);
        assert_eq!(find("bot_pause").flags, flags::CHEAT);
        assert_eq!(find("bot_rocketjump").value, "1");
        assert_eq!(find("bot_interbreedbots").value, "10");
        assert_eq!(find("g_spSkill").value, "2");
        assert_eq!(find("fs_game").flags, flags::INIT | flags::SYSTEM_INFO);
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
