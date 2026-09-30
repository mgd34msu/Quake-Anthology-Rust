//! Quake III presentation: config.
//!
//! Donor provenance: `src/content/q3/presentation/config.ts`.

use qa_core::cvar::{flags as cvar_flags, CvarRegistry, CvarSnapshot};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::numeric::game_atoi;
use crate::q3::base::shared::definitions::*;
use crate::q3::presentation::client_info::ClientInfoStore;
use crate::q3::presentation::hud::Shared;
use crate::q3::presentation::hud_corners::*;
use crate::q3::presentation::state::*;

/// Maximum clients for config reload.
pub(crate) const CONFIG_MAX_CLIENTS: i32 = 64;

/// Maximum cvar value string.
pub(crate) const MAX_CVAR_VALUE_STRING: usize = 256;

/// Maximum token characters.
pub(crate) const MAX_TOKEN_CHARS: usize = 1024;

/// VM cvar symbol (`ClientVmCvarSymbol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientVmCvarSymbol {
    /// cg_ignore.
    CgIgnore,
    /// cg_autoswitch.
    CgAutoswitch,
    /// cg_drawGun.
    CgDrawGun,
    /// cg_zoomFov.
    CgZoomFov,
    /// cg_fov.
    CgFov,
    /// cg_viewsize.
    CgViewsize,
    /// cg_stereoSeparation.
    CgStereoSeparation,
    /// cg_shadows.
    CgShadows,
    /// cg_gibs.
    CgGibs,
    /// cg_draw2D.
    CgDraw2d,
    /// cg_drawStatus.
    CgDrawStatus,
    /// cg_drawTimer.
    CgDrawTimer,
    /// cg_drawFPS.
    CgDrawFps,
    /// cg_drawSnapshot.
    CgDrawSnapshot,
    /// cg_draw3dIcons.
    CgDraw3dIcons,
    /// cg_drawIcons.
    CgDrawIcons,
    /// cg_drawAmmoWarning.
    CgDrawAmmoWarning,
    /// cg_drawAttacker.
    CgDrawAttacker,
    /// cg_drawCrosshair.
    CgDrawCrosshair,
    /// cg_drawCrosshairNames.
    CgDrawCrosshairNames,
    /// cg_drawRewards.
    CgDrawRewards,
    /// cg_crosshairSize.
    CgCrosshairSize,
    /// cg_crosshairHealth.
    CgCrosshairHealth,
    /// cg_crosshairX.
    CgCrosshairX,
    /// cg_crosshairY.
    CgCrosshairY,
    /// cg_brassTime.
    CgBrassTime,
    /// cg_simpleItems.
    CgSimpleItems,
    /// cg_addMarks.
    CgAddMarks,
    /// cg_lagometer.
    CgLagometer,
    /// cg_railTrailTime.
    CgRailTrailTime,
    /// cg_gun_x.
    CgGunX,
    /// cg_gun_y.
    CgGunY,
    /// cg_gun_z.
    CgGunZ,
    /// cg_centertime.
    CgCentertime,
    /// cg_runpitch.
    CgRunpitch,
    /// cg_runroll.
    CgRunroll,
    /// cg_bobup.
    CgBobup,
    /// cg_bobpitch.
    CgBobpitch,
    /// cg_bobroll.
    CgBobroll,
    /// cg_swingSpeed.
    CgSwingSpeed,
    /// cg_animSpeed.
    CgAnimSpeed,
    /// cg_debugAnim.
    CgDebugAnim,
    /// cg_debugPosition.
    CgDebugPosition,
    /// cg_debugEvents.
    CgDebugEvents,
    /// cg_errorDecay.
    CgErrorDecay,
    /// cg_nopredict.
    CgNopredict,
    /// cg_noPlayerAnims.
    CgNoPlayerAnims,
    /// cg_showmiss.
    CgShowmiss,
    /// cg_footsteps.
    CgFootsteps,
    /// cg_tracerChance.
    CgTracerChance,
    /// cg_tracerWidth.
    CgTracerWidth,
    /// cg_tracerLength.
    CgTracerLength,
    /// cg_thirdPersonRange.
    CgThirdPersonRange,
    /// cg_thirdPersonAngle.
    CgThirdPersonAngle,
    /// cg_thirdPerson.
    CgThirdPerson,
    /// cg_teamChatTime.
    CgTeamChatTime,
    /// cg_teamChatHeight.
    CgTeamChatHeight,
    /// cg_forceModel.
    CgForceModel,
    /// cg_predictItems.
    CgPredictItems,
    /// cg_deferPlayers.
    CgDeferPlayers,
    /// cg_drawTeamOverlay.
    CgDrawTeamOverlay,
    /// cg_teamOverlayUserinfo.
    CgTeamOverlayUserinfo,
    /// cg_stats.
    CgStats,
    /// cg_drawFriend.
    CgDrawFriend,
    /// cg_teamChatsOnly.
    CgTeamChatsOnly,
    /// cg_noVoiceChats.
    CgNoVoiceChats,
    /// cg_noVoiceText.
    CgNoVoiceText,
    /// cg_buildScript.
    CgBuildScript,
    /// cg_paused.
    CgPaused,
    /// cg_blood.
    CgBlood,
    /// cg_synchronousClients.
    CgSynchronousClients,
    /// cg_redTeamName.
    CgRedTeamName,
    /// cg_blueTeamName.
    CgBlueTeamName,
    /// cg_currentSelectedPlayer.
    CgCurrentSelectedPlayer,
    /// cg_currentSelectedPlayerName.
    CgCurrentSelectedPlayerName,
    /// cg_singlePlayer.
    CgSinglePlayer,
    /// cg_enableDust.
    CgEnableDust,
    /// cg_enableBreath.
    CgEnableBreath,
    /// cg_singlePlayerActive.
    CgSinglePlayerActive,
    /// cg_recordSPDemo.
    CgRecordSpDemo,
    /// cg_recordSPDemoName.
    CgRecordSpDemoName,
    /// cg_obeliskRespawnDelay.
    CgObeliskRespawnDelay,
    /// cg_hudFiles.
    CgHudFiles,
    /// cg_cameraOrbit.
    CgCameraOrbit,
    /// cg_cameraOrbitDelay.
    CgCameraOrbitDelay,
    /// cg_timescaleFadeEnd.
    CgTimescaleFadeEnd,
    /// cg_timescaleFadeSpeed.
    CgTimescaleFadeSpeed,
    /// cg_timescale.
    CgTimescale,
    /// cg_scorePlum.
    CgScorePlum,
    /// cg_smoothClients.
    CgSmoothClients,
    /// cg_cameraMode.
    CgCameraMode,
    /// pmove_fixed.
    PmoveFixed,
    /// pmove_msec.
    PmoveMsec,
    /// cg_noTaunt.
    CgNoTaunt,
    /// cg_noProjectileTrail.
    CgNoProjectileTrail,
    /// cg_smallFont.
    CgSmallFont,
    /// cg_bigFont.
    CgBigFont,
    /// cg_oldRail.
    CgOldRail,
    /// cg_oldRocket.
    CgOldRocket,
    /// cg_oldPlasma.
    CgOldPlasma,
    /// cg_trueLightning.
    CgTrueLightning,
}

