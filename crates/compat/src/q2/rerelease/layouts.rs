//! Q2 rerelease guest record layouts (Windows x64 packing).
//!
//! Donor: `src/compat/q2/rerelease/layouts.ts` — bridges `rerelease/game.h`
//! struct shapes into [`GuestLayout`]s for the ABI tables and edict accessors.

use qa_guest::core::contracts::{GuestFieldLayout, GuestLayout, GuestStorage};
use thiserror::Error;

/// Layout construction failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayoutError {
    /// Unknown field name.
    #[error("Unknown {0} field {1}")]
    UnknownField(String, String),
}

/// One struct member before packing.
#[derive(Debug, Clone)]
enum MemberType {
    /// Scalar storage.
    Storage(GuestStorage),
    /// Nested record.
    Nested(GuestLayout),
}

/// One struct member: name, type, element count.
#[derive(Debug, Clone)]
struct Member {
    /// Member name.
    name: &'static str,
    /// Member type.
    kind: MemberType,
    /// Element count.
    count: usize,
}

fn member(name: &'static str, kind: MemberType, count: usize) -> Member {
    Member { name, kind, count }
}

fn scalar(name: &'static str, storage: GuestStorage, count: usize) -> Member {
    member(name, MemberType::Storage(storage), count)
}

fn nested(name: &'static str, layout: GuestLayout, count: usize) -> Member {
    member(name, MemberType::Nested(layout), count)
}

fn storage_bytes(storage: GuestStorage) -> usize {
    match storage {
        GuestStorage::Int8 | GuestStorage::Uint8 => 1,
        GuestStorage::Int16 | GuestStorage::Uint16 => 2,
        GuestStorage::Int32 | GuestStorage::Uint32 | GuestStorage::Float32 => 4,
        GuestStorage::Int64
        | GuestStorage::Uint64
        | GuestStorage::Float64
        | GuestStorage::Pointer => 8,
    }
}

fn align_up(cursor: usize, align: usize) -> usize {
    cursor.div_ceil(align) * align
}

/// Pack members with Windows x64 default packing, flattening nested records.
fn structure(name: &str, members: Vec<Member>) -> GuestLayout {
    let mut fields = Vec::new();
    let mut cursor = 0usize;
    let mut alignment = 1usize;
    for field in &members {
        let (width, align) = match &field.kind {
            MemberType::Storage(storage) => {
                let width = storage_bytes(*storage);
                (width, width)
            }
            MemberType::Nested(layout) => (layout.byte_length, layout.alignment),
        };
        alignment = alignment.max(align);
        cursor = align_up(cursor, align);
        match &field.kind {
            MemberType::Storage(storage) => fields.push(GuestFieldLayout {
                name: field.name.to_string(),
                byte_offset: cursor,
                storage: *storage,
                count: field.count,
            }),
            MemberType::Nested(layout) => {
                for index in 0..field.count {
                    for inner in &layout.fields {
                        let prefix = if field.count == 1 {
                            field.name.to_string()
                        } else {
                            format!("{}[{index}]", field.name)
                        };
                        fields.push(GuestFieldLayout {
                            name: format!("{prefix}.{}", inner.name),
                            byte_offset: cursor + index * width + inner.byte_offset,
                            storage: inner.storage,
                            count: inner.count,
                        });
                    }
                }
            }
        }
        cursor += width * field.count;
    }
    GuestLayout::new(
        &format!("q2-rerelease-x64:{name}"),
        align_up(cursor, alignment),
        alignment,
        8,
        fields,
    )
}

/// Byte offset of a named field.
pub fn field_offset(layout: &GuestLayout, name: &str) -> Result<usize, LayoutError> {
    layout
        .fields
        .iter()
        .find(|field| field.name == name)
        .map(|field| field.byte_offset)
        .ok_or_else(|| LayoutError::UnknownField(layout.id.clone(), name.to_string()))
}

/// `vec2_t`: two floats.
#[must_use]
pub fn vec2_layout() -> GuestLayout {
    structure("vec2_t", vec![scalar("xy", GuestStorage::Float32, 2)])
}

/// `vec3_t`: three floats.
#[must_use]
pub fn vec3_layout() -> GuestLayout {
    structure("vec3_t", vec![scalar("xyz", GuestStorage::Float32, 3)])
}

/// `cplane_t` collision plane.
#[must_use]
pub fn plane_layout() -> GuestLayout {
    structure(
        "cplane_t",
        vec![
            scalar("normal", GuestStorage::Float32, 3),
            scalar("dist", GuestStorage::Float32, 1),
            scalar("type", GuestStorage::Uint8, 1),
            scalar("signbits", GuestStorage::Uint8, 1),
            scalar("pad", GuestStorage::Uint8, 2),
        ],
    )
}

