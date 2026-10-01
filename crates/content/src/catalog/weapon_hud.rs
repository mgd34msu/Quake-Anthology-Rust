//! Selected-arsenal HUD icons (`src/content/catalog/weapon-hud.ts`).
//!
//! Resolves the weapon/selected/ammo HUD icon per product and lists the
//! resource paths those icons need. Q1 icons are WAD pictures (classic)
//! or `gfx/weapons` images (rerelease), Q2 icons are `pics` images, and
//! Q3 icons are shader names resolved through the mission-pack gfx
//! shader remap.

use crate::contract::{GameFamily, ItemId, ProviderReference, ResourceRequest, WeaponHudIcon};
use crate::q2::foundation::items::q2_base_item_icons;
use crate::q2::foundation::weapons::definitions::base_weapons;
use crate::q2::foundation::weapons::types::Q2WeaponDefinition;
use crate::q2::missionpacks::items::q2_mission_weapon_icons;
use crate::q2::missionpacks::weapons::definitions::{rogue_weapon_definitions, xatrix_weapon_definitions};
use crate::q3::base::shared::definitions::{ItemType, Product};
use crate::q3::base::shared::items::item_list;
use crate::q3::foundation::arsenal::Q3_WEAPON_ITEMS;

use super::ProductExpectation;

/// Selected-arsenal HUD icons (`WeaponHudIcons`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponHudIcons {
    /// Unselected weapon icon.
    pub weapon: Option<WeaponHudIcon>,
    /// Selected weapon icon.
    pub selected_weapon: Option<WeaponHudIcon>,
    /// Ammo icon.
    pub ammo: Option<WeaponHudIcon>,
}

/// One Q1 HUD picture row: sbar lump plus rerelease wheel slot.
struct Q1Picture {
    /// Weapon item id.
    item: &'static str,
    /// Classic sbar lump stem (`None` for the axe, which has no slot).
    classic: Option<&'static str>,
    /// Rerelease wheel slot.
    wheel: &'static str,
    /// Ammo sbar lump.
    ammo: Option<&'static str>,
}

/// id1 sbar pictures and rerelease id1/wwheel.txt slots, including the axe slot.
const Q1_PICTURES: &[Q1Picture] = &[
    Q1Picture {
        item: "q1:weapon/axe",
        classic: None,
        wheel: "axe",
        ammo: None,
    },
    Q1Picture {
        item: "q1:weapon/shotgun",
        classic: Some("shotgun"),
        wheel: "shotgun1",
        ammo: Some("sb_shells"),
    },
    Q1Picture {
        item: "q1:weapon/supershotgun",
        classic: Some("sshotgun"),
        wheel: "shotgun2",
        ammo: Some("sb_shells"),
    },
    Q1Picture {
        item: "q1:weapon/nailgun",
        classic: Some("nailgun"),
        wheel: "nail1",
        ammo: Some("sb_nails"),
    },
    Q1Picture {
        item: "q1:weapon/supernailgun",
        classic: Some("snailgun"),
        wheel: "nail2",
        ammo: Some("sb_nails"),
    },
    Q1Picture {
        item: "q1:weapon/grenadelauncher",
        classic: Some("rlaunch"),
        wheel: "rocket1",
        ammo: Some("sb_rocket"),
    },
    Q1Picture {
        item: "q1:weapon/rocketlauncher",
        classic: Some("srlaunch"),
        wheel: "rocket2",
        ammo: Some("sb_rocket"),
    },
    Q1Picture {
        item: "q1:weapon/lightning",
        classic: Some("lightng"),
        wheel: "light",
        ammo: Some("sb_cells"),
    },
];

/// Hipnotic sbar.ts lumps and retail rerelease hipnotic/wwheel.txt slots 7-9.
const HIPNOTIC_PICTURES: &[Q1Picture] = &[
    Q1Picture {
        item: "q1:weapon/hipnotic:laser",
        classic: Some("laser"),
        wheel: "ui_h_weapon_laser",
        ammo: Some("sb_cells"),
    },
    Q1Picture {
        item: "q1:weapon/hipnotic:mjolnir",
        classic: Some("mjolnir"),
        wheel: "ui_h_weapon_mjolnir",
        ammo: Some("sb_cells"),
    },
    Q1Picture {
        item: "q1:weapon/hipnotic:proximity",
        classic: Some("prox"),
        wheel: "ui_h_weapon_gren",
        ammo: Some("sb_rocket"),
    },
];

