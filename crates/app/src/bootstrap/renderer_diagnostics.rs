//! Renderer diagnostic source commands.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/renderer-diagnostics.ts`
//! (`registerRendererDiagnostics`, `RendererDiagnosticServices`). The command buffer
//! ([`CommandBuffer`](qa_core::cmd_buffer::CommandBuffer)), invocation
//! ([`Invocation`](qa_core::cmd_buffer::Invocation)), scene-material
//! ([`RegisteredSceneMaterial`](qa_client::render::scene::material_registrations::RegisteredSceneMaterial)),
//! renderer-resource ([`Q3RendererResources`](qa_content::q3::presentation::resources::Q3RendererResources)),
//! and renderer diagnostics ([`RendererDiagnostics`](super::renderer::RendererDiagnostics))
//! arrive as local traits and snapshot rows; every report format and error text matches the
//! donor exactly. Source commands query the active owners at dispatch; snapshots never execute
//! resource queues.

use std::rc::Rc;

/// One diagnostic command body: borrowed services plus the invocation.
type DiagnosticRun<Services> =
    Box<dyn Fn(&Services, DiagnosticInvocation<<Services as RendererDiagnosticServices>::Source>)>;

/// One resident renderer image row.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticImage {
    /// Ordinal.
    pub ordinal: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Encoding name.
    pub encoding: String,
    /// Mip level count.
    pub mip_levels: u32,
    /// Image name.
    pub name: String,
}

/// One registered shader row (finished pass counts plus material name).
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticShader {
    /// Unfogged pass count.
    pub num_unfogged_passes: u32,
    /// Lightmap index.
    pub lightmap_index: i32,
    /// Iterator kind name.
    pub iterator_kind: String,
    /// Sort key.
    pub sort: i32,
    /// Material name.
    pub name: String,
}

/// One registered model row (donor pre-resolves the nested model kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticModel {
    /// Handle.
    pub handle: u32,
    /// Resolved kind text.
    pub kind: String,
    /// Registered path.
    pub path: String,
}

/// One registered skin row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticSkin {
    /// Handle.
    pub handle: u32,
    /// Registered path.
    pub path: String,
    /// Surface name to shader pairs.
    pub surfaces: Vec<(String, String)>,
}

/// One display mode row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticDisplayMode {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Color depth in bits.
    pub color_bits: u32,
    /// Refresh rate in Hz.
    pub refresh_rate: u32,
}

/// GL driver identity (donor `driver`, null for the software renderer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticDriver {
    /// Vendor string.
    pub vendor: String,
    /// Renderer string.
    pub renderer: String,
    /// Version string.
    pub version: String,
    /// Shading-language version string.
    pub shading_language: String,
}

/// Renderer snapshot backing `imagelist`, `modelist`, and `gfxinfo`.
#[derive(Debug, Clone, PartialEq)]
pub struct RendererDiagnosticsSnapshot {
    /// Resident images.
    pub images: Vec<DiagnosticImage>,
    /// Display modes.
    pub display_modes: Vec<DiagnosticDisplayMode>,
    /// Backend name.
    pub backend: String,
    /// Drawable width.
    pub width: u32,
    /// Drawable height.
    pub height: u32,
    /// Driver identity, or none for the software renderer.
    pub driver: Option<DiagnosticDriver>,
}

/// Source-client model and skin registries backing `modellist` and `skinlist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceModelResources {
    /// Registered models.
    pub models: Vec<DiagnosticModel>,
    /// Registered skins.
    pub skins: Vec<DiagnosticSkin>,
}

/// Live owners queried at dispatch (donor `RendererDiagnosticServices`).
pub trait RendererDiagnosticServices {
    /// Command source identity.
    type Source: Clone;
    /// Current renderer snapshot.
    fn renderer(&self) -> RendererDiagnosticsSnapshot;
    /// Registered shaders, in source sorted order when requested.
    fn shaders(&self, sorted: bool) -> Vec<DiagnosticShader>;
    /// Invoking source-client resources, or none without an active registry.
    fn resources(&self, source: &Self::Source) -> Option<SourceModelResources>;
    /// Print report text to a source.
    fn print(&self, text: &str, source: &Self::Source);
}

/// One diagnostic command invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticInvocation<Source> {
    /// Command arguments (donor `invocation.args`).
    pub args: Vec<String>,
    /// Invoking source.
    pub source: Source,
}

