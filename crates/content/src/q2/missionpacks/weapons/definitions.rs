//! Mission-pack weapon definitions (`src/content/q2/missionpacks/weapons/definitions.ts`).

use crate::q2::foundation::weapons::types::Q2WeaponDefinition;

#[allow(clippy::too_many_arguments)]
fn definition(
    name: &str,
    item: &str,
    classname: &str,
    ammo: Option<&str>,
    quantity: i32,
    warning: i32,
    view_model: &str,
    world_model: &str,
    player_model: i32,
    activate_last: i32,
    fire_last: i32,
    idle_last: i32,
    deactivate_last: i32,
    pauses: Vec<i32>,
    fires: Vec<i32>,
    repeating: bool,
) -> Q2WeaponDefinition {
    Q2WeaponDefinition {
        name: name.to_string(),
        item: item.to_string(),
        classname: classname.to_string(),
        ammo: ammo.map(str::to_string),
        quantity,
        warning,
        view_model: view_model.to_string(),
        world_model: world_model.to_string(),
        player_model,
        activate_last,
        fire_last,
        idle_last,
        deactivate_last,
        pauses,
        fires,
        repeating,
    }
}

/// Xatrix weapon definitions (`xatrixWeaponDefinitions`).
pub fn xatrix_weapon_definitions() -> Vec<Q2WeaponDefinition> {
    vec![
        definition("trap", "q2:ammo_trap", "ammo_trap", Some("q2:ammo_trap"), 1, 1, "models/weapons/v_trap/tris.md2", "models/weapons/g_trap/tris.md2", 0, 0, 15, 48, 48, vec![29, 34, 39, 48], vec![12], false),
        definition("ionripper", "q2:weapon_boomer", "weapon_boomer", Some("q2:ammo_cells"), 2, 10, "models/weapons/v_boomer/tris.md2", "models/weapons/g_boom/tris.md2", 13, 4, 6, 36, 39, vec![36], vec![5], false),
        definition("phalanx", "q2:weapon_phalanx", "weapon_phalanx", Some("q2:ammo_magslug"), 1, 5, "models/weapons/v_shotx/tris.md2", "models/weapons/g_shotx/tris.md2", 12, 5, 20, 58, 63, vec![29, 42, 55], vec![7, 8], false),
    ]
}

/// Rogue weapon definitions (`rogueWeaponDefinitions`).
pub fn rogue_weapon_definitions() -> Vec<Q2WeaponDefinition> {
    vec![
        definition("tesla", "q2:ammo_tesla", "ammo_tesla", Some("q2:ammo_tesla"), 1, 2, "models/weapons/v_tesla/tris.md2", "models/ammo/am_tesl/tris.md2", 0, 0, 8, 32, 32, vec![21], vec![2], false),
        definition("proxlauncher", "q2:weapon_proxlauncher", "weapon_proxlauncher", Some("q2:ammo_prox"), 1, 5, "models/weapons/v_launch/tris.md2", "models/weapons/g_launch/tris.md2", 15, 5, 16, 59, 64, vec![34, 51, 59], vec![6], false),
        definition("chainfist", "q2:weapon_chainfist", "weapon_chainfist", None, 0, 0, "models/weapons/v_chainf/tris.md2", "models/weapons/g_chainf/tris.md2", 16, 4, 32, 57, 60, vec![], vec![8, 9, 16, 17, 18, 30, 31], false),
        definition("disintegrator", "q2:weapon_disintegrator", "weapon_disintegrator", Some("q2:ammo_disruptor"), 1, 5, "models/weapons/v_dist/tris.md2", "models/weapons/g_dist/tris.md2", 12, 4, 9, 29, 34, vec![14, 19, 23], vec![5], false),
        definition("etf_rifle", "q2:weapon_etf_rifle", "weapon_etf_rifle", Some("q2:ammo_flechettes"), 1, 30, "models/weapons/v_etf_rifle/tris.md2", "models/weapons/g_etf_rifle/tris.md2", 13, 4, 7, 37, 41, vec![18, 28], vec![6, 7], false),
        definition("heatbeam", "q2:weapon_plasmabeam", "weapon_plasmabeam", Some("q2:ammo_cells"), 2, 10, "models/weapons/v_beamer/tris.md2", "models/weapons/g_beamer/tris.md2", 14, 8, 12, 39, 44, vec![35], vec![9, 10, 11, 12], false),
    ]
}