/// `csurface_t` trace surface.
#[must_use]
pub fn surface_layout() -> GuestLayout {
    structure(
        "csurface_t",
        vec![
            scalar("name", GuestStorage::Uint8, 32),
            scalar("flags", GuestStorage::Uint32, 1),
            scalar("value", GuestStorage::Int32, 1),
            scalar("id", GuestStorage::Uint32, 1),
            scalar("material", GuestStorage::Uint8, 16),
        ],
    )
}

/// `trace_t` sweep result.
#[must_use]
pub fn trace_layout() -> GuestLayout {
    let plane = plane_layout();
    structure(
        "trace_t",
        vec![
            scalar("allsolid", GuestStorage::Uint8, 1),
            scalar("startsolid", GuestStorage::Uint8, 1),
            scalar("fraction", GuestStorage::Float32, 1),
            scalar("endpos", GuestStorage::Float32, 3),
            nested("plane", plane.clone(), 1),
            scalar("surface", GuestStorage::Pointer, 1),
            scalar("contents", GuestStorage::Uint32, 1),
            scalar("ent", GuestStorage::Pointer, 1),
            nested("plane2", plane, 1),
            scalar("surface2", GuestStorage::Pointer, 1),
        ],
    )
}

/// `cvar_t` engine variable record.
#[must_use]
pub fn cvar_layout() -> GuestLayout {
    structure(
        "cvar_t",
        vec![
            scalar("name", GuestStorage::Pointer, 1),
            scalar("string", GuestStorage::Pointer, 1),
            scalar("latched_string", GuestStorage::Pointer, 1),
            scalar("flags", GuestStorage::Uint32, 1),
            scalar("modified_count", GuestStorage::Int32, 1),
            scalar("value", GuestStorage::Float32, 1),
            scalar("next", GuestStorage::Pointer, 1),
            scalar("integer", GuestStorage::Int32, 1),
        ],
    )
}

/// `pmove_state_t` player movement state.
#[must_use]
pub fn pmove_state_layout() -> GuestLayout {
    structure(
        "pmove_state_t",
        vec![
            scalar("pm_type", GuestStorage::Int32, 1),
            scalar("origin", GuestStorage::Float32, 3),
            scalar("velocity", GuestStorage::Float32, 3),
            scalar("pm_flags", GuestStorage::Uint16, 1),
            scalar("pm_time", GuestStorage::Uint16, 1),
            scalar("gravity", GuestStorage::Int16, 1),
            scalar("delta_angles", GuestStorage::Float32, 3),
            scalar("viewheight", GuestStorage::Int8, 1),
        ],
    )
}

/// `usercmd_t` client input command.
#[must_use]
pub fn usercmd_layout() -> GuestLayout {
    structure(
        "usercmd_t",
        vec![
            scalar("msec", GuestStorage::Uint8, 1),
            scalar("buttons", GuestStorage::Uint8, 1),
            scalar("angles", GuestStorage::Float32, 3),
            scalar("forwardmove", GuestStorage::Float32, 1),
            scalar("sidemove", GuestStorage::Float32, 1),
            scalar("server_frame", GuestStorage::Uint32, 1),
        ],
    )
}

/// `touch_list_t`: touched-entity sweep list.
#[must_use]
pub fn touch_list_layout() -> GuestLayout {
    structure(
        "touch_list_t",
        vec![
            scalar("num", GuestStorage::Uint32, 1),
            nested("touches", trace_layout(), 32),
        ],
    )
}

/// `pmove_t` movement working state.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn pmove_layout() -> GuestLayout {
    structure(
        "pmove_t",
        vec![
            nested("s", pmove_state_layout(), 1),
            nested("cmd", usercmd_layout(), 1),
            scalar("snapinitial", GuestStorage::Uint8, 1),
            nested("touch", touch_list_layout(), 1),
            scalar("viewangles", GuestStorage::Float32, 3),
            scalar("mins", GuestStorage::Float32, 3),
            scalar("maxs", GuestStorage::Float32, 3),
            scalar("groundentity", GuestStorage::Pointer, 1),
            nested("groundplane", plane_layout(), 1),
            scalar("watertype", GuestStorage::Uint32, 1),
            scalar("waterlevel", GuestStorage::Uint8, 1),
            scalar("player", GuestStorage::Pointer, 1),
            scalar("trace", GuestStorage::Pointer, 1),
            scalar("clip", GuestStorage::Pointer, 1),
            scalar("pointcontents", GuestStorage::Pointer, 1),
            scalar("viewoffset", GuestStorage::Float32, 3),
            scalar("screen_blend", GuestStorage::Float32, 4),
            scalar("rdflags", GuestStorage::Uint8, 1),
            scalar("jump_sound", GuestStorage::Uint8, 1),
            scalar("step_clip", GuestStorage::Uint8, 1),
            scalar("impact_delta", GuestStorage::Float32, 1),
        ],
    )
}

