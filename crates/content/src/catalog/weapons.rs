//! Selected-arsenal source adapters (`src/content/catalog/weapons.ts`).
//!
//! Canonicalizes selected weapon providers to their source adapters,
//! lists the weapon resource paths outside the map content, and admits
//! weapon provider timing. Q1 base weapon paths reuse the hoisted
//! worldspawn precache tables instead of duplicating them; the Q2
//! roster for a product is shared with the HUD icon seam
//! ([`super::weapon_hud`]) so both stay single-sourced.

use std::collections::HashMap;

use qa_core::identity::ProviderId;
use qa_core::numeric::Arithmetic;
use qa_core::time::ClockProfile;

use crate::contract::{ContentId, GameFamily, ProviderReference, ProviderTiming, ResourceRequest};
use crate::monsters::provider_text;
use crate::q1::foundation::precache_world::{Q1_WORLD_MODELS, Q1_WORLD_SOUNDS};
use crate::q1::missionpacks::types::MISSION_WEAPONS;
use crate::q2::foundation::weapons::definitions::base_weapons;
use crate::q2::foundation::weapons::types::Q2WeaponDefinition;

use super::weapon_hud::{q2_weapon_definitions, weapon_hud_resources};
use super::{
    equipment_providers, family_name, native_provider_timing, CatalogError, InstalledCatalog, LaunchWeaponSources,
    ProductExpectation,
};

/// Stock Quake weapon provider identities (`Q1_WEAPON_PROVIDERS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1WeaponProviders {
    /// Classic id1 weapons provider.
    pub classic: ProviderId,
    /// Rerelease id1 weapons provider.
    pub rerelease: ProviderId,
}

/// Stock Quake weapon provider identities (`Q1_WEAPON_PROVIDERS`).
#[must_use]
pub fn q1_weapon_providers() -> Q1WeaponProviders {
    Q1WeaponProviders {
        classic: super::provider_id("q1:weapons/classic/id1"),
        rerelease: super::provider_id("q1:weapons/rerelease/id1"),
    }
}

/// Stock Hipnotic weapon provider identities (`Q1_HIPNOTIC_WEAPON_PROVIDERS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1HipnoticWeaponProviders {
    /// Classic Hipnotic weapons provider.
    pub classic: ProviderId,
    /// Rerelease Hipnotic weapons provider.
    pub rerelease: ProviderId,
}

/// Stock Hipnotic weapon provider identities (`Q1_HIPNOTIC_WEAPON_PROVIDERS`).
#[must_use]
pub fn q1_hipnotic_weapon_providers() -> Q1HipnoticWeaponProviders {
    Q1HipnoticWeaponProviders {
        classic: super::provider_id("q1:weapons/classic/hipnotic"),
        rerelease: super::provider_id("q1:weapons/rerelease/hipnotic"),
    }
}

/// Stock Quake II weapon provider identities (`Q2_WEAPON_PROVIDERS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2WeaponProviders {
    /// Classic baseq2 weapons provider.
    pub classic: ProviderId,
    /// Rerelease baseq2 weapons provider.
    pub rerelease: ProviderId,
}

/// Stock Quake II weapon provider identities (`Q2_WEAPON_PROVIDERS`).
#[must_use]
pub fn q2_weapon_providers() -> Q2WeaponProviders {
    Q2WeaponProviders {
        classic: super::provider_id("q2:weapons/classic/baseq2"),
        rerelease: super::provider_id("q2:weapons/rerelease/baseq2"),
    }
}

/// Whether a product can supply a selected arsenal (`supportsSelectedWeaponProduct`).
#[must_use]
pub fn supports_selected_weapon_product(product: &ProductExpectation) -> bool {
    if product.family == GameFamily::Q3 {
        return product.campaign == "baseq3" || product.campaign == "missionpack";
    }
    if product.edition != "classic" && product.edition != "rerelease" {
        return false;
    }
    let rerelease_only = product.edition == "rerelease";
    match product.family {
        GameFamily::Q1 => {
            ["id1", "hipnotic", "rogue"].contains(&product.campaign.as_str())
                || (rerelease_only && ["dopa", "mg1", "mg3"].contains(&product.campaign.as_str()))
        }
        GameFamily::Q2 => {
            ["baseq2", "xatrix", "rogue"].contains(&product.campaign.as_str())
                || (rerelease_only && product.campaign == "mg2")
        }
        GameFamily::Q3 => false,
    }
}

fn is_equipment_weapon(provider: &ProviderId) -> bool {
    let equipment = equipment_providers();
    [
        &equipment.threewave,
        &equipment.ctf,
        &equipment.lmctf,
        &equipment.hand_grenades,
    ]
    .contains(&provider)
}

fn family_title(family: GameFamily) -> &'static str {
    match family {
        GameFamily::Q1 => "Q1",
        GameFamily::Q2 => "Q2",
        GameFamily::Q3 => "Q3",
    }
}

