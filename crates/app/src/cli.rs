//! Binary entry dispatch shared by `qa-muse` and `qa-dedicated`.
//!
//! Donor provenance: `src/main.ts` (command branches, dedicated vs
//! windowed assembly, quit propagation). Signal handling stays with the
//! host: the loop checks [`Application::is_finished`](crate::application::Application::is_finished)
//! between frames, and the default SIGINT/SIGTERM disposition terminates a
//! headless run. The weapon-behavior branch runs
//! [`run_weapon_behavior_tool`](crate::bootstrap::weapon_behavior_tool::run_weapon_behavior_tool)
//! with a CLI host: QuakeC snapshots convert the loaded guest program while
//! QVM/native profile validation reports its unported sibling service.

use std::io::Write;

use qa_client::render::NullRenderer;

use qa_content::catalog::{
    discover_installed_content, CatalogError, DiscoverContentOptions, InstalledCatalog, NativeWeaponBehaviorService,
    ProductAvailability, QcFunctionGlobal, QcWeaponFunction, QcWeaponOpcode, QcWeaponProgramSnapshot,
    QcWeaponStatement, QvmWeaponArtifactResolution, QvmWeaponBehaviorService,
};
use qa_content::contract::{
    ContentDigest, ContentId, ExecutableRecipe, ModuleIdentity, NativeWeaponBehaviorDeclaration, ProviderReference,
    QvmAbiProfile, ResolvedResourceReference, WeaponBehaviorDefinition,
};
use qa_content::mounts::MountedContent;
use qa_content::value::SaveJson;
use qa_guest::qc::mod_provider::{QcApiKind, QcFunctionView, QcProgramView, QcValueType as ModQcValueType};
use qa_guest::qc::program::{QcFunction, QcOpcode, QcProgram, QcValueType as ProgramQcValueType, QuakeCApi};
use qa_guest::qc::weapon_behavior_profile::qc_weapon_behavior_capability_error;

use crate::application::Application;
use crate::bootstrap::weapon_behavior_selection::{WeaponBehaviorHost, WeaponBehaviorRequest};
use crate::bootstrap::weapon_behavior_tool::run_weapon_behavior_tool;
use crate::bootstrap::weapon_behavior_tool_options::{ProjectileRole as ToolProjectileRole, WeaponBehaviorToolCommand};
use crate::error::AppError;
use crate::options::{
    parse_application_command, ApplicationCommand, ProjectileRole as OptionsProjectileRole,
    WeaponBehaviorTool as ParsedWeaponBehaviorTool, HELP, WEAPON_BEHAVIOR_HELP,
};
use crate::startup::StartupConfig;

/// Run the application command line, returning the process exit code.
pub fn run(argv: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write, version: &str) -> i32 {
    match run_inner(argv, stdout, version) {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(stderr, "qa-muse: {error}");
            1
        }
    }
}

fn run_inner(argv: &[String], stdout: &mut dyn Write, version: &str) -> Result<(), AppError> {
    match parse_application_command(argv)? {
        ApplicationCommand::Help => {
            let _ = stdout.write_all(HELP.as_bytes());
            Ok(())
        }
        ApplicationCommand::Version => {
            let _ = writeln!(stdout, "Quake Anthology {version}");
            Ok(())
        }
        ApplicationCommand::WeaponBehavior { command } => {
            if matches!(command, ParsedWeaponBehaviorTool::Help) {
                let _ = stdout.write_all(WEAPON_BEHAVIOR_HELP.as_bytes());
                return Ok(());
            }
            run_weapon_behavior(&command, stdout)?;
            Ok(())
        }
        ApplicationCommand::ListContent { corpus_root } => {
            list_content(&corpus_root, stdout);
            Ok(())
        }
        ApplicationCommand::Run { options } | ApplicationCommand::Menu { options } => {
            let config = StartupConfig::from_options(&options)?;
            let mut application = Application::open(&config, NullRenderer::new())?;
            let stats = application.run()?;
            let _ = writeln!(
                stdout,
                "Ran {} host frames, {} server ticks, {} entities ({} render frames)",
                stats.frames, stats.ticks, stats.entities, stats.render_frames
            );
            Ok(())
        }
    }
}