/// `entity_state_t` network entity state.
#[must_use]
pub fn entity_state_layout() -> GuestLayout {
    structure(
        "entity_state_t",
        vec![
            scalar("number", GuestStorage::Uint32, 1),
            scalar("origin", GuestStorage::Float32, 3),
            scalar("angles", GuestStorage::Float32, 3),
            scalar("old_origin", GuestStorage::Float32, 3),
            scalar("modelindex", GuestStorage::Int32, 1),
            scalar("modelindex2", GuestStorage::Int32, 1),
            scalar("modelindex3", GuestStorage::Int32, 1),
            scalar("modelindex4", GuestStorage::Int32, 1),
            scalar("frame", GuestStorage::Int32, 1),
            scalar("skinnum", GuestStorage::Int32, 1),
            scalar("effects", GuestStorage::Uint64, 1),
            scalar("renderfx", GuestStorage::Uint32, 1),
            scalar("solid", GuestStorage::Uint32, 1),
            scalar("sound", GuestStorage::Int32, 1),
            scalar("event", GuestStorage::Uint8, 1),
            scalar("alpha", GuestStorage::Float32, 1),
            scalar("scale", GuestStorage::Float32, 1),
            scalar("instance_bits", GuestStorage::Uint8, 1),
            scalar("loop_volume", GuestStorage::Float32, 1),
            scalar("loop_attenuation", GuestStorage::Float32, 1),
            scalar("owner", GuestStorage::Int32, 1),
            scalar("old_frame", GuestStorage::Int32, 1),
        ],
    )
}

/// `player_state_t` client player state.
#[must_use]
pub fn player_state_layout() -> GuestLayout {
    structure(
        "player_state_t",
        vec![
            nested("pmove", pmove_state_layout(), 1),
            scalar("viewangles", GuestStorage::Float32, 3),
            scalar("viewoffset", GuestStorage::Float32, 3),
            scalar("kick_angles", GuestStorage::Float32, 3),
            scalar("gunangles", GuestStorage::Float32, 3),
            scalar("gunoffset", GuestStorage::Float32, 3),
            scalar("gunindex", GuestStorage::Int32, 1),
            scalar("gunskin", GuestStorage::Int32, 1),
            scalar("gunframe", GuestStorage::Int32, 1),
            scalar("gunrate", GuestStorage::Int32, 1),
            scalar("screen_blend", GuestStorage::Float32, 4),
            scalar("damage_blend", GuestStorage::Float32, 4),
            scalar("fov", GuestStorage::Float32, 1),
            scalar("rdflags", GuestStorage::Uint8, 1),
            scalar("stats", GuestStorage::Int16, 64),
            scalar("team_id", GuestStorage::Uint8, 1),
        ],
    )
}

/// Shared `gclient` prefix: player state plus ping.
#[must_use]
pub fn client_layout() -> GuestLayout {
    structure(
        "gclient_shared_t",
        vec![
            nested("ps", player_state_layout(), 1),
            scalar("ping", GuestStorage::Int32, 1),
        ],
    )
}

/// `height_fog_t` fog volume.
#[must_use]
pub fn height_fog_layout() -> GuestLayout {
    structure(
        "height_fog_t",
        vec![
            scalar("start", GuestStorage::Float32, 4),
            scalar("end", GuestStorage::Float32, 4),
            scalar("falloff", GuestStorage::Float32, 1),
            scalar("density", GuestStorage::Float32, 1),
        ],
    )
}

/// `client_persistant_t`: IT_TOTAL=82, AMMO_MAX=12.
#[must_use]
pub fn persistent_client_layout() -> GuestLayout {
    structure(
        "client_persistant_t",
        vec![
            scalar("userinfo", GuestStorage::Uint8, 2048),
            scalar("social_id", GuestStorage::Uint8, 256),
            scalar("netname", GuestStorage::Uint8, 32),
            scalar("hand", GuestStorage::Int32, 1),
            scalar("autoswitch", GuestStorage::Int32, 1),
            scalar("autoshield", GuestStorage::Int32, 1),
            scalar("connected", GuestStorage::Uint8, 1),
            scalar("spawned", GuestStorage::Uint8, 1),
            scalar("health", GuestStorage::Int32, 1),
            scalar("max_health", GuestStorage::Int32, 1),
            scalar("savedFlags", GuestStorage::Uint64, 1),
            scalar("selected_item", GuestStorage::Int32, 1),
            scalar("selected_item_time", GuestStorage::Int64, 1),
            scalar("inventory", GuestStorage::Int32, 82),
            scalar("max_ammo", GuestStorage::Int16, 12),
            scalar("weapon", GuestStorage::Pointer, 1),
            scalar("lastweapon", GuestStorage::Pointer, 1),
            scalar("power_cubes", GuestStorage::Int32, 1),
            scalar("score", GuestStorage::Int32, 1),
            scalar("game_help1changed", GuestStorage::Int32, 1),
            scalar("game_help2changed", GuestStorage::Int32, 1),
            scalar("helpchanged", GuestStorage::Int32, 1),
            scalar("help_time", GuestStorage::Int64, 1),
            scalar("spectator", GuestStorage::Uint8, 1),
            scalar("bob_skip", GuestStorage::Uint8, 1),
            scalar("wanted_fog", GuestStorage::Float32, 5),
            nested("wanted_heightfog", height_fog_layout(), 1),
            scalar("fog_transition_time", GuestStorage::Int64, 1),
            scalar("megahealth_time", GuestStorage::Int64, 1),
            scalar("lives", GuestStorage::Int32, 1),
            scalar("n64_crouch_warn_times", GuestStorage::Uint8, 1),
            scalar("n64_crouch_warning", GuestStorage::Int64, 1),
        ],
    )
}