impl ClientVmCvarSymbol {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CgIgnore => "cg_ignore",
            Self::CgAutoswitch => "cg_autoswitch",
            Self::CgDrawGun => "cg_drawGun",
            Self::CgZoomFov => "cg_zoomFov",
            Self::CgFov => "cg_fov",
            Self::CgViewsize => "cg_viewsize",
            Self::CgStereoSeparation => "cg_stereoSeparation",
            Self::CgShadows => "cg_shadows",
            Self::CgGibs => "cg_gibs",
            Self::CgDraw2d => "cg_draw2D",
            Self::CgDrawStatus => "cg_drawStatus",
            Self::CgDrawTimer => "cg_drawTimer",
            Self::CgDrawFps => "cg_drawFPS",
            Self::CgDrawSnapshot => "cg_drawSnapshot",
            Self::CgDraw3dIcons => "cg_draw3dIcons",
            Self::CgDrawIcons => "cg_drawIcons",
            Self::CgDrawAmmoWarning => "cg_drawAmmoWarning",
            Self::CgDrawAttacker => "cg_drawAttacker",
            Self::CgDrawCrosshair => "cg_drawCrosshair",
            Self::CgDrawCrosshairNames => "cg_drawCrosshairNames",
            Self::CgDrawRewards => "cg_drawRewards",
            Self::CgCrosshairSize => "cg_crosshairSize",
            Self::CgCrosshairHealth => "cg_crosshairHealth",
            Self::CgCrosshairX => "cg_crosshairX",
            Self::CgCrosshairY => "cg_crosshairY",
            Self::CgBrassTime => "cg_brassTime",
            Self::CgSimpleItems => "cg_simpleItems",
            Self::CgAddMarks => "cg_addMarks",
            Self::CgLagometer => "cg_lagometer",
            Self::CgRailTrailTime => "cg_railTrailTime",
            Self::CgGunX => "cg_gun_x",
            Self::CgGunY => "cg_gun_y",
            Self::CgGunZ => "cg_gun_z",
            Self::CgCentertime => "cg_centertime",
            Self::CgRunpitch => "cg_runpitch",
            Self::CgRunroll => "cg_runroll",
            Self::CgBobup => "cg_bobup",
            Self::CgBobpitch => "cg_bobpitch",
            Self::CgBobroll => "cg_bobroll",
            Self::CgSwingSpeed => "cg_swingSpeed",
            Self::CgAnimSpeed => "cg_animSpeed",
            Self::CgDebugAnim => "cg_debugAnim",
            Self::CgDebugPosition => "cg_debugPosition",
            Self::CgDebugEvents => "cg_debugEvents",
            Self::CgErrorDecay => "cg_errorDecay",
            Self::CgNopredict => "cg_nopredict",
            Self::CgNoPlayerAnims => "cg_noPlayerAnims",
            Self::CgShowmiss => "cg_showmiss",
            Self::CgFootsteps => "cg_footsteps",
            Self::CgTracerChance => "cg_tracerChance",
            Self::CgTracerWidth => "cg_tracerWidth",
            Self::CgTracerLength => "cg_tracerLength",
            Self::CgThirdPersonRange => "cg_thirdPersonRange",
            Self::CgThirdPersonAngle => "cg_thirdPersonAngle",
            Self::CgThirdPerson => "cg_thirdPerson",
            Self::CgTeamChatTime => "cg_teamChatTime",
            Self::CgTeamChatHeight => "cg_teamChatHeight",
            Self::CgForceModel => "cg_forceModel",
            Self::CgPredictItems => "cg_predictItems",
            Self::CgDeferPlayers => "cg_deferPlayers",
            Self::CgDrawTeamOverlay => "cg_drawTeamOverlay",
            Self::CgTeamOverlayUserinfo => "cg_teamOverlayUserinfo",
            Self::CgStats => "cg_stats",
            Self::CgDrawFriend => "cg_drawFriend",
            Self::CgTeamChatsOnly => "cg_teamChatsOnly",
            Self::CgNoVoiceChats => "cg_noVoiceChats",
            Self::CgNoVoiceText => "cg_noVoiceText",
            Self::CgBuildScript => "cg_buildScript",
            Self::CgPaused => "cg_paused",
            Self::CgBlood => "cg_blood",
            Self::CgSynchronousClients => "cg_synchronousClients",
            Self::CgRedTeamName => "cg_redTeamName",
            Self::CgBlueTeamName => "cg_blueTeamName",
            Self::CgCurrentSelectedPlayer => "cg_currentSelectedPlayer",
            Self::CgCurrentSelectedPlayerName => "cg_currentSelectedPlayerName",
            Self::CgSinglePlayer => "cg_singlePlayer",
            Self::CgEnableDust => "cg_enableDust",
            Self::CgEnableBreath => "cg_enableBreath",
            Self::CgSinglePlayerActive => "cg_singlePlayerActive",
            Self::CgRecordSpDemo => "cg_recordSPDemo",
            Self::CgRecordSpDemoName => "cg_recordSPDemoName",
            Self::CgObeliskRespawnDelay => "cg_obeliskRespawnDelay",
            Self::CgHudFiles => "cg_hudFiles",
            Self::CgCameraOrbit => "cg_cameraOrbit",
            Self::CgCameraOrbitDelay => "cg_cameraOrbitDelay",
            Self::CgTimescaleFadeEnd => "cg_timescaleFadeEnd",
            Self::CgTimescaleFadeSpeed => "cg_timescaleFadeSpeed",
            Self::CgTimescale => "cg_timescale",
            Self::CgScorePlum => "cg_scorePlum",
            Self::CgSmoothClients => "cg_smoothClients",
            Self::CgCameraMode => "cg_cameraMode",
            Self::PmoveFixed => "pmove_fixed",
            Self::PmoveMsec => "pmove_msec",
            Self::CgNoTaunt => "cg_noTaunt",
            Self::CgNoProjectileTrail => "cg_noProjectileTrail",
            Self::CgSmallFont => "cg_smallFont",
            Self::CgBigFont => "cg_bigFont",
            Self::CgOldRail => "cg_oldRail",
            Self::CgOldRocket => "cg_oldRocket",
            Self::CgOldPlasma => "cg_oldPlasma",
            Self::CgTrueLightning => "cg_trueLightning",
        }
    }
}

