//! Quake III CD-key profiles and client authorization wiring.
//!
//! Donor provenance: `src/app/bootstrap/keys.ts` (`ApplicationKeyProfile`,
//! `ApplicationKeys`). Synchronous port: key bytes live behind the local
//! [`KeyProfileState`] trait (`Q3CdKeyState` is unported) and ConfigStores
//! are the existing settings stores. The donor's self-borrowing
//! `authorization` field becomes caller-wired [`KeyAuthorizationKeys`] and
//! [`KeyAuthorizationBindings`] adapters so the retained client keeps one
//! descriptor for all native connections.

use qa_content::catalog::InstalledCatalog;
use qa_content::contract::GameFamily;
use qa_content::user_data::{default_user_content_root, user_product_directory};
use qa_core::cvar::CvarRegistry;
use qa_net::q3_client_authorization::{Q3CdKeyAuthorization, Q3ClientAuthorizationBindings};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use thiserror::Error;

use crate::settings::config::ConfigStore;
use crate::settings::SettingsError;

/// Failure of key profile preparation or saving.
#[derive(Debug, Error)]
pub enum KeyError {
    /// Profile misuse.
    #[error("{0}")]
    Profile(String),
    /// Catalog lookup failure.
    #[error(transparent)]
    Catalog(#[from] qa_content::catalog::CatalogError),
    /// Settings store failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// User content path failure.
    #[error(transparent)]
    Path(#[from] qa_content::paths::PathError),
    /// Key state failure.
    #[error("{0}")]
    State(String),
}

/// VM-facing UI key writes (`readUi` writes sink).
pub trait UiKeyWrites {
    /// Copy key bytes.
    fn copy(&mut self, bytes: &[u8]);
    /// Set one byte.
    fn set_byte(&mut self, offset: usize, value: u8);
}

/// CD-key byte state (`Q3CdKeyState` surface used by the profiles).
pub trait KeyProfileState: Default {
    /// Read the base key file.
    fn read_file(&mut self, load_text: &dyn Fn(&str) -> Result<Option<String>, KeyError>) -> Result<(), KeyError>;
    /// Append the mod key file.
    fn append_file(&mut self, load_text: &dyn Fn(&str) -> Result<Option<String>, KeyError>) -> Result<(), KeyError>;
    /// Write the key file at a byte offset (0 for base, 16 for mod).
    fn write_file(&mut self, dump: &dyn Fn(&str, &str) -> Result<(), KeyError>, offset: u32) -> Result<(), KeyError>;
    /// Read the UI key into `destination`.
    fn read_ui(
        &self,
        unique: i32,
        game_directory: &str,
        destination: &mut [u8],
        writes: Option<&mut dyn UiKeyWrites>,
    ) -> Result<(), KeyError>;
    /// Write the UI key from `source`, marking archive flags on `cvars`.
    fn write_ui(
        &mut self,
        unique: i32,
        game_directory: &str,
        source: &[u8],
        cvars: &Rc<RefCell<CvarRegistry>>,
    ) -> Result<(), KeyError>;
    /// Fill the authorization key.
    fn read_authorization(&self, destination: &mut [u8]) -> Result<(), KeyError>;
}

/// A prepared profile owns its UI view before it becomes the authorization profile.
pub struct ApplicationKeyProfile<S> {
    /// Live cvar registry (re-pointed on publish).
    pub cvars: Rc<RefCell<CvarRegistry>>,
    /// Mod game directory for UI key selection.
    pub game_directory: String,
    /// Whether the demo build restricts keys.
    pub demo_restricted: bool,
    state: S,
    base: ConfigStore,
    game: Option<ConfigStore>,
}

impl<S: KeyProfileState> ApplicationKeyProfile<S> {
    /// Read the UI key view.
    pub fn read_ui(
        &self,
        unique: i32,
        game_directory: &str,
        destination: &mut [u8],
        writes: Option<&mut dyn UiKeyWrites>,
    ) -> Result<(), KeyError> {
        self.state.read_ui(unique, game_directory, destination, writes)
    }

    /// Write the UI key view, persisting to the base or mod store.
    pub fn write_ui(&mut self, unique: i32, directory: &str, source: &[u8]) -> Result<(), KeyError> {
        self.state.write_ui(unique, directory, source, &self.cvars)?;
        let use_mod = unique == 1 && !directory.is_empty();
        let store = if use_mod { self.game.as_ref() } else { Some(&self.base) };
        let Some(store) = store else {
            return Err(KeyError::Profile("Selected Q3 UI has no mod key write root".to_owned()));
        };
        let dump = |name: &str, contents: &str| store.dump(name, contents).map_err(KeyError::from);
        self.state.write_file(&dump, if use_mod { 16 } else { 0 })
    }

    /// Fill the authorization key.
    pub fn read_authorization(&self, destination: &mut [u8]) -> Result<(), KeyError> {
        self.state.read_authorization(destination)
    }

    /// Persist base and mod key files.
    pub fn save(&mut self) -> Result<(), KeyError> {
        let dump = |name: &str, contents: &str| self.base.dump(name, contents).map_err(KeyError::from);
        self.state.write_file(&dump, 0)?;
        if let Some(game) = self.game.as_ref() {
            let dump = |name: &str, contents: &str| game.dump(name, contents).map_err(KeyError::from);
            self.state.write_file(&dump, 16)?;
        }
        Ok(())
    }
}

/// Key profile options resolved from application options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyProfileOptions {
    /// Installed catalog product.
    pub product: String,
    /// Writable user content root override.
    pub user_content_root: Option<String>,
    /// Whether the Q3 product is demo-restricted.
    pub demo_restricted: bool,
}

/// The retained client publishes one descriptor for all of its native connections.
pub struct ApplicationKeys<S> {
    current: Option<ApplicationKeyProfile<S>>,
    print: Box<dyn FnMut(&str)>,
}

impl<S: KeyProfileState> ApplicationKeys<S> {
    /// Create key management with a print sink.
    pub fn new(print: Box<dyn FnMut(&str)>) -> Self {
        Self { current: None, print }
    }

