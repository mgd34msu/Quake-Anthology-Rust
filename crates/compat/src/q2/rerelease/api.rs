//! Q2 rerelease game/cgame import and export signatures.
//!
//! Donor: `src/compat/q2/rerelease/api.ts` — bridges `rerelease/game.h`
//! function order and widths into ABI signatures and table layouts.

use qa_guest::core::contracts::{GuestCallSignature, GuestLayout, GuestStorage, GuestValueLayout, NativeCallAbi};

use super::layouts::{
    export_table_layout, field_offset, import_table_layout, rectangle_layout, trace_layout, vec2_layout,
};

/// Rerelease native ABI: 64-bit Windows PE+ with Microsoft x64 calls.
#[must_use]
pub const fn rerelease_abi() -> NativeCallAbi {
    NativeCallAbi::MicrosoftX64
}

fn scalar(storage: GuestStorage) -> GuestValueLayout {
    GuestValueLayout::Scalar(storage)
}

fn aggregate(layout: GuestLayout) -> GuestValueLayout {
    GuestValueLayout::Aggregate(layout)
}

/// Build a non-variadic rerelease call signature.
#[must_use]
pub fn signature(parameters: Vec<GuestValueLayout>, result: Option<GuestValueLayout>) -> GuestCallSignature {
    GuestCallSignature {
        abi: rerelease_abi(),
        parameters,
        result,
        variadic: false,
    }
}

/// One named native entry with its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiEntry {
    /// Entry name from `game.h`.
    pub name: &'static str,
    /// Call signature.
    pub signature: GuestCallSignature,
}

fn entry(name: &'static str, parameters: Vec<GuestValueLayout>, result: Option<GuestValueLayout>) -> ApiEntry {
    ApiEntry {
        name,
        signature: signature(parameters, result),
    }
}

/// Aggregate layouts shared by signatures: trace, rectangle, vec2.
struct Aggregates {
    /// Trace result layout.
    trace: GuestLayout,
    /// HUD rectangle layout.
    rect: GuestLayout,
    /// 2-vector layout.
    vec2: GuestLayout,
}

fn aggregates() -> Aggregates {
    Aggregates {
        trace: trace_layout(),
        rect: rectangle_layout(),
        vec2: vec2_layout(),
    }
}