/// Cvar definition (`CvarDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarDefinition {
    /// Symbol.
    pub symbol: ClientVmCvarSymbol,
    /// Engine name.
    pub name: String,
    /// Default value.
    pub default_value: String,
    /// Flags.
    pub flags: u32,
}

/// Build a definition (`cv`).
pub(crate) fn cv(symbol: ClientVmCvarSymbol, name: &str, default_value: &str, flags: u32) -> CvarDefinition {
    CvarDefinition {
        symbol,
        name: name.to_string(),
        default_value: default_value.to_string(),
        flags,
    }
}

/// Cvar table (`cvarTable`).
#[must_use]
pub fn cvar_table(product: Product) -> Vec<CvarDefinition> {
    use ClientVmCvarSymbol as S;
    let a = cvar_flags::ARCHIVE;
    let c = cvar_flags::CHEAT;
    let r = cvar_flags::READ_ONLY;
    let u = cvar_flags::USER_INFO;
    let s = cvar_flags::SERVER_INFO;
    let n = cvar_flags::NONE;
    let mut table = vec![
        cv(S::CgIgnore, "cg_ignore", "0", n),
        cv(S::CgAutoswitch, "cg_autoswitch", "1", a),
        cv(S::CgDrawGun, "cg_drawGun", "1", a),
        cv(S::CgZoomFov, "cg_zoomfov", "22.5", a),
        cv(S::CgFov, "cg_fov", "90", a),
        cv(S::CgViewsize, "cg_viewsize", "100", a),
        cv(S::CgStereoSeparation, "cg_stereoSeparation", "0.4", a),
        cv(S::CgShadows, "cg_shadows", "1", a),
        cv(S::CgGibs, "cg_gibs", "1", a),
        cv(S::CgDraw2d, "cg_draw2D", "1", a),
        cv(S::CgDrawStatus, "cg_drawStatus", "1", a),
        cv(S::CgDrawTimer, "cg_drawTimer", "0", a),
        cv(S::CgDrawFps, "cg_drawFPS", "0", a),
        cv(S::CgDrawSnapshot, "cg_drawSnapshot", "0", a),
        cv(S::CgDraw3dIcons, "cg_draw3dIcons", "1", a),
        cv(S::CgDrawIcons, "cg_drawIcons", "1", a),
        cv(S::CgDrawAmmoWarning, "cg_drawAmmoWarning", "1", a),
        cv(S::CgDrawAttacker, "cg_drawAttacker", "1", a),
        cv(S::CgDrawCrosshair, "cg_drawCrosshair", "4", a),
        cv(S::CgDrawCrosshairNames, "cg_drawCrosshairNames", "1", a),
        cv(S::CgDrawRewards, "cg_drawRewards", "1", a),
        cv(S::CgCrosshairSize, "cg_crosshairSize", "24", a),
        cv(S::CgCrosshairHealth, "cg_crosshairHealth", "1", a),
        cv(S::CgCrosshairX, "cg_crosshairX", "0", a),
        cv(S::CgCrosshairY, "cg_crosshairY", "0", a),
        cv(S::CgBrassTime, "cg_brassTime", "2500", a),
        cv(S::CgSimpleItems, "cg_simpleItems", "0", a),
        cv(S::CgAddMarks, "cg_marks", "1", a),
        cv(S::CgLagometer, "cg_lagometer", "1", a),
        cv(S::CgRailTrailTime, "cg_railTrailTime", "400", a),
        cv(S::CgGunX, "cg_gunX", "0", c),
        cv(S::CgGunY, "cg_gunY", "0", c),
        cv(S::CgGunZ, "cg_gunZ", "0", c),
        cv(S::CgCentertime, "cg_centertime", "3", c),
        cv(S::CgRunpitch, "cg_runpitch", "0.002", a),
        cv(S::CgRunroll, "cg_runroll", "0.005", a),
        cv(S::CgBobup, "cg_bobup", "0.005", c),
        cv(S::CgBobpitch, "cg_bobpitch", "0.002", a),
        cv(S::CgBobroll, "cg_bobroll", "0.002", a),
        cv(S::CgSwingSpeed, "cg_swingSpeed", "0.3", c),
        cv(S::CgAnimSpeed, "cg_animspeed", "1", c),
        cv(S::CgDebugAnim, "cg_debuganim", "0", c),
        cv(S::CgDebugPosition, "cg_debugposition", "0", c),
        cv(S::CgDebugEvents, "cg_debugevents", "0", c),
        cv(S::CgErrorDecay, "cg_errordecay", "100", n),
        cv(S::CgNopredict, "cg_nopredict", "0", n),
        cv(S::CgNoPlayerAnims, "cg_noplayeranims", "0", c),
        cv(S::CgShowmiss, "cg_showmiss", "0", n),
        cv(S::CgFootsteps, "cg_footsteps", "1", c),
        cv(S::CgTracerChance, "cg_tracerchance", "0.4", c),
        cv(S::CgTracerWidth, "cg_tracerwidth", "1", c),
        cv(S::CgTracerLength, "cg_tracerlength", "100", c),
        cv(S::CgThirdPersonRange, "cg_thirdPersonRange", "40", c),
        cv(S::CgThirdPersonAngle, "cg_thirdPersonAngle", "0", c),
        cv(S::CgThirdPerson, "cg_thirdPerson", "0", n),
        cv(S::CgTeamChatTime, "cg_teamChatTime", "3000", a),
        cv(S::CgTeamChatHeight, "cg_teamChatHeight", "0", a),
        cv(S::CgForceModel, "cg_forceModel", "0", a),
        cv(S::CgPredictItems, "cg_predictItems", "1", a),
        cv(
            S::CgDeferPlayers,
            "cg_deferPlayers",
            if product == Product::Missionpack { "0" } else { "1" },
            a,
        ),
        cv(S::CgDrawTeamOverlay, "cg_drawTeamOverlay", "0", a),
        cv(S::CgTeamOverlayUserinfo, "teamoverlay", "0", r | u),
        cv(S::CgStats, "cg_stats", "0", n),
        cv(S::CgDrawFriend, "cg_drawFriend", "1", a),
        cv(S::CgTeamChatsOnly, "cg_teamChatsOnly", "0", a),
        cv(S::CgNoVoiceChats, "cg_noVoiceChats", "0", a),
        cv(S::CgNoVoiceText, "cg_noVoiceText", "0", a),
        cv(S::CgBuildScript, "com_buildScript", "0", n),
        cv(S::CgPaused, "cl_paused", "0", r),
        cv(S::CgBlood, "com_blood", "1", a),
        cv(S::CgSynchronousClients, "g_synchronousClients", "0", n),
    ];
    if product == Product::Missionpack {
        table.extend([
            cv(S::CgRedTeamName, "g_redteam", "Stroggs", a | s | u),
            cv(S::CgBlueTeamName, "g_blueteam", "Pagans", a | s | u),
            cv(S::CgCurrentSelectedPlayer, "cg_currentSelectedPlayer", "0", a),
            cv(S::CgCurrentSelectedPlayerName, "cg_currentSelectedPlayerName", "", a),
            cv(S::CgSinglePlayer, "ui_singlePlayerActive", "0", u),
            cv(S::CgEnableDust, "g_enableDust", "0", s),
            cv(S::CgEnableBreath, "g_enableBreath", "0", s),
            cv(S::CgSinglePlayerActive, "ui_singlePlayerActive", "0", u),
            cv(S::CgRecordSpDemo, "ui_recordSPDemo", "0", a),
            cv(S::CgRecordSpDemoName, "ui_recordSPDemoName", "", a),
            cv(S::CgObeliskRespawnDelay, "g_obeliskRespawnDelay", "10", s),
            cv(S::CgHudFiles, "cg_hudFiles", "ui/hud.txt", a),
        ]);
    }
    table.extend([
        cv(S::CgCameraOrbit, "cg_cameraOrbit", "0", c),
        cv(S::CgCameraOrbitDelay, "cg_cameraOrbitDelay", "50", a),
        cv(S::CgTimescaleFadeEnd, "cg_timescaleFadeEnd", "1", n),
        cv(S::CgTimescaleFadeSpeed, "cg_timescaleFadeSpeed", "0", n),
        cv(S::CgTimescale, "timescale", "1", n),
        cv(S::CgScorePlum, "cg_scorePlums", "1", u | a),
        cv(S::CgSmoothClients, "cg_smoothClients", "0", u | a),
        cv(S::CgCameraMode, "com_cameraMode", "0", c),
        cv(S::PmoveFixed, "pmove_fixed", "0", n),
        cv(S::PmoveMsec, "pmove_msec", "8", n),
        cv(S::CgNoTaunt, "cg_noTaunt", "0", a),
        cv(S::CgNoProjectileTrail, "cg_noProjectileTrail", "0", a),
        cv(S::CgSmallFont, "ui_smallFont", "0.25", a),
        cv(S::CgBigFont, "ui_bigFont", "0.4", a),
        cv(S::CgOldRail, "cg_oldRail", "1", a),
        cv(S::CgOldRocket, "cg_oldRocket", "1", a),
        cv(S::CgOldPlasma, "cg_oldPlasma", "1", a),
        cv(S::CgTrueLightning, "cg_trueLightning", "0.0", a),
    ]);
    table
}

