//! Shared renderer image settings: cvars, display, policies, persistence.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/image-settings.ts`
//! (`ApplicationImageSettings`). The registry has no value-binding,
//! documentation, transfer, or canonical-snapshot APIs, so validators live
//! in a local map enforced by [`ApplicationImageSettings::set_validated`],
//! cvar documents are the [`IMAGE_CVAR_DOCS`] table, and client-settings
//! transfer copies archived values. Shared/Q1 client cvars come from a
//! caller hook (those donors belong to other lanes); accessibility cvars
//! use the real sibling. The field-of-view validator is absorbed from
//! `src/app/bootstrap/shared-setting-cvars.ts` (`validateFieldOfView`).

use qa_client::audio::output::AudioOutputFormat;
use qa_client::render::scene::image_policy::{image_policy_from_controls, ImageControls, ImagePolicy};
use qa_client::render::scene::models::replacements::{ModelReplacementPolicy, ReplacementDistance};
use qa_client::ui::settings::accessibility::register_accessibility_settings;
use qa_client::ui::settings::{CvarValidator, CvarView, RegisterCvars, SettingCvars};
use qa_client::ui::types::CommandDialect as ClientDialect;
use qa_content::user_data::default_user_content_root;
use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::cmd_buffer::{BufferError, BufferOptions, BufferServices, CommandBuffer, CommandContext};
use qa_core::cvar::flags;
use qa_core::cvar::{CvarError, CvarRegistry};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use thiserror::Error;

use crate::options::DisplayOverrides;
use crate::settings::config::ConfigStore;
use crate::settings::SettingsError;

/// Failure of image settings operations.
#[derive(Debug, Error)]
pub enum ImageSettingsError {
    /// Invalid setting value.
    #[error("{0}")]
    Invalid(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Command failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
    /// Command text failure.
    #[error(transparent)]
    Cmd(#[from] qa_core::cmd::CmdError),
    /// Settings store failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// Image refresh failure.
    #[error("{0}")]
    Refresh(String),
}

/// One cvar document: name, summary, usage, allowed values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageCvarDoc {
    /// Cvar name.
    pub name: &'static str,
    /// Summary.
    pub summary: &'static str,
    /// Usage line.
    pub usage: &'static str,
    /// Allowed values, when enumerated.
    pub allowed: &'static [&'static str],
}