/// Game import entries in `game.h` order.
#[must_use]
pub fn game_imports() -> Vec<ApiEntry> {
    let agg = aggregates();
    let p = || scalar(GuestStorage::Pointer);
    let i = || scalar(GuestStorage::Int32);
    let u = || scalar(GuestStorage::Uint32);
    let q = || scalar(GuestStorage::Uint64);
    let f = || scalar(GuestStorage::Float32);
    let b = || scalar(GuestStorage::Uint8);
    let t = || aggregate(agg.trace.clone());
    vec![
        entry("Broadcast_Print", vec![i(), p()], None),
        entry("Com_Print", vec![p()], None),
        entry("Client_Print", vec![p(), i(), p()], None),
        entry("Center_Print", vec![p(), p()], None),
        entry("sound", vec![p(), b(), i(), f(), f(), f()], None),
        entry("positioned_sound", vec![p(), p(), b(), i(), f(), f(), f()], None),
        entry("local_sound", vec![p(), p(), p(), b(), i(), f(), f(), f(), u()], None),
        entry("configstring", vec![i(), p()], None),
        entry("get_configstring", vec![i()], Some(p())),
        entry("Com_Error", vec![p()], None),
        entry("modelindex", vec![p()], Some(i())),
        entry("soundindex", vec![p()], Some(i())),
        entry("imageindex", vec![p()], Some(i())),
        entry("setmodel", vec![p(), p()], None),
        entry("trace", vec![p(), p(), p(), p(), p(), u()], Some(t())),
        entry("clip", vec![p(), p(), p(), p(), p(), u()], Some(t())),
        entry("pointcontents", vec![p()], Some(u())),
        entry("inPVS", vec![p(), p(), b()], Some(b())),
        entry("inPHS", vec![p(), p(), b()], Some(b())),
        entry("SetAreaPortalState", vec![i(), b()], None),
        entry("AreasConnected", vec![i(), i()], Some(b())),
        entry("linkentity", vec![p()], None),
        entry("unlinkentity", vec![p()], None),
        entry("BoxEdicts", vec![p(), p(), p(), q(), i(), p(), p()], Some(q())),
        entry("multicast", vec![p(), i(), b()], None),
        entry("unicast", vec![p(), b(), u()], None),
        entry("WriteChar", vec![i()], None),
        entry("WriteByte", vec![i()], None),
        entry("WriteShort", vec![i()], None),
        entry("WriteLong", vec![i()], None),
        entry("WriteFloat", vec![f()], None),
        entry("WriteString", vec![p()], None),
        entry("WritePosition", vec![p()], None),
        entry("WriteDir", vec![p()], None),
        entry("WriteAngle", vec![f()], None),
        entry("WriteEntity", vec![p()], None),
        entry("TagMalloc", vec![q(), i()], Some(p())),
        entry("TagFree", vec![p()], None),
        entry("FreeTags", vec![i()], None),
        entry("cvar", vec![p(), p(), u()], Some(p())),
        entry("cvar_set", vec![p(), p()], Some(p())),
        entry("cvar_forceset", vec![p(), p()], Some(p())),
        entry("argc", vec![], Some(i())),
        entry("argv", vec![i()], Some(p())),
        entry("args", vec![], Some(p())),
        entry("AddCommandString", vec![p()], None),
        entry("DebugGraph", vec![f(), i()], None),
        entry("GetExtension", vec![p()], Some(p())),
        entry("Bot_RegisterEdict", vec![p()], None),
        entry("Bot_UnRegisterEdict", vec![p()], None),
        entry("Bot_MoveToPoint", vec![p(), p(), f()], Some(i())),
        entry("Bot_FollowActor", vec![p(), p()], Some(i())),
        entry("GetPathToGoal", vec![p(), p()], Some(b())),
        entry("Loc_Print", vec![p(), i(), p(), p(), q()], None),
        entry("Draw_Line", vec![p(), p(), p(), f(), b()], None),
        entry("Draw_Point", vec![p(), f(), p(), f(), b()], None),
        entry("Draw_Circle", vec![p(), f(), p(), f(), b()], None),
        entry("Draw_Bounds", vec![p(), p(), p(), f(), b()], None),
        entry("Draw_Sphere", vec![p(), f(), p(), f(), b()], None),
        entry("Draw_OrientedWorldText", vec![p(), p(), p(), f(), f(), b()], None),
        entry("Draw_StaticWorldText", vec![p(), p(), p(), p(), f(), f(), b()], None),
        entry("Draw_Cylinder", vec![p(), f(), f(), p(), f(), b()], None),
        entry("Draw_Ray", vec![p(), p(), f(), f(), p(), f(), b()], None),
        entry("Draw_Arrow", vec![p(), p(), f(), p(), p(), f(), b()], None),
        entry("ReportMatchDetails_Multicast", vec![b()], None),
        entry("ServerFrame", vec![], Some(u())),
        entry("SendToClipBoard", vec![p()], None),
        entry("Info_ValueForKey", vec![p(), p(), p(), q()], Some(q())),
        entry("Info_RemoveKey", vec![p(), p()], Some(b())),
        entry("Info_SetValueForKey", vec![p(), p(), p()], Some(b())),
    ]
}

/// Game export entries in `game.h` order.
#[must_use]
pub fn game_exports() -> Vec<ApiEntry> {
    let p = || scalar(GuestStorage::Pointer);
    let i = || scalar(GuestStorage::Int32);
    let b = || scalar(GuestStorage::Uint8);
    let q = || scalar(GuestStorage::Uint64);
    vec![
        entry("PreInit", vec![], None),
        entry("Init", vec![], None),
        entry("Shutdown", vec![], None),
        entry("SpawnEntities", vec![p(), p(), p()], None),
        entry("WriteGameJson", vec![b(), p()], Some(p())),
        entry("ReadGameJson", vec![p()], None),
        entry("WriteLevelJson", vec![b(), p()], Some(p())),
        entry("ReadLevelJson", vec![p()], None),
        entry("CanSave", vec![], Some(b())),
        entry("ClientChooseSlot", vec![p(), p(), b(), p(), q(), b()], Some(p())),
        entry("ClientConnect", vec![p(), p(), p(), b()], Some(b())),
        entry("ClientBegin", vec![p()], None),
        entry("ClientUserinfoChanged", vec![p(), p()], None),
        entry("ClientDisconnect", vec![p()], None),
        entry("ClientCommand", vec![p()], None),
        entry("ClientThink", vec![p(), p()], None),
        entry("RunFrame", vec![b()], None),
        entry("PrepFrame", vec![], None),
        entry("ServerCommand", vec![], None),
        entry("Pmove", vec![p()], None),
        entry("GetExtension", vec![p()], Some(p())),
        entry("Bot_SetWeapon", vec![p(), i(), b()], None),
        entry("Bot_TriggerEdict", vec![p(), p()], None),
        entry("Bot_UseItem", vec![p(), i()], None),
        entry("Bot_GetItemID", vec![p()], Some(i())),
        entry("Edict_ForceLookAtPoint", vec![p(), p()], None),
        entry("Bot_PickedUpItem", vec![p(), p()], Some(b())),
        entry("Entity_IsVisibleToPlayer", vec![p(), p()], Some(b())),
        entry("GetShadowLightData", vec![i()], Some(p())),
    ]
}

