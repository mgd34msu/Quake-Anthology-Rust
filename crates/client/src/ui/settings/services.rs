//! Service-backed settings: audio volume, renderer, native video, language.
//!
//! Donor provenance: `src/ui/settings/services.ts` in full
//! (`bindEffectsVolume`, `bindRendererSettings`, `bindNativeVideoSettings`,
//! `NativeLanguageSettings`). Binding types ([`SettingBinding`],
//! [`SettingCategory`], [`SettingCvars`]) come from the settings root; this
//! module only adds UI-local view traits over the audio, window, and
//! localization services so no native handle crosses into UI code.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{SettingBinding, SettingBindingKind, SettingCategory, SettingCvars};
use crate::text::localization::{LocLoadTier, LocReloadOptions, LocalizationTable};
use crate::ui::types::{UiChoice, UiControlId};

/// Build a control id from a static template; the templates below always
/// carry the `ui:` namespace and a name part, so a failure is a programming
/// bug.
fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// Audio-output surface behind the effects-volume row.
pub trait AudioOutputView {
    /// Whether the output is closed; the row disables itself while closed.
    fn is_closed(&self) -> bool;
    /// Set the effects volume in `[0, 1]`.
    fn set_effects_volume(&mut self, volume: f32);
}

/// Bind the "Sound volume" slider to one audio output.
#[must_use]
pub fn bind_effects_volume(audio: Rc<RefCell<dyn AudioOutputView>>, read: Rc<dyn Fn() -> f32>) -> SettingBinding {
    let enabled_audio = Rc::clone(&audio);
    let write_audio = Rc::clone(&audio);
    SettingBinding {
        id: control_id("ui:audio:effects"),
        label: "Sound volume".to_string(),
        category: SettingCategory::Audio,
        enabled: Rc::new(move || !enabled_audio.borrow().is_closed()),
        kind: SettingBindingKind::Slider {
            read,
            write: Rc::new(move |value| write_audio.borrow_mut().set_effects_volume(value)),
            minimum: 0.0,
            maximum: 1.0,
            step: 0.05,
            format_value: None,
        },
    }
}

/// Renderer backend selector (donor `SdlWindow["backend"]`).
///
/// This is the UI-local backend *choice*; it is distinct from the render
/// crate's [`RendererBackend`](crate::render::RendererBackend) recording trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RendererBackend {
    /// Software rasterizer.
    Cpu,
    /// OpenGL renderer.
    Gl,
}

impl RendererBackend {
    /// Donor backend id.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            RendererBackend::Cpu => "cpu",
            RendererBackend::Gl => "gl",
        }
    }

    /// Choice label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            RendererBackend::Cpu => "CPU",
            RendererBackend::Gl => "OpenGL",
        }
    }
}

/// Render-worker toggle surface.
#[derive(Clone)]
pub struct WorkerToggle {
    /// Read the worker flag.
    pub read: Rc<dyn Fn() -> bool>,
    /// Write the worker flag.
    pub write: Rc<dyn Fn(bool)>,
}

/// Renderer settings inputs; every callback is reference counted so rows can
/// share them.
#[derive(Clone)]
pub struct RendererOptions {
    /// Live backend sampled before every read.
    pub current: Rc<dyn Fn() -> RendererBackend>,
    /// Worker toggle surface, when the backend exposes one.
    pub worker: Option<WorkerToggle>,
    /// Queue a backend change.
    pub apply: Rc<dyn Fn(RendererBackend)>,
    /// Report a queued change to the operator.
    pub report: Rc<dyn Fn(&str)>,
    /// Whether the rows are interactive.
    pub enabled: Rc<dyn Fn() -> bool>,
}

/// Draft renderer plus the last backend seen from the live sampler.
struct RendererDraft {
    active: RendererBackend,
    draft: RendererBackend,
}

/// Resync the draft when the live backend moved out from under it.
fn refresh_renderer(state: &Rc<RefCell<RendererDraft>>, current: &Rc<dyn Fn() -> RendererBackend>) {
    let live = current();
    let mut draft = state.borrow_mut();
    if live != draft.active {
        draft.active = live;
        draft.draft = live;
    }
}

