//! Weapon-behavior authoring tool ported from
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/weapon-behavior-tool.ts`.
//!
//! Catalog discovery, behavior documents, and mount plans reuse
//! [`qa_content`]; QuakeC, QVM, and PE parsing reuse [`qa_guest`]. The
//! command model reuses the sibling
//! [`weapon_behavior_tool_options`](super::weapon_behavior_tool_options)
//! port, and shared behavior helpers reuse
//! [`weapon_behavior_selection`](super::weapon_behavior_selection). Guest
//! behavior services and QuakeC program snapshots arrive through
//! [`WeaponBehaviorHost`](super::weapon_behavior_selection::WeaponBehaviorHost)
//! because those sibling lanes are unported.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use qa_content::catalog::{
    discover_installed_content, discover_native_weapon_behaviors, discover_qc_weapon_behaviors,
    discover_qvm_weapon_behaviors, inspect_qc_trajectory_bindings, load_native_weapon_behavior,
    load_qvm_weapon_behavior, read_native_weapon_behavior_document, read_qvm_weapon_behavior_document,
    read_weapon_behavior_document, resolve_qc_weapon_behavior, weapon_behavior_entry_id, BehaviorMounts, CatalogError,
    CatalogProduct, DiscoverContentOptions, InstalledCatalog, SourceWeaponBehaviorMetadata,
    WeaponBehaviorCompatibility,
};
use qa_content::contract::{ContentDigest, ContractError, GameFamily, NativeWeaponBehaviorDeclaration, ProjectileRole};
use qa_content::mounts::{open_mount_plan, MountError, MountedContent, OpenMountOptions};
use qa_content::paths::PathError;
use qa_content::user_data::user_product_directory;
use qa_content::value::{parse_save_json, save_error, SaveJson, SaveReader, ValueError};
use qa_guest::core::contracts::{ContentDigest as GuestContentDigest, NativeCallAbi};
use qa_guest::pe::format::parse_pe;
use qa_guest::qc::program::load_qc_program;
use qa_guest::qvm::image::{parse_qvm, QvmOpcode};
use qa_guest::GuestError;
use thiserror::Error;

use super::weapon_behavior_selection::{
    behavior_module, default_progs_path, mount_plan_id_for_product, native_weapon_declaration_json,
    projectile_role_name, weapon_behavior_provider, WeaponBehaviorHost,
};
use super::weapon_behavior_tool_options::{
    ProjectileRole as ToolProjectileRole, WeaponBehaviorToolCommand, WEAPON_BEHAVIOR_TOOL_HELP,
};
use crate::settings::json::{stringify_pretty, Json};

/// Weapon-behavior tool run failure.
#[derive(Debug, Error)]
pub enum WeaponBehaviorToolRunError {
    /// Invalid command, product, or document (donor error text).
    #[error("{0}")]
    Invalid(String),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Guest program failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// User-data path failure.
    #[error(transparent)]
    Path(#[from] PathError),
    /// Document value failure.
    #[error(transparent)]
    Value(#[from] ValueError),
    /// Filesystem failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Injected host failure.
    #[error("weapon behavior host: {0}")]
    Host(String),
}

fn invalid(message: impl Into<String>) -> WeaponBehaviorToolRunError {
    WeaponBehaviorToolRunError::Invalid(message.into())
}

/// Tool content roots shared by every non-help command.
struct ToolRoots<'a> {
    product: &'a str,
    corpus_root: &'a str,
    user_content_root: &'a str,
    artifact: Option<&'a str>,
}

/// Shared roots of a non-help command.
fn tool_roots(command: &WeaponBehaviorToolCommand) -> Option<ToolRoots<'_>> {
    match command {
        WeaponBehaviorToolCommand::Help => None,
        WeaponBehaviorToolCommand::Inspect {
            product,
            corpus_root,
            user_content_root,
            artifact,
        }
        | WeaponBehaviorToolCommand::DeclareQvm {
            product,
            corpus_root,
            user_content_root,
            artifact,
            ..
        }
        | WeaponBehaviorToolCommand::DeclareNative {
            product,
            corpus_root,
            user_content_root,
            artifact,
            ..
        }
        | WeaponBehaviorToolCommand::Declare {
            product,
            corpus_root,
            user_content_root,
            artifact,
            ..
        } => Some(ToolRoots {
            product,
            corpus_root,
            user_content_root,
            artifact: artifact.as_deref(),
        }),
    }
}

/// Declare action name for the family-mismatch error.
fn declare_action_name(command: &WeaponBehaviorToolCommand) -> Option<&'static str> {
    match command {
        WeaponBehaviorToolCommand::DeclareQvm { .. } => Some("declare-qvm"),
        WeaponBehaviorToolCommand::DeclareNative { .. } => Some("declare-native"),
        WeaponBehaviorToolCommand::Help
        | WeaponBehaviorToolCommand::Inspect { .. }
        | WeaponBehaviorToolCommand::Declare { .. } => None,
    }
}

/// Reject Q2 providers without the rerelease API.
fn reject_non_rerelease_q2(family: &GameFamily, edition: &str) -> Result<(), WeaponBehaviorToolRunError> {
    if *family == GameFamily::Q2 && edition != "rerelease" {
        return Err(invalid(
            "Native weapon declarations currently require the Q2 rerelease API2023 Windows x64 adapter",
        ));
    }
    Ok(())
}

/// Discover installed content with mods for a tool command.
fn discover_tool_content(
    corpus_root: &str,
    user_content_root: &str,
) -> Result<InstalledCatalog, WeaponBehaviorToolRunError> {
    Ok(discover_installed_content(&DiscoverContentOptions {
        corpus_root: PathBuf::from(corpus_root),
        user_content_root: Some(PathBuf::from(user_content_root)),
        products: None,
        generation: 0,
        discover_mods: true,
        remote_content: None,
    })?)
}

/// Open the authoring mount plan for a product.
fn open_authoring_mounts(
    catalog: &InstalledCatalog,
    product: &CatalogProduct,
) -> Result<MountedContent, WeaponBehaviorToolRunError> {
    let mounts = catalog.mounts_for(product.id.as_str())?;
    let plan = qa_content::contract::ResolvedMountPlan {
        id: mount_plan_id_for_product("weapon-authoring", &product.id)?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        prefix_orders: Vec::new(),
        mounts,
    };
    Ok(open_mount_plan(&plan, OpenMountOptions::default())?)
}

/// Run the weapon-behavior authoring tool (`runWeaponBehaviorTool`).
pub fn run_weapon_behavior_tool<Artifact, Profile, H>(
    command: &WeaponBehaviorToolCommand,
    host: &mut H,
    print: &mut dyn FnMut(String),
) -> Result<(), WeaponBehaviorToolRunError>
where
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    if matches!(command, WeaponBehaviorToolCommand::Help) {
        print(WEAPON_BEHAVIOR_TOOL_HELP.to_owned());
        return Ok(());
    }
    let Some(roots) = tool_roots(command) else {
        print(WEAPON_BEHAVIOR_TOOL_HELP.to_owned());
        return Ok(());
    };
    let catalog = discover_tool_content(roots.corpus_root, roots.user_content_root)?;
    let product = catalog.require(roots.product)?;
    reject_non_rerelease_q2(&product.expectation.family, &product.expectation.edition)?;
    let content = open_authoring_mounts(&catalog, product)?;
    if product.expectation.family == GameFamily::Q2 {
        return run_native_behavior_tool(command, product, &content, host, print);
    }
    if product.expectation.family == GameFamily::Q3 {
        return run_qvm_behavior_tool(command, product, &content, host, print);
    }
    if let Some(action) = declare_action_name(command) {
        return Err(invalid(format!(
            "{action} requires its matching Q3 or Q2 rerelease provider"
        )));
    }
    run_quakec_behavior_tool(command, roots, product, &content, host, print)
}

/// Convert a document value to output JSON.
fn save_json_to_json(value: &SaveJson) -> Result<Json, WeaponBehaviorToolRunError> {
    match value {
        SaveJson::Null => Ok(Json::Null),
        SaveJson::Bool(value) => Ok(Json::Bool(*value)),
        SaveJson::Number(value) => Ok(Json::Number(*value)),
        SaveJson::BigInt(value) => {
            if *value >= -(1 << 53) && *value <= (1 << 53) {
                Ok(Json::Number(*value as f64))
            } else {
                Err(invalid(
                    "Behavior document number is outside the exact JSON integer range",
                ))
            }
        }
        SaveJson::Bytes(_) => Err(invalid("Behavior document stores raw bytes; expected JSON")),
        SaveJson::String(value) => Ok(Json::String(value.clone())),
        SaveJson::Array(items) => Ok(Json::Array(
            items.iter().map(save_json_to_json).collect::<Result<Vec<_>, _>>()?,
        )),
        SaveJson::Object(members) => Ok(Json::Object(
            members
                .iter()
                .map(|(name, member)| save_json_to_json(member).map(|json| (name.clone(), json)))
                .collect::<Result<Vec<_>, _>>()?,
        )),
    }
}

/// Guest digest rendered as a content digest (`sha256:...`).
fn guest_digest_to_content(digest: &GuestContentDigest) -> ContentDigest {
    ContentDigest(format!("{}:{}", digest.algorithm, digest.value))
}

/// Contract projectile role for a tool role.
fn contract_role(role: ToolProjectileRole) -> ProjectileRole {
    match role {
        ToolProjectileRole::Rocket => ProjectileRole::Rocket,
        ToolProjectileRole::Grenade => ProjectileRole::Grenade,
        ToolProjectileRole::Nail => ProjectileRole::Nail,
        ToolProjectileRole::Bolt => ProjectileRole::Bolt,
        ToolProjectileRole::Plasma => ProjectileRole::Plasma,
        ToolProjectileRole::Energy => ProjectileRole::Energy,
        ToolProjectileRole::Grapple => ProjectileRole::Grapple,
    }
}

/// Atomically write a behavior document (`writeBehaviorDocument`): create the
/// directory, write a uniquely-named temporary file with owner-only
/// permissions, sync, rename over the destination, and remove the temporary
/// name. The temporary name uses the process identity plus a counter instead
/// of a UUID; exclusive creation keeps it collision-safe.
fn write_behavior_document(directory: &Path, name: &str, bytes: &str) -> Result<PathBuf, WeaponBehaviorToolRunError> {
    static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(directory)?;
    let destination = directory.join(name);
    let temporary = directory.join(format!(
        ".weapon-behaviors-{}-{}.tmp",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    Ok(destination)
}

/// Print the saved-declaration receipt.
fn print_saved(print: &mut dyn FnMut(String), id: &str, destination: &Path, product: &str) {
    print(format!(
        "Saved {id} to {}\nSelect with --weapon-behavior {product}/{id}\n",
        destination.display()
    ));
}

/// Whether a QuakeC function is an inspectable zero-parameter callback.
fn is_inspectable_callback(index: usize, first_statement: i32, parameter_count: usize) -> bool {
    index != 0 && first_statement >= 0 && parameter_count == 0
}

/// QuakeC inspect report (`callbacks` entries are `(name, index)`;
/// `think_assignments` entries are `(producer, think, statement)`).
fn qc_inspect_json(
    product: &str,
    path: &str,
    digest: &str,
    callbacks: Vec<(String, usize)>,
    think_assignments: Vec<(Option<String>, Option<String>, usize)>,
) -> Json {
    Json::Object(vec![
        ("product".to_owned(), Json::String(product.to_owned())),
        ("artifact".to_owned(), Json::String(path.to_owned())),
        ("digest".to_owned(), Json::String(digest.to_owned())),
        (
            "scope".to_owned(),
            Json::String(
                "Callback identities and statement positions apply only to this exact artifact digest; roles and activation gates require explicit selection."
                    .to_owned(),
            ),
        ),
        (
            "callbacks".to_owned(),
            Json::Array(
                callbacks
                    .into_iter()
                    .map(|(name, index)| {
                        Json::Object(vec![
                            ("name".to_owned(), Json::String(name)),
                            ("index".to_owned(), Json::Number(index as f64)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "thinkAssignments".to_owned(),
            Json::Array(
                think_assignments
                    .into_iter()
                    .map(|(producer, think, statement)| {
                        Json::Object(vec![
                            (
                                "producer".to_owned(),
                                producer.map_or(Json::Null, Json::String),
                            ),
                            ("think".to_owned(), think.map_or(Json::Null, Json::String)),
                            (
                                "statement".to_owned(),
                                Json::Number(statement as f64),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// QuakeC declaration metadata document value.
fn qc_metadata_json(metadata: &SourceWeaponBehaviorMetadata) -> Json {
    let mut members = vec![
        ("id".to_owned(), Json::String(metadata.id.clone())),
        ("title".to_owned(), Json::String(metadata.title.clone())),
        (
            "artifactDigest".to_owned(),
            Json::String(metadata.artifact_digest.as_str().to_owned()),
        ),
        (
            "role".to_owned(),
            Json::String(projectile_role_name(metadata.role).to_owned()),
        ),
        ("aspect".to_owned(), Json::String("trajectory".to_owned())),
        ("fireFunction".to_owned(), Json::String(metadata.fire_function.clone())),
    ];
    if let Some(activation) = &metadata.activation_function {
        members.push(("activationFunction".to_owned(), Json::String(activation.clone())));
    }
    Json::Object(members)
}

/// QuakeC behavior document value.
fn qc_declare_json(path: &str, retained: Vec<Json>, declaration: Json) -> Json {
    let mut behaviors = retained;
    behaviors.push(declaration);
    Json::Object(vec![
        ("version".to_owned(), Json::Number(1.0)),
        ("artifactPath".to_owned(), Json::String(path.to_owned())),
        ("behaviors".to_owned(), Json::Array(behaviors)),
    ])
}

/// Run the QuakeC authoring branch.
#[allow(clippy::too_many_lines)]
fn run_quakec_behavior_tool<Artifact, Profile, H>(
    command: &WeaponBehaviorToolCommand,
    roots: ToolRoots<'_>,
    product: &CatalogProduct,
    content: &MountedContent,
    host: &mut H,
    print: &mut dyn FnMut(String),
) -> Result<(), WeaponBehaviorToolRunError>
where
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    let existing = content.open_behavior("weapon-behaviors.json")?;
    let document = existing
        .as_ref()
        .map(|descriptor| read_weapon_behavior_document(&descriptor.bytes))
        .transpose()?;
    let path = roots
        .artifact
        .map(str::to_owned)
        .or_else(|| document.as_ref().and_then(|document| document.artifact_path.clone()))
        .unwrap_or_else(|| default_progs_path(&product.expectation.edition).to_owned());
    let Some(artifact) = content.open_behavior(&path)? else {
        return Err(invalid(format!("Mounted behavior artifact is missing: {path}")));
    };
    let program = load_qc_program(&artifact.bytes, None, "progs.dat")?;
    let module = behavior_module(&product.id, &path, &artifact.reference.digest);
    if matches!(command, WeaponBehaviorToolCommand::Inspect { .. }) {
        let snapshot = host.weapon_snapshot(&program);
        let callbacks = program
            .functions
            .iter()
            .filter(|function| {
                is_inspectable_callback(function.index, function.first_statement, function.parameter_sizes.len())
            })
            .map(|function| (function.name.clone(), function.index))
            .collect();
        let think_assignments = inspect_qc_trajectory_bindings(&snapshot)
            .into_iter()
            .map(|binding| {
                let producer = program
                    .functions
                    .get(binding.producer_function as usize)
                    .map(|function| function.name.clone());
                let think = program
                    .functions
                    .get(binding.think_function as usize)
                    .map(|function| function.name.clone());
                (producer, think, binding.statement)
            })
            .collect();
        let digest = guest_digest_to_content(&program.digest);
        let report = qc_inspect_json(
            &product.expectation.id,
            &path,
            digest.as_str(),
            callbacks,
            think_assignments,
        );
        print(format!("{}\n", stringify_pretty(&report)));
        return Ok(());
    }
    let WeaponBehaviorToolCommand::Declare {
        id,
        title,
        role,
        fire,
        activate,
        ..
    } = command
    else {
        return Err(invalid(format!(
            "{} requires its matching Q3 or Q2 rerelease provider",
            declare_action_name(command).unwrap_or("declare")
        )));
    };
    let metadata = SourceWeaponBehaviorMetadata {
        id: id.clone(),
        title: title.clone(),
        artifact_digest: guest_digest_to_content(&program.digest),
        role: contract_role(*role),
        fire_function: fire.clone(),
        activation_function: activate.clone(),
    };
    let snapshot = host.weapon_snapshot(&program);
    let validated = resolve_qc_weapon_behavior(&module, &snapshot, &metadata);
    if let WeaponBehaviorCompatibility::Unsupported { reason } = validated {
        return Err(invalid(reason));
    }
    if let Some(document) = &document {
        let old_path = document
            .artifact_path
            .clone()
            .unwrap_or_else(|| default_progs_path(&product.expectation.edition).to_owned());
        let mut foreign = false;
        for entry in &document.behaviors {
            if weapon_behavior_entry_id(entry)? != *id {
                foreign = true;
                break;
            }
        }
        if old_path != path && foreign {
            return Err(invalid(
                "Cannot change the artifact for unrelated existing behavior declarations",
            ));
        }
        discover_qc_weapon_behaviors(content as &dyn BehaviorMounts, &module, &snapshot)?;
    }
    let mut retained = Vec::new();
    for entry in document.map_or(Vec::new(), |document| document.behaviors) {
        if weapon_behavior_entry_id(&entry)? != *id {
            retained.push(save_json_to_json(&entry)?);
        }
    }
    let bytes = format!(
        "{}\n",
        stringify_pretty(&qc_declare_json(&path, retained, qc_metadata_json(&metadata)))
    );
    let directory = user_product_directory(
        Path::new(roots.user_content_root),
        &product.expectation.content_directory,
    )?;
    let destination = write_behavior_document(&directory, "weapon-behaviors.json", &bytes)?;
    print_saved(print, id, &destination, &product.expectation.id);
    Ok(())
}

/// QVM inspect report (`entries` are instruction indices of `OP_ENTER`;
/// `declared` entries are `(id, artifact, digest)`).
fn qvm_inspect_json(
    product: &str,
    path: &str,
    digest: &str,
    instruction_count: usize,
    data_bytes: usize,
    entries: Vec<usize>,
    declared: Vec<(String, String, String)>,
) -> Json {
    Json::Object(vec![
        ("product".to_owned(), Json::String(product.to_owned())),
        ("artifact".to_owned(), Json::String(path.to_owned())),
        ("digest".to_owned(), Json::String(digest.to_owned())),
        (
            "scope".to_owned(),
            Json::String(
                "Instruction entries are exact bytecode boundaries, not inferred symbols or trajectory features. Author-declared ABI and private layout are required."
                    .to_owned(),
            ),
        ),
        (
            "instructionCount".to_owned(),
            Json::Number(instruction_count as f64),
        ),
        ("dataBytes".to_owned(), Json::Number(data_bytes as f64)),
        (
            "entries".to_owned(),
            Json::Array(
                entries
                    .into_iter()
                    .map(|index| {
                        Json::Object(vec![(
                            "instructionIndex".to_owned(),
                            Json::Number(index as f64),
                        )])
                    })
                    .collect(),
            ),
        ),
        (
            "declared".to_owned(),
            Json::Array(
                declared
                    .into_iter()
                    .map(|(id, artifact, digest)| {
                        Json::Object(vec![
                            ("id".to_owned(), Json::String(id)),
                            ("artifact".to_owned(), Json::String(artifact)),
                            ("digest".to_owned(), Json::String(digest)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Profiles document value for QVM declarations.
fn qvm_profiles_json(profiles: Vec<Json>) -> Json {
    Json::Object(vec![
        ("version".to_owned(), Json::Number(1.0)),
        ("profiles".to_owned(), Json::Array(profiles)),
    ])
}

/// Parse a mounted author profile (`JSON.parse(TextDecoder(fatal).decode)`).
fn parse_author_profile(path: &str, bytes: &[u8]) -> Result<SaveJson, WeaponBehaviorToolRunError> {
    let text = std::str::from_utf8(bytes).map_err(|_| save_error(path, "expected UTF-8"))?;
    Ok(parse_save_json(text)?)
}

/// Run the QVM authoring branch (`runQvmBehaviorTool`).
#[allow(clippy::too_many_lines)]
fn run_qvm_behavior_tool<Artifact, Profile, H>(
    command: &WeaponBehaviorToolCommand,
    product: &CatalogProduct,
    content: &MountedContent,
    host: &mut H,
    print: &mut dyn FnMut(String),
) -> Result<(), WeaponBehaviorToolRunError>
where
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    let provider = weapon_behavior_provider(&product.id);
    if let WeaponBehaviorToolCommand::Inspect { artifact, .. } = command {
        let path = artifact.clone().unwrap_or_else(|| "vm/qagame.qvm".to_owned());
        let Some(opened) = content.open_behavior(&path)? else {
            return Err(invalid(format!("Mounted QVM artifact is missing: {path}")));
        };
        let image = parse_qvm(&opened.bytes, &path)?;
        let declarations =
            discover_qvm_weapon_behaviors(content as &dyn BehaviorMounts, &provider, host.qvm_service())?;
        let entries = image
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction.opcode == QvmOpcode::OpEnter)
            .map(|(index, _)| index)
            .collect();
        let declared = declarations
            .unwrap_or_default()
            .iter()
            .map(|entry| {
                (
                    host.qvm_service().qvm_weapon_profile_id(&entry.profile),
                    entry.resource.requested_path.clone(),
                    entry.resource.digest.as_str().to_owned(),
                )
            })
            .collect();
        let report = qvm_inspect_json(
            &product.expectation.id,
            &path,
            opened.reference.digest.as_str(),
            image.instructions.len(),
            image.data_length + image.literal_length + image.bss_length,
            entries,
            declared,
        );
        print(format!("{}\n", stringify_pretty(&report)));
        return Ok(());
    }
    let WeaponBehaviorToolCommand::DeclareQvm {
        artifact,
        profile,
        user_content_root,
        ..
    } = command
    else {
        return Err(invalid(
            "QVM declarations require declare-qvm --profile MOUNTED_PROFILE_JSON; function names alone do not establish a private layout",
        ));
    };
    let Some(opened) = content.open_behavior(profile)? else {
        return Err(invalid(format!("Mounted QVM author profile is missing: {profile}")));
    };
    let declaration = parse_author_profile(profile, &opened.bytes)?;
    let entry = load_qvm_weapon_behavior(
        content as &dyn BehaviorMounts,
        &provider,
        &declaration,
        host.qvm_service(),
    )?;
    if artifact
        .as_ref()
        .is_some_and(|path| *path != entry.resource.requested_path)
    {
        return Err(invalid("--artifact differs from the authored QVM profile"));
    }
    let definition_id = host.qvm_service().qvm_weapon_profile_id(&entry.profile);
    let existing = content.open_behavior("qvm-weapon-behaviors.json")?;
    let mut retained = Vec::new();
    for old in existing
        .as_ref()
        .map(|descriptor| read_qvm_weapon_behavior_document(&descriptor.bytes))
        .transpose()?
        .unwrap_or_default()
    {
        if format!("qvm:{}", SaveReader::at(&old, "profile").field("id").string()?) == definition_id {
            continue;
        }
        load_qvm_weapon_behavior(content as &dyn BehaviorMounts, &provider, &old, host.qvm_service())?;
        retained.push(save_json_to_json(&old)?);
    }
    retained.push(save_json_to_json(&declaration)?);
    let bytes = format!("{}\n", stringify_pretty(&qvm_profiles_json(retained)));
    let directory = user_product_directory(Path::new(user_content_root), &product.expectation.content_directory)?;
    let destination = write_behavior_document(&directory, "qvm-weapon-behaviors.json", &bytes)?;
    print_saved(print, &definition_id, &destination, &product.expectation.id);
    Ok(())
}

/// Native inspect report (`sections` entries are
/// `(name, rva, byte_length, permissions)`).
fn native_inspect_json(
    product: &str,
    path: &str,
    digest: &str,
    abi: &str,
    entry_point_rva: u32,
    sections: Vec<(String, u32, u32, String)>,
    declared: Vec<Json>,
) -> Json {
    Json::Object(vec![
        ("product".to_owned(), Json::String(product.to_owned())),
        ("artifact".to_owned(), Json::String(path.to_owned())),
        ("digest".to_owned(), Json::String(digest.to_owned())),
        ("abi".to_owned(), Json::String(abi.to_owned())),
        (
            "scope".to_owned(),
            Json::String(
                "PE sections identify image ranges only. Private weapon layout, callback signatures and provisioning require an explicit source-backed declaration pinned to this artifact."
                    .to_owned(),
            ),
        ),
        (
            "entryPointRva".to_owned(),
            Json::Number(f64::from(entry_point_rva)),
        ),
        (
            "sections".to_owned(),
            Json::Array(
                sections
                    .into_iter()
                    .map(|(name, rva, byte_length, permissions)| {
                        Json::Object(vec![
                            ("name".to_owned(), Json::String(name)),
                            ("rva".to_owned(), Json::Number(f64::from(rva))),
                            (
                                "byteLength".to_owned(),
                                Json::Number(f64::from(byte_length)),
                            ),
                            ("permissions".to_owned(), Json::String(permissions)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("declared".to_owned(), Json::Array(declared)),
    ])
}

/// Profiles document value for native declarations.
fn native_profiles_json(
    retained: &[NativeWeaponBehaviorDeclaration],
    declaration: &NativeWeaponBehaviorDeclaration,
) -> Json {
    let mut profiles: Vec<Json> = retained.iter().map(native_weapon_declaration_json).collect();
    profiles.push(native_weapon_declaration_json(declaration));
    Json::Object(vec![
        ("version".to_owned(), Json::Number(1.0)),
        ("profiles".to_owned(), Json::Array(profiles)),
    ])
}

/// Run the native authoring branch (`runNativeBehaviorTool`).
#[allow(clippy::too_many_lines)]
fn run_native_behavior_tool<Artifact, Profile, H>(
    command: &WeaponBehaviorToolCommand,
    product: &CatalogProduct,
    content: &MountedContent,
    host: &mut H,
    print: &mut dyn FnMut(String),
) -> Result<(), WeaponBehaviorToolRunError>
where
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    let provider = weapon_behavior_provider(&product.id);
    if let WeaponBehaviorToolCommand::Inspect { artifact, .. } = command {
        let declarations =
            discover_native_weapon_behaviors(content as &dyn BehaviorMounts, &provider, host.native_service())?;
        let path = artifact
            .clone()
            .or_else(|| {
                declarations
                    .as_ref()
                    .and_then(|entries| entries.first().map(|entry| entry.resource.requested_path.clone()))
            })
            .unwrap_or_else(|| "game_x64.dll".to_owned());
        let Some(opened) = content.open_behavior(&path)? else {
            return Err(invalid(format!("Mounted native artifact is missing: {path}")));
        };
        let image = parse_pe(&opened.bytes)?;
        let sections = image
            .sections
            .iter()
            .map(|section| {
                (
                    section.name.clone(),
                    section.rva,
                    section.mapped_size,
                    section.permissions.label().to_owned(),
                )
            })
            .collect();
        let declared = declarations
            .unwrap_or_default()
            .iter()
            .map(|entry| native_weapon_declaration_json(&entry.declaration))
            .collect();
        let report = native_inspect_json(
            &product.expectation.id,
            &path,
            opened.reference.digest.as_str(),
            NativeCallAbi::of(image.abi).kind(),
            image.entry_point_rva,
            sections,
            declared,
        );
        print(format!("{}\n", stringify_pretty(&report)));
        return Ok(());
    }
    let WeaponBehaviorToolCommand::DeclareNative {
        artifact,
        profile,
        user_content_root,
        ..
    } = command
    else {
        return Err(invalid(
            "Native declarations require declare-native --profile MOUNTED_PROFILE_JSON",
        ));
    };
    let Some(opened) = content.open_behavior(profile)? else {
        return Err(invalid(format!("Mounted native author profile is missing: {profile}")));
    };
    let value = parse_author_profile(profile, &opened.bytes)?;
    let entry = load_native_weapon_behavior(content as &dyn BehaviorMounts, &provider, &value, host.native_service())?;
    if artifact
        .as_ref()
        .is_some_and(|path| *path != entry.resource.requested_path)
    {
        return Err(invalid("--artifact differs from the authored native profile"));
    }
    let existing = content.open_behavior("native-weapon-behaviors.json")?;
    let mut retained = Vec::new();
    for old in existing
        .as_ref()
        .map(|descriptor| read_native_weapon_behavior_document(&descriptor.bytes))
        .transpose()?
        .unwrap_or_default()
    {
        if SaveReader::at(&old, "native-profile").field("id").string()? == entry.definition.id {
            continue;
        }
        retained.push(
            load_native_weapon_behavior(content as &dyn BehaviorMounts, &provider, &old, host.native_service())?
                .declaration,
        );
    }
    let bytes = format!(
        "{}\n",
        stringify_pretty(&native_profiles_json(&retained, &entry.declaration))
    );
    let directory = user_product_directory(Path::new(user_content_root), &product.expectation.content_directory)?;
    let destination = write_behavior_document(&directory, "native-weapon-behaviors.json", &bytes)?;
    print_saved(print, &entry.definition.id, &destination, &product.expectation.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{
        NativeWeaponBehaviorService, QcWeaponProgramSnapshot, QvmWeaponArtifactResolution, QvmWeaponBehaviorService,
    };
    use qa_content::contract::{
        ContentId, ExecutableRecipe, ModuleIdentity, NativeWeaponAllocate, NativeWeaponCalls, NativeWeaponClient,
        NativeWeaponCommand, NativeWeaponEntity, NativeWeaponEntry, NativeWeaponEquipped, NativeWeaponFree,
        NativeWeaponRegistrationLayout, NativeWeaponThink, NativeWeaponTime, ProviderReference, QvmAbiProfile,
        ResolvedResourceReference,
    };
    use qa_content::value::{arr, num, obj, str as save_str};
    use qa_guest::qc::program::QcProgram;

    struct StubQvm;
    struct StubNative;

    impl QvmWeaponBehaviorService<String, String> for StubQvm {
        fn resolve_qvm_artifact(
            &self,
            _abi_profile: QvmAbiProfile,
            _bytes: &[u8],
            _module: &ModuleIdentity,
        ) -> Result<QvmWeaponArtifactResolution<String>, CatalogError> {
            Err(CatalogError::Invalid("stub has no artifacts".to_owned()))
        }

        fn read_qvm_weapon_profile(&self, _declaration: &SaveJson, _artifact: &String) -> Result<String, CatalogError> {
            Err(CatalogError::Invalid("stub has no profiles".to_owned()))
        }

        fn qvm_weapon_profile_id(&self, profile: &String) -> String {
            profile.clone()
        }
    }

    impl NativeWeaponBehaviorService for StubNative {
        fn read_native_weapon_declaration(
            &self,
            _value: &SaveJson,
        ) -> Result<NativeWeaponBehaviorDeclaration, CatalogError> {
            Err(CatalogError::Invalid("stub has no declarations".to_owned()))
        }

        fn native_weapon_definition(
            &self,
            _declaration: &NativeWeaponBehaviorDeclaration,
            _module: &ModuleIdentity,
            _image_bytes: &[u8],
        ) -> Result<Option<qa_content::contract::WeaponBehaviorDefinition>, CatalogError> {
            Err(CatalogError::Invalid("stub has no definitions".to_owned()))
        }

        fn builtin_rerelease_weapon_declaration(
            &self,
            _module: &ModuleIdentity,
        ) -> Result<Option<NativeWeaponBehaviorDeclaration>, CatalogError> {
            Err(CatalogError::Invalid("stub has no builtins".to_owned()))
        }
    }

    struct StubHost {
        qvm: StubQvm,
        native: StubNative,
    }

    impl WeaponBehaviorHost<String, String> for StubHost {
        type Error = CatalogError;
        type QuakeCResources = ();
        type RereleaseGuest = ();

        fn qvm_service(&self) -> &dyn QvmWeaponBehaviorService<String, String> {
            &self.qvm
        }

        fn native_service(&self) -> &dyn NativeWeaponBehaviorService {
            &self.native
        }

        fn content_mounts(&mut self, _content: &ContentId) -> Result<MountedContent, Self::Error> {
            Err(CatalogError::Invalid("stub has no mounts".to_owned()))
        }

        fn apply_requests(
            &mut self,
            _catalog: &InstalledCatalog,
            recipe: ExecutableRecipe,
            _requests: &[super::super::weapon_behavior_selection::WeaponBehaviorRequest],
        ) -> Result<ExecutableRecipe, Self::Error> {
            Ok(recipe)
        }

        fn weapon_snapshot(&self, _program: &QcProgram) -> QcWeaponProgramSnapshot {
            QcWeaponProgramSnapshot {
                digest: ContentDigest("sha256:00".to_owned()),
                capability_error: None,
                functions: Vec::new(),
                statements: Vec::new(),
                think_field_offset: None,
                initial_global_words: Vec::new(),
                function_globals: Vec::new(),
            }
        }

        fn prepare_quakec_resources(
            &mut self,
            _program: &QcProgram,
            _mounts: &MountedContent,
        ) -> Result<Self::QuakeCResources, Self::Error> {
            Err(CatalogError::Invalid("stub has no resources".to_owned()))
        }

        fn prepare_rerelease_guest(
            &mut self,
            _owner: &ProviderReference,
            _artifact: &ResolvedResourceReference,
            _image: &[u8],
            _mounts: &MountedContent,
        ) -> Result<Self::RereleaseGuest, Self::Error> {
            Err(CatalogError::Invalid("stub has no guests".to_owned()))
        }
    }

    fn stub_host() -> StubHost {
        StubHost {
            qvm: StubQvm,
            native: StubNative,
        }
    }

    fn inspect_command() -> WeaponBehaviorToolCommand {
        WeaponBehaviorToolCommand::Inspect {
            product: "p".to_owned(),
            corpus_root: "/corpus".to_owned(),
            user_content_root: "/user".to_owned(),
            artifact: Some("progs.dat".to_owned()),
        }
    }

    fn entry(rva: u64) -> NativeWeaponEntry {
        NativeWeaponEntry {
            rva,
            registration: None,
        }
    }

    fn declaration() -> NativeWeaponBehaviorDeclaration {
        let layout = NativeWeaponRegistrationLayout {
            byte_length: 1,
            name: 2,
            tag: 3,
            callback: 4,
        };
        NativeWeaponBehaviorDeclaration {
            version: 1,
            id: "native:rail".to_owned(),
            title: "Rail".to_owned(),
            role: ProjectileRole::Bolt,
            artifact_path: "game_x64.dll".to_owned(),
            artifact_digest: ContentDigest("sha256:ab".to_owned()),
            entity: NativeWeaponEntity {
                byte_length: 1,
                origin: 2,
                angles: 3,
                velocity: 4,
                client: 5,
                owner: 6,
                view_height: 7,
                generation: 8,
                next_think: 9,
                think_callback: 10,
                think_registration: 11,
                touch_callback: 12,
            },
            client: NativeWeaponClient {
                byte_length: 1,
                weapon: 2,
                view_angles: 3,
                forward: 4,
            },
            equipped_weapon: NativeWeaponEquipped {
                byte_length: 1,
                callback: 2,
                expected: entry(3),
            },
            time: NativeWeaponTime { rva: 4 },
            think: NativeWeaponThink {
                tag: 5,
                registration: layout,
            },
            allocate: NativeWeaponAllocate { entry: entry(6) },
            free: NativeWeaponFree { entry: entry(7) },
            projectile_touch: entry(8),
            equip: NativeWeaponCalls { calls: Vec::new() },
            launch: NativeWeaponCalls { calls: Vec::new() },
            activate_rva: Some(10),
            fire_rva: 11,
            initialization_classes: Vec::new(),
            equipment: Vec::new(),
            ammunition: NativeWeaponCommand {
                arguments: Vec::new(),
                tail: String::new(),
            },
            initial_cvars: Vec::new(),
            provisioning_cvars: Vec::new(),
        }
    }

    #[test]
    fn help_prints_tool_help() {
        let mut host = stub_host();
        let mut printed = Vec::new();
        run_weapon_behavior_tool(&WeaponBehaviorToolCommand::Help, &mut host, &mut |text| {
            printed.push(text)
        })
        .unwrap();
        assert_eq!(printed, vec![WEAPON_BEHAVIOR_TOOL_HELP.to_owned()]);
    }

    #[test]
    fn roots_cover_every_action() {
        let command = inspect_command();
        let roots = tool_roots(&command).unwrap();
        assert_eq!(roots.product, "p");
        assert_eq!(roots.artifact, Some("progs.dat"));
        assert!(tool_roots(&WeaponBehaviorToolCommand::Help).is_none());
        let declare = WeaponBehaviorToolCommand::Declare {
            product: "p".to_owned(),
            corpus_root: "/c".to_owned(),
            user_content_root: "/u".to_owned(),
            artifact: None,
            id: "qc:x".to_owned(),
            title: "X".to_owned(),
            role: ToolProjectileRole::Rocket,
            fire: "f".to_owned(),
            activate: None,
        };
        assert!(tool_roots(&declare).unwrap().artifact.is_none());
    }

    #[test]
    fn declare_action_names_match_donor() {
        let qvm = WeaponBehaviorToolCommand::DeclareQvm {
            product: String::new(),
            corpus_root: String::new(),
            user_content_root: String::new(),
            artifact: None,
            profile: "p.json".to_owned(),
        };
        assert_eq!(declare_action_name(&qvm), Some("declare-qvm"));
        let native = WeaponBehaviorToolCommand::DeclareNative {
            product: String::new(),
            corpus_root: String::new(),
            user_content_root: String::new(),
            artifact: None,
            profile: "p.json".to_owned(),
        };
        assert_eq!(declare_action_name(&native), Some("declare-native"));
        assert_eq!(declare_action_name(&inspect_command()), None);
        assert_eq!(declare_action_name(&WeaponBehaviorToolCommand::Help), None);
    }

    #[test]
    fn q2_requires_rerelease() {
        reject_non_rerelease_q2(&GameFamily::Q2, "rerelease").unwrap();
        reject_non_rerelease_q2(&GameFamily::Q1, "classic").unwrap();
        reject_non_rerelease_q2(&GameFamily::Q3, "classic").unwrap();
        let error = reject_non_rerelease_q2(&GameFamily::Q2, "classic").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Native weapon declarations currently require the Q2 rerelease API2023 Windows x64 adapter"
        );
    }

    #[test]
    fn document_values_convert_to_output_json() {
        let value = obj(vec![
            ("name", save_str("rail")),
            ("count", num(3.0)),
            ("tags", arr(vec![save_str("a"), save_str("b")])),
        ]);
        let json = save_json_to_json(&value).unwrap();
        assert_eq!(json.get("name"), Some(&Json::String("rail".to_owned())));
        assert_eq!(json.get("count"), Some(&Json::Number(3.0)));
        assert_eq!(save_json_to_json(&SaveJson::BigInt(42)).unwrap(), Json::Number(42.0));
        assert!(save_json_to_json(&SaveJson::BigInt(1 << 60)).is_err());
        assert!(save_json_to_json(&SaveJson::Bytes(vec![1])).is_err());
    }

    #[test]
    fn guest_digest_formats_with_algorithm() {
        let digest = GuestContentDigest::new("sha256", "ab");
        assert_eq!(guest_digest_to_content(&digest).as_str(), "sha256:ab");
    }

    #[test]
    fn tool_roles_map_to_contract_roles() {
        assert_eq!(contract_role(ToolProjectileRole::Grapple), ProjectileRole::Grapple);
        assert_eq!(contract_role(ToolProjectileRole::Nail), ProjectileRole::Nail);
    }

    #[test]
    fn behavior_document_writes_atomically() {
        let directory = std::env::temp_dir().join(format!("weapon-behavior-tool-{}", std::process::id()));
        let nested = directory.join("nested");
        let destination = write_behavior_document(&nested, "weapon-behaviors.json", "{\"a\":1}\n").unwrap();
        assert_eq!(destination, nested.join("weapon-behaviors.json"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "{\"a\":1}\n");
        write_behavior_document(&nested, "weapon-behaviors.json", "{\"a\":2}\n").unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "{\"a\":2}\n");
        let leftovers: Vec<_> = fs::read_dir(&nested)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect();
        assert_eq!(leftovers, vec!["weapon-behaviors.json".to_string()]);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn saved_receipt_matches_donor() {
        let mut printed = Vec::new();
        print_saved(
            &mut |text| printed.push(text),
            "qc:rail",
            Path::new("/user/id1/weapon-behaviors.json"),
            "q1-classic-id1",
        );
        assert_eq!(
            printed,
            vec![
                "Saved qc:rail to /user/id1/weapon-behaviors.json\nSelect with --weapon-behavior q1-classic-id1/qc:rail\n"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn callback_filter_matches_donor() {
        assert!(is_inspectable_callback(1, 0, 0));
        assert!(!is_inspectable_callback(0, 0, 0));
        assert!(!is_inspectable_callback(1, -1, 0));
        assert!(!is_inspectable_callback(1, 5, 1));
    }

    #[test]
    fn qc_inspect_report_lists_callbacks_and_thinks() {
        let report = qc_inspect_json(
            "q1-classic-id1",
            "progs.dat",
            "sha256:ab",
            vec![("fire_rail".to_owned(), 7)],
            vec![(None, Some("think".to_owned()), 12)],
        );
        assert_eq!(report.get("product"), Some(&Json::String("q1-classic-id1".to_owned())));
        let Json::Array(callbacks) = report.get("callbacks").unwrap() else {
            panic!("expected callbacks array");
        };
        assert_eq!(callbacks.len(), 1);
        assert_eq!(callbacks[0].get("index"), Some(&Json::Number(7.0)));
        let Json::Array(assigns) = report.get("thinkAssignments").unwrap() else {
            panic!("expected thinkAssignments array");
        };
        assert_eq!(assigns[0].get("producer"), Some(&Json::Null));
        assert_eq!(assigns[0].get("statement"), Some(&Json::Number(12.0)));
    }

    #[test]
    fn qc_metadata_omits_missing_activation() {
        let metadata = SourceWeaponBehaviorMetadata {
            id: "qc:rail".to_owned(),
            title: "Rail".to_owned(),
            artifact_digest: ContentDigest("sha256:ab".to_owned()),
            role: ProjectileRole::Bolt,
            fire_function: "fire_rail".to_owned(),
            activation_function: None,
        };
        let json = qc_metadata_json(&metadata);
        assert_eq!(json.get("role"), Some(&Json::String("bolt".to_owned())));
        assert_eq!(json.get("activationFunction"), None);
        let metadata = SourceWeaponBehaviorMetadata {
            activation_function: Some("gate".to_owned()),
            ..metadata
        };
        let json = qc_metadata_json(&metadata);
        assert_eq!(json.get("activationFunction"), Some(&Json::String("gate".to_owned())));
    }

    #[test]
    fn qc_declare_appends_declaration() {
        let json = qc_declare_json(
            "progs.dat",
            vec![Json::String("old".to_owned())],
            Json::String("new".to_owned()),
        );
        assert_eq!(json.get("version"), Some(&Json::Number(1.0)));
        let Json::Array(behaviors) = json.get("behaviors").unwrap() else {
            panic!("expected behaviors array");
        };
        assert_eq!(behaviors.len(), 2);
    }

    #[test]
    fn qvm_inspect_report_counts_data_bytes() {
        let report = qvm_inspect_json(
            "q3-base",
            "vm/qagame.qvm",
            "sha256:cd",
            10,
            20,
            vec![3],
            vec![(
                "qvm:rail".to_owned(),
                "vm/qagame.qvm".to_owned(),
                "sha256:cd".to_owned(),
            )],
        );
        assert_eq!(report.get("instructionCount"), Some(&Json::Number(10.0)));
        assert_eq!(report.get("dataBytes"), Some(&Json::Number(20.0)));
        let Json::Array(entries) = report.get("entries").unwrap() else {
            panic!("expected entries array");
        };
        assert_eq!(entries[0].get("instructionIndex"), Some(&Json::Number(3.0)));
        let Json::Array(declared) = report.get("declared").unwrap() else {
            panic!("expected declared array");
        };
        assert_eq!(declared[0].get("id"), Some(&Json::String("qvm:rail".to_owned())));
    }

    #[test]
    fn qvm_profiles_wrap_versioned_list() {
        let json = qvm_profiles_json(vec![Json::Null]);
        assert_eq!(json.get("version"), Some(&Json::Number(1.0)));
        let Json::Array(profiles) = json.get("profiles").unwrap() else {
            panic!("expected profiles array");
        };
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn author_profile_requires_utf8_json() {
        let value = parse_author_profile("p.json", b"{\"id\":\"x\"}").unwrap();
        assert_eq!(value.get("id"), Some(&SaveJson::String("x".to_owned())));
        assert!(parse_author_profile("p.json", &[0xff, 0xfe]).is_err());
        assert!(parse_author_profile("p.json", b"{oops").is_err());
    }

    #[test]
    fn native_inspect_report_lists_sections() {
        let report = native_inspect_json(
            "q2-rerelease",
            "game_x64.dll",
            "sha256:ef",
            "windows-x86-64",
            4096,
            vec![(".text".to_owned(), 4096, 8192, "read-execute".to_owned())],
            vec![Json::Null],
        );
        assert_eq!(report.get("abi"), Some(&Json::String("windows-x86-64".to_owned())));
        assert_eq!(report.get("entryPointRva"), Some(&Json::Number(4096.0)));
        let Json::Array(sections) = report.get("sections").unwrap() else {
            panic!("expected sections array");
        };
        assert_eq!(sections[0].get("byteLength"), Some(&Json::Number(8192.0)));
    }

    #[test]
    fn native_profiles_append_serialized_declaration() {
        let json = native_profiles_json(&[], &declaration());
        let Json::Array(profiles) = json.get("profiles").unwrap() else {
            panic!("expected profiles array");
        };
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].get("activateRva"), Some(&Json::Number(10.0)));
        assert_eq!(profiles[0].get("abi"), Some(&Json::String("windows-x86-64".to_owned())));
    }
}