/// Run the weapon-behavior tool (donor `main.ts` weapon-behavior branch).
fn run_weapon_behavior(command: &ParsedWeaponBehaviorTool, stdout: &mut dyn Write) -> Result<(), AppError> {
    let command = tool_command(command);
    let mut host = CliWeaponBehaviorHost::default();
    let mut print = |text: String| {
        let _ = stdout.write_all(text.as_bytes());
    };
    run_weapon_behavior_tool(&command, &mut host, &mut print)?;
    Ok(())
}

/// Convert parsed CLI options into the tool command model.
fn tool_command(command: &ParsedWeaponBehaviorTool) -> WeaponBehaviorToolCommand {
    match command {
        ParsedWeaponBehaviorTool::Help => WeaponBehaviorToolCommand::Help,
        ParsedWeaponBehaviorTool::Inspect { content } => WeaponBehaviorToolCommand::Inspect {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
        },
        ParsedWeaponBehaviorTool::DeclareQvm { content, profile } => WeaponBehaviorToolCommand::DeclareQvm {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
            profile: profile.clone(),
        },
        ParsedWeaponBehaviorTool::DeclareNative { content, profile } => WeaponBehaviorToolCommand::DeclareNative {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
            profile: profile.clone(),
        },
        ParsedWeaponBehaviorTool::Declare {
            content,
            id,
            title,
            role,
            fire,
            activate,
        } => WeaponBehaviorToolCommand::Declare {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
            id: id.clone(),
            title: title.clone(),
            role: tool_role(*role),
            fire: fire.clone(),
            activate: activate.clone(),
        },
    }
}

/// Map a parsed projectile role onto the tool role.
fn tool_role(role: OptionsProjectileRole) -> ToolProjectileRole {
    match role {
        OptionsProjectileRole::Rocket => ToolProjectileRole::Rocket,
        OptionsProjectileRole::Grenade => ToolProjectileRole::Grenade,
        OptionsProjectileRole::Nail => ToolProjectileRole::Nail,
        OptionsProjectileRole::Bolt => ToolProjectileRole::Bolt,
        OptionsProjectileRole::Plasma => ToolProjectileRole::Plasma,
        OptionsProjectileRole::Energy => ToolProjectileRole::Energy,
        OptionsProjectileRole::Grapple => ToolProjectileRole::Grapple,
    }
}

/// CLI [`WeaponBehaviorHost`]. QuakeC snapshots convert the loaded guest
/// program; the selection-only operations and the QVM/native profile
/// services report their unported sibling lanes. Artifact and profile are
/// `()` because the CLI services never produce them.
#[derive(Default)]
struct CliWeaponBehaviorHost {
    qvm: CliQvmService,
    native: CliNativeService,
}

impl WeaponBehaviorHost<(), ()> for CliWeaponBehaviorHost {
    type Error = CatalogError;
    type QuakeCResources = ();
    type RereleaseGuest = ();

    fn qvm_service(&self) -> &dyn QvmWeaponBehaviorService<(), ()> {
        &self.qvm
    }

    fn native_service(&self) -> &dyn NativeWeaponBehaviorService {
        &self.native
    }

    fn content_mounts(&mut self, _content: &ContentId) -> Result<MountedContent, Self::Error> {
        Err(CatalogError::Invalid(
            "Weapon behavior content mounts need the unported forContent operation".to_string(),
        ))
    }

    fn apply_requests(
        &mut self,
        _catalog: &InstalledCatalog,
        _recipe: ExecutableRecipe,
        _requests: &[WeaponBehaviorRequest],
    ) -> Result<ExecutableRecipe, Self::Error> {
        Err(CatalogError::Invalid(
            "Weapon behavior recipe requests need the unported applyApplicationMods operation".to_string(),
        ))
    }

    fn weapon_snapshot(&self, program: &QcProgram) -> QcWeaponProgramSnapshot {
        weapon_snapshot(program)
    }

    fn prepare_quakec_resources(
        &mut self,
        _program: &QcProgram,
        _mounts: &MountedContent,
    ) -> Result<Self::QuakeCResources, Self::Error> {
        Err(CatalogError::Invalid(
            "QuakeC resource preparation needs the unported prepareQuakeCResources operation".to_string(),
        ))
    }