const ROGUE_PICTURES: &[Q1Picture] = &[
    Q1Picture {
        item: "q1:weapon/rogue:lava-nailgun",
        classic: Some("r_lava"),
        wheel: "ui_r_weapon_lava",
        ammo: Some("r_ammolava"),
    },
    Q1Picture {
        item: "q1:weapon/rogue:lava-supernailgun",
        classic: Some("r_superlava"),
        wheel: "ui_r_weapon_superlava",
        ammo: Some("r_ammolava"),
    },
    Q1Picture {
        item: "q1:weapon/rogue:multi-grenade",
        classic: Some("r_gren"),
        wheel: "ui_r_weapon_gren",
        ammo: Some("r_ammoplasma"),
    },
    Q1Picture {
        item: "q1:weapon/rogue:multi-rocket",
        classic: Some("r_multirock"),
        wheel: "ui_r_weapon_multirock",
        ammo: Some("r_ammoplasma"),
    },
    Q1Picture {
        item: "q1:weapon/rogue:plasma",
        classic: Some("r_plasma"),
        wheel: "ui_r_weapon_plasma",
        ammo: Some("r_ammomulti"),
    },
];

const MG3_PICTURES: &[Q1Picture] = &[
    Q1Picture {
        item: "q1:weapon/mg3:laser",
        classic: Some("laser"),
        wheel: "ui_h_weapon_laser",
        ammo: Some("sb_cells"),
    },
    Q1Picture {
        item: "q1:weapon/mg3:mjolnir",
        classic: None,
        wheel: "axe",
        ammo: Some("sb_cells"),
    },
];

/// Q1 HUD picture rows for a campaign program (`q1WeaponPictures`).
fn q1_weapon_pictures(program: &str) -> Vec<&'static Q1Picture> {
    let extra = match program {
        "hipnotic" => HIPNOTIC_PICTURES,
        "rogue" => ROGUE_PICTURES,
        "mg3" => MG3_PICTURES,
        _ => &[],
    };
    Q1_PICTURES.iter().chain(extra.iter()).collect()
}

/// Q2 weapon definitions visible to a product (`q2WeaponDefinitions`).
///
/// Rerelease products see both mission packs; classic products see only
/// their own campaign pack. Shared with the weapon resource seam so the
/// roster stays single-sourced.
pub(crate) fn q2_weapon_definitions(product: &ProductExpectation) -> Vec<Q2WeaponDefinition> {
    let rerelease = product.edition == "rerelease";
    let mut definitions: Vec<Q2WeaponDefinition> = base_weapons().into_iter().map(|weapon| weapon.definition).collect();
    if rerelease || product.campaign == "xatrix" {
        definitions.extend(xatrix_weapon_definitions());
    }
    if rerelease || product.campaign == "rogue" {
        definitions.extend(rogue_weapon_definitions());
    }
    definitions
}

fn image(source: &ProviderReference, path: String) -> WeaponHudIcon {
    WeaponHudIcon::Image {
        resource: ResourceRequest {
            content: source.content.clone(),
            path,
        },
    }
}

fn wad(source: &ProviderReference, lump: String) -> WeaponHudIcon {
    WeaponHudIcon::WadPicture {
        resource: ResourceRequest {
            content: source.content.clone(),
            path: "gfx.wad".to_string(),
        },
        lump,
    }
}

fn q1_weapon_icons(source: &ProviderReference, product: &ProductExpectation, item: &str) -> Option<WeaponHudIcons> {
    if product.family != GameFamily::Q1
        || !["id1", "hipnotic", "rogue", "dopa", "mg1", "mg3"].contains(&product.campaign.as_str())
        || (product.edition != "classic" && product.edition != "rerelease")
    {
        return None;
    }
    let pictures = q1_weapon_pictures(&product.campaign)
        .into_iter()
        .find(|entry| entry.item == item)?;
    let rerelease = product.edition == "rerelease";
    let slot = if pictures.wheel.starts_with("ui_") {
        pictures.wheel.to_string()
    } else {
        format!("ww_{}", pictures.wheel)
    };
    let framed = |classic: &str, selected: bool| {
        if classic.starts_with("r_") {
            classic.to_string()
        } else if selected {
            format!("inv2_{classic}")
        } else {
            format!("inv_{classic}")
        }
    };
    let framed_icon = |selected: bool| {
        if rerelease {
            let frame = if selected { 2 } else { 1 };
            Some(image(source, format!("gfx/weapons/{slot}_{frame}.lmp")))
        } else {
            pictures.classic.map(|classic| wad(source, framed(classic, selected)))
        }
    };
    Some(WeaponHudIcons {
        weapon: framed_icon(false),
        selected_weapon: framed_icon(true),
        ammo: pictures.ammo.map(|ammo| wad(source, ammo.to_string())),
    })
}