/// Source `gclient_t` public prefix: shared plus persistent state.
#[must_use]
pub fn private_client_prefix_layout() -> GuestLayout {
    structure(
        "gclient_t_private_prefix",
        vec![
            nested("shared", client_layout(), 1),
            nested("pers", persistent_client_layout(), 1),
        ],
    )
}

/// `client_respawn_t` respawn state.
#[must_use]
pub fn respawn_client_layout() -> GuestLayout {
    let mut members = vec![
        nested("coop_respawn", persistent_client_layout(), 1),
        scalar("entertime", GuestStorage::Int64, 1),
        scalar("score", GuestStorage::Int32, 1),
        scalar("cmd_angles", GuestStorage::Float32, 3),
        scalar("spectator", GuestStorage::Uint8, 1),
        scalar("ctf_team", GuestStorage::Int32, 1),
        scalar("ctf_state", GuestStorage::Int32, 1),
    ];
    for name in [
        "ctf_lasthurtcarrier",
        "ctf_lastreturnedflag",
        "ctf_flagsince",
        "ctf_lastfraggedcarrier",
    ] {
        members.push(scalar(name, GuestStorage::Int64, 1));
    }
    members.push(scalar("id_state", GuestStorage::Uint8, 1));
    members.push(scalar("lastidtime", GuestStorage::Int64, 1));
    members.push(scalar("voted", GuestStorage::Uint8, 1));
    members.push(scalar("ready", GuestStorage::Uint8, 1));
    members.push(scalar("admin", GuestStorage::Uint8, 1));
    members.push(scalar("ghost", GuestStorage::Pointer, 1));
    structure("client_respawn_t", members)
}

fn damage_indicator_layout() -> GuestLayout {
    structure(
        "damage_indicator_t",
        vec![
            scalar("from", GuestStorage::Float32, 3),
            scalar("health", GuestStorage::Int32, 1),
            scalar("armor", GuestStorage::Int32, 1),
            scalar("power", GuestStorage::Int32, 1),
        ],
    )
}

fn kick_layout() -> GuestLayout {
    structure(
        "gclient_t_kick",
        vec![
            scalar("angles", GuestStorage::Float32, 3),
            scalar("origin", GuestStorage::Float32, 3),
            scalar("time", GuestStorage::Int64, 1),
            scalar("total", GuestStorage::Int64, 1),
        ],
    )
}