fn canonical_q3_weapon_source(
    map: &ProviderReference,
    weapon: &ProviderReference,
    catalog: &InstalledCatalog,
) -> Result<ProviderReference, CatalogError> {
    let weapon_text = provider_text(&weapon.provider);
    let product = &catalog.require(weapon.content.as_str())?.expectation;
    if product.family != GameFamily::Q3 || !supports_selected_weapon_product(product) {
        return Ok(weapon.clone());
    }
    let provider = super::provider_id(&format!("q3:weapons/{}/{}", product.edition, product.campaign));
    if weapon_text != "q3:official" && weapon.provider != provider {
        return Err(super::failed(format!(
            "Selected Q3 weapon role does not match {}",
            weapon.content.as_str()
        )));
    }
    let map_text = provider_text(&map.provider);
    if weapon.content == map.content && map_text == "q3:official" {
        return Ok(map.clone());
    }
    if product.campaign == "baseq3" && !map_text.starts_with("q3:") && weapon_text == "q3:official" {
        return Ok(weapon.clone());
    }
    Ok(ProviderReference {
        provider,
        content: weapon.content.clone(),
    })
}

/// Canonical weapon provider for a map (`canonicalWeaponSource`).
pub fn canonical_weapon_source(
    map: &ProviderReference,
    weapon: &ProviderReference,
    catalog: &InstalledCatalog,
) -> Result<ProviderReference, CatalogError> {
    if is_equipment_weapon(&weapon.provider) {
        return Ok(weapon.clone());
    }
    let weapon_text = provider_text(&weapon.provider);
    if weapon_text.starts_with("q3:") {
        return canonical_q3_weapon_source(map, weapon, catalog);
    }
    let family = if weapon_text.starts_with("q1:") {
        Some(GameFamily::Q1)
    } else if weapon_text.starts_with("q2:") {
        Some(GameFamily::Q2)
    } else {
        None
    };
    let Some(family) = family else {
        return Ok(weapon.clone());
    };
    let tag = family_name(family);
    let role_prefix = format!("{tag}:weapons/");
    let official = format!("{tag}:official");
    let role = weapon_text.starts_with(&role_prefix);
    if !role && weapon_text != official {
        return Err(super::failed(format!(
            "Unsupported {} weapon provider {weapon_text}",
            family_title(family)
        )));
    }
    let product = &catalog.require(weapon.content.as_str())?.expectation;
    if product.family != family || !supports_selected_weapon_product(product) {
        if role || weapon.content != map.content || product.family != family {
            return Err(super::failed(format!(
                "Selected {} arsenal has no source adapter: {}",
                family_title(family),
                weapon.content.as_str()
            )));
        }
        return Ok(weapon.clone());
    }
    let provider = super::provider_id(&format!("{tag}:weapons/{}/{}", product.edition, product.campaign));
    if role && weapon.provider != provider {
        return Err(super::failed(format!(
            "Selected weapon role {weapon_text} does not match {}",
            weapon.content.as_str()
        )));
    }
    if weapon.content == map.content && provider_text(&map.provider) == official {
        return Ok(map.clone());
    }
    Ok(ProviderReference {
        provider,
        content: weapon.content.clone(),
    })
}

/// Q1 projectile models in the precache filter
/// (`/^progs\/(missile|grenade|spike|s_spike|bolt2)\.mdl$/`).
fn q1_projectile_model(path: &str) -> bool {
    path.strip_prefix("progs/")
        .and_then(|stem| stem.strip_suffix(".mdl"))
        .is_some_and(|stem| matches!(stem, "missile" | "grenade" | "spike" | "s_spike" | "bolt2"))
}

/// Q1 base weapon paths (`q1BaseWeaponPaths`).
///
/// Filters the hoisted worldspawn precache tables the way the donor's
/// path-collecting callbacks do: weapon/axe-hit sounds plus view and
/// projectile models.
fn q1_base_weapon_paths() -> Vec<String> {
    let mut paths = vec!["gfx/palette.lmp".to_string()];
    for sound in Q1_WORLD_SOUNDS {
        if sound.starts_with("weapons/") || sound.starts_with("player/axhit") {
            paths.push(format!("sound/{sound}"));
        }
    }
    for model in Q1_WORLD_MODELS {
        if model.starts_with("progs/v_") || q1_projectile_model(model) {
            paths.push((*model).to_string());
        }
    }
    paths
}

/// Mission-pack weapon view/world models for one pack id prefix.
fn q1_mission_weapon_models(prefix: &str) -> Vec<String> {
    MISSION_WEAPONS
        .iter()
        .filter(|weapon| weapon.id.as_str().starts_with(prefix))
        .flat_map(|weapon| [weapon.model.to_string(), weapon.world_model.to_string()])
        .collect()
}

/// Hipnotic weapon paths (`q1HipnoticWeaponPaths`).
fn q1_hipnotic_weapon_paths() -> Vec<String> {
    let mut paths = q1_mission_weapon_models("hipnotic:");
    paths.push("progs/lasrspik.mdl".to_string());
    paths.push("progs/proxbomb.mdl".to_string());
    paths.extend(
        [
            "hipweap/laserg.wav",
            "hipweap/laserric.wav",
            "enforcer/enfstop.wav",
            "hipweap/proxbomb.wav",
            "hipweap/proxwarn.wav",
            "hipweap/mjoltink.wav",
            "knight/sword1.wav",
            "hipweap/mjolslap.wav",
            "hipweap/mjolhit.wav",
        ]
        .map(|path| format!("sound/{path}")),
    );
    paths
}