/// Configuration host services (`ClientConfigurationHost`).
pub struct ClientConfigurationHost {
    /// Cvar registry.
    pub cvars: Shared<CvarRegistry>,
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Client store.
    pub clients: Shared<dyn ClientInfoStore>,
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Status visibility override.
    pub status_visible: Option<Rc<dyn Fn() -> bool>>,
}

/// Source string with capacity (`sourceString`).
pub(crate) fn config_source_string(value: &str, capacity: usize) -> String {
    if capacity < 1 {
        panic!("Source string capacity must be positive");
    }
    let units: Vec<char> = value.chars().collect();
    let mut end = units.len();
    for (index, unit) in units.iter().enumerate() {
        if *unit == '\0' {
            end = index;
            break;
        }
        if *unit as u32 > 255 {
            panic!("Cvar VM strings require source byte characters");
        }
    }
    units[..end.min(capacity - 1)].iter().collect()
}

/// Changed snapshot (`changedSnapshot`).
pub(crate) fn changed_snapshot(engine: &CvarSnapshot) -> CvarSnapshot {
    let value = config_source_string(&engine.value, MAX_CVAR_VALUE_STRING);
    if value.chars().count() != engine.value.chars().count() {
        panic!(
            "Cvar_Update: src {} length {} exceeds MAX_CVAR_VALUE_STRING",
            engine.value,
            engine.value.chars().count()
        );
    }
    CvarSnapshot {
        value,
        ..engine.clone()
    }
}