    fn prepare_rerelease_guest(
        &mut self,
        _owner: &ProviderReference,
        _artifact: &ResolvedResourceReference,
        _image: &[u8],
        _mounts: &MountedContent,
    ) -> Result<Self::RereleaseGuest, Self::Error> {
        Err(CatalogError::Invalid(
            "Rerelease guest preparation needs the unported prepareRereleaseGuest operation".to_string(),
        ))
    }
}

/// CLI QVM behavior service: profile validation lives in the unported
/// `qvm-weapon-behaviors` sibling lane.
#[derive(Default)]
struct CliQvmService;

impl QvmWeaponBehaviorService<(), ()> for CliQvmService {
    fn resolve_qvm_artifact(
        &self,
        _abi_profile: QvmAbiProfile,
        _bytes: &[u8],
        _module: &ModuleIdentity,
    ) -> Result<QvmWeaponArtifactResolution<()>, CatalogError> {
        Err(CatalogError::Invalid(
            "QVM weapon behavior support needs the unported qvm-weapon-behaviors service".to_string(),
        ))
    }

    fn read_qvm_weapon_profile(&self, _declaration: &SaveJson, _artifact: &()) -> Result<(), CatalogError> {
        Err(CatalogError::Invalid(
            "QVM weapon behavior support needs the unported qvm-weapon-behaviors service".to_string(),
        ))
    }

    fn qvm_weapon_profile_id(&self, _profile: &()) -> String {
        unreachable!("CLI QVM profiles never validate, so no identity is ever read")
    }
}

/// CLI native behavior service: declaration reads live in the unported
/// `native-weapon-behaviors` sibling lane.
#[derive(Default)]
struct CliNativeService;

impl NativeWeaponBehaviorService for CliNativeService {
    fn read_native_weapon_declaration(
        &self,
        _value: &SaveJson,
    ) -> Result<NativeWeaponBehaviorDeclaration, CatalogError> {
        Err(CatalogError::Invalid(
            "Native weapon behavior support needs the unported native-weapon-behaviors service".to_string(),
        ))
    }

    fn native_weapon_definition(
        &self,
        _declaration: &NativeWeaponBehaviorDeclaration,
        _module: &ModuleIdentity,
        _image_bytes: &[u8],
    ) -> Result<Option<WeaponBehaviorDefinition>, CatalogError> {
        Err(CatalogError::Invalid(
            "Native weapon behavior support needs the unported native-weapon-behaviors service".to_string(),
        ))
    }

    fn builtin_rerelease_weapon_declaration(
        &self,
        _module: &ModuleIdentity,
    ) -> Result<Option<NativeWeaponBehaviorDeclaration>, CatalogError> {
        Err(CatalogError::Invalid(
            "Native weapon behavior support needs the unported native-weapon-behaviors service".to_string(),
        ))
    }
}

/// Convert a loaded QuakeC program into the behavior-resolution snapshot.
/// The donor (`weapon-behaviors.ts`) reads `QcProgram` directly; the
/// snapshot is the port's `qa-content` seam, so every row below mirrors the
/// donor field the resolution logic consumes.
fn weapon_snapshot(program: &QcProgram) -> QcWeaponProgramSnapshot {
    let view = QcProgramSnapshotView::new(program);
    QcWeaponProgramSnapshot {
        digest: ContentDigest(format!("{}:{}", program.digest.algorithm, program.digest.value)),
        capability_error: qc_weapon_behavior_capability_error(&view),
        functions: program
            .functions
            .iter()
            .map(|function| QcWeaponFunction {
                index: u32::try_from(function.index).unwrap_or(u32::MAX),
                name: function.name.clone(),
                first_statement: i64::from(function.first_statement),
                parameter_words: function.parameter_sizes.len(),
            })
            .collect(),
        statements: program
            .statements
            .iter()
            .map(|statement| QcWeaponStatement {
                opcode: match statement.opcode {
                    QcOpcode::Address => QcWeaponOpcode::Address,
                    QcOpcode::StorePFn => QcWeaponOpcode::StorePFn,
                    QcOpcode::StoreFn => QcWeaponOpcode::StoreFn,
                    _ => QcWeaponOpcode::Other,
                },
                a: i32::from(statement.a),
                b: i32::from(statement.b),
                c: i32::from(statement.c),
            })
            .collect(),
        think_field_offset: program
            .field_named("think")
            .and_then(|field| i32::try_from(field.offset).ok()),
        initial_global_words: program
            .initial_globals
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| i32::from_le_bytes(*word))
            .collect(),
        function_globals: program
            .globals
            .iter()
            .filter(|global| global.value_type == ProgramQcValueType::Function)
            .filter_map(|global| {
                i32::try_from(global.offset).ok().map(|offset| QcFunctionGlobal {
                    name: global.name.clone(),
                    offset,
                })
            })
            .collect(),
    }
}

