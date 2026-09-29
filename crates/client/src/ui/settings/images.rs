//! Image and model replacement settings bindings.
//!
//! Ported from the TypeScript donor's `src/ui/settings/images.ts`. The
//! application owns registration and refresh; these menus edit its existing
//! controls, so binding fails when a cvar has no owner.

use std::rc::Rc;

use crate::error::ClientError;
use crate::ui::types::{UiChoice, UiControlId};

use super::{
    bind_cvar_setting, CvarSettingKind, CvarSettingSpec, SettingBinding, SettingBindingKind, SettingCategory,
    SettingCvars,
};

/// Parse a mask string the way the donor `Number()` read does: blank reads as
/// zero, unparseable reads as NaN (which truncates to zero).
fn mask_number(text: &str) -> i32 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN) as i32
}

/// Bind the replacement-image controls (donor `bindImageSettings`).
///
/// Returns the override choice, the formats entry, and one toggle per
/// `ImagetypeT` usage bit (skin 1, sprite 2, wall 4, picture 8, sky 16).
/// The `r_texture_overrides` mask entry itself is not returned; the usage
/// toggles share its read/write/enabled behavior.
pub fn bind_image_settings(registry: &Rc<dyn SettingCvars>) -> Result<Vec<SettingBinding>, ClientError> {
    let replacement = bind_cvar_setting(
        registry,
        CvarSettingSpec {
            name: "r_override_textures".to_string(),
            label: "Replacement images".to_string(),
            category: SettingCategory::Video,
            restart: None,
            kind: CvarSettingKind::Choice {
                choices: vec![
                    UiChoice {
                        id: "0".to_string(),
                        label: "Disabled".to_string(),
                    },
                    UiChoice {
                        id: "1".to_string(),
                        label: "Replace classic formats".to_string(),
                    },
                    UiChoice {
                        id: "2".to_string(),
                        label: "Replace all formats".to_string(),
                    },
                ],
            },
        },
        None,
    )?;
    let formats = bind_cvar_setting(
        registry,
        CvarSettingSpec {
            name: "r_texture_formats".to_string(),
            label: "Formats (source = default)".to_string(),
            category: SettingCategory::Video,
            restart: None,
            kind: CvarSettingKind::TextEntry {
                maximum_length: 128,
                submit_only: false,
            },
        },
        None,
    )?;
    let mask = bind_cvar_setting(
        registry,
        CvarSettingSpec {
            name: "r_texture_overrides".to_string(),
            label: "Image types".to_string(),
            category: SettingCategory::Video,
            restart: None,
            kind: CvarSettingKind::TextEntry {
                maximum_length: 12,
                submit_only: false,
            },
        },
        None,
    )?;
    let SettingBindingKind::TextEntry { read, write, .. } = &mask.kind else {
        return Err(ClientError::BadUi(
            "Image mask requires a numeric text binding".to_string(),
        ));
    };
    let mask_read = Rc::clone(read);
    let mask_write = Rc::clone(write);
    let mask_enabled = Rc::clone(&mask.enabled);
    // quake-2-re-ts ImagetypeT bits, explicitly mapped rather than UI choice
    // ordinals.
    let usages: [(&str, &str, i32); 5] = [
        ("skin", "Replace model skins", 1),
        ("sprite", "Replace sprites", 2),
        ("wall", "Replace wall textures", 4),
        ("picture", "Replace pictures", 8),
        ("sky", "Replace sky images", 16),
    ];
    let mut settings = vec![replacement, formats];
    for (usage, label, bit) in usages {
        let id = UiControlId::new(&format!("ui:settings:image-{usage}"))?;
        let read_reader = Rc::clone(&mask_read);
        let write_reader = Rc::clone(&mask_read);
        let writer = Rc::clone(&mask_write);
        settings.push(SettingBinding {
            id,
            label: label.to_string(),
            category: SettingCategory::Video,
            enabled: Rc::clone(&mask_enabled),
            kind: SettingBindingKind::Toggle {
                read: Rc::new(move || (mask_number(&read_reader()) & bit) != 0),
                write: Rc::new(move |value| {
                    let current = mask_number(&write_reader());
                    let next = if value { current | bit } else { current & !bit };
                    writer(&next.to_string());
                }),
            },
        });
    }
    Ok(settings)
}