/// Instance-owned cvar table cache (`ClientConfiguration`).
pub struct ClientConfiguration {
    /// Product.
    pub product: Product,
    /// Host.
    pub host: ClientConfigurationHost,
    /// Table.
    table: Vec<CvarDefinition>,
    /// Snapshot cache.
    cache: RefCell<HashMap<ClientVmCvarSymbol, CvarSnapshot>>,
    /// Name lookup.
    names: RefCell<HashMap<String, ClientVmCvarSymbol>>,
    /// Force-model modification count.
    force_model_modification_count: Cell<u32>,
    /// Overlay modification count.
    draw_team_overlay_modification_count: Cell<i64>,
    /// Registered.
    registered: Cell<bool>,
    /// Updating.
    updating: Cell<bool>,
}

impl ClientConfiguration {
    /// Assemble configuration.
    pub fn new(product: Product, host: ClientConfigurationHost) -> Self {
        if host.state.borrow().product != product || host.static_state.borrow().product != product {
            panic!("Client configuration services must share one product");
        }
        let table = cvar_table(product);
        let mut names = HashMap::new();
        let mut cache = HashMap::new();
        for definition in &table {
            names.insert(definition.name.to_lowercase(), definition.symbol);
            cache.insert(
                definition.symbol,
                CvarSnapshot {
                    name: definition.name.clone(),
                    value: String::new(),
                    reset_value: String::new(),
                    latched_value: None,
                    flags: 0,
                    modified: false,
                    modification_count: 0,
                    numeric_value: 0.0,
                    integer_value: 0,
                },
            );
        }
        Self {
            product,
            host,
            table,
            cache: RefCell::new(cache),
            names: RefCell::new(names),
            force_model_modification_count: Cell::new(0),
            draw_team_overlay_modification_count: Cell::new(-1),
            registered: Cell::new(false),
            updating: Cell::new(false),
        }
    }