/// Cvar documents (the registry has no documentation storage).
pub const IMAGE_CVAR_DOCS: &[ImageCvarDoc] = &[
    ImageCvarDoc {
        name: "con_scale",
        summary: "Console text size. Auto chooses a readable size; small viewports limit the size to keep text usable.",
        usage: "con_scale <0|1|2|3|4>",
        allowed: &["0: Auto", "1: 1x", "2: 2x", "3: 3x", "4: 4x"],
    },
    ImageCvarDoc {
        name: "r_gamma",
        summary: "Display brightness for CPU and GL output: 1 is unchanged, above 1 brightens, below 1 darkens.",
        usage: "r_gamma <0.5..3>",
        allowed: &["Finite numbers from 0.5 through 3"],
    },
    ImageCvarDoc {
        name: "r_customwidth",
        summary: "Window width in logical pixels, applied while windowed; 0 uses the current width. Restored oversized windows recover to desktop bounds.",
        usage: "r_customwidth <width>",
        allowed: &["0: current width", "Integers from 64 through 16384"],
    },
    ImageCvarDoc {
        name: "r_customheight",
        summary: "Window height in logical pixels, applied while windowed; 0 uses the current height. Restored oversized windows recover to desktop bounds.",
        usage: "r_customheight <height>",
        allowed: &["0: current height", "Integers from 64 through 16384"],
    },
    ImageCvarDoc {
        name: "r_fullscreen",
        summary: "Switch between a window and borderless desktop fullscreen; does not select an exclusive display mode.",
        usage: "r_fullscreen <0|1>",
        allowed: &["0: windowed", "1: borderless desktop fullscreen"],
    },
    ImageCvarDoc {
        name: "r_swapInterval",
        summary: "GL vertical synchronization (vsync), when supported by the display backend. Has no effect on the CPU renderer.",
        usage: "r_swapInterval <0|1>",
        allowed: &["0: off", "1: on"],
    },
    ImageCvarDoc {
        name: "gl_debug_linewidth",
        summary: "Width in pixels for shared debug shapes. Not saved.",
        usage: "gl_debug_linewidth <width>",
        allowed: &["Positive finite numbers"],
    },
    ImageCvarDoc {
        name: "gl_debug_distfrac",
        summary: "Distance culling factor for world text that requests distance culling: text is hidden when its cell size is smaller than forward camera distance times this factor. Not saved.",
        usage: "gl_debug_distfrac <factor>",
        allowed: &[],
    },
    ImageCvarDoc {
        name: "r_override_textures",
        summary: "Replacement image priority: below 1 keeps requested files first, 1 prioritizes replacements for native images, above 1 also prioritizes them for truecolor images. Fallback image searches still run at 0.",
        usage: "r_override_textures <level>",
        allowed: &[],
    },
    ImageCvarDoc {
        name: "r_texture_overrides",
        summary: "Usage bitmask for replacement image priority: skin 1, sprite 2, wall 4, picture 8, sky 16; add bits to combine. -1 selects all, 0 selects none. Does not disable fallback searches.",
        usage: "r_texture_overrides <mask>",
        allowed: &[],
    },
    ImageCvarDoc {
        name: "r_texture_formats",
        summary: "Replacement image search order. source uses the content family's order; otherwise lists png, jpg, tga, jpeg, bmp, gif. Legacy format initials are accepted and unknown letters ignored.",
        usage: "r_texture_formats <source|quoted format list>",
        allowed: &[],
    },
    ImageCvarDoc {
        name: "r_enhancedmodels",
        summary: "Enable loading and drawing available mounted Quake I enhanced model replacements; native models remain the fallback.",
        usage: "r_enhancedmodels <number>",
        allowed: &["0: disabled", "Nonzero: enabled"],
    },
    ImageCvarDoc {
        name: "gl_md5_load",
        summary: "Enable loading available mounted Quake II MD5 model replacements. Drawing them also requires gl_md5_use.",
        usage: "gl_md5_load <number>",
        allowed: &["0: disabled", "Nonzero: enabled"],
    },
    ImageCvarDoc {
        name: "gl_md5_use",
        summary: "Draw loaded Quake II MD5 replacements instead of native models, subject to replacement distance limits. Requires gl_md5_load.",
        usage: "gl_md5_use <number>",
        allowed: &["0: disabled", "Nonzero: enabled"],
    },
    ImageCvarDoc {
        name: "gl_md5_distance",
        summary: "Quake II replacement model view distance in map units when r_model_distance is source. Positive values fall back to native models beyond the limit; nonpositive values remove the cutoff. Shadows bypass this distance cutoff.",
        usage: "gl_md5_distance <distance>",
        allowed: &[],
    },
    ImageCvarDoc {
        name: "r_model_distance",
        summary: "Shared replacement model view distance in map units. source uses no cutoff for Quake I and gl_md5_distance for Quake II. Positive numeric overrides fall back to native models beyond the limit; nonpositive values remove it. Shadows bypass this cutoff.",
        usage: "r_model_distance <source|finite distance>",
        allowed: &[],
    },
    ImageCvarDoc {
        name: "r_smp",
        summary: "Render on a worker after vid_restart.",
        usage: "r_smp <0|1>",
        allowed: &["0: main thread", "1: worker"],
    },
];

/// Document for a cvar, if documented.
#[must_use]
pub fn image_cvar_doc(name: &str) -> Option<&'static ImageCvarDoc> {
    IMAGE_CVAR_DOCS.iter().find(|doc| doc.name == name)
}

/// Field-of-view validation absorbed from `shared-setting-cvars.ts`.
#[must_use]
pub fn validate_field_of_view(text: &str) -> Option<String> {
    fn numeric(text: &str) -> bool {
        let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
        if digits.is_empty() {
            return false;
        }
        match digits.split_once('.') {
            None => digits.bytes().all(|byte| byte.is_ascii_digit()),
            Some((head, tail)) => {
                if tail.contains('.') || !tail.bytes().all(|byte| byte.is_ascii_digit()) {
                    return false;
                }
                if head.is_empty() {
                    !tail.is_empty()
                } else {
                    head.bytes().all(|byte| byte.is_ascii_digit())
                }
            }
        }
    }
    let value: f64 = text.parse().unwrap_or(f64::NAN);
    if !numeric(text) || !value.is_finite() || value < 60.0 || value > 160.0 {
        Some("Field of view must be between 60 and 160 degrees".to_owned())
    } else {
        None
    }
}

/// Shared print sink.
type PrintSink = Rc<RefCell<dyn FnMut(&str)>>;

/// Adapter registering accessibility cvars on a real registry.
struct RegistryAdapter<'a> {
    cvars: &'a mut CvarRegistry,
    validators: HashMap<String, CvarValidator>,
}

impl SettingCvars for RegistryAdapter<'_> {
    fn dialect(&self) -> ClientDialect {
        match self.cvars.dialect() {
            Dialect::Q1Netquake => ClientDialect::Q1Netquake,
            Dialect::Q1Quakeworld => ClientDialect::Q1Quakeworld,
            Dialect::Q2Classic => ClientDialect::Q2Classic,
            Dialect::Q2Rerelease => ClientDialect::Q2Rerelease,
            Dialect::Q3 => ClientDialect::Q3,
        }
    }

    fn find(&self, name: &str) -> Option<CvarView> {
        self.cvars.get(name).map(|snapshot| CvarView {
            value: snapshot.value,
            latched_value: snapshot.latched_value,
            reset_value: snapshot.reset_value,
            flags: snapshot.flags,
        })
    }

    fn set(&self, _name: &str, _value: &str) {}

    fn variable_value(&self, name: &str) -> f32 {
        self.cvars.variable_value(name)
    }
}

