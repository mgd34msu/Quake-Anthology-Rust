//! Q2 rerelease synthetic guest module: tables, entities, dispatch.
//!
//! Donor: `src/compat/q2/rerelease/module.ts` — bridges game/cgame API
//! tables and the live edict table over a headless synthetic runner.

use std::collections::{HashMap, VecDeque};

use qa_core::math::Vec3;
use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue, GuestLayout, GuestStorage,
    GuestValueLayout, ModuleIdentity, RawEntityView,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

use super::layouts::{edict_layout, export_table_layout, field_offset};

/// Module failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModuleError {
    /// Missing import handler result.
    #[error("Missing Q2 rerelease {0} import {1}")]
    MissingImport(String, String),
    /// Unknown export name.
    #[error("Unknown game export {0}")]
    UnknownGameExport(String),
    /// Unknown cgame export name.
    #[error("Unknown cgame export {0}")]
    UnknownCgameExport(String),
    /// Null export function.
    #[error("Q2 rerelease export {0} is null")]
    NullExport(String),
    /// Wrong game API version.
    #[error("Q2 rerelease game API must be 2023")]
    BadGameApi,
    /// Wrong cgame API version.
    #[error("Q2 rerelease cgame API must be 2022")]
    BadCgameApi,
    /// GetGameAPI returned null.
    #[error("GetGameAPI returned null")]
    NullGameApi,
    /// GetCGameAPI returned null.
    #[error("GetCGameAPI returned null")]
    NullCgameApi,
    /// Guest pointer return required.
    #[error("Guest pointer return required")]
    NonPointer,
    /// ClientConnect did not return its native boolean.
    #[error("ClientConnect did not return its native boolean")]
    BadConnectResult,
    /// Client identity exceeds source string limits.
    #[error("Rerelease client identity exceeds its source string limits")]
    IdentityTooLong,
    /// ClientConnect returned unterminated userinfo.
    #[error("ClientConnect returned unterminated userinfo")]
    UnterminatedUserinfo,
    /// Edict table is not initialized.
    #[error("Q2 game edicts are not initialized")]
    NoEdicts,
    /// Invalid edict table geometry.
    #[error("Invalid Q2 rerelease edict table")]
    BadEdictTable,
    /// Edict slot outside capacity.
    #[error("Q2 edict slot is outside source capacity")]
    SlotOutOfRange,
    /// Pointer does not identify an edict.
    #[error("Pointer does not identify a Q2 edict")]
    NotAnEdict,
    /// Frame duration requires an integral tick rate.
    #[error("Q2 source frame duration requires an integral tick rate")]
    BadFrameRate,
    /// Pmove returned a non-void result.
    #[error("Pmove returned a non-void result")]
    BadPmoveResult,
    /// 64-bit guest memory required.
    #[error("Rerelease Windows ABI requires 64-bit guest memory")]
    BadPointerWidth,
    /// World summary required.
    #[error("Original API2023 world requires an artifact-qualified source profile")]
    NoWorldProfile,
    /// Native Pmove already has an input owner.
    #[error("Native Pmove already has an input owner")]
    PmoveOwned,
    /// Guest memory failure.
    #[error("Guest memory fault: {0}")]
    Guest(String),
}

impl From<GuestError> for ModuleError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Which API table a call targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestApi {
    /// Game API.
    Game,
    /// Cgame API.
    Cgame,
}

/// One import call from the synthetic guest.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseImportCall {
    /// Target API.
    pub api: GuestApi,
    /// Import name.
    pub name: String,
    /// Arguments.
    pub arguments: Vec<GuestCallValue>,
}

/// Pointer call value.
#[must_use]
pub fn guest_pointer(value: Option<GuestAddress>) -> GuestCallValue {
    GuestCallValue::Pointer(value)
}

/// Integer call value.
#[must_use]
pub fn guest_int(value: i32) -> GuestCallValue {
    GuestCallValue::Int32(value)
}

/// Boolean call value (uint32 0/1, as in the donor).
#[must_use]
pub fn guest_bool(value: bool) -> GuestCallValue {
    GuestCallValue::Uint32(u32::from(value))
}

/// Unwrap a pointer call result.
pub fn result_pointer(result: &GuestCallResult) -> Result<Option<GuestAddress>, ModuleError> {
    match result {
        GuestCallResult::Value(GuestCallValue::Pointer(address)) => Ok(*address),
        _ => Err(ModuleError::NonPointer),
    }
}

/// Minimal world summary the module itself needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleWorldSummary {
    /// Source-private edict byte length.
    pub edict_byte_length: usize,
    /// Client inventory count.
    pub client_inventory_count: usize,
}