    /// Register cvars (`registerCvars`).
    pub fn register_cvars(&self) {
        if self.updating.get() {
            panic!("Cannot register cvars during a configuration update");
        }
        self.names.borrow_mut().clear();
        for definition in &self.table {
            let engine =
                self.host
                    .cvars
                    .borrow_mut()
                    .register(&definition.name, &definition.default_value, definition.flags);
            let engine = match engine {
                Ok(Some(snapshot)) => snapshot,
                _ => panic!("Cgame cvar registration failed: {}", definition.name),
            };
            self.copy_definition(definition, &engine, true);
            self.names
                .borrow_mut()
                .insert(definition.name.to_lowercase(), definition.symbol);
        }
        let running = self.host.cvars.borrow().get("sv_running");
        self.host.static_state.borrow_mut().local_server = game_atoi(&config_source_string(
            running.map(|snapshot| snapshot.value).unwrap_or_default().as_str(),
            MAX_TOKEN_CHARS,
        ))
        .unwrap_or_default();
        self.force_model_modification_count
            .set(self.read_vm_symbol(ClientVmCvarSymbol::CgForceModel).modification_count);
        let (team_model, team_head) = if self.product == Product::Missionpack {
            ("james", "*james")
        } else {
            ("sarge", "sarge")
        };
        let flags = cvar_flags::USER_INFO | cvar_flags::ARCHIVE;
        for (name, default) in [
            ("model", "sarge"),
            ("headmodel", "sarge"),
            ("team_model", team_model),
            ("team_headmodel", team_head),
        ] {
            let _ = self
                .host
                .cvars
                .borrow_mut()
                .register(name, default, flags)
                .unwrap_or(None);
        }
        self.registered.set(true);
    }