/// Full source `gclient_t` record.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn private_client_layout() -> GuestLayout {
    let mut members = vec![
        nested("shared", client_layout(), 1),
        nested("pers", persistent_client_layout(), 1),
        nested("resp", respawn_client_layout(), 1),
        nested("old_pmove", pmove_state_layout(), 1),
    ];
    for name in [
        "showscores",
        "showeou",
        "showinventory",
        "showhelp",
        "buttons",
        "oldbuttons",
        "latched_buttons",
    ] {
        members.push(scalar(name, GuestStorage::Uint8, 1));
    }
    members.push(nested("cmd", usercmd_layout(), 1));
    members.push(scalar("weapon_fire_finished", GuestStorage::Int64, 1));
    members.push(scalar("weapon_think_time", GuestStorage::Int64, 1));
    members.push(scalar("weapon_fire_buffered", GuestStorage::Uint8, 1));
    members.push(scalar("weapon_thunk", GuestStorage::Uint8, 1));
    members.push(scalar("newweapon", GuestStorage::Pointer, 1));
    for name in [
        "damage_armor",
        "damage_parmor",
        "damage_blood",
        "damage_knockback",
    ] {
        members.push(scalar(name, GuestStorage::Int32, 1));
    }
    members.push(scalar("damage_from", GuestStorage::Float32, 3));
    members.push(nested(
        "damage_indicators",
        damage_indicator_layout(),
        4,
    ));
    members.push(scalar("num_damage_indicators", GuestStorage::Uint8, 1));
    members.push(scalar("killer_yaw", GuestStorage::Float32, 1));
    members.push(scalar("weaponstate", GuestStorage::Int32, 1));
    members.push(nested("kick", kick_layout(), 1));
    members.push(scalar("quake_time", GuestStorage::Int64, 1));
    members.push(scalar("kick_origin", GuestStorage::Float32, 3));
    members.push(scalar("v_dmg_roll", GuestStorage::Float32, 1));
    members.push(scalar("v_dmg_pitch", GuestStorage::Float32, 1));
    members.push(scalar("v_dmg_time", GuestStorage::Int64, 1));
    members.push(scalar("fall_time", GuestStorage::Int64, 1));
    for name in ["fall_value", "damage_alpha", "bonus_alpha"] {
        members.push(scalar(name, GuestStorage::Float32, 1));
    }
    for name in ["damage_blend", "v_angle", "v_forward"] {
        members.push(scalar(name, GuestStorage::Float32, 3));
    }
    members.push(scalar("bobtime", GuestStorage::Float32, 1));
    members.push(scalar("oldviewangles", GuestStorage::Float32, 3));
    members.push(scalar("oldvelocity", GuestStorage::Float32, 3));
    members.push(scalar("oldgroundentity", GuestStorage::Pointer, 1));
    members.push(scalar("flash_time", GuestStorage::Int64, 1));
    members.push(scalar("next_drown_time", GuestStorage::Int64, 1));
    members.push(scalar("old_waterlevel", GuestStorage::Uint8, 1));
    members.push(scalar("breather_sound", GuestStorage::Int32, 1));
    members.push(scalar("machinegun_shots", GuestStorage::Int32, 1));
    members.push(scalar("anim_end", GuestStorage::Int32, 1));
    members.push(scalar("anim_priority", GuestStorage::Int32, 1));
    members.push(scalar("anim_duck", GuestStorage::Uint8, 1));
    members.push(scalar("anim_run", GuestStorage::Uint8, 1));
    for name in [
        "anim_time",
        "quad_time",
        "invincible_time",
        "breather_time",
        "enviro_time",
        "invisible_time",
    ] {
        members.push(scalar(name, GuestStorage::Int64, 1));
    }
    members.push(scalar("grenade_blew_up", GuestStorage::Uint8, 1));
    for name in ["grenade_time", "grenade_finished_time", "quadfire_time"] {
        members.push(scalar(name, GuestStorage::Int64, 1));
    }
    members.push(scalar("silencer_shots", GuestStorage::Int32, 1));
    members.push(scalar("weapon_sound", GuestStorage::Int32, 1));
    members.push(scalar("pickup_msg_time", GuestStorage::Int64, 1));
    members.push(scalar("flood_locktill", GuestStorage::Int64, 1));
    members.push(scalar("flood_when", GuestStorage::Int64, 10));
    members.push(scalar("flood_whenhead", GuestStorage::Int32, 1));
    members.push(scalar("respawn_time", GuestStorage::Int64, 1));
    members.push(scalar("chase_target", GuestStorage::Pointer, 1));
    members.push(scalar("update_chase", GuestStorage::Uint8, 1));
    for name in ["double_time", "ir_time", "nuke_time", "tracker_pain_time"] {
        members.push(scalar(name, GuestStorage::Int64, 1));
    }
    members.push(scalar("owned_sphere", GuestStorage::Pointer, 1));
    members.push(scalar("empty_click_sound", GuestStorage::Int64, 1));
    members.push(scalar("inmenu", GuestStorage::Uint8, 1));
    members.push(scalar("menu", GuestStorage::Pointer, 1));
    members.push(scalar("menutime", GuestStorage::Int64, 1));
    members.push(scalar("menudirty", GuestStorage::Uint8, 1));
    members.push(scalar("ctf_grapple", GuestStorage::Pointer, 1));
    members.push(scalar("ctf_grapplestate", GuestStorage::Int32, 1));
    for name in [
        "ctf_grapplereleasetime",
        "ctf_regentime",
        "ctf_techsndtime",
        "ctf_lasttechmsg",
    ] {
        members.push(scalar(name, GuestStorage::Int64, 1));
    }
    members.push(scalar("trail_head", GuestStorage::Pointer, 1));
    members.push(scalar("trail_tail", GuestStorage::Pointer, 1));
    members.push(scalar("no_weapon_chains", GuestStorage::Uint8, 1));
    members.push(scalar("landmark_free_fall", GuestStorage::Uint8, 1));
    members.push(scalar("landmark_name", GuestStorage::Pointer, 1));
    members.push(scalar("landmark_rel_pos", GuestStorage::Float32, 3));
    members.push(scalar("landmark_noise_time", GuestStorage::Int64, 1));
    members.push(scalar("invisibility_fade_time", GuestStorage::Int64, 1));
    members.push(scalar("chase_msg_time", GuestStorage::Int64, 1));
    members.push(scalar("menu_sign", GuestStorage::Int32, 1));
    members.push(scalar("last_ladder_pos", GuestStorage::Float32, 3));
    members.push(scalar("last_ladder_sound", GuestStorage::Int64, 1));
    members.push(scalar("coop_respawn_state", GuestStorage::Int32, 1));
    members.push(scalar("last_damage_time", GuestStorage::Int64, 1));
    members.push(scalar("sight_entity", GuestStorage::Pointer, 1));
    members.push(scalar("sight_entity_time", GuestStorage::Int64, 1));
    members.push(scalar("sound_entity", GuestStorage::Pointer, 1));
    members.push(scalar("sound_entity_time", GuestStorage::Int64, 1));
    members.push(scalar("sound2_entity", GuestStorage::Pointer, 1));
    members.push(scalar("sound2_entity_time", GuestStorage::Int64, 1));
    members.push(scalar("num_lag_origins", GuestStorage::Uint8, 1));
    members.push(scalar("next_lag_origin", GuestStorage::Uint8, 1));
    members.push(scalar("is_lag_compensated", GuestStorage::Uint8, 1));
    members.push(scalar("lag_restore_origin", GuestStorage::Float32, 3));
    members.push(scalar("slow_view_angles", GuestStorage::Float32, 3));
    members.push(scalar("slow_view_angle_time", GuestStorage::Int64, 1));
    members.push(scalar("help_draw_points", GuestStorage::Uint8, 1));
    members.push(scalar("help_draw_index", GuestStorage::Uint64, 1));
    members.push(scalar("help_draw_count", GuestStorage::Uint64, 1));
    members.push(scalar("help_draw_time", GuestStorage::Int64, 1));
    members.push(scalar("step_frame", GuestStorage::Uint32, 1));
    members.push(scalar("help_poi_image", GuestStorage::Int32, 1));
    members.push(scalar("help_poi_location", GuestStorage::Float32, 3));
    members.push(scalar("awaiting_respawn", GuestStorage::Uint8, 1));
    members.push(scalar("respawn_timeout", GuestStorage::Int64, 1));
    members.push(scalar("fog", GuestStorage::Float32, 5));
    members.push(nested("heightfog", height_fog_layout(), 1));
    members.push(scalar("last_attacker_time", GuestStorage::Int64, 1));
    members.push(scalar("last_firing_time", GuestStorage::Int64, 1));
    structure("gclient_t", members)
}