/// Pmove interceptor around scripted Pmove calls.
pub type PmoveInterceptor = Box<dyn FnMut(&mut SparseGuestMemory, GuestAddress) -> Result<(), ModuleError>>;

/// Mutable guest state shared with scripted handlers.
pub struct ModuleState {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    /// Game import table address.
    pub game_import_table: GuestAddress,
    /// Cgame import table address.
    pub cgame_import_table: GuestAddress,
    /// Game export table address.
    pub game_table: GuestAddress,
    /// Cgame export table address.
    pub cgame_table: GuestAddress,
    /// Recorded native calls (target offset, argument count).
    pub calls: Vec<(u64, usize)>,
    /// Queued scripted export results by export name.
    pub scripted: HashMap<String, VecDeque<GuestCallResult>>,
    /// Entity table geometry.
    pub entities: EntityTableState,
    /// World summary, if selected.
    pub world_summary: Option<ModuleWorldSummary>,
    /// Pmove interceptors around scripted Pmove calls.
    pub pmove_interceptors: Vec<PmoveInterceptor>,
    /// Whether an input owner is bound.
    pub input_bound: bool,
}

/// Live entity table geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityTableState {
    /// Table base address.
    pub base: GuestAddress,
    /// Record stride in bytes.
    pub stride_bytes: usize,
    /// Live count.
    pub count: u32,
    /// Capacity.
    pub capacity: u32,
}

/// Client connect outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConnectOutcome {
    /// Whether the client was accepted.
    pub accepted: bool,
    /// Returned userinfo.
    pub userinfo: String,
}

/// Game export names in `game.h` order.
pub const GAME_EXPORT_NAMES: &[&str] = &[
    "PreInit",
    "Init",
    "Shutdown",
    "SpawnEntities",
    "WriteGameJson",
    "ReadGameJson",
    "WriteLevelJson",
    "ReadLevelJson",
    "CanSave",
    "ClientChooseSlot",
    "ClientConnect",
    "ClientBegin",
    "ClientUserinfoChanged",
    "ClientDisconnect",
    "ClientCommand",
    "ClientThink",
    "RunFrame",
    "PrepFrame",
    "ServerCommand",
    "Pmove",
    "GetExtension",
    "Bot_SetWeapon",
    "Bot_TriggerEdict",
    "Bot_UseItem",
    "Bot_GetItemID",
    "Edict_ForceLookAtPoint",
    "Bot_PickedUpItem",
    "Entity_IsVisibleToPlayer",
    "GetShadowLightData",
];

/// Cgame export names in `game.h` order.
pub const CGAME_EXPORT_NAMES: &[&str] = &[
    "Init",
    "Shutdown",
    "DrawHUD",
    "TouchPics",
    "LayoutFlags",
    "GetActiveWeaponWheelWeapon",
    "GetOwnedWeaponWheelWeapons",
    "GetWeaponWheelAmmoCount",
    "GetPowerupWheelCount",
    "GetHitMarkerDamage",
    "Pmove",
    "ParseConfigString",
    "ParseCenterPrint",
    "ClearNotify",
    "ClearCenterprint",
    "NotifyMessage",
    "GetMonsterFlashOffset",
    "GetExtension",
];

/// Game import names in `game.h` order.
pub const GAME_IMPORT_NAMES: &[&str] = &[
    "Broadcast_Print",
    "Com_Print",
    "Client_Print",
    "Center_Print",
    "sound",
    "positioned_sound",
    "local_sound",
    "configstring",
    "get_configstring",
    "Com_Error",
    "modelindex",
    "soundindex",
    "imageindex",
    "setmodel",
    "trace",
    "clip",
    "pointcontents",
    "inPVS",
    "inPHS",
    "SetAreaPortalState",
    "AreasConnected",
    "linkentity",
    "unlinkentity",
    "BoxEdicts",
    "multicast",
    "unicast",
    "WriteChar",
    "WriteByte",
    "WriteShort",
    "WriteLong",
    "WriteFloat",
    "WriteString",
    "WritePosition",
    "WriteDir",
    "WriteAngle",
    "WriteEntity",
    "TagMalloc",
    "TagFree",
    "FreeTags",
    "cvar",
    "cvar_set",
    "cvar_forceset",
    "argc",
    "argv",
    "args",
    "AddCommandString",
    "DebugGraph",
    "GetExtension",
    "Bot_RegisterEdict",
    "Bot_UnRegisterEdict",
    "Bot_MoveToPoint",
    "Bot_FollowActor",
    "GetPathToGoal",
    "Loc_Print",
    "Draw_Line",
    "Draw_Point",
    "Draw_Circle",
    "Draw_Bounds",
    "Draw_Sphere",
    "Draw_OrientedWorldText",
    "Draw_StaticWorldText",
    "Draw_Cylinder",
    "Draw_Ray",
    "Draw_Arrow",
    "ReportMatchDetails_Multicast",
    "ServerFrame",
    "SendToClipBoard",
    "Info_ValueForKey",
    "Info_RemoveKey",
    "Info_SetValueForKey",
];