impl RegisterCvars for RegistryAdapter<'_> {
    fn register(&mut self, name: &str, value: &str, archive: bool) {
        let _ = self
            .cvars
            .register(name, value, if archive { flags::ARCHIVE } else { 0 });
    }

    fn bind_validator(&mut self, name: &str, validate: CvarValidator) {
        self.validators.insert(name.to_owned(), validate);
    }
}

/// Options for opening image settings.
#[derive(Clone)]
pub struct ImageSettingsOptions {
    /// Audio output format for shared client settings.
    pub audio_output_format: Option<AudioOutputFormat>,
    /// Defer persistence until enabled.
    pub defer_persistence: bool,
    /// Command context for config execution.
    pub context: CommandContext,
    /// Command dialect for config execution.
    pub dialect: Dialect,
    /// User content root override.
    pub user_content_root: Option<String>,
    /// Initial gamma.
    pub gamma: Option<f64>,
    /// Render worker override.
    pub render_worker: Option<bool>,
    /// Display overrides.
    pub display_overrides: Option<DisplayOverrides>,
    /// Print sink.
    pub print: PrintSink,
}

/// One persisted name/value entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedEntry {
    /// Cvar name.
    pub name: String,
    /// Cvar value.
    pub value: String,
}

/// Display window state for image settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDisplayState {
    /// Logical width.
    pub width: f64,
    /// Logical height.
    pub height: f64,
    /// Borderless fullscreen.
    pub fullscreen: bool,
    /// GL backend.
    pub backend_gl: bool,
    /// Display bounds width.
    pub display_width: f64,
    /// Display bounds height.
    pub display_height: f64,
    /// Window identity generation.
    pub generation: u64,
}

/// Renderer surface for display application.
pub trait ImageDisplayRenderer {
    /// Current output gamma.
    fn output_gamma(&self) -> f32;
    /// Set output gamma.
    fn set_output_gamma(&mut self, gamma: f32) -> Result<(), String>;
    /// Current window state.
    fn display_state(&self) -> ImageDisplayState;
    /// Resize the window.
    fn set_window_size(&mut self, width: f64, height: f64);
    /// Set fullscreen.
    fn set_fullscreen(&mut self, fullscreen: bool);
    /// Current swap interval.
    fn swap_interval(&self) -> i32;
    /// Set swap interval.
    fn set_swap_interval(&mut self, interval: i32);
}

/// Prepared images token: commit or discard.
pub trait PreparedImages {
    /// Commit the refresh.
    fn commit(self);
    /// Discard the refresh.
    fn discard(self);
}

/// Prepared presentation binding: commit or discard.
pub trait PreparedBinding {
    /// Commit the binding.
    fn commit(self);
    /// Discard the binding.
    fn discard(self);
}

/// Image refresh assets.
pub trait ImageRefreshAssets {
    /// Prepared images token.
    type Images: PreparedImages;
    /// Current image policy.
    fn image_policy(&self) -> ImagePolicy;
    /// Current model policy.
    fn model_policy(&self) -> ModelReplacementPolicy;
    /// Prepare an image refresh.
    fn prepare(
        &mut self,
        policy: &ImagePolicy,
        model_policy: &ModelReplacementPolicy,
    ) -> Result<Self::Images, ImageSettingsError>;
    /// Set the model policy without reloading images.
    fn set_model_policy(&mut self, model_policy: ModelReplacementPolicy);
    /// Finish an image refresh.
    fn finish(&mut self);
}

/// One seat presentation participating in image refresh.
pub trait ImageRefreshPresentation {
    /// Prepared binding token.
    type Binding: PreparedBinding;
    /// Prepare the presentation binding.
    fn prepare(&mut self, policy: &ImagePolicy) -> Result<Self::Binding, ImageSettingsError>;
}

/// Rerelease sky refresh.
pub trait RereleaseRefresh {
    /// Prepare the sky refresh, returning its commit closure.
    fn prepare(&mut self, policy: &ImagePolicy) -> Result<Option<Box<dyn FnOnce()>>, ImageSettingsError>;
}

/// View-settings cvar binding.
pub trait ViewCvarBinding {
    /// Bind view cvars, returning a release closure.
    fn bind_cvars(&mut self, cvars: &mut CvarRegistry) -> Result<Box<dyn FnOnce()>, ImageSettingsError>;
}

/// Shared renderer choices are local configuration, independent of source game state.
pub struct ApplicationImageSettings {
    /// Settings cvars.
    pub cvars: CvarRegistry,
    options: ImageSettingsOptions,
    store: ConfigStore,
    applied: String,
    saved: String,
    persistence_enabled: bool,
    applied_debug_line_width: f64,
    display_applied: String,
    display_generation: Option<u64>,
    restored_size: Option<(f64, f64)>,
    persisted: Vec<PersistedEntry>,
    applied_values: Vec<PersistedEntry>,
    validators: HashMap<String, CvarValidator>,
    candidate_fov_validation: bool,
}