/// Cgame import entries in `game.h` order.
#[must_use]
pub fn cgame_imports() -> Vec<ApiEntry> {
    let agg = aggregates();
    let p = || scalar(GuestStorage::Pointer);
    let i = || scalar(GuestStorage::Int32);
    let u = || scalar(GuestStorage::Uint32);
    let q = || scalar(GuestStorage::Uint64);
    let f = || scalar(GuestStorage::Float32);
    let b = || scalar(GuestStorage::Uint8);
    let v2 = || aggregate(agg.vec2.clone());
    vec![
        entry("Com_Print", vec![p()], None),
        entry("get_configstring", vec![i()], Some(p())),
        entry("Com_Error", vec![p()], None),
        entry("TagMalloc", vec![q(), i()], Some(p())),
        entry("TagFree", vec![p()], None),
        entry("FreeTags", vec![i()], None),
        entry("cvar", vec![p(), p(), u()], Some(p())),
        entry("cvar_set", vec![p(), p()], Some(p())),
        entry("cvar_forceset", vec![p(), p()], Some(p())),
        entry("AddCommandString", vec![p()], None),
        entry("GetExtension", vec![p()], Some(p())),
        entry("CL_FrameValid", vec![], Some(b())),
        entry("CL_FrameTime", vec![], Some(f())),
        entry("CL_ClientTime", vec![], Some(q())),
        entry("CL_ClientRealTime", vec![], Some(q())),
        entry("CL_ServerFrame", vec![], Some(i())),
        entry("CL_ServerProtocol", vec![], Some(i())),
        entry("CL_GetClientName", vec![i()], Some(p())),
        entry("CL_GetClientPic", vec![i()], Some(p())),
        entry("CL_GetClientDogtag", vec![i()], Some(p())),
        entry("CL_GetKeyBinding", vec![p()], Some(p())),
        entry("Draw_RegisterPic", vec![p()], Some(b())),
        entry("Draw_GetPicSize", vec![p(), p(), p()], None),
        entry("SCR_DrawChar", vec![i(), i(), i(), i(), b()], None),
        entry("SCR_DrawPic", vec![i(), i(), i(), i(), p()], None),
        entry("SCR_DrawColorPic", vec![i(), i(), i(), i(), p(), p()], None),
        entry("SCR_SetAltTypeface", vec![b()], None),
        entry("SCR_DrawFontString", vec![p(), i(), i(), i(), p(), b(), i()], None),
        entry("SCR_MeasureFontString", vec![p(), i()], Some(v2())),
        entry("SCR_FontLineHeight", vec![i()], Some(f())),
        entry("CL_GetTextInput", vec![p(), p()], Some(b())),
        entry("CL_GetWarnAmmoCount", vec![i()], Some(i())),
        entry("Localize", vec![p(), p(), q()], Some(p())),
        entry("SCR_DrawBind", vec![i(), p(), p(), i(), i(), i()], Some(i())),
        entry("CL_InAutoDemoLoop", vec![], Some(b())),
    ]
}

