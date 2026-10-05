//! Quake III client pure content (port of Quake-Anthology-TS `src/app/bootstrap/network/q3-client-content.ts`).
//!
//! The donor is `async`; this sync port resolves every host call inline.
//! The loaded content surface is the structural [`Q3ClientLoadedContent`]
//! trait plus a loader closure; [`Q3ClientContent::open_loaded`] binds the
//! canonical application loader
//! ([`load_application_content`](crate::bootstrap::content::load_application_content))
//! and its [`LoadedApplicationContent`](crate::bootstrap::content::LoadedApplicationContent)
//! there.

use qa_content::catalog::{
    remote_content_selection, CatalogError, InstalledCatalog, LaunchQvmCompatibility, RemoteContentBase,
};
use qa_content::contract::{GameFamily, ResolvedMountPlan};
use qa_content::mounts::{MountedContent, PureMountPolicy};
use qa_core::numeric::{native_atoi, NumericError};
use qa_net::q3_content::Q3ContentError;
use qa_net::q3_net::{q3_info_value, Q3NetError};
use qa_net::q3_pak_references::{PakReferenceFlag, Q3PakError, ServerPakSet};
use thiserror::Error;

use super::q3_downloads::{Q3ApplicationPackages, Q3CatalogMounts, Q3DownloadError};
use crate::bootstrap::content::{
    load_application_content, ApplicationContentOptions, ApplicationContentPreparer, ApplicationContentSource,
    ContentError, LoadedApplicationContent, Q3ProductPreparer, WorldByteArena,
};
use crate::options::ApplicationOptions;

/// Parsed pure server settings (donor `PureSystemInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PureSystemInfo {
    /// Pure server.
    pub pure: bool,
    /// Remote game directory.
    pub game: String,
    /// Server pak checksums.
    pub checksums: Vec<i32>,
    /// Checksum feed as an unsigned 32-bit value.
    pub checksum_feed: u32,
}