/// Cgame import names in `game.h` order.
pub const CGAME_IMPORT_NAMES: &[&str] = &[
    "Com_Print",
    "get_configstring",
    "Com_Error",
    "TagMalloc",
    "TagFree",
    "FreeTags",
    "cvar",
    "cvar_set",
    "cvar_forceset",
    "AddCommandString",
    "GetExtension",
    "CL_FrameValid",
    "CL_FrameTime",
    "CL_ClientTime",
    "CL_ClientRealTime",
    "CL_ServerFrame",
    "CL_ServerProtocol",
    "CL_GetClientName",
    "CL_GetClientPic",
    "CL_GetClientDogtag",
    "CL_GetKeyBinding",
    "Draw_RegisterPic",
    "Draw_GetPicSize",
    "SCR_DrawChar",
    "SCR_DrawPic",
    "SCR_DrawColorPic",
    "SCR_SetAltTypeface",
    "SCR_DrawFontString",
    "SCR_MeasureFontString",
    "SCR_FontLineHeight",
    "CL_GetTextInput",
    "CL_GetWarnAmmoCount",
    "Localize",
    "SCR_DrawBind",
    "CL_InAutoDemoLoop",
];

/// Host callback dispatching rerelease import calls.
type ImportHandler = Box<dyn FnMut(&mut ModuleState, &RereleaseImportCall) -> Result<GuestCallResult, ModuleError>>;

/// One interpreter, memory and synchronous runner serve both API tables.
/// Headless port: scripted handlers answer native calls; imports dispatch
/// to a host callback.
pub struct RereleaseGuestModule {
    /// Mutable guest state.
    pub state: ModuleState,
    /// Game export table layout.
    pub game_layout: GuestLayout,
    /// Cgame export table layout.
    pub cgame_layout: GuestLayout,
    import_handler: ImportHandler,
}