/// [`QcProgramView`] over a loaded guest program for capability checks.
struct QcProgramSnapshotView<'a> {
    program: &'a QcProgram,
    digest: String,
}

impl<'a> QcProgramSnapshotView<'a> {
    fn new(program: &'a QcProgram) -> Self {
        Self {
            program,
            digest: format!("{}:{}", program.digest.algorithm, program.digest.value),
        }
    }
}

/// Map a program value type onto the provider view type.
fn snapshot_value_type(value: ProgramQcValueType) -> ModQcValueType {
    match value {
        ProgramQcValueType::Void => ModQcValueType::Void,
        ProgramQcValueType::String => ModQcValueType::String,
        ProgramQcValueType::Float => ModQcValueType::Float,
        ProgramQcValueType::Vector => ModQcValueType::Vector,
        ProgramQcValueType::Entity => ModQcValueType::Entity,
        ProgramQcValueType::Field => ModQcValueType::Field,
        ProgramQcValueType::Function => ModQcValueType::Function,
        ProgramQcValueType::Pointer => ModQcValueType::Pointer,
        ProgramQcValueType::Opaque => ModQcValueType::Opaque,
    }
}

/// Map a program function onto the provider view row.
fn snapshot_function_view(function: &QcFunction) -> QcFunctionView {
    QcFunctionView {
        index: i32::try_from(function.index).unwrap_or(i32::MAX),
        name: function.name.clone(),
        first_statement: function.first_statement,
        parameter_start: i32::try_from(function.parameter_start).unwrap_or(i32::MAX),
        parameter_sizes: function.parameter_sizes.iter().map(|size| i32::from(*size)).collect(),
        named_builtin: function.named_builtin,
    }
}

impl QcProgramView for QcProgramSnapshotView<'_> {
    fn digest(&self) -> &str {
        &self.digest
    }

    fn api_kind(&self) -> QcApiKind {
        match self.program.api {
            QuakeCApi::Netquake => QcApiKind::Q1Netquake,
            QuakeCApi::Quakeworld => QcApiKind::Q1Quakeworld,
        }
    }

    fn field_type(&self, name: &str) -> Option<ModQcValueType> {
        self.program
            .field_named(name)
            .map(|field| snapshot_value_type(field.value_type))
    }

    fn global_type(&self, name: &str) -> Option<ModQcValueType> {
        self.program
            .global_named(name)
            .map(|global| snapshot_value_type(global.value_type))
    }

    fn function_named(&self, name: &str) -> Option<QcFunctionView> {
        self.program.function_named(name).ok().map(snapshot_function_view)
    }

    fn function_at(&self, index: i32) -> Option<QcFunctionView> {
        usize::try_from(index)
            .ok()
            .and_then(|at| self.program.functions.get(at))
            .map(snapshot_function_view)
    }

    fn functions(&self) -> Vec<QcFunctionView> {
        self.program.functions.iter().map(snapshot_function_view).collect()
    }
}

