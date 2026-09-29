//! Navigation loading from mounted geometry content, ported from
//! `src/bots/navigation/load.ts`. Supplied navigation wins; absent assets
//! construct through the selected shared collision world. The donor is
//! async over its mount store; this port is synchronous over the
//! [`NavigationResources`] seam.

use crate::aas::parse_aas;
use crate::construct::{construct_navigation, NavigationConstruction};
use crate::content::{ContentId, NavigationResources, ResourceReference};
use crate::error::BotsError;
use crate::graph::navigation_from_asset;
use crate::md4::block_checksum;
use crate::nav::parse_kex_navigation;
use crate::runtime::NavigationRuntime;
use crate::scene::WorldKind;
use crate::types::{NavigationAsset, NavigationMapIdentity};

/// Navigation load options: construction plus resource access.
pub struct NavigationLoadOptions<'a> {
    /// Construction inputs.
    pub construction: NavigationConstruction<'a>,
    /// Geometry content's mounted resources, independently of
    /// presentation/arsenal content.
    pub resources: &'a dyn NavigationResources,
    /// Selected map bytes for AAS checksum verification.
    pub map_bytes: &'a [u8],
    /// Checksumless NAV must belong to the selected geometry content,
    /// not an inherited namesake.
    pub navigation_content: Option<&'a ContentId>,
}

/// Preload inputs.
pub struct PreloadOptions<'a> {
    /// Map identity.
    pub map: &'a NavigationMapIdentity,
    /// Resource store.
    pub resources: &'a dyn NavigationResources,
    /// Map bytes for AAS checksum verification.
    pub map_bytes: &'a [u8],
    /// Required NAV content owner.
    pub navigation_content: Option<&'a ContentId>,
}

/// Prepared navigation: the selected asset with its resource reference.
pub struct PreparedNavigation {
    /// Map identity.
    pub map: NavigationMapIdentity,
    /// Selected asset, when the store held one.
    pub asset: Option<NavigationAsset>,
    /// Resource reference, when the store held an asset.
    pub resource: Option<ResourceReference>,
}

/// Loaded navigation: the runtime with its resource reference.
pub struct LoadedNavigation<'w> {
    /// Navigation runtime.
    pub runtime: NavigationRuntime<'w>,
    /// Resource reference, when the store held an asset.
    pub resource: Option<ResourceReference>,
}

/// Select the navigation asset for a map from mounted resources.
pub fn preload_navigation(options: &PreloadOptions<'_>) -> Result<PreparedNavigation, BotsError> {
    let map = options.map;
    let mut relative = map.name.as_str();
    if relative.len() >= 5 && relative[..5].eq_ignore_ascii_case("maps/") {
        relative = &relative[5..];
    }
    if relative.len() >= 4 && relative[relative.len() - 4..].eq_ignore_ascii_case(".bsp") {
        relative = &relative[..relative.len() - 4];
    }
    if relative.is_empty()
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(BotsError::BadResourcePath);
    }
    let paths = match map.format {
        WorldKind::Q3Bsp => {
            vec![
                format!("maps/{relative}.aas"),
                format!("bots/navigation/{relative}.nav"),
            ]
        }
        _ => {
            vec![
                format!("bots/navigation/{relative}.nav"),
                format!("maps/{relative}.aas"),
            ]
        }
    };
    for path in &paths {
        let Some(opened) = options.resources.open(path) else {
            continue;
        };
        if path.ends_with(".nav") {
            if let Some(expected) = options.navigation_content {
                if opened.reference.provenance.mount_content != *expected {
                    continue;
                }
            }
        }
        let asset = if path.ends_with(".aas") {
            NavigationAsset::Aas(Box::new(parse_aas(
                &opened.bytes,
                path,
                Some(block_checksum(options.map_bytes)? as i32),
            )?))
        } else {
            NavigationAsset::Kex(parse_kex_navigation(&opened.bytes, path)?)
        };
        return Ok(PreparedNavigation {
            map: map.clone(),
            asset: Some(asset),
            resource: Some(opened.reference),
        });
    }
    Ok(PreparedNavigation {
        map: map.clone(),
        asset: None,
        resource: None,
    })
}

/// Build a runtime from prepared navigation.
pub fn load_prepared_navigation<'a>(
    options: &NavigationConstruction<'a>,
    prepared: PreparedNavigation,
) -> Result<LoadedNavigation<'a>, BotsError> {
    let (map, geometry, profile, world) = (options.map, options.geometry, options.profile, options.world);
    if map.format != geometry.kind()
        || map.digest != prepared.map.digest
        || map.name != prepared.map.name
        || map.format != prepared.map.format
    {
        return Err(BotsError::MapMismatch);
    }
    let graph = match prepared.asset {
        None => construct_navigation(options)?,
        Some(asset) => navigation_from_asset(map.clone(), asset, profile.clone(), world)?,
    };
    Ok(LoadedNavigation {
        runtime: NavigationRuntime::new(graph, world)?,
        resource: prepared.resource,
    })
}

/// Load navigation for a map, constructing when the store holds no asset.
pub fn load_navigation<'a>(options: &NavigationLoadOptions<'a>) -> Result<LoadedNavigation<'a>, BotsError> {
    if options.construction.map.format != options.construction.geometry.kind() {
        return Err(BotsError::MapMismatch);
    }
    load_prepared_navigation(
        &options.construction,
        preload_navigation(&PreloadOptions {
            map: options.construction.map,
            resources: options.resources,
            map_bytes: options.map_bytes,
            navigation_content: options.navigation_content,
        })?,
    )
}