/// Rogue weapon paths (`q1RogueWeaponPaths`).
fn q1_rogue_weapon_paths() -> Vec<String> {
    let mut paths = q1_mission_weapon_models("rogue:");
    paths.extend(
        [
            "progs/hook.mdl",
            "progs/v_grpple.mdl",
            "sound/weapons/chain1.wav",
            "sound/pendulum/hit.wav",
            "progs/lspike.mdl",
            "progs/mervup.mdl",
            "progs/rockup.mdl",
            "progs/rockup_d.mdl",
            "progs/plasma.mdl",
            "sound/plasma/flight.wav",
            "sound/plasma/fire.wav",
            "sound/plasma/explode.wav",
            "sound/weapons/spike2.wav",
            "sound/weapons/lhit.wav",
        ]
        .map(str::to_string),
    );
    paths
}

/// MachineGames episode 3 weapon paths (`q1Mg3WeaponPaths`).
fn q1_mg3_weapon_paths() -> Vec<String> {
    let mut paths = [
        "progs/lasrspik.mdl",
        "progs/v_laserg.mdl",
        "progs/v_hammer.mdl",
        "progs/v_hammer_glow.mdl",
        "progs/v_bloodshot.mdl",
        "progs/v_bloodshot2.mdl",
    ]
    .map(str::to_string)
    .to_vec();
    paths.extend(
        q1_hipnotic_weapon_paths()
            .into_iter()
            .filter(|path| path.starts_with("sound/") && !path.contains("prox")),
    );
    paths
}

/// Shared base weapon/projectile effect sounds (`q2BaseWeaponPaths`).
const Q2_BASE_WEAPON_SOUNDS: &[&str] = &[
    "blastf1a", "shotgf1b", "shotgr1b", "sshotf1b", "machgf1b", "machgf2b", "machgf3b", "machgf4b", "machgf5b",
    "chngnu1a", "chngnl1a", "chngnd1a", "hgrent1a", "hgrena1b", "hgrenc1b", "hgrenb1a", "hgrenb2a", "grenlf1a",
    "grenlr1b", "grenlb1b", "rockfly", "rocklf1a", "rocklr1b", "hyprbu1a", "hyprbl1a", "hyprbf1a", "hyprbd1a",
    "rg_hum", "railgf1a", "bfg__f1y", "bfg__l1a", "bfg__x1b", "bfg_hum", "noammo",
];

/// Base g_items precaches plus the shared base weapon/projectile effect paths.
fn q2_base_weapon_paths(rerelease: bool) -> Vec<String> {
    let mut models: Vec<String> = base_weapons()
        .into_iter()
        .flat_map(|weapon| [weapon.definition.view_model, weapon.definition.world_model])
        .filter(|path| !path.is_empty())
        .collect();
    let (grenade, grenade2) = if rerelease {
        ("grenade4", "grenade3")
    } else {
        ("grenade", "grenade2")
    };
    models.extend(
        [
            "laser",
            "rocket",
            "debris2",
            "smoke",
            "explode",
            "r_explode",
            grenade,
            grenade2,
        ]
        .map(|name| format!("models/objects/{name}/tris.md2")),
    );
    let mut paths = Vec::new();
    for model in &models {
        paths.push(model.clone());
        if model == "models/objects/r_explode/tris.md2" {
            paths.extend((1..=7).map(|index| format!("models/objects/r_explode/skin{index}.pcx")));
        } else {
            paths.push(model.replacen("tris.md2", "skin.pcx", 1));
        }
    }
    if rerelease {
        paths.extend(
            [
                "models/objects/explode/rskin.pcx",
                "models/objects/explode/skin2.pcx",
                "models/objects/laser/skinb.pcx",
                "models/objects/laser/sking.pcx",
            ]
            .map(str::to_string),
        );
    } else {
        paths.push("models/weapons/v_proxyl/skin.pcx".to_string());
    }
    paths.push("pics/colormap.pcx".to_string());
    paths.push("sound/misc/lasfly.wav".to_string());
    paths.extend(
        Q2_BASE_WEAPON_SOUNDS
            .iter()
            .map(|name| format!("sound/weapons/{name}.wav")),
    );
    for (index, count) in [2, 4, 6].iter().enumerate() {
        let slot = index + 1;
        paths.push(format!("sprites/s_bfg{slot}.sp2"));
        paths.extend((0..*count).map(|frame| format!("sprites/s_bfg{slot}_{frame}.pcx")));
    }
    if rerelease {
        paths.push("sound/weapons/change.wav".to_string());
        paths.push("sound/weapons/lowammo.wav".to_string());
    }
    paths
}