impl ApplicationImageSettings {
    fn image_setting(name: &str) -> bool {
        const OWNED: &[&str] = &["gamma", "volume", "bgmvolume", "music_shuffle", "music_menu_track"];
        if OWNED.contains(&name) {
            return false;
        }
        if crate::bootstrap::audio::output_settings::AUDIO_OUTPUT_CVAR_NAMES.contains(&name) {
            return false;
        }
        if qa_client::input::device::input_device_cvar_names()
            .iter()
            .any(|owned| owned == name)
        {
            return false;
        }
        true
    }

    fn archived_values(&self) -> Vec<PersistedEntry> {
        self.cvars
            .snapshots(flags::ARCHIVE)
            .into_iter()
            .filter(|snapshot| (snapshot.flags & flags::ARCHIVE) != 0 && Self::image_setting(&snapshot.name))
            .map(|snapshot| PersistedEntry {
                name: snapshot.name,
                value: snapshot.value,
            })
            .collect()
    }

    fn escape_json(text: &str, out: &mut String) {
        for ch in text.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
                _ => out.push(ch),
            }
        }
    }

    fn signature(&self) -> String {
        let mut out = String::from("[");
        for (index, value) in self.archived_values().iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            let mut name = String::new();
            Self::escape_json(&value.name, &mut name);
            let mut val = String::new();
            Self::escape_json(&value.value, &mut val);
            out.push_str(&format!("[\"{name}\",\"{val}\"]"));
        }
        out.push(']');
        out
    }

    fn register_owned(cvars: &mut CvarRegistry, gamma: f64) -> Result<(), ImageSettingsError> {
        cvars.register("fov", "90", flags::ARCHIVE)?;
        cvars.register("con_scale", "0", flags::ARCHIVE)?;
        cvars.register("r_gamma", &gamma.to_string(), flags::ARCHIVE)?;
        cvars.register("r_customwidth", "0", flags::ARCHIVE)?;
        cvars.register("r_customheight", "0", flags::ARCHIVE)?;
        cvars.register("r_fullscreen", "0", flags::ARCHIVE)?;
        cvars.register("r_swapInterval", "1", flags::ARCHIVE)?;
        cvars.register("r_smp", "0", flags::ARCHIVE)?;
        cvars.register("gl_debug_linewidth", "2", 0)?;
        cvars.register("gl_debug_distfrac", "0.004", 0)?;
        cvars.register("r_override_textures", "1", flags::ARCHIVE)?;
        cvars.register("r_texture_overrides", "-1", flags::ARCHIVE)?;
        cvars.register("r_texture_formats", "source", flags::ARCHIVE)?;
        cvars.register("r_enhancedmodels", "1", flags::ARCHIVE)?;
        cvars.register("gl_md5_load", "1", flags::ARCHIVE)?;
        cvars.register("gl_md5_use", "1", flags::ARCHIVE)?;
        cvars.register("gl_md5_distance", "2048", flags::ARCHIVE)?;
        cvars.register("r_model_distance", "source", flags::ARCHIVE)?;
        Ok(())
    }

    fn create(
        options: &ImageSettingsOptions,
        register_extra: &dyn Fn(&mut CvarRegistry) -> Result<(), ImageSettingsError>,
    ) -> Result<Self, ImageSettingsError> {
        let root: PathBuf = options
            .user_content_root
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(default_user_content_root)
            .join("settings");
        let mut cvars = CvarRegistry::new(options.dialect);
        Self::register_owned(&mut cvars, options.gamma.unwrap_or(1.0))?;
        register_extra(&mut cvars)?;
        let mut adapter = RegistryAdapter {
            cvars: &mut cvars,
            validators: HashMap::new(),
        };
        register_accessibility_settings(&mut adapter);
        let RegistryAdapter { validators, .. } = adapter;
        let mut settings = Self {
            cvars,
            options: options.clone(),
            store: ConfigStore::new(root),
            applied: String::new(),
            saved: String::new(),
            persistence_enabled: !options.defer_persistence,
            applied_debug_line_width: 2.0,
            display_applied: String::new(),
            display_generation: None,
            restored_size: None,
            persisted: Vec::new(),
            applied_values: Vec::new(),
            validators,
            candidate_fov_validation: false,
        };
        settings.validators.insert(
            "r_smp".to_owned(),
            Rc::new(|value| {
                if value == "0" || value == "1" {
                    None
                } else {
                    Some("r_smp must be 0 or 1".to_owned())
                }
            }),
        );
        Ok(settings)
    }

    /// Open settings, restoring `images.cfg` and applying overrides.
    pub fn open(
        options: ImageSettingsOptions,
        services: &mut dyn BufferServices,
        register_extra: &dyn Fn(&mut CvarRegistry) -> Result<(), ImageSettingsError>,
    ) -> Result<Self, ImageSettingsError> {
        let mut settings = Self::create(&options, register_extra)?;
        if let Some(text) = settings.store.load_text("images.cfg")? {
            let mut touched = Vec::new();
            for line in text.lines() {
                let tokens = tokenize_command(line, options.dialect, TextMode::Source)?;
                let mut words = tokens.argv.iter();
                let Some(command) = words.next() else { continue };
                let name = if command == "set" || command == "seta" {
                    words.next()
                } else {
                    Some(command)
                };
                if let Some(name) = name {
                    touched.push(name.clone());
                }
            }
            let mut commands = CommandBuffer::new(options.dialect, options.context.clone(), BufferOptions::default())?;
            {
                let print = Rc::clone(&settings.options.print);
                commands.set_printer(move |text, _| print.borrow_mut()(text));
            }
            commands.append(&text, Some(&options.context), None)?;
            commands.execute(&mut settings.cvars, services)?;
            let mut loaded: HashMap<String, PersistedEntry> = HashMap::new();
            for name in touched {
                if let Some(state) = settings.cvars.get(&name) {
                    if (state.flags & flags::ARCHIVE) != 0 {
                        loaded.insert(
                            state.name.clone(),
                            PersistedEntry {
                                name: state.name.clone(),
                                value: state.value.clone(),
                            },
                        );
                    }
                }
            }
            settings.persisted = loaded.into_values().collect();
            settings.restored_size = Some((
                f64::from(settings.cvars.variable_value("r_customwidth")),
                f64::from(settings.cvars.variable_value("r_customheight")),
            ));
        }
        let saved = settings.signature();
        if let Some(worker) = options.render_worker {
            settings.cvars.set("r_smp", if worker { "1" } else { "0" }, true)?;
        }
        if let Some(overrides) = options.display_overrides.as_ref() {
            if let Some(width) = overrides.width {
                settings.cvars.set("r_customwidth", &width.to_string(), true)?;
            }
            if let Some(height) = overrides.height {
                settings.cvars.set("r_customheight", &height.to_string(), true)?;
            }
            if let Some(gamma) = overrides.gamma {
                settings.cvars.set("r_gamma", &gamma.to_string(), true)?;
            }
        }
        settings.applied = settings.signature();
        settings.saved = saved;
        settings.applied_values = settings.archived_values();
        Ok(settings)
    }

    /// Enable deferred persistence.
    pub fn enable_persistence(&mut self) {
        self.persistence_enabled = true;
    }

    /// Entries restored from `images.cfg`.
    #[must_use]
    pub fn persisted_entries(&self) -> &[PersistedEntry] {
        &self.persisted
    }

    /// Set a cvar through local validators.
    pub fn set_validated(&mut self, name: &str, value: &str) -> Result<(), ImageSettingsError> {
        if let Some(validate) = self.validators.get(name) {
            if let Some(reason) = validate(value) {
                return Err(ImageSettingsError::Invalid(reason));
            }
        }
        self.cvars.set(name, value, false)?;
        Ok(())
    }

    /// Bind view settings, suspending candidate fov validation while bound.
    pub fn bind_view_settings(&mut self, view: &mut dyn ViewCvarBinding) -> Result<ViewBinding, ImageSettingsError> {
        let staged = self.candidate_fov_validation;
        self.validators.remove("fov");
        self.candidate_fov_validation = false;
        match view.bind_cvars(&mut self.cvars) {
            Ok(release) => Ok(ViewBinding {
                release: Some(release),
                restore_fov: staged,
            }),
            Err(error) => {
                if staged {
                    self.stage_candidate_fov();
                }
                Err(error)
            }
        }
    }

    fn stage_candidate_fov(&mut self) {
        self.validators
            .insert("fov".to_owned(), Rc::new(validate_field_of_view));
        self.candidate_fov_validation = true;
    }

    /// Release a view binding, restoring staged fov validation.
    pub fn release_view_binding(&mut self, mut binding: ViewBinding) {
        if let Some(release) = binding.release.take() {
            release();
            if binding.restore_fov {
                self.stage_candidate_fov();
            }
        }
    }

    /// Prepare transferable client settings with staged fov validation.
    pub fn prepare_client_settings(
        &self,
        register_extra: &dyn Fn(&mut CvarRegistry) -> Result<(), ImageSettingsError>,
    ) -> Result<PreparedClientSettings, ImageSettingsError> {
        let mut options = self.options.clone();
        options.defer_persistence = true;
        let mut settings = Self::create(&options, register_extra)?;
        settings.stage_candidate_fov();
        let staged = self
            .archived_values()
            .into_iter()
            .map(|entry| (entry.name, entry.value))
            .collect();
        Ok(PreparedClientSettings { settings, staged })
    }

    /// Current image policy.
    #[must_use]
    pub fn policy(&self) -> ImagePolicy {
        image_policy_from_controls(&ImageControls {
            override_level: self.cvars.variable_value("r_override_textures").trunc() as i32,
            override_mask: self.cvars.variable_value("r_texture_overrides").trunc() as i32,
            formats: self.cvars.variable_string("r_texture_formats"),
        })
    }

    /// Debug line width, repairing invalid values.
    pub fn debug_line_width(&mut self) -> Result<f64, ImageSettingsError> {
        let width = f64::from(self.cvars.variable_value("gl_debug_linewidth"));
        if width.is_finite() && width > 0.0 {
            self.applied_debug_line_width = width;
        } else {
            self.cvars
                .set("gl_debug_linewidth", &self.applied_debug_line_width.to_string(), true)?;
            self.options.print.borrow_mut()("Debug line width must be a positive finite number.\n");
        }
        Ok(self.applied_debug_line_width)
    }

    /// Display gamma.
    #[must_use]
    pub fn gamma(&self) -> f32 {
        self.cvars.variable_value("r_gamma")
    }

    /// Model replacement policy.
    pub fn model_policy(&self) -> Result<ModelReplacementPolicy, ImageSettingsError> {
        let selected = self.cvars.variable_string("r_model_distance");
        let trimmed = selected.trim().to_lowercase();
        let distance = if trimmed == "source" {
            ReplacementDistance::Source
        } else {
            match trimmed.parse::<f64>() {
                Ok(value) if value.is_finite() => ReplacementDistance::Units(value as f32),
                _ => {
                    return Err(ImageSettingsError::Invalid(
                        "r_model_distance requires source or a finite distance".to_owned(),
                    ));
                }
            }
        };
        Ok(ModelReplacementPolicy {
            q1_enhanced: self.cvars.variable_value("r_enhancedmodels") != 0.0,
            q2_load: self.cvars.variable_value("gl_md5_load") != 0.0,
            q2_use: self.cvars.variable_value("gl_md5_use") != 0.0,
            q2_distance: self.cvars.variable_value("gl_md5_distance"),
            distance,
        })
    }

    fn display_signature(&self) -> String {
        ["r_customwidth", "r_customheight", "r_fullscreen", "r_swapInterval"]
            .iter()
            .map(|name| self.cvars.variable_string(name))
            .collect::<Vec<_>>()
            .join("/")
    }

    fn apply_display(&mut self, renderer: &mut dyn ImageDisplayRenderer) -> Result<(), ImageSettingsError> {
        if renderer.output_gamma() != self.gamma() {
            match renderer.set_output_gamma(self.gamma()) {
                Ok(()) => {}
                Err(error) => {
                    self.cvars.set("r_gamma", &renderer.output_gamma().to_string(), true)?;
                    self.options.print.borrow_mut()(&format!("Brightness rejected: {error}\n"));
                }
            }
        }
        let state = renderer.display_state();
        if Some(state.generation) != self.display_generation || self.display_signature() != self.display_applied {
            let restored = if self.display_applied.is_empty() {
                self.restored_size
            } else {
                None
            };
            let width = {
                let configured = f64::from(self.cvars.variable_value("r_customwidth"));
                if configured == 0.0 {
                    state.width
                } else {
                    configured
                }
            };
            let height = {
                let configured = f64::from(self.cvars.variable_value("r_customheight"));
                if configured == 0.0 {
                    state.height
                } else {
                    configured
                }
            };
            let valid = [width, height]
                .iter()
                .all(|value| value.is_finite() && value.fract() == 0.0 && *value >= 64.0 && *value <= 16384.0);
            if !valid {
                self.options.print.borrow_mut()("Display settings rejected: Invalid saved window size\n");
            } else {
                if !state.fullscreen {
                    let width_override = self
                        .options
                        .display_overrides
                        .as_ref()
                        .and_then(|overrides| overrides.width);
                    let height_override = self
                        .options
                        .display_overrides
                        .as_ref()
                        .and_then(|overrides| overrides.height);
                    let restored_width = width_override.is_none() && restored.is_some_and(|size| size.0 == width);
                    let restored_height = height_override.is_none() && restored.is_some_and(|size| size.1 == height);
                    renderer.set_window_size(
                        if restored_width {
                            width.min(state.display_width)
                        } else {
                            width
                        },
                        if restored_height {
                            height.min(state.display_height)
                        } else {
                            height
                        },
                    );
                }
                let fullscreen = self.cvars.variable_value("r_fullscreen");
                if fullscreen != 0.0 && fullscreen != 1.0 {
                    self.options.print.borrow_mut()("Display settings rejected: Invalid fullscreen setting\n");
                } else {
                    renderer.set_fullscreen(fullscreen == 1.0);
                }
                if state.backend_gl {
                    let interval = self.cvars.variable_value("r_swapInterval");
                    if interval != 0.0 && interval != 1.0 {
                        self.options.print.borrow_mut()("Display settings rejected: Vertical sync must be on or off\n");
                    } else if renderer.swap_interval() != interval as i32 {
                        renderer.set_swap_interval(interval as i32);
                    }
                }
            }
        }
        let state = renderer.display_state();
        if !state.fullscreen {
            self.cvars.set("r_customwidth", &state.width.to_string(), true)?;
            self.cvars.set("r_customheight", &state.height.to_string(), true)?;
        }
        self.cvars
            .set("r_fullscreen", if state.fullscreen { "1" } else { "0" }, true)?;
        if state.backend_gl {
            self.cvars.set(
                "r_swapInterval",
                if renderer.swap_interval() == 0 { "0" } else { "1" },
                true,
            )?;
        }
        self.display_applied = self.display_signature();
        self.display_generation = Some(state.generation);
        Ok(())
    }

    /// Apply display settings and persist.
    pub fn refresh_display(&mut self, renderer: &mut dyn ImageDisplayRenderer) -> Result<(), ImageSettingsError> {
        self.apply_display(renderer)?;
        self.save()
    }

    /// Refresh images, display, and persistence.
    #[allow(clippy::too_many_arguments)]
    pub fn refresh<A: ImageRefreshAssets, P: ImageRefreshPresentation, R: RereleaseRefresh>(
        &mut self,
        assets: &mut A,
        presentations: &mut [P],
        rerelease: Option<&mut R>,
        renderer: Option<&mut dyn ImageDisplayRenderer>,
    ) -> Result<(), ImageSettingsError> {
        if let Some(renderer) = renderer {
            self.apply_display(renderer)?;
        }
        let selected = self.signature();
        if selected == self.applied {
            return Ok(());
        }
        let policy = self.policy();
        let model_policy = self.model_policy()?;
        let current_model = assets.model_policy();
        let load_changed =
            model_policy.q1_enhanced != current_model.q1_enhanced || model_policy.q2_load != current_model.q2_load;
        if load_changed || policy != assets.image_policy() {
            enum Prepared<B, I> {
                Pending,
                Ready(B, I),
            }
            let mut staged: Prepared<Vec<P::Binding>, A::Images> = Prepared::Pending;
            let prepared: Result<Option<Box<dyn FnOnce()>>, ImageSettingsError> = (|| {
                let images = assets.prepare(&policy, &model_policy)?;
                let mut bindings = Vec::new();
                for presentation in presentations.iter_mut() {
                    bindings.push(presentation.prepare(&policy)?);
                }
                let sky = match rerelease {
                    Some(rerelease) => rerelease.prepare(&policy)?,
                    None => None,
                };
                staged = Prepared::Ready(bindings, images);
                Ok(sky)
            })();
            match (prepared, staged) {
                (Ok(sky), Prepared::Ready(bindings, images)) => {
                    images.commit();
                    for binding in bindings {
                        binding.commit();
                    }
                    if let Some(commit) = sky {
                        commit();
                    }
                }
                (Err(error), Prepared::Ready(bindings, images)) => {
                    for binding in bindings {
                        binding.discard();
                    }
                    images.discard();
                    self.rollback_applied()?;
                    self.options.print.borrow_mut()(&format!("Image settings rejected: {error}\n"));
                    return Ok(());
                }
                (Err(error), Prepared::Pending) => {
                    self.rollback_applied()?;
                    self.options.print.borrow_mut()(&format!("Image settings rejected: {error}\n"));
                    return Ok(());
                }
                (Ok(_), Prepared::Pending) => {}
            }
        } else {
            assets.set_model_policy(model_policy);
        }
        assets.finish();
        self.applied = selected;
        self.applied_values = self.archived_values();
        self.save()
    }

    fn rollback_applied(&mut self) -> Result<(), ImageSettingsError> {
        let values = self.applied_values.clone();
        for value in &values {
            self.cvars.set(&value.name, &value.value, true)?;
        }
        Ok(())
    }

    fn save(&mut self) -> Result<(), ImageSettingsError> {
        if !self.persistence_enabled {
            return Ok(());
        }
        let selected = self.signature();
        if selected == self.saved {
            return Ok(());
        }
        let commands = self.cvars.archive_commands(&Self::image_setting);
        self.store.dump(
            "images.cfg",
            &format!("// Generated by quake-typescript\n{}\n", commands.join("\n")),
        )?;
        self.saved = selected;
        Ok(())
    }

    /// Persist and close.
    pub fn close(&mut self) -> Result<(), ImageSettingsError> {
        self.save()
    }
}