/// Quake III client content failure.
#[derive(Debug, Error)]
pub enum Q3ClientContentError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q3NetError),
    /// Pak reference failure.
    #[error(transparent)]
    Pak(#[from] Q3PakError),
    /// Integer-parse failure.
    #[error(transparent)]
    Numeric(#[from] NumericError),
    /// Content reference failure.
    #[error(transparent)]
    Content(#[from] Q3ContentError),
    /// Package failure.
    #[error(transparent)]
    Download(#[from] Q3DownloadError),
    /// Canonical content-loader failure.
    #[error(transparent)]
    Load(#[from] ContentError),
}

/// Loaded content surface used by [`Q3ClientContent`] (donor
/// `LoadedApplicationContent`, projected to the catalog, mounts, and close
/// this module touches).
pub trait Q3ClientLoadedContent {
    /// Catalog and mount plan projection for package walks.
    fn catalog_mounts(&self) -> Q3CatalogMounts<'_>;
    /// Mounted content for reference collection.
    fn mounted_content(&self) -> Vec<&MountedContent>;
    /// Retire the loaded content.
    fn close(self);
}

/// Canonical loaded content behind [`Q3ClientLoadedContent`.
///
/// The catalog and package plan come from the loaded content itself: the
/// mount plan is the main mounts' resolved plan, and reference collection
/// walks the main plus opened mounts (donor `openedMounts`).
impl<P: ApplicationContentPreparer> Q3ClientLoadedContent for LoadedApplicationContent<'_, P> {
    fn catalog_mounts(&self) -> Q3CatalogMounts<'_> {
        Q3CatalogMounts {
            catalog: &self.catalog,
            plan: &self.mounts.plan,
        }
    }

    fn mounted_content(&self) -> Vec<&MountedContent> {
        self.opened_mount_refs()
    }

    fn close(mut self) {
        LoadedApplicationContent::close(&mut self);
    }
}

/// Quake III client content (donor `Q3ClientContent`).
///
/// Each filesystem restart owns fresh mounts and references, including
/// same-map feed changes.
pub struct Q3ClientContent<C> {
    content: Option<C>,
    packages: Q3ApplicationPackages,
    policy: Option<PureMountPolicy>,
    settings: Q3PureSystemInfo,
    closed: bool,
}

impl<C: Q3ClientLoadedContent> Q3ClientContent<C> {
    /// Open client content for a pure server (`open`).
    ///
    /// The `load` closure stands in for
    /// [`load_application_content`](super::super::content::load_application_content):
    /// it receives the resolved pure policy and returns the loaded content.
    /// It captures the application options and presentation source the real
    /// loader needs.
    pub fn open(
        options: &ApplicationOptions,
        info: &str,
        checksum_feed: i64,
        catalog: &InstalledCatalog,
        plan: &ResolvedMountPlan,
        load: impl FnOnce(Option<&PureMountPolicy>) -> Result<C, Q3ClientContentError>,
    ) -> Result<Self, Q3ClientContentError> {
        let settings = q3_pure_system_info(info, checksum_feed)?;
        let product = catalog.require(&options.product)?;
        if product.expectation.family != GameFamily::Q3 {
            return Err(Q3ClientContentError::Message(
                "Q3 client content requires a Q3 product".to_string(),
            ));
        }
        let game = product
            .expectation
            .content_directory
            .split('/')
            .next_back()
            .unwrap_or("")
            .to_lowercase();
        if settings.game != game {
            return Err(Q3ClientContentError::Message(
                "Server game directory differs from the selected Q3 content".to_string(),
            ));
        }
        let seed_mounts = Q3CatalogMounts { catalog, plan };
        let seed = Q3ApplicationPackages::open(&seed_mounts, settings.checksum_feed as i32)?;
        let policy = if settings.checksums.is_empty() {
            None
        } else {
            Some(seed.references.pure_mount_policy(&settings.checksums)?)
        };
        let content = load(policy.as_ref())?;
        let packages = {
            let mounts = content.catalog_mounts();
            Q3ApplicationPackages::open(&mounts, settings.checksum_feed as i32)
        };
        match packages {
            Ok(packages) => Ok(Self {
                content: Some(content),
                packages,
                policy,
                settings,
                closed: false,
            }),
            Err(error) => {
                content.close();
                Err(error.into())
            }
        }
    }

    /// Whether the server is pure (`pure`).
    pub fn pure(&self) -> bool {
        self.settings.pure
    }

    /// Borrow the loaded content, unless closed.
    pub fn content(&self) -> Option<&C> {
        self.content.as_ref()
    }

    /// Borrow the application packages.
    pub fn packages(&self) -> &Q3ApplicationPackages {
        &self.packages
    }

    /// Borrow the pure mount policy, when the server sent checksums.
    pub fn policy(&self) -> Option<&PureMountPolicy> {
        self.policy.as_ref()
    }

    /// Whether these settings describe the same filesystem (`matches`).
    ///
    /// Unparsable candidates never match.
    pub fn matches(&self, info: &str, checksum_feed: i64) -> bool {
        if self.closed {
            return false;
        }
        let Ok(next) = q3_pure_system_info(info, checksum_feed) else {
            return false;
        };
        let previous = &self.settings;
        next.pure == previous.pure
            && next.game == previous.game
            && next.checksum_feed == previous.checksum_feed
            && next.checksums.len() == previous.checksums.len()
            && next
                .checksums
                .iter()
                .zip(previous.checksums.iter())
                .all(|(value, previous)| (*value as u32) == (*previous as u32))
    }

    /// Collect references and report the pure command (`referencedPureCommand`).
    ///
    /// Call after the actual cgame/UI media initialization, before entering
    /// the server.
    pub fn referenced_pure_command(&mut self, server_id: i32) -> Result<String, Q3ClientContentError> {
        if self.closed {
            return Err(Q3ClientContentError::Message("Q3 client content is closed".to_string()));
        }
        let content = self.content.as_ref().expect("content is present while open");
        let mounted = content.mounted_content();
        let catalog = content.catalog_mounts().catalog;
        self.packages.collect(&mounted, catalog)?;
        if self.pure() {
            let references = self.packages.references.references().snapshot();
            for (flag, name) in [(PakReferenceFlag::Cgame, "cgame"), (PakReferenceFlag::Ui, "UI")] {
                if !references.iter().any(|reference| reference.flags & (flag as u8) != 0) {
                    return Err(Q3ClientContentError::Message(format!(
                        "Pure Q3 admission requires an actually loaded {name} module"
                    )));
                }
            }
        }
        Ok(self.packages.references.referenced_pure_command(server_id))
    }

    /// Retire the content (`close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(content) = self.content.take() {
            content.close();
        }
    }
}