/// List installed content discovered beneath the corpus root.
fn list_content(corpus_root: &str, stdout: &mut dyn Write) {
    let _ = writeln!(stdout, "corpus root: {corpus_root}");
    match discover_installed_content(&DiscoverContentOptions::new(corpus_root.into())) {
        Ok(catalog) => {
            for product in &catalog.products {
                let status = match &product.availability {
                    ProductAvailability::Installed => "installed".to_string(),
                    ProductAvailability::Missing { requirements } => {
                        format!("missing: {}", requirements.join(", "))
                    }
                    ProductAvailability::Unresolved { reason } => format!("unresolved: {reason}"),
                };
                let _ = writeln!(stdout, "{} [{}]", product.expectation.id, status);
                for archive in &product.archives {
                    let _ = writeln!(stdout, "  {}", archive.path);
                }
            }
        }
        Err(error) => {
            let _ = writeln!(stdout, "(catalog discovery failed: {error})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_guest::qc::program::load_qc_program;

    /// Minimal version-6 `progs.dat`: two statements, one function global,
    /// one `think` field, a null function plus `fire_rocket`, and 28
    /// reserved global words.
    fn snapshot_program_bytes() -> Vec<u8> {
        fn push_i32(bytes: &mut Vec<u8>, value: i32) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fn push_u16(bytes: &mut Vec<u8>, value: u16) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 6);
        push_i32(&mut bytes, 5927);
        push_i32(&mut bytes, 60);
        push_i32(&mut bytes, 2);
        push_i32(&mut bytes, 76);
        push_i32(&mut bytes, 1);
        push_i32(&mut bytes, 84);
        push_i32(&mut bytes, 1);
        push_i32(&mut bytes, 92);
        push_i32(&mut bytes, 2);
        push_i32(&mut bytes, 164);
        push_i32(&mut bytes, 24);
        push_i32(&mut bytes, 188);
        push_i32(&mut bytes, 28);
        push_i32(&mut bytes, 16);
        debug_assert_eq!(bytes.len(), 60);
        push_u16(&mut bytes, 30);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 2);
        push_u16(&mut bytes, 3);
        push_u16(&mut bytes, 36);
        push_u16(&mut bytes, 4);
        push_u16(&mut bytes, 5);
        push_u16(&mut bytes, 6);
        debug_assert_eq!(bytes.len(), 76);
        push_u16(&mut bytes, 6);
        push_u16(&mut bytes, 9);
        push_i32(&mut bytes, 7);
        debug_assert_eq!(bytes.len(), 84);
        push_u16(&mut bytes, 6);
        push_u16(&mut bytes, 7);
        push_i32(&mut bytes, 1);
        debug_assert_eq!(bytes.len(), 92);
        for (first, name, file) in [(-1, 0, 0), (0, 7, 19)] {
            push_i32(&mut bytes, first);
            push_i32(&mut bytes, 0);
            push_i32(&mut bytes, 0);
            push_i32(&mut bytes, 0);
            push_i32(&mut bytes, name);
            push_i32(&mut bytes, file);
            push_i32(&mut bytes, 0);
            bytes.extend_from_slice(&[0; 8]);
        }
        debug_assert_eq!(bytes.len(), 164);
        bytes.extend_from_slice(b"\0think\0fire_rocket\0w.qc\0");
        debug_assert_eq!(bytes.len(), 188);
        for word in 0..28 {
            push_i32(&mut bytes, i32::from(word == 1));
        }
        bytes
    }

    fn run_text(argv: &[&str]) -> (i32, String, String) {
        let owned: Vec<String> = argv.iter().map(|arg| (*arg).to_string()).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(&owned, &mut stdout, &mut stderr, "0.1.0");
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn help_and_version_branches() {
        let (code, stdout, _) = run_text(&["--help"]);
        assert_eq!(code, 0);
        assert!(stdout.starts_with("Quake\n\nUsage: qa-muse"));
        let (code, stdout, _) = run_text(&["--version"]);
        assert_eq!(code, 0);
        assert_eq!(stdout, "Quake Anthology 0.1.0\n");
    }

    #[test]
    fn headless_run_reports_stats() {
        let (code, stdout, _) = run_text(&["--dedicated", "--movement", "q3", "--frames", "4"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("4 host frames"), "{stdout}");
        assert!(stdout.contains("server ticks"), "{stdout}");
    }

    #[test]
    fn errors_exit_nonzero() {
        let (code, _, stderr) = run_text(&["--bogus", "x"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown option"), "{stderr}");
        let (code, _, stderr) = run_text(&["--bogus"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Missing value"), "{stderr}");
        let (code, stdout, _) = run_text(&["weapon-behavior", "--help"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("declare-qvm"), "{stdout}");
    }

    #[test]
    fn weapon_behavior_dispatch_reaches_tool() {
        let (code, _, stderr) = run_text(&["weapon-behavior", "inspect", "pkg"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown requested content or mod: pkg"), "{stderr}");
        assert!(!stderr.contains("not ported yet"), "{stderr}");
    }

    #[test]
    fn tool_command_converts_every_action() {
        let content = crate::options::ToolContent {
            product: "q1".to_string(),
            corpus_root: "/corpus".to_string(),
            user_content_root: "/user".to_string(),
            artifact: Some("progs.dat".to_string()),
        };
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::Help),
            WeaponBehaviorToolCommand::Help
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::Inspect {
                content: content.clone()
            }),
            WeaponBehaviorToolCommand::Inspect {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
            }
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::DeclareQvm {
                content: content.clone(),
                profile: "prof.json".to_string(),
            }),
            WeaponBehaviorToolCommand::DeclareQvm {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
                profile: "prof.json".to_string(),
            }
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::DeclareNative {
                content: content.clone(),
                profile: "prof.json".to_string(),
            }),
            WeaponBehaviorToolCommand::DeclareNative {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
                profile: "prof.json".to_string(),
            }
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::Declare {
                content,
                id: "ns:rl".to_string(),
                title: "Rocket".to_string(),
                role: OptionsProjectileRole::Grapple,
                fire: "fire_grapple".to_string(),
                activate: Some("check".to_string()),
            }),
            WeaponBehaviorToolCommand::Declare {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
                id: "ns:rl".to_string(),
                title: "Rocket".to_string(),
                role: ToolProjectileRole::Grapple,
                fire: "fire_grapple".to_string(),
                activate: Some("check".to_string()),
            }
        );
    }

    #[test]
    fn tool_role_maps_every_role() {
        let pairs = [
            (OptionsProjectileRole::Rocket, ToolProjectileRole::Rocket),
            (OptionsProjectileRole::Grenade, ToolProjectileRole::Grenade),
            (OptionsProjectileRole::Nail, ToolProjectileRole::Nail),
            (OptionsProjectileRole::Bolt, ToolProjectileRole::Bolt),
            (OptionsProjectileRole::Plasma, ToolProjectileRole::Plasma),
            (OptionsProjectileRole::Energy, ToolProjectileRole::Energy),
            (OptionsProjectileRole::Grapple, ToolProjectileRole::Grapple),
        ];
        for (parsed, expected) in pairs {
            assert_eq!(tool_role(parsed), expected);
        }
    }

    #[test]
    fn weapon_snapshot_mirrors_program_rows() {
        let program = load_qc_program(&snapshot_program_bytes(), None, "test.dat").unwrap();
        let snapshot = weapon_snapshot(&program);
        assert!(snapshot.digest.as_str().starts_with("sha256:"));
        assert_eq!(snapshot.digest.as_str().len(), 7 + 64);
        assert!(snapshot
            .capability_error
            .is_some_and(|reason| reason.contains("QuakeC trajectory adapter requires")));
        assert_eq!(snapshot.functions.len(), 2);
        assert_eq!(snapshot.functions[1].index, 1);
        assert_eq!(snapshot.functions[1].name, "fire_rocket");
        assert_eq!(snapshot.functions[1].first_statement, 0);
        assert_eq!(snapshot.functions[1].parameter_words, 0);
        assert_eq!(snapshot.statements.len(), 2);
        assert_eq!(snapshot.statements[0].opcode, QcWeaponOpcode::Address);
        assert_eq!(
            (
                snapshot.statements[0].a,
                snapshot.statements[0].b,
                snapshot.statements[0].c
            ),
            (1, 2, 3)
        );
        assert_eq!(snapshot.statements[1].opcode, QcWeaponOpcode::StoreFn);
        assert_eq!(snapshot.think_field_offset, Some(7));
        assert_eq!(snapshot.initial_global_words.len(), 28);
        assert_eq!(snapshot.initial_global_words[0], 0);
        assert_eq!(snapshot.initial_global_words[1], 1);
        assert_eq!(snapshot.function_globals.len(), 1);
        assert_eq!(snapshot.function_globals[0].name, "fire_rocket");
        assert_eq!(snapshot.function_globals[0].offset, 9);
    }

    #[test]
    fn list_content_reports_catalog_products() {
        let root = std::env::temp_dir().join("qa-muse-list-content");
        let _ = std::fs::create_dir_all(&root);
        let root = root.to_string_lossy().into_owned();
        let (code, stdout, _) = run_text(&["--list-content", "--content-root", root.as_str()]);
        assert_eq!(code, 0);
        assert!(stdout.contains(format!("corpus root: {root}").as_str()), "{stdout}");
        assert!(stdout.contains("q1-classic-id1"), "{stdout}");
        assert!(!stdout.contains("not ported yet"), "{stdout}");
    }
}