/// Expansion g_items precaches plus projectile dependencies used by their source callbacks.
fn q2_expansion_weapon_paths(name: &str) -> &'static [&'static str] {
    match name {
        "trap" => &[
            "models/weapons/z_trap/tris.md2",
            "models/objects/trapfx/tris.md2",
            "models/objects/gibs/chest/tris.md2",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/misc/fhit3.wav",
            "sound/weapons/trapcock.wav",
            "sound/weapons/traploop.wav",
            "sound/weapons/trapsuck.wav",
            "sound/weapons/trapdown.wav",
            "sound/items/s_health.wav",
        ],
        "ionripper" => &[
            "models/objects/boomrang/tris.md2",
            "sound/weapons/rg_hum.wav",
            "sound/weapons/rippfire.wav",
            "sound/misc/lasfly.wav",
        ],
        "phalanx" => &[
            "sprites/s_photon.sp2",
            "sound/weapons/plasshot.wav",
            "sound/weapons/rockfly.wav",
        ],
        "tesla" => &[
            "models/weapons/g_tesla/tris.md2",
            "sound/weapons/teslaopen.wav",
            "sound/weapons/hgrenb1a.wav",
            "sound/weapons/hgrenb2a.wav",
        ],
        "proxlauncher" => &[
            "models/weapons/g_prox/tris.md2",
            "sound/weapons/grenlf1a.wav",
            "sound/weapons/grenlr1b.wav",
            "sound/weapons/grenlb1b.wav",
            "sound/weapons/proxwarn.wav",
            "sound/weapons/proxopen.wav",
        ],
        "chainfist" => &[
            "sound/weapons/sawidle.wav",
            "sound/weapons/sawhit.wav",
            "sound/weapons/sawslice.wav",
        ],
        "disintegrator" => &[
            "models/proj/disintegrator/tris.md2",
            "sound/weapons/disrupt.wav",
            "sound/weapons/disint2.wav",
            "sound/weapons/disrupthit.wav",
        ],
        "etf_rifle" => &["models/proj/flechette/tris.md2", "sound/weapons/nail1.wav"],
        "heatbeam" => &["sound/weapons/bfg__l1a.wav"],
        _ => &[],
    }
}

/// Registered weapon resource paths (`q2RegisteredWeaponResources`).
///
/// The donor reads the roster through the live weapon registry; the
/// caller builds that combined list instead (base plus the product's
/// mission-pack definitions, per the `weaponResources` call site).
pub fn q2_registered_weapon_resources(definitions: &[Q2WeaponDefinition], rerelease: bool) -> Vec<String> {
    let mut paths = q2_base_weapon_paths(rerelease);
    for weapon in definitions {
        paths.push(weapon.view_model.clone());
        paths.push(weapon.world_model.clone());
        paths.extend(
            q2_expansion_weapon_paths(&weapon.name)
                .iter()
                .filter(|path| rerelease || **path != "sound/weapons/sawslice.wav")
                .map(ToString::to_string),
        );
        if weapon.name == "tesla" && !rerelease {
            paths.push("models/weapons/v_tesla2/tris.md2".to_string());
        }
        if weapon.name == "heatbeam" && !rerelease {
            paths.push("models/weapons/v_beamer2/tris.md2".to_string());
        }
    }
    if rerelease {
        paths.push("sound/weapons/railgr1b.wav".to_string());
    }
    paths.into_iter().filter(|path| !path.is_empty()).collect()
}

fn q1_product_weapon_paths(campaign: &str) -> Vec<String> {
    let mut paths = q1_base_weapon_paths();
    match campaign {
        "hipnotic" => paths.extend(q1_hipnotic_weapon_paths()),
        "rogue" => paths.extend(q1_rogue_weapon_paths()),
        "mg3" => paths.extend(q1_mg3_weapon_paths()),
        _ => {}
    }
    paths
}

/// Weapon resource requests for selected arsenals (`weaponResources`).
pub fn weapon_resources(
    map: &ProviderReference,
    weapons: &[ProviderReference],
    catalog: &InstalledCatalog,
) -> Result<Vec<ResourceRequest>, CatalogError> {
    let mut out = Vec::new();
    for reference in weapons {
        let weapon = canonical_weapon_source(map, reference, catalog)?;
        if is_equipment_weapon(&weapon.provider) {
            continue;
        }
        let weapon_text = provider_text(&weapon.provider);
        let product = &catalog.require(weapon.content.as_str())?.expectation;
        let classic_or_rerelease = product.edition == "classic" || product.edition == "rerelease";
        if weapon_text.starts_with("q2:")
            && product.family == GameFamily::Q2
            && ["baseq2", "xatrix", "rogue", "mg2"].contains(&product.campaign.as_str())
            && classic_or_rerelease
        {
            let rerelease = product.edition == "rerelease";
            out.extend(
                q2_registered_weapon_resources(&q2_weapon_definitions(product), rerelease)
                    .into_iter()
                    .map(|path| ResourceRequest {
                        content: weapon.content.clone(),
                        path,
                    }),
            );
            out.extend(weapon_hud_resources(&weapon, product));
            continue;
        }
        if weapon_text.starts_with("q3:") && product.family == GameFamily::Q3 {
            out.extend(weapon_hud_resources(&weapon, product));
            continue;
        }
        if !weapon_text.starts_with("q1:")
            || product.family != GameFamily::Q1
            || !["id1", "hipnotic", "rogue", "dopa", "mg1", "mg3"].contains(&product.campaign.as_str())
            || !classic_or_rerelease
        {
            continue;
        }
        out.extend(
            q1_product_weapon_paths(&product.campaign)
                .into_iter()
                .map(|path| ResourceRequest {
                    content: weapon.content.clone(),
                    path,
                }),
        );
        out.extend(weapon_hud_resources(&weapon, product));
    }
    Ok(out)
}

/// Weapon resources outside the map content (`selectedWeaponResources`).
pub fn selected_weapon_resources(
    map: &ProviderReference,
    weapons: &[ProviderReference],
    catalog: &InstalledCatalog,
) -> Result<Vec<ResourceRequest>, CatalogError> {
    Ok(weapon_resources(map, weapons, catalog)?
        .into_iter()
        .filter(|resource| resource.content != map.content)
        .collect())
}