fn q2_weapon_icons(source: &ProviderReference, product: &ProductExpectation, item: &str) -> Option<WeaponHudIcons> {
    if product.family != GameFamily::Q2
        || !["baseq2", "xatrix", "rogue", "mg2"].contains(&product.campaign.as_str())
        || (product.edition != "classic" && product.edition != "rerelease")
    {
        return None;
    }
    let pictures: Vec<(ItemId, String)> = q2_base_item_icons()
        .into_iter()
        .chain(q2_mission_weapon_icons())
        .collect();
    let picture = pictures.iter().find(|entry| entry.0 == item)?;
    let weapon = q2_weapon_definitions(product)
        .into_iter()
        .find(|entry| entry.item == item)?;
    let ammo = weapon
        .ammo
        .as_deref()
        .and_then(|ammo| pictures.iter().find(|entry| entry.0 == ammo));
    Some(WeaponHudIcons {
        weapon: Some(image(source, format!("pics/{}.pcx", picture.1))),
        selected_weapon: Some(image(source, format!("pics/{}.pcx", picture.1))),
        ammo: ammo.map(|ammo| image(source, format!("pics/{}.pcx", ammo.1))),
    })
}

fn q3_weapon_icons(source: &ProviderReference, product: &ProductExpectation, item: &str) -> Option<WeaponHudIcons> {
    if product.family != GameFamily::Q3 || (product.campaign != "baseq3" && product.campaign != "missionpack") {
        return None;
    }
    let definition = Q3_WEAPON_ITEMS.iter().find(|entry| entry.item == item)?;
    let campaign = match product.campaign.as_str() {
        "baseq3" => Product::Baseq3,
        _ => Product::Missionpack,
    };
    let items = item_list(campaign);
    let weapon = items
        .iter()
        .find(|entry| entry.item_type() == ItemType::ItWeapon && entry.tag() == definition.weapon as i32)?;
    let ammo = items
        .iter()
        .find(|entry| entry.item_type() == ItemType::ItAmmo && entry.tag() == definition.weapon as i32);
    let icon = |name: Option<&str>| {
        name.map(|name| WeaponHudIcon::Shader {
            content: source.content.clone(),
            name: name.to_string(),
        })
    };
    Some(WeaponHudIcons {
        weapon: icon(weapon.icon),
        selected_weapon: icon(weapon.icon),
        ammo: icon(ammo.and_then(|ammo| ammo.icon)),
    })
}

/// HUD icons for one weapon item (`weaponHudIcons`).
#[must_use]
pub fn weapon_hud_icons(
    source: &ProviderReference,
    product: &ProductExpectation,
    item: &str,
) -> Option<WeaponHudIcons> {
    q1_weapon_icons(source, product, item)
        .or_else(|| q2_weapon_icons(source, product, item))
        .or_else(|| q3_weapon_icons(source, product, item))
}

/// Retail missionpack scripts/gfx.shader maps these shader names to these images.
fn team_arena_image(name: &str) -> String {
    match name {
        "icons/iconw_nailgun" => "icons/nailgun128.tga",
        "icons/iconw_chaingun" => "icons/chaingun128.tga",
        "icons/iconw_proxlauncher" => "icons/proxmine.tga",
        "icons/icona_nailgun" => "icons/ammo_nailgun.tga",
        "icons/icona_chaingun" => "icons/ammo_chaingun.tga",
        "icons/icona_proxlauncher" => "icons/ammo_proxmine.tga",
        _ => return format!("{name}.tga"),
    }
    .to_string()
}

/// Push a path once, keeping donor insertion order.
fn admit_path(paths: &mut Vec<String>, path: String) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