    /// Read a VM symbol (`readVmSymbol`).
    #[must_use]
    pub fn read_vm_symbol(&self, symbol: ClientVmCvarSymbol) -> CvarSnapshot {
        let value = self.cache.borrow().get(&symbol).cloned().unwrap_or_else(|| {
            panic!(
                "VM cvar {} is not registered for {}",
                symbol.as_str(),
                self.product.as_str()
            );
        });
        if symbol == ClientVmCvarSymbol::CgDrawStatus
            && self.host.status_visible.as_ref().is_some_and(|visible| !visible())
        {
            return CvarSnapshot {
                value: "0".to_string(),
                numeric_value: 0.0,
                integer_value: 0,
                ..value
            };
        }
        value
    }

    /// Write a VM numeric value (`setVmNumericValue`).
    pub fn set_vm_numeric_value(&self, symbol: ClientVmCvarSymbol, value: f64) {
        let previous = self.read_vm_symbol(symbol);
        self.cache.borrow_mut().insert(
            symbol,
            CvarSnapshot {
                numeric_value: value as f32,
                ..previous
            },
        );
    }

    /// Write a VM integer (`setVmInteger`).
    pub fn set_vm_integer(&self, symbol: ClientVmCvarSymbol, value: i32) {
        let previous = self.read_vm_symbol(symbol);
        self.cache.borrow_mut().insert(
            symbol,
            CvarSnapshot {
                integer_value: value,
                ..previous
            },
        );
    }

    /// Read a VM cvar by name (`readVmCvar`).
    #[must_use]
    pub fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
        let symbol = self
            .names
            .borrow()
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or_else(|| {
                panic!("Cgame VM cvar {name} is not registered for {}", self.product.as_str());
            });
        self.read_vm_symbol(symbol)
    }

    /// Force a model change (`forceModelChange`).
    pub fn force_model_change(&self) {
        if !self.registered.get() {
            panic!("Client cvars must be registered before forcing models");
        }
        if self.updating.get() {
            panic!("Client configuration update is already active");
        }
        self.updating.set(true);
        self.reload_client_info();
        self.updating.set(false);
    }

    /// Update cvars (`updateCvars`).
    pub fn update_cvars(&self) {
        if !self.registered.get() {
            panic!("Client cvars must be registered before updating");
        }
        if self.updating.get() {
            panic!("Client configuration update is already active");
        }
        self.updating.set(true);
        for definition in &self.table.clone() {
            if let Some(engine) = self.host.cvars.borrow().get(&definition.name) {
                self.copy_definition(definition, &engine, false);
            }
        }
        let overlay = self.read_vm_symbol(ClientVmCvarSymbol::CgDrawTeamOverlay);
        if self.draw_team_overlay_modification_count.get() != i64::from(overlay.modification_count) {
            self.draw_team_overlay_modification_count
                .set(i64::from(overlay.modification_count));
            let _ = self.host.cvars.borrow_mut().set(
                "teamoverlay",
                if overlay.integer_value > 0 { "1" } else { "0" },
                true,
            );
            let _ = self.host.cvars.borrow_mut().set("teamoverlay", "1", true);
        }
        let force_model = self.read_vm_symbol(ClientVmCvarSymbol::CgForceModel);
        if self.force_model_modification_count.get() != force_model.modification_count {
            self.force_model_modification_count.set(force_model.modification_count);
            self.reload_client_info();
        }
        self.updating.set(false);
    }

    /// Copy one definition (`copyDefinition`).
    fn copy_definition(&self, definition: &CvarDefinition, engine: &CvarSnapshot, forced: bool) {
        let previous = self.cache.borrow().get(&definition.symbol).cloned();
        if !forced {
            if let Some(previous) = &previous {
                if previous.modification_count == engine.modification_count {
                    return;
                }
            }
        }
        let retained = previous.unwrap_or_else(|| CvarSnapshot {
            name: engine.name.clone(),
            value: String::new(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 0,
            numeric_value: 0.0,
            integer_value: 0,
        });
        self.cache.borrow_mut().insert(
            definition.symbol,
            CvarSnapshot {
                value: retained.value,
                numeric_value: retained.numeric_value,
                integer_value: retained.integer_value,
                modification_count: engine.modification_count,
                ..engine.clone()
            },
        );
        let changed = changed_snapshot(engine);
        self.cache.borrow_mut().insert(definition.symbol, changed);
    }

    /// Reload client info (`reloadClientInfo`).
    fn reload_client_info(&self) {
        for index in 0..CONFIG_MAX_CLIENTS {
            let config = self.host.strings.borrow().config_string(CS_PLAYERS + index as usize);
            if !config.is_empty() {
                self.host.clients.borrow_mut().new_client_info(index, &config);
            }
        }
    }
}