    /// Published source profile.
    pub fn active(&self) -> Result<&ApplicationKeyProfile<S>, KeyError> {
        self.current
            .as_ref()
            .ok_or_else(|| KeyError::Profile("Q3 keys have no published source profile".to_owned()))
    }

    /// Prepare a profile for the selected product, or `None` outside Q3.
    pub fn prepare(
        &self,
        options: &KeyProfileOptions,
        catalog: &InstalledCatalog,
    ) -> Result<Option<ApplicationKeyProfile<S>>, KeyError> {
        let product = catalog.product(&options.product)?;
        if product.expectation.family != GameFamily::Q3 {
            return Ok(None);
        }
        let mut base = product;
        while let Some(parent) = base.expectation.base_product.as_ref() {
            base = catalog.product(parent)?;
        }
        let user_root = options
            .user_content_root
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(default_user_content_root);
        let stores =
            |selected: &qa_content::catalog::CatalogProduct| -> Result<(ConfigStore, Option<ConfigStore>), KeyError> {
                let root = match selected.user_content.as_ref() {
                    Some(user) => PathBuf::from(&user.root),
                    None => user_product_directory(&user_root, &selected.expectation.content_directory)?,
                };
                let write = ConfigStore::new(root);
                let fallback = match selected.loose_root.as_ref() {
                    Some(loose) if Path::new(loose) != write.root => Some(ConfigStore::new(PathBuf::from(loose))),
                    _ => None,
                };
                Ok((write, fallback))
            };
        let (base_write, base_fallback) = stores(base)?;
        let mod_stores = if product.id == base.id {
            None
        } else {
            Some(stores(product)?)
        };
        let mut state = S::default();
        let load_base = |name: &str| -> Result<Option<String>, KeyError> {
            Ok(base_write.load_text(name)?.or_else(|| {
                base_fallback
                    .as_ref()
                    .and_then(|fallback| fallback.load_text(name).unwrap_or(None))
            }))
        };
        state.read_file(&load_base)?;
        if let Some((mod_write, mod_fallback)) = mod_stores.as_ref() {
            let load_mod = |name: &str| -> Result<Option<String>, KeyError> {
                Ok(mod_write.load_text(name)?.or_else(|| {
                    mod_fallback
                        .as_ref()
                        .and_then(|fallback| fallback.load_text(name).unwrap_or(None))
                }))
            };
            state.append_file(&load_mod)?;
        }
        let game_directory = match mod_stores.as_ref() {
            None => String::new(),
            Some(_) => Path::new(&product.expectation.content_directory)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned(),
        };
        Ok(Some(ApplicationKeyProfile {
            cvars: Rc::new(RefCell::new(CvarRegistry::new(qa_core::cmd::Dialect::Q3))),
            game_directory,
            demo_restricted: options.demo_restricted,
            state,
            base: base_write,
            game: mod_stores.map(|(write, _)| write),
        }))
    }