fn armor_info_layout() -> GuestLayout {
    structure(
        "armorInfo_t",
        vec![
            scalar("item_id", GuestStorage::Int32, 1),
            scalar("max_count", GuestStorage::Int32, 1),
        ],
    )
}

/// `sv_entity_t` server entity mirror.
#[must_use]
pub fn server_entity_layout() -> GuestLayout {
    let mut members = vec![
        scalar("init", GuestStorage::Uint8, 1),
        scalar("ent_flags", GuestStorage::Uint64, 1),
        scalar("buttons", GuestStorage::Uint8, 1),
        scalar("spawnflags", GuestStorage::Uint32, 1),
    ];
    for name in [
        "item_id",
        "armor_type",
        "armor_value",
        "health",
        "max_health",
        "starting_health",
        "weapon",
        "team",
        "lobby_usernum",
        "respawntime",
        "viewheight",
        "last_attackertime",
    ] {
        members.push(scalar(name, GuestStorage::Int32, 1));
    }
    members.push(scalar("waterlevel", GuestStorage::Uint8, 1));
    for name in [
        "viewangles",
        "viewforward",
        "velocity",
        "start_origin",
        "end_origin",
    ] {
        members.push(scalar(name, GuestStorage::Float32, 3));
    }
    for name in ["enemy", "ground_entity", "classname", "targetname"] {
        members.push(scalar(name, GuestStorage::Pointer, 1));
    }
    members.push(scalar("netname", GuestStorage::Uint8, 32));
    members.push(scalar("inventory", GuestStorage::Int32, 256));
    members.push(nested("armor_info", armor_info_layout(), 3));
    structure("sv_entity_t", members)
}