impl RereleaseGuestModule {
    /// Create a module over guest memory with scripted tables.
    pub fn new(
        memory: SparseGuestMemory,
        world_summary: Option<ModuleWorldSummary>,
        frame_milliseconds: u32,
        import_handler: impl FnMut(&mut ModuleState, &RereleaseImportCall) -> Result<GuestCallResult, ModuleError> + 'static,
    ) -> Result<Self, ModuleError> {
        if memory.pointer_bytes() != 8 {
            return Err(ModuleError::BadPointerWidth);
        }
        if frame_milliseconds == 0 || 1000 % frame_milliseconds != 0 {
            return Err(ModuleError::BadFrameRate);
        }
        let game_layout = export_table_layout("game", GAME_EXPORT_NAMES);
        let cgame_layout = export_table_layout("cgame", CGAME_EXPORT_NAMES);
        let mut state = ModuleState {
            memory,
            game_import_table: GuestAddress::new(0, 0),
            cgame_import_table: GuestAddress::new(0, 0),
            game_table: GuestAddress::new(0, 0),
            cgame_table: GuestAddress::new(0, 0),
            calls: Vec::new(),
            scripted: HashMap::new(),
            entities: EntityTableState {
                base: GuestAddress::new(0, 0),
                stride_bytes: 0,
                count: 0,
                capacity: 0,
            },
            world_summary,
            pmove_interceptors: Vec::new(),
            input_bound: false,
        };
        let game_import_table = Self::build_import_table(&mut state.memory, GAME_IMPORT_NAMES, frame_milliseconds)?;
        let cgame_import_table = Self::build_import_table(&mut state.memory, CGAME_IMPORT_NAMES, frame_milliseconds)?;
        let game_table = state
            .memory
            .allocate(&GuestAllocationOptions::bytes(game_layout.byte_length))?;
        state.memory.write_i32(game_table, 2023)?;
        for name in GAME_EXPORT_NAMES {
            let slot = state.memory.allocate(&GuestAllocationOptions::bytes(1))?;
            let at = state.memory.offset(
                game_table,
                field_offset(&game_layout, name).map_err(|_| ModuleError::UnknownGameExport((*name).to_string()))?
                    as i64,
            )?;
            state.memory.write_pointer(at, Some(slot))?;
        }
        let cgame_table = state
            .memory
            .allocate(&GuestAllocationOptions::bytes(cgame_layout.byte_length))?;
        state.memory.write_i32(cgame_table, 2022)?;
        for name in CGAME_EXPORT_NAMES {
            let slot = state.memory.allocate(&GuestAllocationOptions::bytes(1))?;
            let at = state.memory.offset(
                cgame_table,
                field_offset(&cgame_layout, name).map_err(|_| ModuleError::UnknownCgameExport((*name).to_string()))?
                    as i64,
            )?;
            state.memory.write_pointer(at, Some(slot))?;
        }
        let stride = edict_layout().byte_length;
        let capacity = 1024u32;
        let base = state
            .memory
            .allocate(&GuestAllocationOptions::bytes(stride * capacity as usize))?;
        state.entities = EntityTableState {
            base,
            stride_bytes: stride,
            count: 1,
            capacity,
        };
        state.memory.write_pointer(
            state.memory.offset(
                game_table,
                field_offset(&game_layout, "edicts")
                    .map_err(|_| ModuleError::UnknownGameExport("edicts".to_string()))? as i64,
            )?,
            Some(base),
        )?;
        state.memory.write_u64(
            state.memory.offset(
                game_table,
                field_offset(&game_layout, "edict_size")
                    .map_err(|_| ModuleError::UnknownGameExport("edict_size".to_string()))? as i64,
            )?,
            stride as u64,
        )?;
        state.memory.write_u32(
            state.memory.offset(
                game_table,
                field_offset(&game_layout, "num_edicts")
                    .map_err(|_| ModuleError::UnknownGameExport("num_edicts".to_string()))? as i64,
            )?,
            1,
        )?;
        state.memory.write_u32(
            state.memory.offset(
                game_table,
                field_offset(&game_layout, "max_edicts")
                    .map_err(|_| ModuleError::UnknownGameExport("max_edicts".to_string()))? as i64,
            )?,
            capacity,
        )?;
        state.game_import_table = game_import_table;
        state.cgame_import_table = cgame_import_table;
        state.game_table = game_table;
        state.cgame_table = cgame_table;
        Ok(Self {
            state,
            game_layout,
            cgame_layout,
            import_handler: Box::new(import_handler),
        })
    }

    fn build_import_table(
        memory: &mut SparseGuestMemory,
        names: &[&str],
        frame_milliseconds: u32,
    ) -> Result<GuestAddress, ModuleError> {
        let layout = super::layouts::import_table_layout("game", names);
        let table = memory.allocate(&GuestAllocationOptions::bytes(layout.byte_length))?;
        memory.write_u32(table, 1000 / frame_milliseconds)?;
        memory.write_f32(memory.offset(table, 4)?, frame_milliseconds as f32 / 1000.0)?;
        memory.write_u32(memory.offset(table, 8)?, frame_milliseconds)?;
        for name in names {
            let slot = memory.allocate(&GuestAllocationOptions::bytes(1))?;
            let at = memory.offset(
                table,
                field_offset(&layout, name)
                    .map_err(|_| ModuleError::MissingImport("table".to_string(), (*name).to_string()))?
                    as i64,
            )?;
            memory.write_pointer(at, Some(slot))?;
        }
        Ok(table)
    }

    /// Require the selected world summary.
    pub fn require_world_summary(&self) -> Result<&ModuleWorldSummary, ModuleError> {
        self.state.world_summary.as_ref().ok_or(ModuleError::NoWorldProfile)
    }

    /// Bind the game table, checking API 2023.
    pub fn bind_game(&mut self) -> Result<GuestAddress, ModuleError> {
        let address = self.state.game_table;
        self.state
            .memory
            .check(address, self.game_layout.byte_length, GuestAccess::Read)?;
        if self.state.memory.read_i32(address)? != 2023 {
            return Err(ModuleError::BadGameApi);
        }
        Ok(address)
    }

    /// Bind the cgame table, checking API 2022.
    pub fn bind_cgame(&mut self) -> Result<GuestAddress, ModuleError> {
        let address = self.state.cgame_table;
        self.state
            .memory
            .check(address, self.cgame_layout.byte_length, GuestAccess::Read)?;
        if self.state.memory.read_i32(address)? != 2022 {
            return Err(ModuleError::BadCgameApi);
        }
        Ok(address)
    }