    /// Publish a profile, re-pointing it at the live registry.
    pub fn publish(&mut self, mut profile: Option<ApplicationKeyProfile<S>>, cvars: Rc<RefCell<CvarRegistry>>) {
        if let Some(profile) = profile.as_mut() {
            profile.cvars = cvars;
        }
        self.current = profile;
    }

    /// Persist the published profile, if any.
    pub fn save(&mut self) -> Result<(), KeyError> {
        if let Some(current) = self.current.as_mut() {
            current.save()?;
        }
        Ok(())
    }

    /// Print through the key sink.
    pub fn print(&mut self, text: &str) {
        (self.print)(text);
    }
}

/// Authorization key reader over the published profile.
pub struct KeyAuthorizationKeys<'a, S> {
    keys: &'a ApplicationKeys<S>,
}

impl<'a, S: KeyProfileState> KeyAuthorizationKeys<'a, S> {
    /// Borrow key reading from published keys.
    pub fn new(keys: &'a ApplicationKeys<S>) -> Self {
        Self { keys }
    }
}

impl<S: KeyProfileState> Q3CdKeyAuthorization for KeyAuthorizationKeys<'_, S> {
    fn read_authorization(&mut self, out: &mut [u8; 33]) {
        let profile = self.keys.active().expect("Q3 keys have no published source profile");
        profile.read_authorization(out).expect("key authorization failed");
    }
}

/// Authorization bindings over the published profile.
pub struct KeyAuthorizationBindings<'a, S> {
    keys: &'a mut ApplicationKeys<S>,
}

impl<'a, S: KeyProfileState> KeyAuthorizationBindings<'a, S> {
    /// Borrow bindings from published keys.
    pub fn new(keys: &'a mut ApplicationKeys<S>) -> Self {
        Self { keys }
    }
}