impl<'a, P: ApplicationContentPreparer> Q3ClientContent<LoadedApplicationContent<'a, P>>
where
    P::Error: Into<ContentError>,
{
    /// Open client content through the canonical application loader (`open`).
    ///
    /// The seed catalog and plan come from the currently loaded content;
    /// the seed catalog is reused for the load when a presentation source
    /// is given, matching the donor.
    #[allow(clippy::too_many_arguments)]
    pub fn open_loaded<Q: Q3ProductPreparer>(
        options: &ApplicationContentOptions,
        info: &str,
        checksum_feed: i64,
        catalog: &InstalledCatalog,
        plan: &ResolvedMountPlan,
        presentation_source: Option<ApplicationContentSource>,
        compat: &dyn LaunchQvmCompatibility,
        q3: &mut Q,
        preparer: &mut P,
        arena: &'a mut WorldByteArena,
    ) -> Result<Self, Q3ClientContentError>
    where
        Q::Error: Into<ContentError>,
    {
        Self::open(&options.base, info, checksum_feed, catalog, plan, |policy| {
            let installed = if presentation_source.is_some() {
                Some(catalog.clone())
            } else {
                None
            };
            load_application_content(
                options,
                None,
                policy.cloned(),
                installed,
                presentation_source,
                compat,
                q3,
                preparer,
                arena,
            )
            .map_err(Q3ClientContentError::from)
        })
    }
}