/// Bind the render-worker toggle (when exposed), the renderer choice, and the
/// apply button. The choice edits a draft; apply queues it and reports.
#[must_use]
pub fn bind_renderer_settings(options: RendererOptions) -> Vec<SettingBinding> {
    let live = (options.current)();
    let state = Rc::new(RefCell::new(RendererDraft {
        active: live,
        draft: live,
    }));
    let mut bindings = Vec::new();
    if let Some(worker) = options.worker.clone() {
        let gate = Rc::clone(&options.enabled);
        let read = worker.read.clone();
        let write_gate = Rc::clone(&options.enabled);
        let write_worker = worker.write.clone();
        let write_current = Rc::clone(&options.current);
        let write_apply = Rc::clone(&options.apply);
        let write_report = Rc::clone(&options.report);
        bindings.push(SettingBinding {
            id: control_id("ui:video:render-worker"),
            label: "Render worker".to_string(),
            category: SettingCategory::Display,
            enabled: Rc::new(move || gate()),
            kind: SettingBindingKind::Toggle {
                read: Rc::new(move || read()),
                write: Rc::new(move |value| {
                    if !write_gate() {
                        return;
                    }
                    write_worker(value);
                    write_apply(write_current());
                    write_report("Render worker change queued.");
                }),
            },
        });
    }
    let choice_gate = Rc::clone(&options.enabled);
    let read_state = Rc::clone(&state);
    let read_current = Rc::clone(&options.current);
    let write_state = Rc::clone(&state);
    let write_current = Rc::clone(&options.current);
    bindings.push(SettingBinding {
        id: control_id("ui:video:renderer"),
        label: "Renderer".to_string(),
        category: SettingCategory::Display,
        enabled: Rc::new(move || choice_gate()),
        kind: SettingBindingKind::Choice {
            read: Rc::new(move || {
                refresh_renderer(&read_state, &read_current);
                read_state.borrow().draft.as_str().to_string()
            }),
            write: Rc::new(move |value| {
                refresh_renderer(&write_state, &write_current);
                let next = match value {
                    "cpu" => RendererBackend::Cpu,
                    "gl" => RendererBackend::Gl,
                    _ => panic!("Unknown renderer: {value}"),
                };
                write_state.borrow_mut().draft = next;
            }),
            choices: Rc::new(|| {
                [RendererBackend::Cpu, RendererBackend::Gl]
                    .into_iter()
                    .map(|backend| UiChoice {
                        id: backend.as_str().to_string(),
                        label: backend.label().to_string(),
                    })
                    .collect()
            }),
        },
    });
    let enabled_state = Rc::clone(&state);
    let enabled_current = Rc::clone(&options.current);
    let enabled_gate = Rc::clone(&options.enabled);
    let apply_state = Rc::clone(&state);
    let apply_current = Rc::clone(&options.current);
    let apply_gate = Rc::clone(&options.enabled);
    let apply_fn = Rc::clone(&options.apply);
    let apply_report = Rc::clone(&options.report);
    bindings.push(SettingBinding {
        id: control_id("ui:video:renderer-apply"),
        label: "Apply renderer".to_string(),
        category: SettingCategory::Display,
        enabled: Rc::new(move || {
            refresh_renderer(&enabled_state, &enabled_current);
            let gate = enabled_gate();
            let fresh = enabled_state.borrow();
            gate && fresh.draft != fresh.active
        }),
        kind: SettingBindingKind::Button {
            activate: Rc::new(move || {
                refresh_renderer(&apply_state, &apply_current);
                let draft = apply_state.borrow().draft;
                let active = apply_state.borrow().active;
                if !apply_gate() || draft == active {
                    return;
                }
                apply_fn(draft);
                apply_report(&format!("Renderer change queued: {}.", draft.label()));
            }),
        },
    });
    bindings
}

/// Native-window surface behind the display rows (donor `SdlWindow`).
pub trait WindowView {
    /// Current logical size in pixels.
    fn logical_size(&self) -> (u32, u32);
    /// Display modes in pixels.
    fn display_modes(&self) -> Vec<(u32, u32)>;
    /// Whether borderless fullscreen is active.
    fn fullscreen(&self) -> bool;
    /// Whether the OpenGL backend is active; gates the vsync row.
    fn is_gl(&self) -> bool;
    /// Resize the window; errors surface through the report sink.
    fn set_size(&mut self, width: u32, height: u32) -> Result<(), String>;
    /// Set borderless fullscreen; errors surface through the report sink.
    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), String>;
}

/// Builtin resolution table appended after the live display modes, in donor
/// order.
const BUILTIN_RESOLUTIONS: &[(u32, u32)] = &[
    (640, 480),
    (800, 600),
    (960, 600),
    (1024, 768),
    (1280, 720),
    (1280, 800),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (3440, 1440),
    (3840, 2160),
];

/// Donor resolution id (`{width}x{height}`).
fn resolution_id(size: (u32, u32)) -> String {
    format!("{}x{}", size.0, size.1)
}

/// Sorted, de-duplicated resolution table: live display modes, the current
/// size, then the builtin table.
fn resolution_choices(window: &Rc<RefCell<dyn WindowView>>) -> Vec<UiChoice> {
    let view = window.borrow();
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    for size in view
        .display_modes()
        .into_iter()
        .chain([view.logical_size()])
        .chain(BUILTIN_RESOLUTIONS.iter().copied())
    {
        if !sizes.contains(&size) {
            sizes.push(size);
        }
    }
    sizes.sort();
    sizes
        .into_iter()
        .map(|size| UiChoice {
            id: resolution_id(size),
            label: format!("{} x {}", size.0, size.1),
        })
        .collect()
}

