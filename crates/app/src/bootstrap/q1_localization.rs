//! Source message localization shared by the console and HUD.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/q1-localization.ts`
//! (`Q1MessageLocalization`). Catalog tables, tier loading, and classic formatting come from
//! the ported [`LocalizationCatalog`](qa_client::text::localization::LocalizationCatalog)
//! and [`classic_q1_text`](qa_content::q1::foundation::text::classic_q1_text); the content
//! catalog and mounts arrive through the [`Q1MessageAssets`] seam, canonically backed by
//! [`ApplicationAssets`](super::assets::ApplicationAssets), and the donor's async loads
//! are sync through the host. One documented gap: the
//! donor's per-slot splice for a known entry with unknown `$args` needs entry argument
//! slots that the merged table keeps private, so that case falls back to plain `localize`
//! (unknown `$args` render without their `$`, exactly as `localizeSource` does).

use std::collections::HashMap;
use std::rc::Rc;

use qa_client::text::localization::{LocLoadTier, LocReloadOptions, LocalizationCatalog, LocalizationProfile};
use qa_content::contract::{ContentId, GameFamily};
use qa_content::q1::foundation::text::{classic_q1_text, Q1TextArg};
use qa_core::identity::SeatId;

use super::assets::{ApplicationAssets, AssetScene};
use super::content::ApplicationContentPreparer;
use super::menu_font::{MenuCharsetImages, MountedMenuFonts, TypographyMounts};

/// A message format argument (donor `string | number`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1MessageArg {
    /// String argument.
    Text(String),
    /// Numeric argument.
    Number(f64),
}

/// One message part (donor `Q1MessagePart`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MessagePart {
    /// Part text.
    pub text: String,
    /// Part arguments.
    pub args: Vec<Q1MessageArg>,
}

/// Product identity the resolver reads (donor `catalog.product(content).expectation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1MessageProduct {
    /// Whether the product family is `q1`.
    pub q1_family: bool,
    /// Whether the product edition is `rerelease`.
    pub rerelease: bool,
}

/// Content catalog and mount reads (donor `ApplicationAssets["content"]` subset).
pub trait Q1MessageAssets {
    /// Product identity for content.
    fn product(&self, content: &ContentId) -> Q1MessageProduct;
    /// Open a mounted file, or [`None`] when absent.
    fn open(&mut self, content: &ContentId, path: &str) -> Option<Vec<u8>>;
}

/// Canonical [`ApplicationAssets`] backing (donor `this.assets.content`
/// reads): the catalog product expectation plus per-content mounts. Unknown
/// content reports a non-q1 product instead of throwing, so foreign content
/// resolves text untouched.
impl<
        'a,
        P: ApplicationContentPreparer,
        S: AssetScene,
        C: MenuCharsetImages,
        F: MountedMenuFonts,
        M: TypographyMounts,
    > Q1MessageAssets for ApplicationAssets<'a, P, S, C, F, M>
{
    fn product(&self, content: &ContentId) -> Q1MessageProduct {
        match self.content.catalog.product(content.as_str()) {
            Ok(product) => Q1MessageProduct {
                q1_family: product.expectation.family == GameFamily::Q1,
                rerelease: product.expectation.edition == "rerelease",
            },
            Err(_) => Q1MessageProduct {
                q1_family: false,
                rerelease: false,
            },
        }
    }

    fn open(&mut self, content: &ContentId, path: &str) -> Option<Vec<u8>> {
        let mounts = self.content.for_content(content).ok()?;
        mounts
            .open(path, |_| true)
            .ok()
            .flatten()
            .map(|resource| resource.bytes)
    }
}

/// Format a numeric argument the way JavaScript `String(number)` does.
fn js_number_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Stringify a message argument (donor `values.map(String)`).
fn message_arg_string(arg: &Q1MessageArg) -> String {
    match arg {
        Q1MessageArg::Text(text) => text.clone(),
        Q1MessageArg::Number(value) => js_number_string(*value),
    }
}

/// Resolve source message arguments (donor `Q1MessageLocalization`).
pub struct Q1MessageLocalization<A> {
    seat: SeatId,
    assets: A,
    language: Rc<dyn Fn() -> String>,
    catalogs: HashMap<(ContentId, String), LocalizationCatalog>,
}

impl<A: Q1MessageAssets> Q1MessageLocalization<A> {
    /// Build the resolver with the default `english` language.
    pub fn new(seat: SeatId, assets: A) -> Self {
        Self::with_language(seat, assets, Rc::new(|| "english".to_string()))
    }

    /// Build the resolver with an explicit language source.
    pub fn with_language(seat: SeatId, assets: A, language: Rc<dyn Fn() -> String>) -> Self {
        Self {
            seat,
            assets,
            language,
            catalogs: HashMap::new(),
        }
    }