/// Parse pure server settings from system info (`systemInfo`).
pub fn q3_pure_system_info(info: &str, checksum_feed: i64) -> Result<Q3PureSystemInfo, Q3ClientContentError> {
    if !(i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(&checksum_feed) {
        return Err(Q3ClientContentError::Message(
            "Q3 checksum feed must be a 32-bit integer".to_string(),
        ));
    }
    let mut loaded = ServerPakSet::new();
    loaded.set_checksums(&q3_info_value(info, "sv_paks")?)?;
    let pure = native_atoi(&q3_info_value(info, "sv_pure")?) != 0;
    let game = remote_content_selection(RemoteContentBase::Q3Baseq3, &q3_info_value(info, "fs_game")?)?.directory;
    Ok(Q3PureSystemInfo {
        pure,
        game,
        checksums: loaded.checksums().to_vec(),
        checksum_feed: checksum_feed as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogArchive, CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::{create_content_id, ContentIdentity, MountPlanId};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};

    fn empty_plan() -> ResolvedMountPlan {
        ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:q3content".to_string()),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        }
    }

    fn product(id: &str, family: GameFamily, content_directory: &str) -> CatalogProduct {
        let content = create_content_id(&ContentIdentity {
            family,
            edition: "classic".to_string(),
            package: "baseq3".to_string(),
            revision: "installed".to_string(),
        })
        .expect("content id");
        CatalogProduct {
            id: content,
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: "classic".to_string(),
                campaign: "baseq3".to_string(),
                title: id.to_string(),
                content_directory: content_directory.to_string(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::<CatalogArchive>::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn options(product: &str) -> ApplicationOptions {
        ApplicationOptions {
            product: product.to_string(),
            ..ApplicationOptions::default()
        }
    }

    struct MockContent<'a> {
        catalog: &'a InstalledCatalog,
        plan: &'a ResolvedMountPlan,
        mounted: MountedContent,
        closed: std::cell::Cell<bool>,
    }

    impl<'a> MockContent<'a> {
        fn new(catalog: &'a InstalledCatalog, plan: &'a ResolvedMountPlan) -> Self {
            let mounted = open_mount_plan(
                plan,
                OpenMountOptions {
                    pure: None,
                    q3_restriction: None,
                    links: Vec::new(),
                    loose_comparison: None,
                },
            )
            .expect("mounts");
            Self {
                catalog,
                plan,
                mounted,
                closed: std::cell::Cell::new(false),
            }
        }
    }

    impl Q3ClientLoadedContent for MockContent<'_> {
        fn catalog_mounts(&self) -> Q3CatalogMounts<'_> {
            Q3CatalogMounts {
                catalog: self.catalog,
                plan: self.plan,
            }
        }

        fn mounted_content(&self) -> Vec<&MountedContent> {
            vec![&self.mounted]
        }

        fn close(self) {
            self.closed.set(true);
        }
    }

    fn catalog_with(products: Vec<CatalogProduct>) -> InstalledCatalog {
        InstalledCatalog::new(String::new(), products, Vec::new(), 1, None).expect("catalog")
    }

    #[test]
    fn parses_pure_system_info() {
        let info = "\\sv_pure\\1\\fs_game\\\\sv_paks\\42 -7";
        let settings = q3_pure_system_info(info, 9).expect("parse");
        assert!(settings.pure);
        assert_eq!(settings.game, "baseq3");
        assert_eq!(settings.checksums, vec![42, -7]);
        assert_eq!(settings.checksum_feed, 9);

        let settings = q3_pure_system_info(info, -1).expect("negative feed");
        assert_eq!(settings.checksum_feed, 0xffff_ffff);

        for feed in [i64::from(i32::MIN) - 1, i64::from(u32::MAX) + 1] {
            let error = q3_pure_system_info(info, feed).expect_err("range");
            assert_eq!(error.to_string(), "Q3 checksum feed must be a 32-bit integer");
        }
    }

    #[test]
    fn open_gates_product_family_and_game() {
        let plan = empty_plan();
        let catalog = catalog_with(vec![product("q3-baseq3", GameFamily::Q3, "q3/baseq3")]);
        let info = "\\sv_pure\\0\\fs_game\\missionpack";
        let Err(error) = Q3ClientContent::<MockContent<'_>>::open(
            &options("q3-baseq3"),
            info,
            1,
            &catalog,
            &plan,
            |_: Option<&PureMountPolicy>| {
                unreachable!("game gate first");
            },
        ) else {
            panic!("game gate");
        };
        assert_eq!(
            error.to_string(),
            "Server game directory differs from the selected Q3 content"
        );

        let catalog = catalog_with(vec![product("q1-id1", GameFamily::Q1, "q1/id1")]);
        let Err(error) = Q3ClientContent::<MockContent<'_>>::open(
            &options("q1-id1"),
            "\\sv_pure\\0\\fs_game\\",
            1,
            &catalog,
            &plan,
            |_: Option<&PureMountPolicy>| {
                unreachable!("family gate first");
            },
        ) else {
            panic!("family gate");
        };
        assert_eq!(error.to_string(), "Q3 client content requires a Q3 product");
    }

    #[test]
    fn open_matches_and_closes() {
        let plan = empty_plan();
        let catalog = catalog_with(vec![product("q3-baseq3", GameFamily::Q3, "q3/baseq3")]);
        let info = "\\sv_pure\\0\\fs_game\\";
        let mut content = Q3ClientContent::open(&options("q3-baseq3"), info, 7, &catalog, &plan, |policy| {
            assert!(policy.is_none());
            Ok(MockContent::new(&catalog, &plan))
        })
        .expect("open");
        assert!(!content.pure());
        assert!(content.content().is_some());
        assert!(content.matches(info, 7));
        assert!(!content.matches("\\sv_pure\\1\\fs_game\\", 7));
        assert!(!content.matches(info, 8));
        // Unparsable candidates never match.
        assert!(!content.matches(info, i64::from(u32::MAX) + 1));
        // Impure servers skip the cgame/UI gate.
        let command = content.referenced_pure_command(3).expect("command");
        assert!(command.starts_with("cp "));
        content.close();
        assert!(content.content().is_none());
        assert!(!content.matches(info, 7));
        let error = content.referenced_pure_command(3).expect_err("closed");
        assert_eq!(error.to_string(), "Q3 client content is closed");
        content.close();
    }

    #[test]
    fn pure_servers_require_loaded_modules() {
        let plan = empty_plan();
        let catalog = catalog_with(vec![product("q3-baseq3", GameFamily::Q3, "q3/baseq3")]);
        let info = "\\sv_pure\\1\\fs_game\\";
        let mut content = Q3ClientContent::open(&options("q3-baseq3"), info, 7, &catalog, &plan, |policy| {
            assert!(policy.is_none());
            Ok(MockContent::new(&catalog, &plan))
        })
        .expect("open");
        assert!(content.pure());
        let error = content.referenced_pure_command(3).expect_err("cgame");
        assert_eq!(
            error.to_string(),
            "Pure Q3 admission requires an actually loaded cgame module"
        );
    }

    #[test]
    fn loader_failure_rejects_before_packages() {
        let plan = empty_plan();
        let catalog = catalog_with(vec![product("q3-baseq3", GameFamily::Q3, "q3/baseq3")]);
        let info = "\\sv_pure\\0\\fs_game\\";
        let Err(error) = Q3ClientContent::<MockContent>::open(&options("q3-baseq3"), info, 7, &catalog, &plan, |_| {
            Err(Q3ClientContentError::Message("loader down".to_string()))
        }) else {
            panic!("loader gate");
        };
        assert_eq!(error.to_string(), "loader down");
        // Unknown products reject through the catalog.
        let Err(error) = Q3ClientContent::<MockContent>::open(
            &options("missing"),
            info,
            7,
            &catalog,
            &plan,
            |_: Option<&PureMountPolicy>| {
                unreachable!("catalog gate first");
            },
        ) else {
            panic!("catalog gate");
        };
        assert!(matches!(error, Q3ClientContentError::Catalog(_)));
    }
}