    fn function(
        state: &mut ModuleState,
        table: GuestAddress,
        layout: &GuestLayout,
        name: &str,
    ) -> Result<GuestAddress, ModuleError> {
        let offset = field_offset(layout, name).map_err(|_| ModuleError::UnknownGameExport(name.to_string()))? as i64;
        let address = state.memory.read_pointer(state.memory.offset(table, offset)?)?;
        address.ok_or_else(|| ModuleError::NullExport(name.to_string()))
    }

    /// Invoke a scripted export or route an import slot to the host.
    pub fn invoke(
        &mut self,
        target: GuestAddress,
        arguments: &[GuestCallValue],
    ) -> Result<GuestCallResult, ModuleError> {
        self.state.calls.push((target.offset, arguments.len()));
        if let Some(name) = self.export_name_at(target, true)? {
            return self.run_export(&name, arguments);
        }
        if let Some(name) = self.export_name_at(target, false)? {
            return self.run_export(&name, arguments);
        }
        if let Some((api, name)) = self.import_name_at(target)? {
            let call = RereleaseImportCall {
                api,
                name,
                arguments: arguments.to_vec(),
            };
            let handler = &mut self.import_handler;
            return handler(&mut self.state, &call);
        }
        Err(ModuleError::MissingImport(
            "game".to_string(),
            format!("0x{:x}", target.offset),
        ))
    }

    fn export_name_at(&mut self, target: GuestAddress, game: bool) -> Result<Option<String>, ModuleError> {
        let (table, layout, names) = if game {
            (self.state.game_table, self.game_layout.clone(), GAME_EXPORT_NAMES)
        } else {
            (self.state.cgame_table, self.cgame_layout.clone(), CGAME_EXPORT_NAMES)
        };
        for name in names {
            let offset =
                field_offset(&layout, name).map_err(|_| ModuleError::UnknownGameExport((*name).to_string()))? as i64;
            let slot = self
                .state
                .memory
                .read_pointer(self.state.memory.offset(table, offset)?)?;
            if slot == Some(target) {
                return Ok(Some((*name).to_string()));
            }
        }
        Ok(None)
    }

    fn import_name_at(&mut self, target: GuestAddress) -> Result<Option<(GuestApi, String)>, ModuleError> {
        for (api, table, names) in [
            (GuestApi::Game, self.state.game_import_table, GAME_IMPORT_NAMES),
            (GuestApi::Cgame, self.state.cgame_import_table, CGAME_IMPORT_NAMES),
        ] {
            let layout =
                super::layouts::import_table_layout(if api == GuestApi::Game { "game" } else { "cgame" }, names);
            for name in names {
                let offset = field_offset(&layout, name)
                    .map_err(|_| ModuleError::MissingImport("table".to_string(), (*name).to_string()))?
                    as i64;
                let slot = self
                    .state
                    .memory
                    .read_pointer(self.state.memory.offset(table, offset)?)?;
                if slot == Some(target) {
                    return Ok(Some((api, (*name).to_string())));
                }
            }
        }
        Ok(None)
    }

    fn run_export(&mut self, name: &str, arguments: &[GuestCallValue]) -> Result<GuestCallResult, ModuleError> {
        if name == "Pmove" && !self.state.pmove_interceptors.is_empty() {
            let movement = match arguments.first() {
                Some(GuestCallValue::Pointer(Some(address))) => *address,
                _ => return Err(ModuleError::NonPointer),
            };
            let mut interceptors = std::mem::take(&mut self.state.pmove_interceptors);
            let mut result = Ok(());
            for interceptor in interceptors.iter_mut() {
                result = interceptor(&mut self.state.memory, movement);
                if result.is_err() {
                    break;
                }
            }
            self.state.pmove_interceptors = interceptors;
            result?;
        }
        if let Some(queue) = self.state.scripted.get_mut(name) {
            if let Some(result) = queue.pop_front() {
                return Ok(result);
            }
        }
        Ok(GuestCallResult::Void)
    }

    /// Queue a scripted result for an export.
    pub fn script(&mut self, name: &str, result: GuestCallResult) {
        self.state
            .scripted
            .entry(name.to_string())
            .or_default()
            .push_back(result);
    }

    /// Call a game export by name.
    pub fn call_game(&mut self, name: &str, arguments: &[GuestCallValue]) -> Result<GuestCallResult, ModuleError> {
        if !GAME_EXPORT_NAMES.contains(&name) {
            return Err(ModuleError::UnknownGameExport(name.to_string()));
        }
        let table = self.bind_game()?;
        let layout = self.game_layout.clone();
        let target = Self::function(&mut self.state, table, &layout, name)?;
        self.invoke(target, arguments)
    }