/// Cgame export entries in `game.h` order.
#[must_use]
pub fn cgame_exports() -> Vec<ApiEntry> {
    let agg = aggregates();
    let p = || scalar(GuestStorage::Pointer);
    let i = || scalar(GuestStorage::Int32);
    let u = || scalar(GuestStorage::Uint32);
    let b = || scalar(GuestStorage::Uint8);
    let h = || scalar(GuestStorage::Int16);
    let uh = || scalar(GuestStorage::Uint16);
    let r = || aggregate(agg.rect.clone());
    vec![
        entry("Init", vec![], None),
        entry("Shutdown", vec![], None),
        entry("DrawHUD", vec![i(), p(), r(), r(), i(), i(), p()], None),
        entry("TouchPics", vec![], None),
        entry("LayoutFlags", vec![p()], Some(h())),
        entry("GetActiveWeaponWheelWeapon", vec![p()], Some(i())),
        entry("GetOwnedWeaponWheelWeapons", vec![p()], Some(u())),
        entry("GetWeaponWheelAmmoCount", vec![p(), i()], Some(h())),
        entry("GetPowerupWheelCount", vec![p(), i()], Some(h())),
        entry("GetHitMarkerDamage", vec![p()], Some(h())),
        entry("Pmove", vec![p()], None),
        entry("ParseConfigString", vec![i(), p()], None),
        entry("ParseCenterPrint", vec![p(), i(), b()], None),
        entry("ClearNotify", vec![i()], None),
        entry("ClearCenterprint", vec![i()], None),
        entry("NotifyMessage", vec![i(), p(), b()], None),
        entry("GetMonsterFlashOffset", vec![uh(), p()], None),
        entry("GetExtension", vec![p()], Some(p())),
    ]
}

/// Entry names in table order.
#[must_use]
pub fn entry_names(entries: &[ApiEntry]) -> Vec<&'static str> {
    entries.iter().map(|entry| entry.name).collect()
}

/// Game import table layout.
#[must_use]
pub fn game_import_layout() -> GuestLayout {
    let names = entry_names(&game_imports());
    import_table_layout("game", &names)
}

/// Game export table layout.
#[must_use]
pub fn game_export_layout() -> GuestLayout {
    let names = entry_names(&game_exports());
    export_table_layout("game", &names)
}

/// Cgame import table layout.
#[must_use]
pub fn cgame_import_layout() -> GuestLayout {
    let names = entry_names(&cgame_imports());
    import_table_layout("cgame", &names)
}

/// Cgame export table layout.
#[must_use]
pub fn cgame_export_layout() -> GuestLayout {
    let names = entry_names(&cgame_exports());
    export_table_layout("cgame", &names)
}

/// `GetGameAPI`/`GetCGameAPI` signature: pointer in, pointer out.
#[must_use]
pub fn get_api_signature() -> GuestCallSignature {
    signature(vec![scalar(GuestStorage::Pointer)], Some(scalar(GuestStorage::Pointer)))
}

/// Find an entry by name.
#[must_use]
pub fn find_entry<'a>(entries: &'a [ApiEntry], name: &str) -> Option<&'a ApiEntry> {
    entries.iter().find(|entry| entry.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_counts_and_abi_match_game_h() {
        assert_eq!(rerelease_abi(), NativeCallAbi::MicrosoftX64);
        assert_eq!(rerelease_abi().pointer_bytes(), 8);
        assert_eq!(game_imports().len(), 67);
        assert_eq!(game_exports().len(), 29);
        assert_eq!(cgame_imports().len(), 35);
        assert_eq!(cgame_exports().len(), 18);
        let trace = find_entry(&game_imports(), "trace").unwrap();
        assert_eq!(trace.signature.parameters.len(), 6);
        assert!(matches!(trace.signature.result, Some(GuestValueLayout::Aggregate(_))));
        let draw = find_entry(&cgame_exports(), "DrawHUD").unwrap();
        assert_eq!(draw.signature.parameters.len(), 7);
        assert!(draw.signature.result.is_none());
        let get_api = get_api_signature();
        assert_eq!(get_api.parameters.len(), 1);
        assert!(!get_api.variadic);
    }

    #[test]
    fn table_layouts_cover_every_entry() {
        for (layout, entries) in [
            (game_import_layout(), game_imports()),
            (game_export_layout(), game_exports()),
            (cgame_import_layout(), cgame_imports()),
            (cgame_export_layout(), cgame_exports()),
        ] {
            for entry in &entries {
                assert!(field_offset(&layout, entry.name).is_ok(), "missing {}", entry.name);
            }
            assert_eq!(layout.pointer_bytes, 8);
        }
        let game = game_export_layout();
        assert!(field_offset(&game, "edicts").is_ok());
        assert!(field_offset(&game, "num_edicts").is_ok());
        let cgame = cgame_export_layout();
        assert!(field_offset(&cgame, "edicts").is_err());
    }
}