/// Published `edict_shared_t` prefix.
#[must_use]
pub fn edict_layout() -> GuestLayout {
    let mut members = vec![
        nested("s", entity_state_layout(), 1),
        scalar("client", GuestStorage::Pointer, 1),
        nested("sv", server_entity_layout(), 1),
        scalar("inuse", GuestStorage::Uint8, 1),
        scalar("linked", GuestStorage::Uint8, 1),
        scalar("linkcount", GuestStorage::Int32, 1),
        scalar("areanum", GuestStorage::Int32, 1),
        scalar("areanum2", GuestStorage::Int32, 1),
        scalar("svflags", GuestStorage::Uint32, 1),
    ];
    for name in ["mins", "maxs", "absmin", "absmax", "size"] {
        members.push(scalar(name, GuestStorage::Float32, 3));
    }
    members.push(scalar("solid", GuestStorage::Uint8, 1));
    members.push(scalar("clipmask", GuestStorage::Uint32, 1));
    members.push(scalar("owner", GuestStorage::Pointer, 1));
    structure("edict_shared_t", members)
}

fn save_function_layout() -> GuestLayout {
    structure(
        "save_data_t",
        vec![
            scalar("value", GuestStorage::Pointer, 1),
            scalar("list", GuestStorage::Pointer, 1),
        ],
    )
}

/// Source-private edict prefix (`g_local.h` proven range only).
#[must_use]
pub fn private_edict_prefix_layout() -> GuestLayout {
    let save = save_function_layout();
    let mut members = vec![
        nested("shared", edict_layout(), 1),
        scalar("spawn_count", GuestStorage::Int32, 1),
        scalar("movetype", GuestStorage::Int32, 1),
        scalar("flags", GuestStorage::Uint64, 1),
        scalar("model", GuestStorage::Pointer, 1),
        scalar("freetime", GuestStorage::Int64, 1),
        scalar("message", GuestStorage::Pointer, 1),
        scalar("classname", GuestStorage::Pointer, 1),
        scalar("spawnflags", GuestStorage::Uint32, 1),
        scalar("timestamp", GuestStorage::Int64, 1),
        scalar("angle", GuestStorage::Float32, 1),
    ];
    for name in [
        "target",
        "targetname",
        "killtarget",
        "team",
        "pathtarget",
        "deathtarget",
        "healthtarget",
        "itemtarget",
        "combattarget",
        "target_ent",
    ] {
        members.push(scalar(name, GuestStorage::Pointer, 1));
    }
    for name in ["speed", "accel", "decel"] {
        members.push(scalar(name, GuestStorage::Float32, 1));
    }
    for name in ["movedir", "pos1", "pos2", "pos3", "velocity", "avelocity"] {
        members.push(scalar(name, GuestStorage::Float32, 3));
    }
    members.push(scalar("mass", GuestStorage::Int32, 1));
    members.push(scalar("air_finished", GuestStorage::Int64, 1));
    members.push(scalar("gravity", GuestStorage::Float32, 1));
    members.push(scalar("goalentity", GuestStorage::Pointer, 1));
    members.push(scalar("movetarget", GuestStorage::Pointer, 1));
    members.push(scalar("yaw_speed", GuestStorage::Float32, 1));
    members.push(scalar("ideal_yaw", GuestStorage::Float32, 1));
    members.push(scalar("nextthink", GuestStorage::Int64, 1));
    for name in [
        "prethink",
        "postthink",
        "think",
        "touch",
        "use",
        "pain",
        "die",
    ] {
        members.push(nested(name, save.clone(), 1));
    }
    for name in [
        "touch_debounce_time",
        "pain_debounce_time",
        "damage_debounce_time",
        "fly_sound_debounce_time",
        "last_move_time",
    ] {
        members.push(scalar(name, GuestStorage::Int64, 1));
    }
    members.push(scalar("health", GuestStorage::Int32, 1));
    members.push(scalar("max_health", GuestStorage::Int32, 1));
    members.push(scalar("gib_health", GuestStorage::Int32, 1));
    members.push(scalar("show_hostile", GuestStorage::Int64, 1));
    members.push(scalar("powerarmor_time", GuestStorage::Int64, 1));
    members.push(scalar("map", GuestStorage::Pointer, 1));
    members.push(scalar("viewheight", GuestStorage::Int32, 1));
    members.push(scalar("deadflag", GuestStorage::Uint8, 1));
    members.push(scalar("takedamage", GuestStorage::Uint8, 1));
    members.push(scalar("dmg", GuestStorage::Int32, 1));
    members.push(scalar("radius_dmg", GuestStorage::Int32, 1));
    members.push(scalar("dmg_radius", GuestStorage::Float32, 1));
    members.push(scalar("sounds", GuestStorage::Int32, 1));
    members.push(scalar("count", GuestStorage::Int32, 1));
    for name in ["chain", "enemy", "oldenemy", "activator", "groundentity"] {
        members.push(scalar(name, GuestStorage::Pointer, 1));
    }
    members.push(scalar(
        "groundentity_linkcount",
        GuestStorage::Int32,
        1,
    ));
    structure("edict_t_private_prefix", members)
}