    /// Call a cgame export by name.
    pub fn call_cgame(&mut self, name: &str, arguments: &[GuestCallValue]) -> Result<GuestCallResult, ModuleError> {
        if !CGAME_EXPORT_NAMES.contains(&name) {
            return Err(ModuleError::UnknownCgameExport(name.to_string()));
        }
        let table = self.bind_cgame()?;
        let layout = self.cgame_layout.clone();
        let target = Self::function(&mut self.state, table, &layout, name)
            .map_err(|_| ModuleError::UnknownCgameExport(name.to_string()))?;
        self.invoke(target, arguments)
    }

    /// `PreInit` then `Init` version handshake.
    pub fn pre_init(&mut self) -> Result<(), ModuleError> {
        self.call_game("PreInit", &[])?;
        Ok(())
    }

    /// `Init` entry.
    pub fn init(&mut self) -> Result<(), ModuleError> {
        self.call_game("Init", &[])?;
        Ok(())
    }

    /// `PrepFrame` entry.
    pub fn prep_frame(&mut self) -> Result<(), ModuleError> {
        self.call_game("PrepFrame", &[])?;
        Ok(())
    }

    /// `RunFrame` entry.
    pub fn run_frame(&mut self, main_loop: bool) -> Result<(), ModuleError> {
        self.call_game("RunFrame", &[guest_bool(main_loop)])?;
        Ok(())
    }

    /// Bind an input movement owner for Pmove interception.
    pub fn bind_input_movement(&mut self) -> Result<(), ModuleError> {
        if self.state.input_bound {
            return Err(ModuleError::PmoveOwned);
        }
        self.state.input_bound = true;
        Ok(())
    }

    /// Release the input movement owner.
    pub fn release_input_movement(&mut self) {
        self.state.input_bound = false;
    }