impl HudCvarReader for ClientConfiguration {
    fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
        self.read_vm_cvar(name)
    }
}

/// Cached VM cvar reader (`readVmCvar`).
pub trait HudCvarReader {
    /// Read a cached VM cvar.
    fn read_vm_cvar(&self, name: &str) -> CvarSnapshot;
}

/// Configstring source (`configString`).
pub trait HudConfigStrings {
    /// Read a configstring.
    fn config_string(&self, index: usize) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::presentation::hud::tests::*;
    use crate::q3::presentation::hud::{shared, Shared};
    use qa_core::cmd::Dialect;

    #[test]
    fn cvar_table_shapes() {
        let base = cvar_table(Product::Baseq3);
        let mission = cvar_table(Product::Missionpack);
        assert_eq!(base.len(), 89);
        assert_eq!(mission.len(), 101);
        let defer = base
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgDeferPlayers)
            .unwrap();
        assert_eq!(defer.default_value, "1");
        let defer = mission
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgDeferPlayers)
            .unwrap();
        assert_eq!(defer.default_value, "0");
        let marks = base
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgAddMarks)
            .unwrap();
        assert_eq!(marks.name, "cg_marks");
        let overlay = base
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgTeamOverlayUserinfo)
            .unwrap();
        assert_eq!(overlay.flags, cvar_flags::READ_ONLY | cvar_flags::USER_INFO);
        let red = mission
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgRedTeamName)
            .unwrap();
        assert_eq!(red.default_value, "Stroggs");
    }

    #[test]
    fn configuration_register_read_update() {
        let cvars = shared(CvarRegistry::new(Dialect::Q3));
        let state = shared(ClientGameState::new(Product::Missionpack, 0, 0).unwrap());
        let static_state = shared(ClientGameStaticState::new(Product::Missionpack));
        let store: Shared<dyn ClientInfoStore> = shared(FakeStore {
            state: state.clone(),
            slots: static_state
                .borrow()
                .client_info
                .iter()
                .map(|info| shared(info.clone()))
                .collect::<Vec<_>>(),
            loads: Cell::new(0),
        });
        let strings: Shared<dyn HudConfigStrings> = shared(FakeStrings::default());
        let configuration = ClientConfiguration::new(
            Product::Missionpack,
            ClientConfigurationHost {
                cvars: cvars.clone(),
                state,
                static_state,
                clients: store,
                strings,
                status_visible: None,
            },
        );
        configuration.register_cvars();
        assert_eq!(configuration.read_vm_cvar("cg_fov").value, "90");
        assert_eq!(configuration.read_vm_cvar("CG_FOV").value, "90");
        cvars.borrow_mut().set("cg_fov", "110", true).unwrap();
        configuration.update_cvars();
        assert_eq!(configuration.read_vm_cvar("cg_fov").value, "110");
        configuration.set_vm_integer(ClientVmCvarSymbol::CgCurrentSelectedPlayer, 0);
    }

    #[test]
    fn configuration_status_override() {
        let cvars = shared(CvarRegistry::new(Dialect::Q3));
        let state = shared(ClientGameState::new(Product::Baseq3, 0, 0).unwrap());
        let static_state = shared(ClientGameStaticState::new(Product::Baseq3));
        let store: Shared<dyn ClientInfoStore> = shared(FakeStore {
            state: state.clone(),
            slots: static_state
                .borrow()
                .client_info
                .iter()
                .map(|info| shared(info.clone()))
                .collect::<Vec<_>>(),
            loads: Cell::new(0),
        });
        let strings: Shared<dyn HudConfigStrings> = shared(FakeStrings::default());
        let configuration = ClientConfiguration::new(
            Product::Baseq3,
            ClientConfigurationHost {
                cvars,
                state,
                static_state,
                clients: store,
                strings,
                status_visible: Some(Rc::new(|| false)),
            },
        );
        configuration.register_cvars();
        let status = configuration.read_vm_cvar("cg_drawStatus");
        assert_eq!(status.value, "0");
        assert_eq!(status.integer_value, 0);
    }
}