    /// Load (or reuse) the catalog for content and language.
    fn catalog(&mut self, content: &ContentId, language: &str) -> &LocalizationCatalog {
        let key = (content.clone(), language.to_string());
        if !self.catalogs.contains_key(&key) {
            let tier = |assets: &mut A, name: &str| LocLoadTier {
                base: assets.open(content, &format!("localization/loc_{name}.txt")),
                mods: assets
                    .open(content, &format!("localization/loc_{name}_mod.txt"))
                    .into_iter()
                    .collect(),
            };
            let primary = tier(&mut self.assets, language);
            let fallback = if language == "english" {
                LocLoadTier::default()
            } else {
                tier(&mut self.assets, "english")
            };
            let mut table = LocalizationCatalog::new(self.seat.clone(), LocalizationProfile::Q1Rerelease);
            table
                .table
                .load_ordered(&primary, &fallback, &LocReloadOptions::default(), None);
            if primary.base.is_none() && fallback.base.is_none() {
                for mods in fallback.mods.iter().chain(primary.mods.iter()) {
                    table.table.merge(Some(mods), &LocReloadOptions::default(), None);
                }
            }
            self.catalogs.insert(key.clone(), table);
        }
        &self.catalogs[&key]
    }

    /// Localize one value with its arguments (donor `localize`).
    fn localize_value(table: &LocalizationCatalog, rerelease: bool, value: &str, values: &[Q1MessageArg]) -> String {
        let unlocalized = !value.starts_with('$') || table.table.lookup(value, &[]).is_none();
        if !rerelease && unlocalized {
            let args: Vec<Q1TextArg> = values
                .iter()
                .map(|arg| match arg {
                    Q1MessageArg::Text(text) => Q1TextArg::Text(text.clone()),
                    Q1MessageArg::Number(value) => Q1TextArg::Number(*value),
                })
                .collect();
            return classic_q1_text(value, &args);
        }
        if value.starts_with('$') && table.table.lookup(value, &[]).is_none() {
            return value.to_string();
        }
        let strings: Vec<String> = values.iter().map(message_arg_string).collect();
        table.localize(value, &strings)
    }

    /// Resolve a message (donor `resolve`).
    pub fn resolve(
        &mut self,
        content: &ContentId,
        text: &str,
        args: &[Q1MessageArg],
        parts: &[Q1MessagePart],
    ) -> String {
        let product = self.assets.product(content);
        if !product.q1_family {
            return text.to_string();
        }
        if !product.rerelease && !text.starts_with('$') && args.is_empty() && parts.is_empty() {
            return text.to_string();
        }
        let language = (self.language)();
        let table = self.catalog(content, &language);
        if parts.is_empty() {
            Self::localize_value(table, product.rerelease, text, args)
        } else {
            parts
                .iter()
                .map(|part| Self::localize_value(table, product.rerelease, &part.text, &part.args))
                .collect::<Vec<_>>()
                .join("")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::collections::HashMap;

    struct Stub {
        product: Q1MessageProduct,
        files: HashMap<String, Vec<u8>>,
    }

    impl Q1MessageAssets for Stub {
        fn product(&self, _content: &ContentId) -> Q1MessageProduct {
            self.product
        }
        fn open(&mut self, _content: &ContentId, path: &str) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
    }

    fn content() -> ContentId {
        ContentId("q1:rerelease:baseq3:1".to_string())
    }

    fn resolver(files: HashMap<String, Vec<u8>>, rerelease: bool) -> Q1MessageLocalization<Stub> {
        let owner = IdentityOwner::create("q1-loc").unwrap();
        Q1MessageLocalization::new(
            owner.seat(0),
            Stub {
                product: Q1MessageProduct {
                    q1_family: true,
                    rerelease,
                },
                files,
            },
        )
    }

    #[test]
    fn non_q1_returns_text_untouched() {
        let owner = IdentityOwner::create("q1-loc-foreign").unwrap();
        let mut resolver = Q1MessageLocalization::new(
            owner.seat(0),
            Stub {
                product: Q1MessageProduct {
                    q1_family: false,
                    rerelease: true,
                },
                files: HashMap::new(),
            },
        );
        assert_eq!(resolver.resolve(&content(), "$NOPE", &[], &[]), "$NOPE");
    }

    #[test]
    fn classic_plain_text_skips_catalogs() {
        let mut resolver = resolver(HashMap::new(), false);
        assert_eq!(resolver.resolve(&content(), "Hello", &[], &[]), "Hello");
    }

    #[test]
    fn rerelease_key_localizes_with_args() {
        let mut files = HashMap::new();
        files.insert(
            "localization/loc_english.txt".to_string(),
            b"GREETING = \"Hello {0}!\"\n".to_vec(),
        );
        let mut resolver = resolver(files, true);
        let text = resolver.resolve(&content(), "$GREETING", &[Q1MessageArg::Text("World".to_string())], &[]);
        assert_eq!(text, "Hello World!");
    }

    #[test]
    fn unknown_key_returns_value() {
        let mut resolver = resolver(HashMap::new(), true);
        assert_eq!(resolver.resolve(&content(), "$NOPE", &[], &[]), "$NOPE");
    }

    #[test]
    fn parts_join() {
        let mut files = HashMap::new();
        files.insert("localization/loc_english.txt".to_string(), b"A = \"x\"\n".to_vec());
        let mut resolver = resolver(files, true);
        let text = resolver.resolve(
            &content(),
            "",
            &[],
            &[
                Q1MessagePart {
                    text: "$A".to_string(),
                    args: vec![],
                },
                Q1MessagePart {
                    text: "$A".to_string(),
                    args: vec![],
                },
            ],
        );
        assert_eq!(text, "xx");
    }
}