/// Weapon provider timing (`selectedWeaponTiming`).
pub fn selected_weapon_timing(
    map: &ProviderReference,
    weapons: &[ProviderReference],
    catalog: &InstalledCatalog,
) -> Result<Vec<ProviderTiming>, CatalogError> {
    let map_family = catalog.require(map.content.as_str())?.expectation.family;
    let mut providers: HashMap<ProviderId, ContentId> = HashMap::new();
    let mut out = Vec::new();
    for reference in weapons {
        let weapon = canonical_weapon_source(map, reference, catalog)?;
        if is_equipment_weapon(&weapon.provider) {
            continue;
        }
        if weapon.content == map.content {
            continue;
        }
        let product = &catalog.require(weapon.content.as_str())?.expectation;
        let tag = family_name(product.family);
        let weapon_text = provider_text(&weapon.provider);
        if !supports_selected_weapon_product(product) || !weapon_text.starts_with(&format!("{tag}:")) {
            continue;
        }
        let prior = providers.get(&weapon.provider);
        if weapon.provider == map.provider || (prior.is_some() && prior != Some(&weapon.content)) {
            return Err(super::failed(format!(
                "Selected weapon provider {weapon_text} has conflicting content"
            )));
        }
        if (map_family == product.family && !weapon_text.starts_with(&format!("{tag}:weapons/"))) || prior.is_some() {
            continue;
        }
        providers.insert(weapon.provider.clone(), weapon.content.clone());
        out.push(native_provider_timing(
            &weapon,
            product.family,
            product.edition == "rerelease",
        ));
    }
    Ok(out)
}

/// Admit one weapon timing row (`admitWeaponTiming`).
///
/// The numeric comparison covers the Rust profile fields; donor-only
/// rounding, scalar-storage, and integer-overflow details have no
/// counterpart (storage is always binary32, overflow always wraps).
/// Likewise the Q2 classic clock carries no frame length and the Q2
/// rerelease/Q3 clocks carry no preparation/maximum-command fields.
pub fn admit_weapon_timing(timing: &mut Vec<ProviderTiming>, weapon: &ProviderTiming) -> Result<(), CatalogError> {
    let Some(existing) = timing.iter().find(|entry| entry.provider == weapon.provider) else {
        timing.push(weapon.clone());
        return Ok(());
    };
    let numeric_matches = existing.numeric.id == weapon.numeric.id
        && existing.numeric.arithmetic == Arithmetic::Binary32EachOp
        && weapon.numeric.arithmetic == Arithmetic::Binary32EachOp
        && existing.numeric.float_to_int == weapon.numeric.float_to_int;
    let clock_matches = match (&existing.clock, &weapon.clock) {
        (
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: min,
                maximum_frame_seconds: max,
                fixed_frame_seconds: fixed,
            },
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: want_min,
                maximum_frame_seconds: want_max,
                fixed_frame_seconds: want_fixed,
            },
        ) => min == want_min && max == want_max && fixed == want_fixed,
        (ClockProfile::Q2Classic, ClockProfile::Q2Classic) => true,
        (
            ClockProfile::Q2Rerelease { frame_milliseconds },
            ClockProfile::Q2Rerelease {
                frame_milliseconds: wanted,
            },
        ) => frame_milliseconds == wanted,
        (
            ClockProfile::Q3 {
                server_frame_milliseconds: server,
                fixed_movement_milliseconds: fixed,
            },
            ClockProfile::Q3 {
                server_frame_milliseconds: want_server,
                fixed_movement_milliseconds: want_fixed,
            },
        ) => server == want_server && fixed == want_fixed,
        _ => false,
    };
    if !numeric_matches || !clock_matches {
        return Err(super::failed(format!(
            "Selected weapon provider {} has conflicting timing",
            provider_text(&weapon.provider)
        )));
    }
    Ok(())
}

/// Catalog-owned selected-arsenal adapters.
///
/// Implements the launch seam ([`LaunchWeaponSources`]) with the
/// functions above so launch resolution stays free of direct
/// mission-pack dependencies.
#[derive(Debug, Clone, Copy, Default)]
pub struct CatalogWeaponSources;

impl LaunchWeaponSources for CatalogWeaponSources {
    fn canonical_weapon_source(
        &self,
        map: &ProviderReference,
        weapon: &ProviderReference,
        catalog: &InstalledCatalog,
    ) -> Result<ProviderReference, CatalogError> {
        canonical_weapon_source(map, weapon, catalog)
    }

    fn selected_weapon_resources(
        &self,
        map: &ProviderReference,
        weapons: &[ProviderReference],
        catalog: &InstalledCatalog,
    ) -> Result<Vec<ResourceRequest>, CatalogError> {
        selected_weapon_resources(map, weapons, catalog)
    }

    fn selected_weapon_timing(
        &self,
        map: &ProviderReference,
        weapons: &[ProviderReference],
        catalog: &InstalledCatalog,
    ) -> Result<Vec<ProviderTiming>, CatalogError> {
        selected_weapon_timing(map, weapons, catalog)
    }

    fn admit_weapon_timing(
        &self,
        timing: &mut Vec<ProviderTiming>,
        weapon: &ProviderTiming,
    ) -> Result<(), CatalogError> {
        admit_weapon_timing(timing, weapon)
    }

