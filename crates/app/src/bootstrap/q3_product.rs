//! Quake III application product selection with demo restriction.
//!
//! Port of `src/app/bootstrap/q3-product.ts`
//! (`prepareQ3ApplicationProduct`). The catalog, mount plans, restriction resolver,
//! startup phases, and cvar registry are the ported catalog, mounts, restriction,
//! startup-command, and cvar helpers; the product policy registrar
//! (`registerQ3ProductPolicy` from `src/core/q3-product-policy.ts`, out of scope)
//! arrives through the [`Q3ProductPolicyRegistrar`] seam, and the donor's async mount
//! reads are sync through the host. The product read is skipped when a demo restriction
//! is already forced, exactly like the donor's resolver early return. Two documented
//! folds: options arrive as [`Q3ProductOptions`], a `startupCommands`/`q3Product`
//! projection of ported [`ApplicationOptions::q3_product`](crate::options::ApplicationOptions::q3_product),
//! and the donor's identity owner only feeds the cvar context
//! which the ported registry does not take.

use qa_content::catalog::{CatalogError, InstalledCatalog};
use qa_content::contract::{create_mount_plan_id, ContentId, ContractError, GameFamily, ResolvedMountPlan};
use qa_content::hash::hex_lower;
use qa_content::mounts::{open_mount_plan, MountError, OpenMountOptions, Q3Restriction};
use qa_content::q3::base::records::Q3BaseError;
use qa_content::q3::product_restriction::{
    q3_mount_restriction, resolve_q3_mount_restriction, Q3MountRestriction, Q3ProductPolicy,
};
use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, CvarError, CvarRegistry};
use thiserror::Error;

use super::startup_commands::{startup_command_phases, StartupCommandsError};

/// Resolved application product (donor `Q3ApplicationProduct`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ApplicationProduct {
    /// Product policy.
    pub policy: Q3ProductPolicy,
    /// Mount restriction.
    pub restriction: Q3MountRestriction,
}

/// Product policy registrar (donor `registerQ3ProductPolicy`, out of scope).
pub trait Q3ProductPolicyRegistrar {
    /// Register the policy from applied startup cvars.
    fn register_policy(&self, cvars: &CvarRegistry) -> Q3ProductPolicy;
}

/// Product selection options (donor `Pick<ApplicationOptions, "startupCommands" | "q3Product">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Q3ProductOptions {
    /// Startup command lines.
    pub startup_commands: Vec<String>,
    /// Preselected product, skipping policy resolution.
    pub q3_product: Option<Q3ApplicationProduct>,
}

/// Selected catalog and product (donor return).
pub struct Q3ProductSelection {
    /// Selected catalog.
    pub catalog: InstalledCatalog,
    /// Selected product, when the content family is Quake III.
    pub q3_product: Option<Q3ApplicationProduct>,
}