/// Parse a custom-size draft; blank and non-numeric drafts read as invalid.
/// Hex and exponent forms the donor `Number()` read accepts stay invalid here:
/// the text entries take plain pixel counts.
fn parse_dimension(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Donor size policy: integral widths of 320-8192, heights of 200-8192.
fn valid_size(width: f64, height: f64) -> bool {
    width.fract() == 0.0
        && height.fract() == 0.0
        && (320.0..=8192.0).contains(&width)
        && (200.0..=8192.0).contains(&height)
}

/// Parse a resolution choice id back into pixels.
fn parse_resolution(id: &str) -> Option<(u32, u32)> {
    let (width, height) = id.split_once('x')?;
    Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
}

/// Bind brightness (plus reset), fullscreen, resolution, custom size, and
/// vsync rows. Display controls use window pixels and the same output gamma
/// on both renderers.
///
/// The donor tracks a window getter whose identity can change and refreshes
/// the custom-size drafts on change; here the shared window handle is stable,
/// so drafts initialize once and update on every resize through these rows.
#[must_use]
pub fn bind_native_video_settings(
    window: Rc<RefCell<dyn WindowView>>,
    registry: Option<Rc<dyn SettingCvars>>,
    report: Rc<dyn Fn(&str)>,
) -> Vec<SettingBinding> {
    let mut bindings = Vec::new();
    if let Some(registry) = registry.clone() {
        let read_gamma = Rc::clone(&registry);
        let write_gamma = Rc::clone(&registry);
        let reset_gamma = Rc::clone(&registry);
        let reset_check = Rc::clone(&registry);
        bindings.push(SettingBinding {
            id: control_id("ui:video:brightness"),
            label: "Brightness".to_string(),
            category: SettingCategory::Display,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Slider {
                read: Rc::new(move || read_gamma.variable_value("r_gamma")),
                write: Rc::new(move |value| write_gamma.set("r_gamma", &value.to_string())),
                minimum: 0.5,
                maximum: 3.0,
                step: 0.05,
                format_value: None,
            },
        });
        bindings.push(SettingBinding {
            id: control_id("ui:video:brightness-reset"),
            label: "Reset brightness".to_string(),
            category: SettingCategory::Display,
            enabled: Rc::new(move || reset_check.variable_value("r_gamma") != 1.0),
            kind: SettingBindingKind::Button {
                activate: Rc::new(move || reset_gamma.set("r_gamma", "1")),
            },
        });
    }
    let fullscreen_read = Rc::clone(&window);
    let fullscreen_write = Rc::clone(&window);
    let fullscreen_report = Rc::clone(&report);
    bindings.push(SettingBinding {
        id: control_id("ui:video:fullscreen"),
        label: "Borderless fullscreen".to_string(),
        category: SettingCategory::Display,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Toggle {
            read: Rc::new(move || fullscreen_read.borrow().fullscreen()),
            write: Rc::new(move |value| {
                if let Err(error) = fullscreen_write.borrow_mut().set_fullscreen(value) {
                    fullscreen_report(&error);
                }
            }),
        },
    });
    let (width, height) = window.borrow().logical_size();
    let draft: Rc<RefCell<(String, String)>> = Rc::new(RefCell::new((width.to_string(), height.to_string())));
    let resize_window = Rc::clone(&window);
    let resize_draft = Rc::clone(&draft);
    let resize: Rc<dyn Fn(f64, f64) -> Result<(), String>> = Rc::new(move |width, height| {
        if !valid_size(width, height) {
            return Err("Use a width of 320-8192 and a height of 200-8192 pixels.".to_string());
        }
        resize_window.borrow_mut().set_size(width as u32, height as u32)?;
        *resize_draft.borrow_mut() = (width.to_string(), height.to_string());
        Ok(())
    });
    let resolution_read = Rc::clone(&window);
    let resolution_gate = Rc::clone(&window);
    let resolution_window = Rc::clone(&window);
    let resolution_choices_window = Rc::clone(&window);
    let resolution_resize = Rc::clone(&resize);
    let resolution_report = Rc::clone(&report);
    bindings.push(SettingBinding {
        id: control_id("ui:video:resolution"),
        label: "Window resolution".to_string(),
        category: SettingCategory::Display,
        enabled: Rc::new(move || !resolution_gate.borrow().fullscreen()),
        kind: SettingBindingKind::Choice {
            read: Rc::new(move || resolution_id(resolution_read.borrow().logical_size())),
            choices: Rc::new(move || resolution_choices(&resolution_choices_window)),
            write: Rc::new(move |value| {
                let selected = resolution_choices(&resolution_window)
                    .into_iter()
                    .find(|choice| choice.id == value);
                let Some(selected) = selected else {
                    resolution_report("Unavailable resolution");
                    return;
                };
                let Some((width, height)) = parse_resolution(&selected.id) else {
                    resolution_report("Invalid resolution");
                    return;
                };
                #[allow(clippy::cast_precision_loss)]
                let result = resolution_resize(f64::from(width), f64::from(height));
                if let Err(error) = result {
                    resolution_report(&error);
                }
            }),
        },
    });
    for (id, label, is_width) in [
        ("ui:video:custom-width", "Custom width (320-8192)", true),
        ("ui:video:custom-height", "Custom height (200-8192)", false),
    ] {
        let gate = Rc::clone(&window);
        let read_draft = Rc::clone(&draft);
        let write_draft = Rc::clone(&draft);
        bindings.push(SettingBinding {
            id: control_id(id),
            label: label.to_string(),
            category: SettingCategory::Display,
            enabled: Rc::new(move || !gate.borrow().fullscreen()),
            kind: SettingBindingKind::TextEntry {
                read: Rc::new(move || {
                    let current = read_draft.borrow();
                    if is_width {
                        current.0.clone()
                    } else {
                        current.1.clone()
                    }
                }),
                write: Rc::new(move |value| {
                    let mut current = write_draft.borrow_mut();
                    if is_width {
                        current.0 = value.to_string();
                    } else {
                        current.1 = value.to_string();
                    }
                }),
                maximum_length: 4,
                commit: None,
            },
        });
    }
    let apply_gate = Rc::clone(&window);
    let apply_check = Rc::clone(&draft);
    let apply_draft = Rc::clone(&draft);
    let apply_resize = Rc::clone(&resize);
    let apply_report = Rc::clone(&report);
    bindings.push(SettingBinding {
        id: control_id("ui:video:custom-apply"),
        label: "Apply custom window size".to_string(),
        category: SettingCategory::Display,
        enabled: Rc::new(move || {
            if apply_gate.borrow().fullscreen() {
                return false;
            }
            let current = apply_check.borrow();
            match (parse_dimension(&current.0), parse_dimension(&current.1)) {
                (Some(width), Some(height)) => valid_size(width, height),
                _ => false,
            }
        }),
        kind: SettingBindingKind::Button {
            activate: Rc::new(move || {
                let (width_text, height_text) = {
                    let current = apply_draft.borrow();
                    (current.0.clone(), current.1.clone())
                };
                let result = match (parse_dimension(&width_text), parse_dimension(&height_text)) {
                    (Some(width), Some(height)) => apply_resize(width, height),
                    _ => Err("Use a width of 320-8192 and a height of 200-8192 pixels.".to_string()),
                };
                if let Err(error) = result {
                    apply_report(&error);
                }
            }),
        },
    });
    let vsync_gate_window = Rc::clone(&window);
    let vsync_gate_registry = registry.clone();
    let vsync_read_window = Rc::clone(&window);
    let vsync_read_registry = registry.clone();
    let vsync_write_window = Rc::clone(&window);
    bindings.push(SettingBinding {
        id: control_id("ui:video:vsync"),
        label: "Vertical sync".to_string(),
        category: SettingCategory::Display,
        enabled: Rc::new(move || vsync_gate_window.borrow().is_gl() && vsync_gate_registry.is_some()),
        kind: SettingBindingKind::Toggle {
            read: Rc::new(move || {
                vsync_read_window.borrow().is_gl()
                    && vsync_read_registry
                        .as_ref()
                        .is_some_and(|registry| registry.variable_value("r_swapInterval") != 0.0)
            }),
            write: Rc::new(move |value| {
                if vsync_write_window.borrow().is_gl() {
                    if let Some(registry) = registry.as_ref() {
                        registry.set("r_swapInterval", if value { "1" } else { "0" });
                    }
                }
            }),
        },
    });
    bindings
}

/// One installable language: choice row plus its tier loader.
#[derive(Clone)]
pub struct LanguageChoice {
    /// Choice id.
    pub id: String,
    /// Choice label.
    pub label: String,
    /// Load the primary and fallback tiers.
    pub load: Rc<dyn Fn() -> Result<(LocLoadTier, LocLoadTier), String>>,
}

/// Localization surface behind the language rows (donor
/// `LocalizationCatalog`).
pub trait LocalizationView {
    /// Load the primary tier, falling back to the fallback tier.
    fn load_ordered(&mut self, primary: LocLoadTier, fallback: LocLoadTier);
}

impl LocalizationView for LocalizationTable {
    /// Load through the real table with default reload options and no log,
    /// matching the donor default arguments.
    fn load_ordered(&mut self, primary: LocLoadTier, fallback: LocLoadTier) {
        let _ = LocalizationTable::load_ordered(self, &primary, &fallback, &LocReloadOptions::default(), None);
    }
}

/// Synchronous language selector (donor `NativeLanguageSettings`).
///
/// The donor `select` awaits the tier loader and guards re-entrant selection
/// with a loading flag; here `load` is synchronous, so the flag only brackets
/// the load and the row never observes it set. Selection failures route to
/// the `failed` sink from the row write, exactly like the donor catch.
pub struct NativeLanguageSettings {
    localization: Rc<RefCell<dyn LocalizationView>>,
    available: Vec<LanguageChoice>,
    selection: Rc<RefCell<String>>,
    loading: Rc<Cell<bool>>,
    failed: Rc<dyn Fn(String)>,
}

impl NativeLanguageSettings {
    /// Track the available choices with the current selection.
    pub fn new(
        localization: Rc<RefCell<dyn LocalizationView>>,
        available: Vec<LanguageChoice>,
        current: String,
        failed: Rc<dyn Fn(String)>,
    ) -> Self {
        Self {
            localization,
            available,
            selection: Rc::new(RefCell::new(current)),
            loading: Rc::new(Cell::new(false)),
            failed,
        }
    }

    /// Switch language, loading its tiers. Unknown ids and loader failures
    /// leave the selection unchanged.
    pub fn select(&mut self, id: &str) -> Result<(), String> {
        Self::select_inner(&self.localization, &self.available, &self.selection, &self.loading, id)
    }

    fn select_inner(
        localization: &Rc<RefCell<dyn LocalizationView>>,
        available: &[LanguageChoice],
        selection: &Rc<RefCell<String>>,
        loading: &Rc<Cell<bool>>,
        id: &str,
    ) -> Result<(), String> {
        let Some(choice) = available.iter().find(|choice| choice.id == id) else {
            return Err(format!("Language is not installed: {id}"));
        };
        if loading.get() {
            return Ok(());
        }
        loading.set(true);
        let result = (choice.load)().map(|(primary, fallback)| {
            localization.borrow_mut().load_ordered(primary, fallback);
            *selection.borrow_mut() = id.to_string();
        });
        loading.set(false);
        result
    }

    /// Bind the "Language" choice row; write failures route to the failed sink.
    #[must_use]
    pub fn binding(&self) -> SettingBinding {
        let available = self.available.clone();
        let listed = self.available.clone();
        let enabled_loading = Rc::clone(&self.loading);
        let read_selection = Rc::clone(&self.selection);
        let write_localization = Rc::clone(&self.localization);
        let write_selection = Rc::clone(&self.selection);
        let write_loading = Rc::clone(&self.loading);
        let write_failed = Rc::clone(&self.failed);
        SettingBinding {
            id: control_id("ui:language:selection"),
            label: "Language".to_string(),
            category: SettingCategory::Language,
            enabled: Rc::new(move || !enabled_loading.get()),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || read_selection.borrow().clone()),
                write: Rc::new(move |value| {
                    if let Err(error) =
                        Self::select_inner(&write_localization, &available, &write_selection, &write_loading, value)
                    {
                        write_failed(error);
                    }
                }),
                choices: Rc::new(move || {
                    listed
                        .iter()
                        .map(|choice| UiChoice {
                            id: choice.id.clone(),
                            label: choice.label.clone(),
                        })
                        .collect()
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::ui::settings::CvarView;
    use crate::ui::types::CommandDialect;

    struct FakeAudio {
        closed: bool,
        volume: f32,
    }

    impl AudioOutputView for FakeAudio {
        fn is_closed(&self) -> bool {
            self.closed
        }

        fn set_effects_volume(&mut self, volume: f32) {
            self.volume = volume;
        }
    }

    struct FakeCvars {
        values: RefCell<HashMap<String, String>>,
    }

    impl FakeCvars {
        fn with(pairs: &[(&str, &str)]) -> Self {
            Self {
                values: RefCell::new(
                    pairs
                        .iter()
                        .map(|(name, value)| (name.to_string(), value.to_string()))
                        .collect(),
                ),
            }
        }
    }

    impl SettingCvars for FakeCvars {
        fn dialect(&self) -> CommandDialect {
            CommandDialect::Q3
        }

        fn find(&self, name: &str) -> Option<CvarView> {
            self.values.borrow().get(name).map(|value| CvarView {
                value: value.clone(),
                latched_value: None,
                reset_value: value.clone(),
                flags: 0,
            })
        }

        fn set(&self, name: &str, value: &str) {
            self.values.borrow_mut().insert(name.to_string(), value.to_string());
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.values
                .borrow()
                .get(name)
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(0.0)
        }
    }

    struct FakeWindow {
        size: (u32, u32),
        modes: Vec<(u32, u32)>,
        fullscreen: bool,
        gl: bool,
        size_error: Option<String>,
        fullscreen_error: Option<String>,
    }

    impl FakeWindow {
        fn new(size: (u32, u32)) -> Self {
            Self {
                size,
                modes: Vec::new(),
                fullscreen: false,
                gl: false,
                size_error: None,
                fullscreen_error: None,
            }
        }
    }

    impl WindowView for FakeWindow {
        fn logical_size(&self) -> (u32, u32) {
            self.size
        }

        fn display_modes(&self) -> Vec<(u32, u32)> {
            self.modes.clone()
        }

        fn fullscreen(&self) -> bool {
            self.fullscreen
        }

        fn is_gl(&self) -> bool {
            self.gl
        }

        fn set_size(&mut self, width: u32, height: u32) -> Result<(), String> {
            if let Some(error) = self.size_error.clone() {
                return Err(error);
            }
            self.size = (width, height);
            Ok(())
        }

        fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), String> {
            if let Some(error) = self.fullscreen_error.clone() {
                return Err(error);
            }
            self.fullscreen = fullscreen;
            Ok(())
        }
    }

    struct FakeLocalization {
        loads: Vec<(LocLoadTier, LocLoadTier)>,
    }

    impl LocalizationView for FakeLocalization {
        fn load_ordered(&mut self, primary: LocLoadTier, fallback: LocLoadTier) {
            self.loads.push((primary, fallback));
        }
    }

    #[allow(clippy::type_complexity)]
    fn reports() -> (Rc<dyn Fn(&str)>, Rc<RefCell<Vec<String>>>) {
        let sink: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let push = Rc::clone(&sink);
        (
            Rc::new(move |message: &str| push.borrow_mut().push(message.to_string())),
            sink,
        )
    }

    fn binding<'b>(bindings: &'b [SettingBinding], id: &str) -> &'b SettingBinding {
        bindings
            .iter()
            .find(|binding| binding.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing binding {id}"))
    }

    #[test]
    fn effects_volume_disables_while_closed_and_writes_through() {
        let audio = Rc::new(RefCell::new(FakeAudio {
            closed: true,
            volume: 0.5,
        }));
        let binding = bind_effects_volume(Rc::clone(&audio) as Rc<RefCell<dyn AudioOutputView>>, Rc::new(|| 0.5));
        assert_eq!(binding.id.as_str(), "ui:audio:effects");
        assert_eq!(binding.category, SettingCategory::Audio);
        assert!(!(binding.enabled)());
        audio.borrow_mut().closed = false;
        assert!((binding.enabled)());
        let SettingBindingKind::Slider {
            read,
            write,
            minimum,
            maximum,
            step,
            ..
        } = &binding.kind
        else {
            panic!("effects row is a slider");
        };
        assert_eq!((*minimum, *maximum, *step), (0.0, 1.0, 0.05));
        assert_eq!(read(), 0.5);
        write(0.75);
        assert_eq!(audio.borrow().volume, 0.75);
    }

    #[test]
    fn renderer_draft_apply_and_refresh() {
        let backend: Rc<RefCell<RendererBackend>> = Rc::new(RefCell::new(RendererBackend::Cpu));
        let applied: Rc<RefCell<Vec<RendererBackend>>> = Rc::new(RefCell::new(Vec::new()));
        let read_backend = Rc::clone(&backend);
        let write_backend = Rc::clone(&applied);
        let (report, messages) = reports();
        let bindings = bind_renderer_settings(RendererOptions {
            current: Rc::new(move || *read_backend.borrow()),
            worker: None,
            apply: Rc::new(move |next| write_backend.borrow_mut().push(next)),
            report,
            enabled: Rc::new(|| true),
        });
        assert_eq!(bindings.len(), 2);
        let choice = binding(&bindings, "ui:video:renderer");
        let apply = binding(&bindings, "ui:video:renderer-apply");
        let SettingBindingKind::Choice { read, write, choices } = &choice.kind else {
            panic!("renderer row is a choice");
        };
        assert_eq!(read(), "cpu");
        assert_eq!(choices().len(), 2);
        assert!(!(apply.enabled)());
        write("gl");
        assert_eq!(read(), "gl");
        assert!((apply.enabled)());
        let SettingBindingKind::Button { activate } = &apply.kind else {
            panic!("apply row is a button");
        };
        activate();
        assert_eq!(*applied.borrow(), vec![RendererBackend::Gl]);
        assert_eq!(*messages.borrow(), vec!["Renderer change queued: OpenGL.".to_string()]);
        *backend.borrow_mut() = RendererBackend::Gl;
        assert_eq!(read(), "gl");
        assert!(!(apply.enabled)());
    }

    #[test]
    #[should_panic(expected = "Unknown renderer")]
    fn renderer_rejects_unknown_backend() {
        let bindings = bind_renderer_settings(RendererOptions {
            current: Rc::new(|| RendererBackend::Cpu),
            worker: None,
            apply: Rc::new(|_| {}),
            report: Rc::new(|_| {}),
            enabled: Rc::new(|| true),
        });
        let SettingBindingKind::Choice { write, .. } = &binding(&bindings, "ui:video:renderer").kind else {
            panic!("renderer row is a choice");
        };
        write("vulkan");
    }

    #[test]
    fn renderer_worker_toggle_queues_current_backend() {
        let flag: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let read_flag = Rc::clone(&flag);
        let write_flag = Rc::clone(&flag);
        let applied: Rc<RefCell<Vec<RendererBackend>>> = Rc::new(RefCell::new(Vec::new()));
        let write_applied = Rc::clone(&applied);
        let gate: Rc<Cell<bool>> = Rc::new(Cell::new(true));
        let read_gate = Rc::clone(&gate);
        let (report, messages) = reports();
        let bindings = bind_renderer_settings(RendererOptions {
            current: Rc::new(|| RendererBackend::Gl),
            worker: Some(WorkerToggle {
                read: Rc::new(move || read_flag.get()),
                write: Rc::new(move |value| write_flag.set(value)),
            }),
            apply: Rc::new(move |next| write_applied.borrow_mut().push(next)),
            report,
            enabled: Rc::new(move || read_gate.get()),
        });
        let worker = binding(&bindings, "ui:video:render-worker");
        let SettingBindingKind::Toggle { read, write } = &worker.kind else {
            panic!("worker row is a toggle");
        };
        assert!(!(read()));
        write(true);
        assert!(flag.get());
        assert_eq!(*applied.borrow(), vec![RendererBackend::Gl]);
        assert_eq!(*messages.borrow(), vec!["Render worker change queued.".to_string()]);
        gate.set(false);
        assert!(!(worker.enabled)());
        write(false);
        assert!(flag.get());
        assert_eq!(applied.borrow().len(), 1);
    }

    #[test]
    fn video_resolution_table_sorts_and_dedupes() {
        let window = Rc::new(RefCell::new(FakeWindow {
            modes: vec![(1920, 1080), (800, 600), (1920, 1080)],
            ..FakeWindow::new((1366, 768))
        }));
        let (report, _) = reports();
        let bindings = bind_native_video_settings(window, None, report);
        let resolution = binding(&bindings, "ui:video:resolution");
        let SettingBindingKind::Choice { read, choices, .. } = &resolution.kind else {
            panic!("resolution row is a choice");
        };
        assert_eq!(read(), "1366x768");
        let ids: Vec<String> = choices().into_iter().map(|choice| choice.id).collect();
        assert!(ids.contains(&"1366x768".to_string()));
        assert!(ids.contains(&"640x480".to_string()));
        assert!(ids.contains(&"3840x2160".to_string()));
        let mut sorted = ids.clone();
        sorted.sort_by_key(|id| {
            let (width, height) = id.split_once('x').unwrap();
            (width.parse::<u32>().unwrap(), height.parse::<u32>().unwrap())
        });
        assert_eq!(ids, sorted);
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn video_resolution_write_resizes_and_reports_unknown() {
        let window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let (report, messages) = reports();
        let bindings = bind_native_video_settings(Rc::clone(&window) as Rc<RefCell<dyn WindowView>>, None, report);
        let resolution = binding(&bindings, "ui:video:resolution");
        let SettingBindingKind::Choice { write, .. } = &resolution.kind else {
            panic!("resolution row is a choice");
        };
        write("1920x1080");
        assert_eq!(window.borrow().size, (1920, 1080));
        assert!(messages.borrow().is_empty());
        write("9999x9999");
        assert_eq!(*messages.borrow(), vec!["Unavailable resolution".to_string()]);
    }

    #[test]
    fn video_custom_size_validates_before_apply() {
        let window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let (report, messages) = reports();
        let bindings = bind_native_video_settings(Rc::clone(&window) as Rc<RefCell<dyn WindowView>>, None, report);
        let width = binding(&bindings, "ui:video:custom-width");
        let height = binding(&bindings, "ui:video:custom-height");
        let apply = binding(&bindings, "ui:video:custom-apply");
        let SettingBindingKind::TextEntry {
            read: read_width,
            write: write_width,
            maximum_length,
            ..
        } = &width.kind
        else {
            panic!("custom width is a text entry");
        };
        let SettingBindingKind::TextEntry {
            write: write_height, ..
        } = &height.kind
        else {
            panic!("custom height is a text entry");
        };
        let SettingBindingKind::Button { activate } = &apply.kind else {
            panic!("custom apply is a button");
        };
        assert_eq!(*maximum_length, 4);
        assert_eq!(read_width(), "800");
        assert!((apply.enabled)());
        write_width("100");
        assert!(!(apply.enabled)());
        activate();
        assert_eq!(
            *messages.borrow(),
            vec!["Use a width of 320-8192 and a height of 200-8192 pixels.".to_string()]
        );
        assert_eq!(window.borrow().size, (800, 600));
        write_width("1280");
        write_height("720px");
        assert!(!(apply.enabled)());
        write_height("720");
        assert!((apply.enabled)());
        activate();
        assert_eq!(window.borrow().size, (1280, 720));
        assert_eq!(read_width(), "1280");
    }

    #[test]
    fn video_brightness_reads_writes_and_resets() {
        let window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let registry = Rc::new(FakeCvars::with(&[("r_gamma", "1")]));
        let (report, _) = reports();
        let bindings = bind_native_video_settings(window, Some(Rc::clone(&registry) as Rc<dyn SettingCvars>), report);
        let brightness = binding(&bindings, "ui:video:brightness");
        let reset = binding(&bindings, "ui:video:brightness-reset");
        assert!(!(reset.enabled)());
        let SettingBindingKind::Slider {
            read,
            write,
            minimum,
            maximum,
            step,
            ..
        } = &brightness.kind
        else {
            panic!("brightness is a slider");
        };
        assert_eq!((*minimum, *maximum, *step), (0.5, 3.0, 0.05));
        assert_eq!(read(), 1.0);
        write(1.5);
        assert_eq!(registry.variable_value("r_gamma"), 1.5);
        assert!((reset.enabled)());
        let SettingBindingKind::Button { activate } = &reset.kind else {
            panic!("brightness reset is a button");
        };
        activate();
        assert_eq!(registry.variable_value("r_gamma"), 1.0);

        let bare_window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let (bare_report, _) = reports();
        let bare = bind_native_video_settings(bare_window, None, bare_report);
        assert!(bare.iter().all(|binding| !binding.id.as_str().contains("brightness")));
    }

    #[test]
    fn video_fullscreen_forwards_and_reports_failures() {
        let window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let (report, messages) = reports();
        let bindings = bind_native_video_settings(Rc::clone(&window) as Rc<RefCell<dyn WindowView>>, None, report);
        let fullscreen = binding(&bindings, "ui:video:fullscreen");
        let SettingBindingKind::Toggle { read, write } = &fullscreen.kind else {
            panic!("fullscreen is a toggle");
        };
        assert!(!(read()));
        write(true);
        assert!(window.borrow().fullscreen);
        assert!(!(binding(&bindings, "ui:video:resolution").enabled)());
        window.borrow_mut().fullscreen_error = Some("no fullscreen".to_string());
        write(false);
        assert_eq!(*messages.borrow(), vec!["no fullscreen".to_string()]);
    }

    #[test]
    fn video_vsync_gates_on_gl_and_registry() {
        let window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let registry = Rc::new(FakeCvars::with(&[("r_swapInterval", "0")]));
        let (report, _) = reports();
        let bindings = bind_native_video_settings(
            Rc::clone(&window) as Rc<RefCell<dyn WindowView>>,
            Some(Rc::clone(&registry) as Rc<dyn SettingCvars>),
            report,
        );
        let vsync = binding(&bindings, "ui:video:vsync");
        assert!(!(vsync.enabled)());
        window.borrow_mut().gl = true;
        assert!((vsync.enabled)());
        let SettingBindingKind::Toggle { read, write } = &vsync.kind else {
            panic!("vsync is a toggle");
        };
        assert!(!(read()));
        write(true);
        assert_eq!(registry.variable_value("r_swapInterval"), 1.0);
        assert!(read());

        let cpu_window = Rc::new(RefCell::new(FakeWindow::new((800, 600))));
        let (cpu_report, _) = reports();
        let bare = bind_native_video_settings(cpu_window, None, cpu_report);
        assert!(!(binding(&bare, "ui:video:vsync").enabled)());
    }

    fn tier(base: &[u8]) -> LocLoadTier {
        LocLoadTier {
            base: Some(base.to_vec()),
            mods: Vec::new(),
        }
    }

    #[allow(clippy::type_complexity)]
    fn language_harness() -> (
        NativeLanguageSettings,
        Rc<RefCell<FakeLocalization>>,
        Rc<RefCell<Vec<String>>>,
    ) {
        let localization = Rc::new(RefCell::new(FakeLocalization { loads: Vec::new() }));
        let failures: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let report_failures = Rc::clone(&failures);
        let settings = NativeLanguageSettings::new(
            Rc::clone(&localization) as Rc<RefCell<dyn LocalizationView>>,
            vec![
                LanguageChoice {
                    id: "english".to_string(),
                    label: "English".to_string(),
                    load: Rc::new(|| Ok((tier(b"en"), tier(b"base")))),
                },
                LanguageChoice {
                    id: "broken".to_string(),
                    label: "Broken".to_string(),
                    load: Rc::new(|| Err("disk is gone".to_string())),
                },
            ],
            "english".to_string(),
            Rc::new(move |error: String| report_failures.borrow_mut().push(error)),
        );
        (settings, localization, failures)
    }

    #[test]
    fn language_select_loads_tiers_and_updates_selection() {
        let (mut settings, localization, _) = language_harness();
        assert!(settings.select("english").is_ok());
        assert_eq!(localization.borrow().loads.len(), 1);
        let (primary, fallback) = &localization.borrow().loads[0];
        assert_eq!(primary.base, Some(b"en".to_vec()));
        assert_eq!(fallback.base, Some(b"base".to_vec()));
        let binding = settings.binding();
        let SettingBindingKind::Choice { read, choices, .. } = &binding.kind else {
            panic!("language row is a choice");
        };
        assert_eq!(read(), "english");
        assert_eq!(choices().len(), 2);
        assert!((binding.enabled)());
    }

    #[test]
    fn language_failures_keep_selection_and_route_to_sink() {
        let (mut settings, localization, failures) = language_harness();
        let missing = settings.select("klingon");
        assert_eq!(missing, Err("Language is not installed: klingon".to_string()));
        let broken = settings.select("broken");
        assert_eq!(broken, Err("disk is gone".to_string()));
        assert!(localization.borrow().loads.is_empty());
        assert!(failures.borrow().is_empty());

        let binding = settings.binding();
        let SettingBindingKind::Choice { read, write, .. } = &binding.kind else {
            panic!("language row is a choice");
        };
        write("broken");
        assert_eq!(read(), "english");
        assert_eq!(*failures.borrow(), vec!["disk is gone".to_string()]);
        write("english");
        assert_eq!(read(), "english");
        assert_eq!(localization.borrow().loads.len(), 1);
    }

    #[test]
    fn localization_table_adapter_loads_with_defaults() {
        let mut table = LocalizationTable::default();
        let view: &mut dyn LocalizationView = &mut table;
        view.load_ordered(tier(b"base"), tier(b"fallback"));
        assert_eq!(table.size(), 0);
    }
}