    fn weapon_provider_ids(&self) -> Vec<ProviderId> {
        let q1 = q1_weapon_providers();
        let hipnotic = q1_hipnotic_weapon_providers();
        let q2 = q2_weapon_providers();
        vec![
            q1.classic,
            q1.rerelease,
            hipnotic.classic,
            hipnotic.rerelease,
            q2.classic,
            q2.rerelease,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogProduct, ProductAvailability};
    use crate::contract::{create_content_id, ContentIdentity};
    use crate::monsters::provider_text;
    use qa_core::numeric::{Arithmetic, FloatToInt};

    fn expectation(id: &str, family: GameFamily, edition: &str, campaign: &str) -> ProductExpectation {
        ProductExpectation {
            id: id.to_string(),
            family,
            edition: edition.to_string(),
            campaign: campaign.to_string(),
            title: id.to_string(),
            content_directory: campaign.to_string(),
            base_product: None,
            required_content_archives: Vec::new(),
            required_programs: Vec::new(),
            map_witness: None,
            unresolved_reason: None,
        }
    }

    fn installed_product(
        id: &str,
        family: GameFamily,
        edition: &str,
        campaign: &str,
        revision: &str,
    ) -> CatalogProduct {
        let content = create_content_id(&ContentIdentity {
            family,
            edition: edition.to_string(),
            package: campaign.to_string(),
            revision: revision.to_string(),
        })
        .unwrap();
        CatalogProduct {
            id: content,
            expectation: expectation(id, family, edition, campaign),
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "/corpus".to_string(),
            vec![
                installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", "installed"),
                installed_product(
                    "q1-rerelease-hipnotic",
                    GameFamily::Q1,
                    "rerelease",
                    "hipnotic",
                    "installed",
                ),
                installed_product("q1-classic-dopa", GameFamily::Q1, "classic", "dopa", "installed"),
                installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", "installed"),
                installed_product("q2-classic-baseq2-alt", GameFamily::Q2, "classic", "baseq2", "alt"),
                installed_product("q3-classic-baseq3", GameFamily::Q3, "classic", "baseq3", "installed"),
            ],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
    }

    fn content(catalog: &InstalledCatalog, id: &str) -> ContentId {
        catalog.product(id).unwrap().id.clone()
    }

    fn reference(provider: &str, content: ContentId) -> ProviderReference {
        let (namespace, name) = provider.split_once(':').unwrap();
        ProviderReference {
            provider: ProviderId::new(namespace, name),
            content,
        }
    }

    fn definition(name: &str, view_model: &str, world_model: &str) -> Q2WeaponDefinition {
        Q2WeaponDefinition {
            name: name.to_string(),
            item: format!("q2:weapon_{name}"),
            classname: format!("weapon_{name}"),
            ammo: None,
            quantity: 1,
            warning: 0,
            view_model: view_model.to_string(),
            world_model: world_model.to_string(),
            player_model: 0,
            activate_last: 0,
            fire_last: 0,
            idle_last: 0,
            deactivate_last: 0,
            pauses: Vec::new(),
            fires: Vec::new(),
            repeating: false,
        }
    }

    #[test]
    fn supports_matrix() {
        assert!(supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q3,
            "classic",
            "baseq3"
        )));
        assert!(supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q3,
            "rerelease",
            "missionpack"
        )));
        assert!(!supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q3,
            "classic",
            "arena"
        )));
        assert!(supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q1,
            "classic",
            "rogue"
        )));
        assert!(!supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q1,
            "classic",
            "dopa"
        )));
        assert!(supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q1,
            "rerelease",
            "mg3"
        )));
        assert!(!supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q1,
            "demo",
            "id1"
        )));
        assert!(supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q2,
            "classic",
            "xatrix"
        )));
        assert!(!supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q2,
            "classic",
            "mg2"
        )));
        assert!(supports_selected_weapon_product(&expectation(
            "a",
            GameFamily::Q2,
            "rerelease",
            "mg2"
        )));
    }

    #[test]
    fn canonical_q1_official_resolves_role() {
        let catalog = catalog();
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let weapon = reference("q1:official", content(&catalog, "q1-rerelease-hipnotic"));
        let canonical = canonical_weapon_source(&map, &weapon, &catalog).unwrap();
        assert_eq!(provider_text(&canonical.provider), "q1:weapons/rerelease/hipnotic");
        assert_eq!(canonical.content, weapon.content);
    }

    #[test]
    fn canonical_same_content_official_map_wins() {
        let catalog = catalog();
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let weapon = reference("q1:official", content(&catalog, "q1-classic-id1"));
        assert_eq!(canonical_weapon_source(&map, &weapon, &catalog).unwrap(), map);
    }

    #[test]
    fn canonical_rejects_mismatches() {
        let catalog = catalog();
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let role = reference("q1:weapons/classic/id1", content(&catalog, "q1-rerelease-hipnotic"));
        let error = canonical_weapon_source(&map, &role, &catalog).unwrap_err().to_string();
        assert!(error.contains("Selected weapon role"), "{error}");
        let bogus = reference("q1:bogus", content(&catalog, "q1-classic-id1"));
        let error = canonical_weapon_source(&map, &bogus, &catalog).unwrap_err().to_string();
        assert!(error.contains("Unsupported Q1 weapon provider"), "{error}");
        let dopa_role = reference("q1:weapons/classic/dopa", content(&catalog, "q1-classic-dopa"));
        let error = canonical_weapon_source(&map, &dopa_role, &catalog)
            .unwrap_err()
            .to_string();
        assert!(error.contains("has no source adapter"), "{error}");
    }

    #[test]
    fn canonical_passes_through_equipment_and_foreign_families() {
        let catalog = catalog();
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let equipment = equipment_providers();
        let grapple = ProviderReference {
            provider: equipment.threewave,
            content: content(&catalog, "q1-classic-id1"),
        };
        assert_eq!(canonical_weapon_source(&map, &grapple, &catalog).unwrap(), grapple);
        let foreign = reference("q9:official", content(&catalog, "q1-rerelease-hipnotic"));
        assert_eq!(canonical_weapon_source(&map, &foreign, &catalog).unwrap(), foreign);
        let dopa_map = reference("q1:official", content(&catalog, "q1-classic-dopa"));
        let dopa = reference("q1:official", content(&catalog, "q1-classic-dopa"));
        assert_eq!(canonical_weapon_source(&dopa_map, &dopa, &catalog).unwrap(), dopa);
    }

    #[test]
    fn canonical_q3_branches() {
        let catalog = catalog();
        let q3 = content(&catalog, "q3-classic-baseq3");
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let official = reference("q3:official", q3.clone());
        assert_eq!(canonical_weapon_source(&map, &official, &catalog).unwrap(), official);
        let q3_map = reference("q3:official", q3.clone());
        assert_eq!(canonical_weapon_source(&q3_map, &official, &catalog).unwrap(), q3_map);
        let role = reference("q3:weapons/classic/missionpack", q3);
        let error = canonical_weapon_source(&map, &role, &catalog).unwrap_err().to_string();
        assert!(error.contains("Selected Q3 weapon role does not match"), "{error}");
    }

    #[test]
    fn q1_base_paths_filter_precache_tables() {
        let paths = q1_base_weapon_paths();
        assert_eq!(paths[0], "gfx/palette.lmp");
        assert!(paths.contains(&"sound/weapons/r_exp3.wav".to_string()));
        assert!(paths.contains(&"sound/player/axhit1.wav".to_string()));
        assert!(!paths.iter().any(|path| path == "sound/misc/talk.wav"));
        assert!(paths.contains(&"progs/v_axe.mdl".to_string()));
        assert!(paths.contains(&"progs/missile.mdl".to_string()));
        assert!(paths.contains(&"progs/s_spike.mdl".to_string()));
        assert!(paths.contains(&"progs/bolt2.mdl".to_string()));
        assert!(!paths.iter().any(|path| path == "progs/player.mdl"));
        assert!(!paths.iter().any(|path| path == "progs/bolt.mdl"));
    }

    #[test]
    fn q1_expansion_paths() {
        let hipnotic = q1_hipnotic_weapon_paths();
        assert!(hipnotic.contains(&"progs/v_laserg.mdl".to_string()));
        assert!(hipnotic.contains(&"progs/g_laserg.mdl".to_string()));
        assert!(hipnotic.contains(&"progs/lasrspik.mdl".to_string()));
        let rogue = q1_rogue_weapon_paths();
        assert!(rogue.contains(&"progs/v_lava.mdl".to_string()));
        assert!(rogue.contains(&"progs/hook.mdl".to_string()));
        let mg3 = q1_mg3_weapon_paths();
        assert!(mg3.contains(&"progs/v_hammer_glow.mdl".to_string()));
        assert!(mg3.contains(&"sound/hipweap/laserg.wav".to_string()));
        assert!(!mg3.iter().any(|path| path.contains("prox")));
        assert!(!mg3.iter().any(|path| path == "progs/proxbomb.mdl"));
    }

    #[test]
    fn q2_base_paths_cover_editions() {
        let classic = q2_base_weapon_paths(false);
        assert!(classic.contains(&"models/weapons/v_blast/tris.md2".to_string()));
        assert!(classic.contains(&"models/weapons/v_blast/skin.pcx".to_string()));
        assert!(classic.contains(&"models/objects/r_explode/skin7.pcx".to_string()));
        assert!(classic.contains(&"models/objects/grenade/tris.md2".to_string()));
        assert!(classic.contains(&"models/weapons/v_proxyl/skin.pcx".to_string()));
        assert!(!classic.iter().any(|path| path.contains("grenade4")));
        assert!(!classic.iter().any(|path| path == "sound/weapons/change.wav"));
        assert!(classic.contains(&"sprites/s_bfg1.sp2".to_string()));
        assert!(classic.contains(&"sprites/s_bfg3_5.pcx".to_string()));
        assert!(!classic.iter().any(|path| path == "sprites/s_bfg1_2.pcx"));

        let rerelease = q2_base_weapon_paths(true);
        assert!(rerelease.contains(&"models/objects/grenade4/tris.md2".to_string()));
        assert!(rerelease.contains(&"models/objects/laser/skinb.pcx".to_string()));
        assert!(rerelease.contains(&"sound/weapons/change.wav".to_string()));
        assert!(rerelease.contains(&"sound/weapons/lowammo.wav".to_string()));
        assert!(!rerelease.iter().any(|path| path == "models/weapons/v_proxyl/skin.pcx"));
    }

    #[test]
    fn q2_registered_resources_cover_expansions() {
        let definitions = vec![
            definition("tesla", "v_tesla.md2", "g_tesla.md2"),
            definition("chainfist", "v_chain.md2", ""),
        ];
        let classic = q2_registered_weapon_resources(&definitions, false);
        assert!(!classic.iter().any(|path| path.is_empty()));
        assert!(classic.contains(&"models/weapons/v_tesla2/tris.md2".to_string()));
        assert!(classic.contains(&"sound/weapons/sawidle.wav".to_string()));
        assert!(!classic.iter().any(|path| path == "sound/weapons/sawslice.wav"));
        assert!(!classic.iter().any(|path| path == "sound/weapons/railgr1b.wav"));

        let rerelease = q2_registered_weapon_resources(&definitions, true);
        assert!(rerelease.contains(&"sound/weapons/sawslice.wav".to_string()));
        assert!(rerelease.contains(&"sound/weapons/railgr1b.wav".to_string()));
        assert!(!rerelease.iter().any(|path| path == "models/weapons/v_tesla2/tris.md2"));
    }

    #[test]
    fn weapon_resources_cover_families() {
        let catalog = catalog();
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));

        let hipnotic = reference("q1:official", content(&catalog, "q1-rerelease-hipnotic"));
        let requests = weapon_resources(&map, &[hipnotic], &catalog).unwrap();
        assert!(requests.iter().any(|request| request.path == "progs/v_laserg.mdl"));
        assert!(requests.iter().any(|request| request.path == "gfx.wad"));
        assert!(requests
            .iter()
            .all(|request| request.content == content(&catalog, "q1-rerelease-hipnotic")));

        let same = reference("q1:official", content(&catalog, "q1-classic-id1"));
        assert!(selected_weapon_resources(&map, &[same], &catalog).unwrap().is_empty());

        let q2 = reference("q2:official", content(&catalog, "q2-classic-baseq2"));
        let requests = weapon_resources(&map, &[q2], &catalog).unwrap();
        assert!(requests
            .iter()
            .any(|request| request.path == "models/weapons/v_blast/tris.md2"));
        assert!(requests.iter().any(|request| request.path == "pics/colormap.pcx"));

        let q3 = reference("q3:official", content(&catalog, "q3-classic-baseq3"));
        let requests = weapon_resources(&map, &[q3], &catalog).unwrap();
        assert!(!requests.is_empty());
        assert!(requests
            .iter()
            .all(|request| request.content == content(&catalog, "q3-classic-baseq3")));
    }

    #[test]
    fn selected_timing_admits_and_conflicts() {
        let catalog = catalog();
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let q2 = reference("q2:official", content(&catalog, "q2-classic-baseq2"));
        let timing = selected_weapon_timing(&map, &[q2], &catalog).unwrap();
        assert_eq!(timing.len(), 1);
        assert_eq!(provider_text(&timing[0].provider), "q2:weapons/classic/baseq2");

        let same = reference("q1:official", content(&catalog, "q1-classic-id1"));
        assert!(selected_weapon_timing(&map, &[same], &catalog).unwrap().is_empty());

        let alt = reference("q2:official", content(&catalog, "q2-classic-baseq2-alt"));
        let q2 = reference("q2:official", content(&catalog, "q2-classic-baseq2"));
        let error = selected_weapon_timing(&map, &[q2, alt], &catalog)
            .unwrap_err()
            .to_string();
        assert!(error.contains("conflicting content"), "{error}");
    }

    #[test]
    fn admit_timing_matches_or_rejects() {
        let catalog = catalog();
        let source = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let profile = native_provider_timing(&source, GameFamily::Q1, false);
        let mut timing = Vec::new();
        admit_weapon_timing(&mut timing, &profile).unwrap();
        assert_eq!(timing.len(), 1);
        admit_weapon_timing(&mut timing, &profile).unwrap();
        assert_eq!(timing.len(), 1);

        let mut clock = profile.clone();
        clock.clock = ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.002,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        };
        let error = admit_weapon_timing(&mut timing, &clock).unwrap_err().to_string();
        assert!(error.contains("conflicting timing"), "{error}");

        let mut numeric = profile.clone();
        numeric.numeric = qa_core::numeric::NumericProfile {
            id: "q2:binary32",
            arithmetic: Arithmetic::Binary32EachOp,
            float_to_int: FloatToInt::CheckedTruncation,
        };
        let error = admit_weapon_timing(&mut timing, &numeric).unwrap_err().to_string();
        assert!(error.contains("conflicting timing"), "{error}");
    }

    #[test]
    fn catalog_sources_implement_the_launch_seam() {
        let catalog = catalog();
        let sources = CatalogWeaponSources;
        assert_eq!(LaunchWeaponSources::weapon_provider_ids(&sources).len(), 6);
        let map = reference("q1:official", content(&catalog, "q1-classic-id1"));
        let weapon = reference("q1:official", content(&catalog, "q1-rerelease-hipnotic"));
        let canonical = LaunchWeaponSources::canonical_weapon_source(&sources, &map, &weapon, &catalog).unwrap();
        assert_eq!(provider_text(&canonical.provider), "q1:weapons/rerelease/hipnotic");
        let timing = LaunchWeaponSources::selected_weapon_timing(&sources, &map, &[weapon], &catalog).unwrap();
        assert_eq!(timing.len(), 1);
    }
}