/// `vrect_t` HUD rectangle.
#[must_use]
pub fn rectangle_layout() -> GuestLayout {
    structure(
        "vrect_t",
        vec![
            scalar("x", GuestStorage::Int32, 1),
            scalar("y", GuestStorage::Int32, 1),
            scalar("width", GuestStorage::Int32, 1),
            scalar("height", GuestStorage::Int32, 1),
        ],
    )
}

/// `cg_server_data_t` HUD server data.
#[must_use]
pub fn cgame_server_data_layout() -> GuestLayout {
    structure(
        "cg_server_data_t",
        vec![
            scalar("layout", GuestStorage::Uint8, 1024),
            scalar("inventory", GuestStorage::Int16, 256),
        ],
    )
}

/// Import table layout: tick header plus one pointer per import name.
#[must_use]
pub fn import_table_layout(kind: &str, names: &[&str]) -> GuestLayout {
    let mut members = vec![
        scalar("tick_rate", GuestStorage::Uint32, 1),
        scalar("frame_time_s", GuestStorage::Float32, 1),
        scalar("frame_time_ms", GuestStorage::Uint32, 1),
    ];
    for name in names {
        members.push(scalar(name, GuestStorage::Pointer, 1));
    }
    structure(&format!("{kind}_import_t"), members)
}

/// Export table layout; the game table splices edict fields after 19 entries.
#[must_use]
pub fn export_table_layout(kind: &str, names: &[&str]) -> GuestLayout {
    let mut members = vec![scalar("apiversion", GuestStorage::Int32, 1)];
    for name in names {
        members.push(scalar(name, GuestStorage::Pointer, 1));
    }
    if kind == "game" {
        let at = members.len().min(20);
        for (index, name) in [
            "edicts",
            "edict_size",
            "num_edicts",
            "max_edicts",
            "server_flags",
        ]
        .into_iter()
        .enumerate()
        {
            let storage = match name {
                "edicts" => GuestStorage::Pointer,
                "edict_size" => GuestStorage::Uint64,
                _ => GuestStorage::Uint32,
            };
            members.insert(at + index, scalar(name, storage, 1));
        }
    }
    structure(&format!("{kind}_export_t"), members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_and_player_offsets_match_source() {
        let trace = trace_layout();
        assert_eq!(trace.byte_length, 96);
        assert_eq!(field_offset(&trace, "fraction").unwrap(), 4);
        assert_eq!(field_offset(&trace, "surface").unwrap(), 40);
        assert_eq!(field_offset(&trace, "contents").unwrap(), 48);
        assert_eq!(field_offset(&trace, "ent").unwrap(), 56);
        assert_eq!(field_offset(&trace, "surface2").unwrap(), 88);
        let player = player_state_layout();
        assert_eq!(player.byte_length, 296);
        assert_eq!(field_offset(&player, "viewangles").unwrap(), 52);
        assert_eq!(field_offset(&player, "stats").unwrap(), 166);
        assert_eq!(field_offset(&player, "team_id").unwrap(), 294);
        let pmove = pmove_layout();
        assert_eq!(field_offset(&pmove, "s.origin").unwrap(), 4);
        assert_eq!(field_offset(&pmove, "s.pm_flags").unwrap(), 28);
        assert_eq!(field_offset(&pmove, "s.viewheight").unwrap(), 48);
        assert_eq!(field_offset(&pmove, "mins").unwrap(), 3172);
        assert_eq!(field_offset(&pmove, "maxs").unwrap(), 3184);
    }

    #[test]
    fn edict_and_table_layouts_pack() {
        let edict = edict_layout();
        assert_eq!(edict.byte_length, 1472);
        assert_eq!(field_offset(&edict, "client").unwrap(), 120);
        assert_eq!(field_offset(&edict, "inuse").unwrap(), 1376);
        assert_eq!(field_offset(&edict, "svflags").unwrap(), 1392);
        assert_eq!(field_offset(&edict, "owner").unwrap(), 1464);
        let prefix = private_edict_prefix_layout();
        assert_eq!(field_offset(&prefix, "spawn_count").unwrap(), 1472);
        let game = export_table_layout("game", &["PreInit", "Init"]);
        assert!(field_offset(&game, "edicts").is_ok());
        assert!(field_offset(&game, "num_edicts").is_ok());
        let cgame = export_table_layout("cgame", &["Init"]);
        assert!(field_offset(&cgame, "edicts").is_err());
        let imports = import_table_layout("game", &["Com_Print"]);
        assert_eq!(field_offset(&imports, "tick_rate").unwrap(), 0);
        assert!(field_offset(&imports, "Com_Print").is_ok());
        assert_eq!(
            field_offset(&imports, "missing").unwrap_err(),
            LayoutError::UnknownField(imports.id.clone(), "missing".to_string())
        );
    }
}