impl<S: KeyProfileState> Q3ClientAuthorizationBindings for KeyAuthorizationBindings<'_, S> {
    fn demo_restricted(&mut self) -> bool {
        self.keys
            .active()
            .map(|profile| profile.demo_restricted)
            .unwrap_or(false)
    }

    fn print(&mut self, text: &str) {
        self.keys.print(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeState {
        files: Vec<String>,
        writes: Vec<u32>,
        ui: Vec<u8>,
    }

    impl KeyProfileState for FakeState {
        fn read_file(&mut self, load_text: &dyn Fn(&str) -> Result<Option<String>, KeyError>) -> Result<(), KeyError> {
            let _ = load_text("q3key")?;
            self.files.push("base".to_owned());
            Ok(())
        }

        fn append_file(
            &mut self,
            load_text: &dyn Fn(&str) -> Result<Option<String>, KeyError>,
        ) -> Result<(), KeyError> {
            let _ = load_text("q3key")?;
            self.files.push("mod".to_owned());
            Ok(())
        }

        fn write_file(
            &mut self,
            dump: &dyn Fn(&str, &str) -> Result<(), KeyError>,
            offset: u32,
        ) -> Result<(), KeyError> {
            dump("q3key", "key-bytes")?;
            self.writes.push(offset);
            Ok(())
        }

        fn read_ui(
            &self,
            _unique: i32,
            _game_directory: &str,
            destination: &mut [u8],
            _writes: Option<&mut dyn UiKeyWrites>,
        ) -> Result<(), KeyError> {
            let len = self.ui.len().min(destination.len());
            destination[..len].copy_from_slice(&self.ui[..len]);
            Ok(())
        }

        fn write_ui(
            &mut self,
            _unique: i32,
            _game_directory: &str,
            source: &[u8],
            cvars: &Rc<RefCell<CvarRegistry>>,
        ) -> Result<(), KeyError> {
            self.ui = source.to_vec();
            cvars.borrow_mut().mark_modified_flags(qa_core::cvar::flags::ARCHIVE);
            Ok(())
        }

        fn read_authorization(&self, destination: &mut [u8]) -> Result<(), KeyError> {
            if destination.len() < 33 {
                return Err(KeyError::State("CD key authorization requires 33 bytes".to_owned()));
            }
            destination[..16].copy_from_slice(&[7u8; 16]);
            destination[32] = 0;
            Ok(())
        }
    }

    fn catalog(family: GameFamily) -> InstalledCatalog {
        use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
        use qa_content::contract::ContentId;
        InstalledCatalog::new(
            "/corpus".to_owned(),
            vec![CatalogProduct {
                id: ContentId("product".to_owned()),
                expectation: ProductExpectation {
                    id: "product".to_owned(),
                    family,
                    edition: "classic".to_owned(),
                    campaign: "baseq3".to_owned(),
                    title: "Q3".to_owned(),
                    content_directory: "baseq3".to_owned(),
                    base_product: None,
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
            }],
            Vec::new(),
            1,
            None,
        )
        .expect("catalog")
    }

    #[test]
    fn prepare_returns_none_outside_q3() {
        let keys = ApplicationKeys::<FakeState>::new(Box::new(|_| {}));
        let options = KeyProfileOptions {
            product: "product".to_owned(),
            user_content_root: Some(
                std::env::temp_dir()
                    .join(format!("qa-keys-{}", std::process::id()))
                    .to_string_lossy()
                    .into_owned(),
            ),
            demo_restricted: false,
        };
        let q1 = catalog(GameFamily::Q1);
        assert!(keys.prepare(&options, &q1).expect("prepare").is_none());
        let q3 = catalog(GameFamily::Q3);
        let profile = keys.prepare(&options, &q3).expect("prepare").expect("profile");
        assert_eq!(profile.game_directory, "");
        assert!(!profile.demo_restricted);
    }

    #[test]
    fn publish_repoints_cvars_and_save_persists() {
        let mut keys = ApplicationKeys::<FakeState>::new(Box::new(|_| {}));
        assert!(keys.active().is_err());
        let root = std::env::temp_dir().join(format!("qa-keys-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let options = KeyProfileOptions {
            product: "product".to_owned(),
            user_content_root: Some(root.to_string_lossy().into_owned()),
            demo_restricted: true,
        };
        let q3 = catalog(GameFamily::Q3);
        let profile = keys.prepare(&options, &q3).expect("prepare").expect("profile");
        let cvars = Rc::new(RefCell::new(CvarRegistry::new(qa_core::cmd::Dialect::Q3)));
        keys.publish(Some(profile), Rc::clone(&cvars));
        assert!(keys.active().expect("active").demo_restricted);
        keys.save().expect("save");
        assert!(root.join("baseq3").join("q3key").exists());
        assert!(keys.active().is_ok());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn bindings_read_demo_and_print() {
        let mut keys = ApplicationKeys::<FakeState>::new(Box::new(|_| {}));
        let mut bindings = KeyAuthorizationBindings::new(&mut keys);
        assert!(!bindings.demo_restricted());
        bindings.print("hello\n");
    }
}