/// Resource paths for every HUD icon of a product (`weaponHudResources`).
#[must_use]
pub fn weapon_hud_resources(source: &ProviderReference, product: &ProductExpectation) -> Vec<ResourceRequest> {
    let items: Vec<ItemId> = match product.family {
        GameFamily::Q1 => q1_weapon_pictures(&product.campaign)
            .into_iter()
            .map(|entry| entry.item.to_string())
            .collect(),
        GameFamily::Q2 => q2_weapon_definitions(product)
            .into_iter()
            .map(|entry| entry.item)
            .collect(),
        GameFamily::Q3 => Q3_WEAPON_ITEMS.iter().map(|entry| entry.item.clone()).collect(),
    };
    let mut paths: Vec<String> = Vec::new();
    for item in &items {
        let Some(icons) = weapon_hud_icons(source, product, item) else {
            continue;
        };
        for icon in [&icons.weapon, &icons.selected_weapon, &icons.ammo] {
            let Some(icon) = icon else {
                continue;
            };
            match icon {
                WeaponHudIcon::Shader { name, .. } => admit_path(&mut paths, team_arena_image(name)),
                WeaponHudIcon::Image { resource } | WeaponHudIcon::WadPicture { resource, .. } => {
                    admit_path(&mut paths, resource.path.clone());
                }
            }
        }
    }
    if !paths.is_empty() && product.family == GameFamily::Q1 {
        admit_path(&mut paths, "gfx/palette.lmp".to_string());
        if product.edition == "rerelease" {
            admit_path(&mut paths, "wwheel.txt".to_string());
        }
    }
    if !paths.is_empty() && product.family == GameFamily::Q2 {
        admit_path(&mut paths, "pics/colormap.pcx".to_string());
    }
    if !paths.is_empty() && product.family == GameFamily::Q3 && product.campaign == "missionpack" {
        admit_path(&mut paths, "scripts/gfx.shader".to_string());
    }
    paths
        .into_iter()
        .map(|path| ResourceRequest {
            content: source.content.clone(),
            path,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ContentId;
    use qa_core::identity::ProviderId;

    fn expectation(family: GameFamily, edition: &str, campaign: &str) -> ProductExpectation {
        ProductExpectation {
            id: format!("{family}-{edition}-{campaign}"),
            family,
            edition: edition.to_string(),
            campaign: campaign.to_string(),
            title: campaign.to_string(),
            content_directory: campaign.to_string(),
            base_product: None,
            required_content_archives: Vec::new(),
            required_programs: Vec::new(),
            map_witness: None,
            unresolved_reason: None,
        }
    }

    fn source(content: &str) -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("q1", "official"),
            content: ContentId(content.to_string()),
        }
    }

    #[test]
    fn q1_classic_icons_use_sbar_lumps() {
        let product = expectation(GameFamily::Q1, "classic", "id1");
        let icons = weapon_hud_icons(&source("content"), &product, "q1:weapon/shotgun").unwrap();
        assert!(matches!(
            icons.weapon,
            Some(WeaponHudIcon::WadPicture { ref lump, .. }) if lump == "inv_shotgun"
        ));
        assert!(matches!(
            icons.selected_weapon,
            Some(WeaponHudIcon::WadPicture { ref lump, .. }) if lump == "inv2_shotgun"
        ));
        assert!(matches!(
            icons.ammo,
            Some(WeaponHudIcon::WadPicture { ref lump, .. }) if lump == "sb_shells"
        ));
    }

    #[test]
    fn q1_classic_axe_has_no_slot() {
        let product = expectation(GameFamily::Q1, "classic", "id1");
        let icons = weapon_hud_icons(&source("content"), &product, "q1:weapon/axe").unwrap();
        assert_eq!(icons.weapon, None);
        assert_eq!(icons.selected_weapon, None);
        assert_eq!(icons.ammo, None);
    }

    #[test]
    fn q1_rerelease_icons_use_wheel_images() {
        let product = expectation(GameFamily::Q1, "rerelease", "id1");
        let icons = weapon_hud_icons(&source("content"), &product, "q1:weapon/shotgun").unwrap();
        assert!(matches!(
            icons.weapon,
            Some(WeaponHudIcon::Image { ref resource }) if resource.path == "gfx/weapons/ww_shotgun1_1.lmp"
        ));
        assert!(matches!(
            icons.selected_weapon,
            Some(WeaponHudIcon::Image { ref resource }) if resource.path == "gfx/weapons/ww_shotgun1_2.lmp"
        ));
        assert!(matches!(icons.ammo, Some(WeaponHudIcon::WadPicture { .. })));
    }

    #[test]
    fn q1_rogue_lumps_skip_inv_prefix() {
        let product = expectation(GameFamily::Q1, "classic", "rogue");
        let icons = weapon_hud_icons(&source("content"), &product, "q1:weapon/rogue:plasma").unwrap();
        assert!(matches!(
            icons.weapon,
            Some(WeaponHudIcon::WadPicture { ref lump, .. }) if lump == "r_plasma"
        ));
    }

    #[test]
    fn q1_unknown_item_or_campaign_is_null() {
        let product = expectation(GameFamily::Q1, "classic", "id1");
        assert_eq!(weapon_hud_icons(&source("content"), &product, "q1:weapon/nope"), None);
        let ctf = expectation(GameFamily::Q1, "classic", "ctf");
        assert_eq!(weapon_hud_icons(&source("content"), &ctf, "q1:weapon/shotgun"), None);
    }

    #[test]
    fn q2_icons_share_weapon_and_selected_pictures() {
        let product = expectation(GameFamily::Q2, "classic", "baseq2");
        let icons = weapon_hud_icons(&source("content"), &product, "q2:weapon_blaster").unwrap();
        assert_eq!(icons.weapon, icons.selected_weapon);
        assert!(matches!(
            icons.weapon,
            Some(WeaponHudIcon::Image { ref resource }) if resource.path.starts_with("pics/") && resource.path.ends_with(".pcx")
        ));
        assert_eq!(icons.ammo, None);
        let shotgun = weapon_hud_icons(&source("content"), &product, "q2:weapon_shotgun").unwrap();
        assert!(matches!(shotgun.ammo, Some(WeaponHudIcon::Image { .. })));
        assert_eq!(weapon_hud_icons(&source("content"), &product, "q2:weapon_nope"), None);
    }

    #[test]
    fn q3_icons_are_shaders() {
        let product = expectation(GameFamily::Q3, "classic", "baseq3");
        let icons = weapon_hud_icons(&source("content"), &product, "q3:weapon/machinegun").unwrap();
        assert!(matches!(icons.weapon, Some(WeaponHudIcon::Shader { .. })));
        assert_eq!(icons.weapon, icons.selected_weapon);
        assert!(matches!(icons.ammo, Some(WeaponHudIcon::Shader { .. })));
        let gauntlet = weapon_hud_icons(&source("content"), &product, "q3:weapon/gauntlet").unwrap();
        assert_eq!(gauntlet.ammo, None);
    }

    #[test]
    fn hud_resources_carry_family_extras() {
        let classic = expectation(GameFamily::Q1, "classic", "id1");
        let paths: Vec<String> = weapon_hud_resources(&source("a"), &classic)
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert!(paths.contains(&"gfx.wad".to_string()));
        assert!(paths.contains(&"gfx/palette.lmp".to_string()));
        assert!(!paths.contains(&"wwheel.txt".to_string()));

        let rerelease = expectation(GameFamily::Q1, "rerelease", "id1");
        let paths: Vec<String> = weapon_hud_resources(&source("a"), &rerelease)
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert!(paths.contains(&"wwheel.txt".to_string()));

        let q2 = expectation(GameFamily::Q2, "classic", "baseq2");
        let paths: Vec<String> = weapon_hud_resources(&source("a"), &q2)
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert!(paths.contains(&"pics/colormap.pcx".to_string()));

        let missionpack = expectation(GameFamily::Q3, "classic", "missionpack");
        let paths: Vec<String> = weapon_hud_resources(&source("a"), &missionpack)
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert!(paths.contains(&"scripts/gfx.shader".to_string()));
        assert!(paths.contains(&"icons/nailgun128.tga".to_string()));

        let baseq3 = expectation(GameFamily::Q3, "classic", "baseq3");
        let paths: Vec<String> = weapon_hud_resources(&source("a"), &baseq3)
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert!(!paths.contains(&"scripts/gfx.shader".to_string()));
    }
}