/// Failure to prepare the application product.
#[derive(Debug, Error)]
pub enum Q3ProductError {
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Mount plan identity failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Startup command failure.
    #[error(transparent)]
    Startup(#[from] StartupCommandsError),
    /// Restriction failure.
    #[error(transparent)]
    Base(#[from] Q3BaseError),
}

/// Read `productid.txt` for content (donor resolver callback).
fn read_product_id(catalog: &InstalledCatalog, selected: &str) -> Result<Option<Vec<u8>>, Q3ProductError> {
    let mounts = catalog.mounts_for(selected)?;
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("q3-product-identification", &hex_lower(selected.as_bytes()))?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        mounts,
        prefix_orders: Vec::new(),
    };
    let mounted = open_mount_plan(&plan, OpenMountOptions::default())?;
    let bytes = mounted.open("productid.txt", |_| true)?.map(|opened| opened.bytes);
    mounted.close();
    Ok(bytes)
}

/// Prepare the application product (donor `prepareQ3ApplicationProduct`).
pub fn prepare_q3_application_product(
    catalog: InstalledCatalog,
    content: &ContentId,
    options: &Q3ProductOptions,
    registrar: &impl Q3ProductPolicyRegistrar,
) -> Result<Q3ProductSelection, Q3ProductError> {
    let selected = content.as_str().to_string();
    if catalog.product(&selected)?.expectation.family != GameFamily::Q3 {
        return Ok(Q3ProductSelection {
            catalog,
            q3_product: None,
        });
    }
    let product = match &options.q3_product {
        Some(product) => *product,
        None => {
            let mut cvars = CvarRegistry::new(Dialect::Q3);
            for variable in startup_command_phases(&options.startup_commands, Dialect::Q3)?.variables {
                if ["com_prereleasedemo", "com_prereleaseteamarenademo", "fs_restrict"]
                    .contains(&variable.name.to_lowercase().as_str())
                {
                    cvars.set(&variable.name, &variable.value, true)?;
                }
            }
            let policy = registrar.register_policy(&cvars);
            let forced = cvars
                .register("fs_restrict", "0", flags::INIT)?
                .map_or(0, |snapshot| snapshot.integer_value)
                != 0;
            let product_id = if matches!(q3_mount_restriction(policy, forced), Q3MountRestriction::Demo { .. }) {
                None
            } else {
                read_product_id(&catalog, &selected)?
            };
            Q3ApplicationProduct {
                policy,
                restriction: resolve_q3_mount_restriction(policy, forced, product_id.as_deref())?,
            }
        }
    };
    if product.restriction == Q3MountRestriction::None {
        return Ok(Q3ProductSelection {
            catalog,
            q3_product: Some(product),
        });
    }
    let restricted_products = {
        let media = catalog.require("q3-demota")?;
        catalog
            .products
            .iter()
            .map(|source| {
                if source.expectation.family != GameFamily::Q3 {
                    return source.clone();
                }
                let mut restricted = source.clone();
                restricted.expectation.base_product = None;
                restricted.availability = media.availability.clone();
                restricted.archives = media.archives.clone();
                restricted.loose_root = media.loose_root.clone();
                restricted.user_content = media.user_content.clone();
                restricted.maps = media.maps.clone();
                restricted
            })
            .collect::<Vec<_>>()
    };
    let restricted = InstalledCatalog::new(
        catalog.corpus_root.clone(),
        restricted_products,
        catalog.root_archives.clone(),
        catalog.generation,
        catalog.user_content_root.clone(),
    )?;
    let mounts = restricted.mounts_for(&selected)?;
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("q3-restricted-product", &hex_lower(selected.as_bytes()))?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        mounts,
        prefix_orders: Vec::new(),
    };
    let verified = open_mount_plan(
        &plan,
        OpenMountOptions {
            q3_restriction: Some(Q3Restriction::Demo),
            ..OpenMountOptions::default()
        },
    )?;
    verified.close();
    Ok(Q3ProductSelection {
        catalog: restricted,
        q3_product: Some(product),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};

    struct Probe;

    impl Q3ProductPolicyRegistrar for Probe {
        fn register_policy(&self, cvars: &CvarRegistry) -> Q3ProductPolicy {
            if cvars
                .get("com_prereleasedemo")
                .map_or(0, |snapshot| snapshot.integer_value)
                != 0
            {
                Q3ProductPolicy::PrereleaseDemo {
                    team_arena_ui: qa_content::q3::product_restriction::TeamArenaUi::Retail,
                }
            } else {
                Q3ProductPolicy::Retail
            }
        }
    }

    struct Retail;

    impl Q3ProductPolicyRegistrar for Retail {
        fn register_policy(&self, _cvars: &CvarRegistry) -> Q3ProductPolicy {
            Q3ProductPolicy::Retail
        }
    }

    fn product(id: &str, family: GameFamily) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: "classic".to_string(),
                campaign: "baseq3".to_string(),
                title: id.to_string(),
                content_directory: id.to_string(),
                base_product: Some("base".to_string()),
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn catalog(products: Vec<CatalogProduct>) -> InstalledCatalog {
        InstalledCatalog::new("/corpus".to_string(), products, Vec::new(), 1, None).unwrap()
    }

    #[test]
    fn foreign_family_selects_no_product() {
        let selection = prepare_q3_application_product(
            catalog(vec![product("q1:classic:id1:1", GameFamily::Q1)]),
            &ContentId("q1:classic:id1:1".to_string()),
            &Q3ProductOptions::default(),
            &Retail,
        )
        .unwrap();
        assert!(selection.q3_product.is_none());
        assert_eq!(selection.catalog.products.len(), 1);
    }

    #[test]
    fn unrestricted_product_passes_through() {
        let expected = Q3ApplicationProduct {
            policy: Q3ProductPolicy::Retail,
            restriction: Q3MountRestriction::None,
        };
        let selection = prepare_q3_application_product(
            catalog(vec![product("q3:classic:baseq3:1", GameFamily::Q3)]),
            &ContentId("q3:classic:baseq3:1".to_string()),
            &Q3ProductOptions {
                startup_commands: Vec::new(),
                q3_product: Some(expected),
            },
            &Retail,
        )
        .unwrap();
        assert_eq!(selection.q3_product, Some(expected));
        assert_eq!(selection.catalog.products.len(), 1);
    }

    #[test]
    fn startup_cvars_feed_policy_registration() {
        let selection = prepare_q3_application_product(
            catalog(vec![
                product("q3:classic:baseq3:1", GameFamily::Q3),
                product("q3-demota", GameFamily::Q3),
            ]),
            &ContentId("q3:classic:baseq3:1".to_string()),
            &Q3ProductOptions {
                startup_commands: vec!["set com_prereleasedemo 1".to_string()],
                q3_product: None,
            },
            &Probe,
        )
        .unwrap();
        let product = selection.q3_product.unwrap();
        assert!(matches!(product.policy, Q3ProductPolicy::PrereleaseDemo { .. }));
        assert!(matches!(product.restriction, Q3MountRestriction::Demo { .. }));
    }

    #[test]
    fn demo_restriction_rebuilds_catalog_from_demota() {
        let restricted = Q3ApplicationProduct {
            policy: Q3ProductPolicy::Retail,
            restriction: Q3MountRestriction::Demo {
                directory: "demota",
                pak_checksum: 437558517,
            },
        };
        let selection = prepare_q3_application_product(
            catalog(vec![
                product("q3:classic:baseq3:1", GameFamily::Q3),
                product("q3-demota", GameFamily::Q3),
            ]),
            &ContentId("q3:classic:baseq3:1".to_string()),
            &Q3ProductOptions {
                startup_commands: Vec::new(),
                q3_product: Some(restricted),
            },
            &Retail,
        )
        .unwrap();
        assert_eq!(selection.q3_product, Some(restricted));
        assert!(selection
            .catalog
            .products
            .iter()
            .all(|product| product.expectation.base_product.is_none()));
    }
}