    /// Client connect with mutable 2048-byte userinfo exchange.
    pub fn client_connect(
        &mut self,
        slot: u32,
        userinfo: &str,
        social_id: &str,
        is_bot: bool,
    ) -> Result<ClientConnectOutcome, ModuleError> {
        if userinfo.len() >= 2048 || social_id.len() >= 256 || userinfo.contains('\0') || social_id.contains('\0') {
            return Err(ModuleError::IdentityTooLong);
        }
        let view = self.entity_at_slot(slot)?;
        let info = self.state.memory.allocate(&GuestAllocationOptions::bytes(2048))?;
        let social = self.string(social_id)?;
        self.state.memory.write(info, userinfo.as_bytes())?;
        let result = self.call_game(
            "ClientConnect",
            &[
                guest_pointer(Some(view.address)),
                guest_pointer(Some(info)),
                guest_pointer(Some(social)),
                guest_bool(is_bot),
            ],
        )?;
        let accepted = match result {
            GuestCallResult::Value(GuestCallValue::Uint32(value)) => value != 0,
            _ => {
                self.state.memory.unmap(info, 2048)?;
                self.state.memory.unmap(social, social_id.len() + 1)?;
                return Err(ModuleError::BadConnectResult);
            }
        };
        let bytes = self.state.memory.copy(info, 2048)?;
        self.state.memory.unmap(info, 2048)?;
        self.state.memory.unmap(social, social_id.len() + 1)?;
        let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
            return Err(ModuleError::UnterminatedUserinfo);
        };
        Ok(ClientConnectOutcome {
            accepted,
            userinfo: String::from_utf8_lossy(&bytes[..end]).into_owned(),
        })
    }

    /// `ClientBegin` entry.
    pub fn client_begin(&mut self, slot: u32) -> Result<(), ModuleError> {
        let view = self.entity_at_slot(slot)?;
        self.call_game("ClientBegin", &[guest_pointer(Some(view.address))])?;
        Ok(())
    }

    /// `ClientThink` entry with a host-provided command blob.
    pub fn client_think(&mut self, slot: u32, command: &[u8; 28]) -> Result<(), ModuleError> {
        let view = self.entity_at_slot(slot)?;
        let address = self.state.memory.allocate(&GuestAllocationOptions::bytes(28))?;
        self.state.memory.write(address, command)?;
        let result = self.call_game(
            "ClientThink",
            &[guest_pointer(Some(view.address)), guest_pointer(Some(address))],
        );
        self.state.memory.unmap(address, 28)?;
        result?;
        Ok(())
    }

    /// `ClientDisconnect` entry.
    pub fn client_disconnect(&mut self, slot: u32) -> Result<(), ModuleError> {
        let view = self.entity_at_slot(slot)?;
        self.call_game("ClientDisconnect", &[guest_pointer(Some(view.address))])?;
        Ok(())
    }

    /// `SpawnEntities` entry.
    pub fn spawn_entities(&mut self, map: &str, entities: &str, spawnpoint: &str) -> Result<(), ModuleError> {
        let strings = [map, entities, spawnpoint];
        let mut addresses = Vec::with_capacity(3);
        for text in strings {
            addresses.push(self.string(text)?);
        }
        let args: Vec<GuestCallValue> = addresses.iter().map(|at| guest_pointer(Some(*at))).collect();
        let result = self.call_game("SpawnEntities", &args);
        for (address, text) in addresses.iter().zip(strings) {
            self.state.memory.unmap(*address, text.len() + 1)?;
        }
        result?;
        Ok(())
    }

    /// Intern an API string in guest memory.
    pub fn string(&mut self, text: &str) -> Result<GuestAddress, ModuleError> {
        let bytes = text.as_bytes();
        let address = self
            .state
            .memory
            .allocate(&GuestAllocationOptions::bytes(bytes.len() + 1))?;
        self.state.memory.write(address, bytes)?;
        Ok(address)
    }

    /// Entity view at a slot; export fields stay live.
    pub fn entity_at_slot(&mut self, slot: u32) -> Result<RawEntityView, ModuleError> {
        let table = self.bind_game()?;
        let layout = self.game_layout.clone();
        let read_field = |state: &mut ModuleState, name: &str| {
            field_offset(&layout, name)
                .map_err(|_| ModuleError::UnknownGameExport(name.to_string()))
                .and_then(|offset| state.memory.offset(table, offset as i64).map_err(ModuleError::from))
        };
        let edict_layout = edict_layout();
        let edicts_addr = read_field(&mut self.state, "edicts")?;
        let size_addr = read_field(&mut self.state, "edict_size")?;
        let num_addr = read_field(&mut self.state, "num_edicts")?;
        let max_addr = read_field(&mut self.state, "max_edicts")?;
        let base = self.state.memory.read_pointer(edicts_addr)?;
        let stride = self.state.memory.read_u64(size_addr)?;
        let count = self.state.memory.read_u32(num_addr)?;
        let capacity = self.state.memory.read_u32(max_addr)?;
        let Some(base) = base else {
            return Err(ModuleError::NoEdicts);
        };
        if stride < edict_layout.byte_length as u64 || count > capacity {
            return Err(ModuleError::BadEdictTable);
        }
        if slot >= capacity {
            return Err(ModuleError::SlotOutOfRange);
        }
        let address = self.state.memory.offset(base, i64::from(slot) * stride as i64)?;
        let bytes = self.state.memory.copy(address, stride as usize)?;
        Ok(RawEntityView {
            module: self.state.memory.module().clone(),
            slot,
            address,
            stride_bytes: stride as usize,
            public_layout: edict_layout,
            bytes,
        })
    }

    /// Entity view from a guest pointer.
    pub fn entity_from_pointer(&mut self, address: GuestAddress) -> Result<RawEntityView, ModuleError> {
        let table = self.bind_game()?;
        let layout = self.game_layout.clone();
        let edict_layout = edict_layout();
        self.state
            .memory
            .check(address, edict_layout.byte_length, GuestAccess::Read)?;
        let base_field =
            field_offset(&layout, "edicts").map_err(|_| ModuleError::UnknownGameExport("edicts".to_string()))? as i64;
        let stride_field = field_offset(&layout, "edict_size")
            .map_err(|_| ModuleError::UnknownGameExport("edict_size".to_string()))? as i64;
        let capacity_field = field_offset(&layout, "max_edicts")
            .map_err(|_| ModuleError::UnknownGameExport("max_edicts".to_string()))? as i64;
        let base = self
            .state
            .memory
            .read_pointer(self.state.memory.offset(table, base_field)?)?
            .ok_or(ModuleError::NoEdicts)?;
        let stride = self
            .state
            .memory
            .read_u64(self.state.memory.offset(table, stride_field)?)?;
        let capacity = self
            .state
            .memory
            .read_u32(self.state.memory.offset(table, capacity_field)?)?;
        if stride == 0 {
            return Err(ModuleError::BadEdictTable);
        }
        if address.offset < base.offset
            || !(address.offset - base.offset).is_multiple_of(stride)
            || (address.offset - base.offset) / stride >= u64::from(capacity)
        {
            return Err(ModuleError::NotAnEdict);
        }
        self.entity_at_slot(((address.offset - base.offset) / stride) as u32)
    }

    /// Split index and server player number are independent ABI arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_hud(
        &mut self,
        split: i32,
        server_data: GuestAddress,
        viewport: [i32; 4],
        safe_area: [i32; 4],
        scale: i32,
        player_number: i32,
        player_state: GuestAddress,
    ) -> Result<(), ModuleError> {
        let rect = |values: [i32; 4]| {
            let mut bytes = [0u8; 16];
            for (index, value) in values.iter().enumerate() {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            GuestCallValue::Aggregate {
                layout: super::layouts::rectangle_layout(),
                bytes: bytes.to_vec(),
            }
        };
        self.call_cgame(
            "DrawHUD",
            &[
                guest_int(split),
                guest_pointer(Some(server_data)),
                rect(viewport),
                rect(safe_area),
                guest_int(scale),
                guest_int(player_number),
                guest_pointer(Some(player_state)),
            ],
        )?;
        Ok(())
    }

    /// Module identity for handler bookkeeping.
    #[must_use]
    pub fn module_identity(&self) -> &ModuleIdentity {
        self.state.memory.module()
    }

    /// Record a synthetic velocity write (host-side helper for tests).
    #[must_use]
    pub fn velocity_storage() -> GuestValueLayout {
        GuestValueLayout::Scalar(GuestStorage::Float32)
    }

    /// Zero vector helper available to handlers.
    #[must_use]
    pub fn zero_vec() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::ContentDigest;

    fn test_module() -> RereleaseGuestModule {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "module-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        RereleaseGuestModule::new(
            memory,
            Some(ModuleWorldSummary {
                edict_byte_length: 1472,
                client_inventory_count: 84,
            }),
            25,
            |_state, call| {
                if call.name == "ServerFrame" {
                    Ok(GuestCallResult::Value(GuestCallValue::Uint32(7)))
                } else {
                    Err(ModuleError::MissingImport("game".to_string(), call.name.clone()))
                }
            },
        )
        .expect("module")
    }

    #[test]
    fn tables_bind_and_exports_dispatch() {
        let mut module = test_module();
        assert_eq!(module.bind_game().expect("game").offset, module.state.game_table.offset);
        assert_eq!(
            module.bind_cgame().expect("cgame").offset,
            module.state.cgame_table.offset
        );
        module.script("CanSave", GuestCallResult::Value(GuestCallValue::Int32(1)));
        let saved = module.call_game("CanSave", &[]).expect("cansave");
        assert_eq!(saved, GuestCallResult::Value(GuestCallValue::Int32(1)));
        module.pre_init().expect("preinit");
        module.init().expect("init");
        module.prep_frame().expect("prep");
        module.run_frame(true).expect("run");
        assert!(module.state.calls.len() >= 5);
        let missing = module.call_game("Nope", &[]).unwrap_err();
        assert_eq!(missing, ModuleError::UnknownGameExport("Nope".to_string()));
        assert!(module.require_world_summary().is_ok());
    }

    #[test]
    fn entities_and_client_exchange_work() {
        let mut module = test_module();
        let view = module.entity_at_slot(0).expect("slot 0");
        assert_eq!(view.slot, 0);
        assert_eq!(view.stride_bytes, edict_layout().byte_length);
        let back = module.entity_from_pointer(view.address).expect("pointer");
        assert_eq!(back.slot, 0);
        assert_eq!(module.entity_at_slot(5000).unwrap_err(), ModuleError::SlotOutOfRange);
        module.script("ClientConnect", GuestCallResult::Value(GuestCallValue::Uint32(1)));
        let outcome = module.client_connect(1, "name\\x", "social", false).expect("connect");
        assert!(outcome.accepted);
        assert_eq!(outcome.userinfo, "name\\x");
        module.client_begin(1).expect("begin");
        module.client_think(1, &[0u8; 28]).expect("think");
        module.client_disconnect(1).expect("disconnect");
        module.spawn_entities("base1", "{ }", "start").expect("spawn");
        let data = module
            .state
            .memory
            .allocate(&GuestAllocationOptions::bytes(1536))
            .expect("alloc");
        let player = module
            .state
            .memory
            .allocate(&GuestAllocationOptions::bytes(296))
            .expect("alloc");
        module
            .draw_hud(0, data, [0, 0, 640, 480], [8, 8, 624, 464], 1, 0, player)
            .expect("hud");
        assert_eq!(
            result_pointer(&GuestCallResult::Void).unwrap_err(),
            ModuleError::NonPointer
        );
    }
}