/// Bind the enhanced-model controls (donor `bindModelSettings`).
pub fn bind_model_settings(registry: &Rc<dyn SettingCvars>) -> Result<Vec<SettingBinding>, ClientError> {
    let mut settings = Vec::new();
    for (name, label) in [
        ("r_enhancedmodels", "Q1 enhanced models"),
        ("gl_md5_load", "Load Q2 enhanced models"),
        ("gl_md5_use", "Draw Q2 enhanced models"),
    ] {
        settings.push(bind_cvar_setting(
            registry,
            CvarSettingSpec {
                name: name.to_string(),
                label: label.to_string(),
                category: SettingCategory::Video,
                restart: None,
                kind: CvarSettingKind::Toggle,
            },
            None,
        )?);
    }
    settings.push(bind_cvar_setting(
        registry,
        CvarSettingSpec {
            name: "r_model_distance".to_string(),
            label: "Model range (map units)".to_string(),
            category: SettingCategory::Video,
            restart: None,
            kind: CvarSettingKind::TextEntry {
                maximum_length: 16,
                submit_only: true,
            },
        },
        None,
    )?);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::ui::settings::gameplay::test_support::MemoryCvars;
    use crate::ui::settings::{SettingBindingKind, SettingCvars};
    use crate::ui::types::CommandDialect;

    use super::{bind_image_settings, bind_model_settings};

    fn image_registry() -> Rc<dyn SettingCvars> {
        Rc::new(
            MemoryCvars::new(CommandDialect::Q2Rerelease)
                .with("r_override_textures", "1")
                .with("r_texture_formats", "p8 wal")
                .with("r_texture_overrides", "5"),
        )
    }

    fn kind_name(kind: &SettingBindingKind) -> &'static str {
        match kind {
            SettingBindingKind::Toggle { .. } => "toggle",
            SettingBindingKind::Slider { .. } => "slider",
            SettingBindingKind::Choice { .. } => "choice",
            SettingBindingKind::TextEntry { .. } => "text-entry",
            SettingBindingKind::Button { .. } => "button",
        }
    }

    #[test]
    fn image_bindings_cover_overrides_and_usage_bits() {
        let registry = image_registry();
        let bindings = bind_image_settings(&registry).expect("image bindings");
        assert_eq!(bindings.len(), 7);
        let ids: Vec<&str> = bindings.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "ui:settings:r_override_textures",
                "ui:settings:r_texture_formats",
                "ui:settings:image-skin",
                "ui:settings:image-sprite",
                "ui:settings:image-wall",
                "ui:settings:image-picture",
                "ui:settings:image-sky",
            ]
        );
        match &bindings[0].kind {
            SettingBindingKind::Choice { choices, write, .. } => {
                assert_eq!(choices().len(), 3);
                write("2");
                assert_eq!(
                    registry.find("r_override_textures").map(|v| v.value),
                    Some("2".to_string())
                );
            }
            other => panic!("expected override choice, got {}", kind_name(other)),
        }
        // Mask 5 = skin + wall.
        let states: Vec<bool> = bindings[2..]
            .iter()
            .map(|binding| match &binding.kind {
                SettingBindingKind::Toggle { read, .. } => read(),
                other => panic!("expected usage toggle, got {}", kind_name(other)),
            })
            .collect();
        assert_eq!(states, vec![true, false, true, false, false]);
    }

    #[test]
    #[should_panic(expected = "Unknown cvar setting choice")]
    fn image_override_rejects_unknown_choice() {
        let registry = image_registry();
        let bindings = bind_image_settings(&registry).expect("image bindings");
        match &bindings[0].kind {
            SettingBindingKind::Choice { write, .. } => write("7"),
            other => panic!("expected override choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn image_usage_writes_preserve_other_bits() {
        let registry = image_registry();
        let bindings = bind_image_settings(&registry).expect("image bindings");
        match &bindings[3].kind {
            SettingBindingKind::Toggle { write, .. } => write(true),
            other => panic!("expected sprite toggle, got {}", kind_name(other)),
        }
        assert_eq!(
            registry.find("r_texture_overrides").map(|v| v.value),
            Some("7".to_string())
        );
        match &bindings[2].kind {
            SettingBindingKind::Toggle { write, .. } => write(false),
            other => panic!("expected skin toggle, got {}", kind_name(other)),
        }
        assert_eq!(
            registry.find("r_texture_overrides").map(|v| v.value),
            Some("6".to_string())
        );
    }

    #[test]
    fn image_binding_fails_without_owner() {
        let registry: Rc<dyn SettingCvars> =
            Rc::new(MemoryCvars::new(CommandDialect::Q2Rerelease).with("r_override_textures", "0"));
        assert!(bind_image_settings(&registry).is_err());
    }

    #[test]
    fn model_bindings_cover_toggles_and_range_entry() {
        let registry: Rc<dyn SettingCvars> = Rc::new(
            MemoryCvars::new(CommandDialect::Q2Rerelease)
                .with("r_enhancedmodels", "1")
                .with("gl_md5_load", "0")
                .with("gl_md5_use", "0")
                .with("r_model_distance", "2048"),
        );
        let bindings = bind_model_settings(&registry).expect("model bindings");
        assert_eq!(bindings.len(), 4);
        match &bindings[0].kind {
            SettingBindingKind::Toggle { read, write, .. } => {
                assert!(read());
                write(false);
                assert!(!read());
            }
            other => panic!("expected toggle, got {}", kind_name(other)),
        }
        match &bindings[3].kind {
            SettingBindingKind::TextEntry {
                read,
                write,
                maximum_length,
                commit,
            } => {
                assert_eq!(*maximum_length, 16);
                let commit = commit.as_ref().expect("submit-only commit");
                write("4096");
                assert_eq!(read(), "4096");
                assert_eq!(
                    registry.find("r_model_distance").map(|v| v.value),
                    Some("2048".to_string())
                );
                (commit.submit)("4096");
                assert_eq!(
                    registry.find("r_model_distance").map(|v| v.value),
                    Some("4096".to_string())
                );
                write("1024");
                (commit.cancel)();
                assert_eq!(read(), "4096");
            }
            other => panic!("expected range entry, got {}", kind_name(other)),
        }
    }

    #[test]
    fn model_binding_fails_without_owner() {
        let registry: Rc<dyn SettingCvars> = Rc::new(MemoryCvars::new(CommandDialect::Q2Rerelease));
        assert!(bind_model_settings(&registry).is_err());
    }
}