/// View binding with fov-validation restoration.
pub struct ViewBinding {
    release: Option<Box<dyn FnOnce()>>,
    restore_fov: bool,
}

/// Staged client settings transfer.
pub struct PreparedClientSettings {
    settings: ApplicationImageSettings,
    staged: Vec<(String, String)>,
}

impl PreparedClientSettings {
    /// Borrow the staged settings.
    #[must_use]
    pub fn settings(&self) -> &ApplicationImageSettings {
        &self.settings
    }

    /// Validate the staged publication.
    pub fn validate_publication(&self) -> Result<(), ImageSettingsError> {
        for (name, value) in &self.staged {
            if let Some(validate) = self.settings.validators.get(name) {
                if let Some(reason) = validate(value) {
                    return Err(ImageSettingsError::Invalid(reason));
                }
            }
        }
        Ok(())
    }

    /// Publish staged values into the settings.
    pub fn publish(mut self) -> Result<ApplicationImageSettings, ImageSettingsError> {
        let staged = std::mem::take(&mut self.staged);
        for (name, value) in &staged {
            self.settings.cvars.set(name, value, true)?;
        }
        self.settings.applied = self.settings.signature();
        self.settings.applied_values = self.settings.archived_values();
        Ok(self.settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct NullServices;

    impl BufferServices for NullServices {
        fn read_script(&mut self, _name: &str, _source: &CommandContext) -> qa_core::cmd_buffer::ScriptRead {
            qa_core::cmd_buffer::ScriptRead::Ready(None)
        }

        fn forward_to_server(&mut self, _command: &qa_core::cmd_buffer::ForwardedCommand) {}
    }

    fn options(root: Option<String>) -> ImageSettingsOptions {
        ImageSettingsOptions {
            audio_output_format: None,
            defer_persistence: false,
            context: CommandContext::new(
                IdentityOwner::create("images").expect("owner").session().clone(),
                qa_core::cmd_buffer::CommandOrigin::LocalConsole,
            ),
            dialect: Dialect::Q3,
            user_content_root: root,
            gamma: None,
            render_worker: None,
            display_overrides: None,
            print: Rc::new(RefCell::new(|_: &str| {})),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-images-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn opens_with_defaults_and_validates() {
        let root = scratch("open");
        let mut settings = ApplicationImageSettings::open(
            options(Some(root.to_string_lossy().into_owned())),
            &mut NullServices,
            &|_| Ok(()),
        )
        .expect("open");
        assert_eq!(settings.gamma(), 1.0);
        assert!(settings.persisted_entries().is_empty());
        assert!(settings.set_validated("fov", "120").is_ok());
        assert!(settings.set_validated("r_smp", "2").is_err());
        assert!(settings.set_validated("r_smp", "1").is_ok());
        let policy = settings.model_policy().expect("model policy");
        assert!(policy.q1_enhanced);
        assert!(image_cvar_doc("r_gamma").is_some());
        assert!(image_cvar_doc("bogus").is_none());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn restores_config_and_applies_overrides() {
        let root = scratch("restore");
        std::fs::create_dir_all(root.join("settings")).expect("settings");
        std::fs::write(
            root.join("settings").join("images.cfg"),
            "seta fov 100\nseta r_gamma 1.5\n",
        )
        .expect("write");
        let mut config = options(Some(root.to_string_lossy().into_owned()));
        config.render_worker = Some(true);
        let settings = ApplicationImageSettings::open(config, &mut NullServices, &|_| Ok(())).expect("open");
        assert_eq!(settings.cvars.variable_string("fov"), "100");
        assert_eq!(settings.cvars.variable_string("r_gamma"), "1.5");
        assert_eq!(settings.cvars.variable_string("r_smp"), "1");
        assert_eq!(settings.persisted_entries().len(), 2);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn debug_width_repairs_and_client_transfer_validates() {
        let root = scratch("transfer");
        let printed = Rc::new(RefCell::new(Vec::new()));
        let printed_inner = Rc::clone(&printed);
        let mut config = options(Some(root.to_string_lossy().into_owned()));
        config.print = Rc::new(RefCell::new(move |text: &str| {
            printed_inner.borrow_mut().push(text.to_owned());
        }));
        let mut settings = ApplicationImageSettings::open(config, &mut NullServices, &|_| Ok(())).expect("open");
        settings.cvars.set("gl_debug_linewidth", "0", true).expect("set");
        assert_eq!(settings.debug_line_width().expect("width"), 2.0);
        assert!(!printed.borrow().is_empty());
        settings.cvars.set("fov", "90", true).expect("set");
        let transfer = settings.prepare_client_settings(&|_| Ok(())).expect("transfer");
        transfer.validate_publication().expect("validate");
        let mut child = transfer.publish().expect("publish");
        assert_eq!(child.cvars.variable_string("fov"), "90");
        assert!(child.set_validated("fov", "40").is_err());
        assert!(child.set_validated("fov", "100").is_ok());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn fov_validator_matches_donor_range() {
        assert!(validate_field_of_view("90").is_none());
        assert!(validate_field_of_view("60").is_none());
        assert!(validate_field_of_view("160").is_none());
        assert_eq!(
            validate_field_of_view("59").as_deref(),
            Some("Field of view must be between 60 and 160 degrees")
        );
        assert!(validate_field_of_view("161").is_some());
        assert!(validate_field_of_view("wide").is_some());
        assert!(validate_field_of_view("").is_some());
    }
}