/// Absorbed engine command buffer surface.
pub trait DiagnosticCommands {
    /// Command source identity.
    type Source: Clone;
    /// Whether a command name is already registered.
    fn exists(&self, name: &str) -> bool;
    /// Register an engine command; false means registration failed.
    fn register_engine(
        &mut self,
        name: &str,
        summary: &str,
        usage: &str,
        handler: Box<dyn Fn(DiagnosticInvocation<Self::Source>)>,
    ) -> bool;
    /// Remove a command registration.
    fn unregister(&mut self, name: &str);
}

/// Diagnostic registration failure (donor `Error` texts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRegistrationError(pub String);

impl std::fmt::Display for DiagnosticRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DiagnosticRegistrationError {}

/// Registered diagnostic commands; [`RendererDiagnosticRegistration::unregister_all`] is the
/// donor's returned disposer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RendererDiagnosticRegistration {
    names: Vec<String>,
}

impl RendererDiagnosticRegistration {
    /// Remove every registered diagnostic command.
    pub fn unregister_all<Commands: DiagnosticCommands>(&self, commands: &mut Commands) {
        for name in &self.names {
            commands.unregister(name);
        }
    }

    /// Registered command names in donor order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// Register the six renderer diagnostic commands.
pub fn register_renderer_diagnostics<Services, Commands>(
    services: Rc<Services>,
    commands: &mut Commands,
) -> Result<RendererDiagnosticRegistration, DiagnosticRegistrationError>
where
    Services: RendererDiagnosticServices + 'static,
    Commands: DiagnosticCommands<Source = Services::Source>,
{
    let mut names: Vec<String> = Vec::new();
    let mut add = |commands: &mut Commands,
                   name: &str,
                   summary: &str,
                   usage: &str,
                   run: DiagnosticRun<Services>|
     -> Result<(), DiagnosticRegistrationError> {
        if commands.exists(name) {
            return Err(DiagnosticRegistrationError(format!(
                "Renderer diagnostic command already registered: {name}"
            )));
        }
        let services = Rc::clone(&services);
        let handler: Box<dyn Fn(DiagnosticInvocation<Services::Source>)> =
            Box::new(move |invocation| run(&services, invocation));
        if !commands.register_engine(name, handler_summary(summary), handler_usage(usage), handler) {
            return Err(DiagnosticRegistrationError(format!(
                "Cannot register renderer diagnostic {name}"
            )));
        }
        names.push(name.to_string());
        Ok(())
    };
    add(
        commands,
        "imagelist",
        "List actual resident renderer images.",
        "imagelist",
        Box::new(|services, invocation| {
            services.print(&format_image_list(&services.renderer().images), &invocation.source);
        }),
    )?;
    add(
        commands,
        "shaderlist",
        "List registered shaders, optionally in source sorted order.",
        "shaderlist [sorted]",
        Box::new(|services, invocation| {
            services.print(
                &format_shader_list(&services.shaders(!invocation.args.is_empty())),
                &invocation.source,
            );
        }),
    )?;
    add(
        commands,
        "modellist",
        "List model handles registered by the invoking source client.",
        "modellist",
        Box::new(|services, invocation| {
            let resources = services
                .resources(&invocation.source)
                .expect("No active source client model registry");
            services.print(&format_model_list(&resources.models), &invocation.source);
        }),
    )?;
    add(
        commands,
        "skinlist",
        "List source skin handles and their registered surface shaders.",
        "skinlist",
        Box::new(|services, invocation| {
            let resources = services
                .resources(&invocation.source)
                .expect("No active source client skin registry");
            services.print(&format_skin_list(&resources.skins), &invocation.source);
        }),
    )?;
    add(
        commands,
        "modelist",
        "List display modes returned by the active video device.",
        "modelist",
        Box::new(|services, invocation| {
            services.print(
                &format_mode_list(&services.renderer().display_modes),
                &invocation.source,
            );
        }),
    )?;
    add(
        commands,
        "gfxinfo",
        "Report the actual backend, drawable size and GL driver when present.",
        "gfxinfo",
        Box::new(|services, invocation| {
            services.print(&format_gfx_info(&services.renderer()), &invocation.source);
        }),
    )?;
    Ok(RendererDiagnosticRegistration { names })
}

fn handler_summary(summary: &str) -> &str {
    summary
}

fn handler_usage(usage: &str) -> &str {
    usage
}

/// Format the `imagelist` report.
#[must_use]
pub fn format_image_list(images: &[DiagnosticImage]) -> String {
    let mut out = String::from("ordinal width height encoding mipLevels name\n");
    for image in images {
        out.push_str(&format!(
            "{} {} {} {} {} {}\n",
            image.ordinal, image.width, image.height, image.encoding, image.mip_levels, image.name
        ));
    }
    out.push_str(&format!("{} resident images\n", images.len()));
    out
}

/// Format the `shaderlist` report.
#[must_use]
pub fn format_shader_list(shaders: &[DiagnosticShader]) -> String {
    let mut out = String::from("passes lightmap iterator sort name\n");
    for shader in shaders {
        out.push_str(&format!(
            "{} {} {} {} {}\n",
            shader.num_unfogged_passes, shader.lightmap_index, shader.iterator_kind, shader.sort, shader.name
        ));
    }
    out.push_str(&format!("{} registered shaders\n", shaders.len()));
    out
}

/// Format the `modellist` report.
#[must_use]
pub fn format_model_list(models: &[DiagnosticModel]) -> String {
    let mut out = String::from("handle kind name\n");
    for model in models {
        out.push_str(&format!("{} {} {}\n", model.handle, model.kind, model.path));
    }
    out.push_str(&format!("{} registered models\n", models.len()));
    out
}

/// Format the `skinlist` report.
#[must_use]
pub fn format_skin_list(skins: &[DiagnosticSkin]) -> String {
    let mut out = String::new();
    for skin in skins {
        out.push_str(&format!("{} {}\n", skin.handle, skin.path));
        for (name, shader) in &skin.surfaces {
            out.push_str(&format!("  {name} = {shader}\n"));
        }
    }
    out.push_str(&format!("{} registered skins\n", skins.len()));
    out
}

/// Format the `modelist` report.
#[must_use]
pub fn format_mode_list(modes: &[DiagnosticDisplayMode]) -> String {
    let mut out = String::new();
    for (index, mode) in modes.iter().enumerate() {
        out.push_str(&format!(
            "{index}: {}x{} {} bit {} Hz\n",
            mode.width, mode.height, mode.color_bits, mode.refresh_rate
        ));
    }
    out.push_str(&format!("{} display modes\n", modes.len()));
    out
}

/// Format the `gfxinfo` report.
#[must_use]
pub fn format_gfx_info(info: &RendererDiagnosticsSnapshot) -> String {
    let mut out = format!("backend: {}\ndrawable: {}x{}\n", info.backend, info.width, info.height);
    match &info.driver {
        None => out.push_str("driver: software renderer\n"),
        Some(driver) => out.push_str(&format!(
            "vendor: {}\nrenderer: {}\nversion: {}\nshadingLanguage: {}\n",
            driver.vendor, driver.renderer, driver.version, driver.shading_language
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    /// Test command handler.
    type TestHandler = Box<dyn Fn(DiagnosticInvocation<String>)>;

    #[derive(Default)]
    struct TestCommands {
        handlers: HashMap<String, TestHandler>,
        summaries: HashMap<String, String>,
    }

    impl DiagnosticCommands for TestCommands {
        type Source = String;

        fn exists(&self, name: &str) -> bool {
            self.handlers.contains_key(name)
        }

        fn register_engine(
            &mut self,
            name: &str,
            summary: &str,
            usage: &str,
            handler: Box<dyn Fn(DiagnosticInvocation<Self::Source>)>,
        ) -> bool {
            assert!(!usage.is_empty());
            self.summaries.insert(name.to_string(), summary.to_string());
            self.handlers.insert(name.to_string(), handler).is_none()
        }

        fn unregister(&mut self, name: &str) {
            self.handlers.remove(name);
        }
    }

    struct TestServices {
        printed: RefCell<Vec<String>>,
        resources: Option<SourceModelResources>,
    }

    impl RendererDiagnosticServices for TestServices {
        type Source = String;

        fn renderer(&self) -> RendererDiagnosticsSnapshot {
            RendererDiagnosticsSnapshot {
                images: vec![DiagnosticImage {
                    ordinal: 0,
                    width: 64,
                    height: 64,
                    encoding: "rgba".to_string(),
                    mip_levels: 7,
                    name: "logo".to_string(),
                }],
                display_modes: vec![DiagnosticDisplayMode {
                    width: 800,
                    height: 600,
                    color_bits: 32,
                    refresh_rate: 60,
                }],
                backend: "gl".to_string(),
                width: 800,
                height: 600,
                driver: None,
            }
        }

        fn shaders(&self, sorted: bool) -> Vec<DiagnosticShader> {
            assert!(sorted);
            vec![DiagnosticShader {
                num_unfogged_passes: 2,
                lightmap_index: 0,
                iterator_kind: "generic".to_string(),
                sort: 7,
                name: "base".to_string(),
            }]
        }

        fn resources(&self, _source: &Self::Source) -> Option<SourceModelResources> {
            self.resources.clone()
        }

        fn print(&self, text: &str, _source: &Self::Source) {
            self.printed.borrow_mut().push(text.to_string());
        }
    }

    fn invoke(commands: &TestCommands, name: &str, args: &[&str]) {
        let handler = commands.handlers.get(name).expect("registered");
        handler(DiagnosticInvocation {
            args: args.iter().map(ToString::to_string).collect(),
            source: "seat".to_string(),
        });
    }

    #[test]
    fn registers_six_commands_with_donor_summaries() {
        let services = Rc::new(TestServices {
            printed: RefCell::new(Vec::new()),
            resources: Some(SourceModelResources {
                models: Vec::new(),
                skins: Vec::new(),
            }),
        });
        let mut commands = TestCommands::default();
        let registration = register_renderer_diagnostics(services, &mut commands).unwrap();
        assert_eq!(registration.names().len(), 6);
        assert_eq!(
            commands.summaries["gfxinfo"],
            "Report the actual backend, drawable size and GL driver when present."
        );
        registration.unregister_all(&mut commands);
        assert!(commands.handlers.is_empty());
    }

    #[test]
    fn rejects_duplicate_command() {
        let services = Rc::new(TestServices {
            printed: RefCell::new(Vec::new()),
            resources: None,
        });
        let mut commands = TestCommands::default();
        commands.handlers.insert(
            "imagelist".to_string(),
            Box::new(|_| {}) as Box<dyn Fn(DiagnosticInvocation<String>)>,
        );
        let error = register_renderer_diagnostics(services, &mut commands).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Renderer diagnostic command already registered: imagelist"
        );
    }

    #[test]
    fn imagelist_reports_resident_images() {
        let images = vec![DiagnosticImage {
            ordinal: 3,
            width: 64,
            height: 32,
            encoding: "rgba".to_string(),
            mip_levels: 7,
            name: "logo".to_string(),
        }];
        assert_eq!(
            format_image_list(&images),
            "ordinal width height encoding mipLevels name\n3 64 32 rgba 7 logo\n1 resident images\n"
        );
        assert!(format_image_list(&[]).ends_with("0 resident images\n"));
    }

    #[test]
    fn shaderlist_uses_sorted_flag_and_counts() {
        let shaders = vec![DiagnosticShader {
            num_unfogged_passes: 2,
            lightmap_index: 0,
            iterator_kind: "generic".to_string(),
            sort: 7,
            name: "base".to_string(),
        }];
        assert_eq!(
            format_shader_list(&shaders),
            "passes lightmap iterator sort name\n2 0 generic 7 base\n1 registered shaders\n"
        );
        let services = Rc::new(TestServices {
            printed: RefCell::new(Vec::new()),
            resources: None,
        });
        let mut commands = TestCommands::default();
        register_renderer_diagnostics(Rc::clone(&services), &mut commands).unwrap();
        invoke(&commands, "shaderlist", &["sorted"]);
        assert!(services.printed.borrow()[0].contains("1 registered shaders"));
    }

    #[test]
    fn modellist_and_skinlist_format_rows() {
        let models = vec![DiagnosticModel {
            handle: 1,
            kind: "md3".to_string(),
            path: "models/a".to_string(),
        }];
        assert_eq!(
            format_model_list(&models),
            "handle kind name\n1 md3 models/a\n1 registered models\n"
        );
        let skins = vec![DiagnosticSkin {
            handle: 2,
            path: "skins/a".to_string(),
            surfaces: vec![("head".to_string(), "skin/head".to_string())],
        }];
        assert_eq!(
            format_skin_list(&skins),
            "2 skins/a\n  head = skin/head\n1 registered skins\n"
        );
    }

    #[test]
    fn modelist_and_gfxinfo_cover_driver_branches() {
        let modes = vec![DiagnosticDisplayMode {
            width: 800,
            height: 600,
            color_bits: 32,
            refresh_rate: 60,
        }];
        assert_eq!(format_mode_list(&modes), "0: 800x600 32 bit 60 Hz\n1 display modes\n");
        let info = RendererDiagnosticsSnapshot {
            images: Vec::new(),
            display_modes: Vec::new(),
            backend: "soft".to_string(),
            width: 320,
            height: 200,
            driver: None,
        };
        assert_eq!(
            format_gfx_info(&info),
            "backend: soft\ndrawable: 320x200\ndriver: software renderer\n"
        );
        let info = RendererDiagnosticsSnapshot {
            driver: Some(DiagnosticDriver {
                vendor: "v".to_string(),
                renderer: "r".to_string(),
                version: "1".to_string(),
                shading_language: "sl".to_string(),
            }),
            ..info
        };
        assert!(format_gfx_info(&info).contains("shadingLanguage: sl\n"));
    }
}
