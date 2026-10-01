//! Gameplay and weapon-behavior mod choices, application, and preparation.
//!
//! Donor provenance: `src/app/bootstrap/mod-selection.ts`
//! (`ApplicationModChoice`, `applicationModChoices`,
//! `applyApplicationMods`, `prepareApplicationMods`). Choice discovery,
//! selection-set application, and declaration validation are a direct
//! port over the workspace's mods, catalog, and mount siblings.
//! Weapon-behavior choices (`weapon-behavior-selection.ts`), QuakeC
//! declaration reading, and mod preparation (`simulation/quakec-mod.ts`,
//! `simulation/qvm-mod.ts`, `simulation/native-mod.ts`) arrive through
//! caller hooks (those donors belong to other lanes).

use qa_content::catalog::{CatalogProduct, InstalledCatalog, ProductAvailability};
use qa_content::contract::{
    create_mount_plan_id, mod_selection_key, read_mod_selection, ContentId, ModAvailability, ModDeclaration,
    ModDescription, ModPurpose, ModSelection, ProviderReference, ResolvedGameplayMod, ResolvedMountPlan,
    ResolvedWeaponBehaviorSelection,
};
use qa_content::mods::{discover_gameplay_mods, DiscoveredGameplayMod, ModSelectionSet, ModsError};
use qa_content::mounts::{MountError, MountPreparationScope, MountedContent, OpenMountOptions};
use qa_content::value::SaveReader;
use std::collections::HashSet;
use thiserror::Error;

/// Failure of mod selection or preparation.
#[derive(Debug, Error)]
pub enum ModSelectionError {
    /// Selection or preparation failure.
    #[error("{0}")]
    Mod(String),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] qa_content::catalog::CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Mods failure.
    #[error(transparent)]
    Mods(#[from] ModsError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] qa_content::contract::ContractError),
}

/// One application mod choice.
#[derive(Debug, Clone)]
pub struct ApplicationModChoice {
    /// Choice description.
    pub description: ModDescription,
    /// Choice implementation.
    pub implementation: ModChoiceImplementation,
}

/// Mod choice implementation.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ModChoiceImplementation {
    /// Unavailable choice.
    Unavailable,
    /// Gameplay mod.
    Gameplay {
        /// Resolved selection.
        selection: ResolvedGameplayMod,
    },
    /// Weapon behavior.
    WeaponBehavior {
        /// Resolved selection.
        selection: ResolvedWeaponBehaviorSelection,
    },
}

/// One weapon-behavior choice entry (`weapon-behavior-selection.ts` shape).
#[derive(Debug, Clone)]
pub struct WeaponBehaviorChoiceEntry {
    /// Choice id.
    pub id: String,
    /// Choice title.
    pub title: String,
    /// Resolved selection, when resolvable.
    pub selection: Option<ResolvedWeaponBehaviorSelection>,
    /// Unavailability reason, when unavailable.
    pub unavailable: Option<String>,
}

/// QuakeC declaration reader for gameplay discovery.
pub type QuakecDeclarationReader =
    dyn Fn(SaveReader) -> Result<qa_content::contract::ModCallbackDeclaration, ModsError>;

fn source_title(product: &CatalogProduct) -> String {
    let family = match product.expectation.family {
        qa_content::contract::GameFamily::Q1 => "Quake",
        qa_content::contract::GameFamily::Q2 => "Quake II",
        qa_content::contract::GameFamily::Q3 => "Quake III",
    };
    if product.expectation.edition == "rerelease" {
        format!("{family} rerelease")
    } else {
        family.to_owned()
    }
}

/// Discover installed gameplay and weapon-behavior choices.
pub fn application_mod_choices(
    catalog: &InstalledCatalog,
    weapon_choices: &dyn Fn(&str) -> Result<Vec<WeaponBehaviorChoiceEntry>, ModSelectionError>,
    read_quakec: &QuakecDeclarationReader,
) -> Result<Vec<ApplicationModChoice>, ModSelectionError> {
    let scope = MountPreparationScope::new();
    let mut choices = Vec::new();
    for product in &catalog.products {
        if product.availability != ProductAvailability::Installed {
            continue;
        }
        let source_title = source_title(product);
        let discovered: Result<Vec<DiscoveredGameplayMod>, ModSelectionError> = (|| {
            let content_mounts = catalog.mounts_for(product.id.as_str())?;
            let hex: String = product.id.as_str().bytes().map(|byte| format!("{byte:02x}")).collect();
            let plan = ResolvedMountPlan {
                id: create_mount_plan_id("gameplay-mods", &hex)?,
                mounts: content_mounts.clone(),
                default_order: content_mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
                prefix_orders: Vec::new(),
            };
            let opened = scope.open(
                &plan,
                OpenMountOptions {
                    pure: None,
                    q3_restriction: None,
                    links: Vec::new(),
                    loose_comparison: None,
                },
            )?;
            Ok(discover_gameplay_mods(product, &opened, read_quakec)?)
        })();
        match discovered {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        DiscoveredGameplayMod::Available(resolved) => {
                            choices.push(ApplicationModChoice {
                                description: ModDescription {
                                    selection: resolved.selection.clone(),
                                    source: resolved.source.clone(),
                                    title: resolved.title.clone(),
                                    source_title: source_title.clone(),
                                    purpose: ModPurpose::Addition,
                                    requires: resolved.requires.clone(),
                                    conflicts: resolved.conflicts.clone(),
                                    availability: ModAvailability::Available,
                                },
                                implementation: ModChoiceImplementation::Gameplay { selection: resolved },
                            });
                        }
                        DiscoveredGameplayMod::Unavailable(description) => {
                            choices.push(ApplicationModChoice {
                                description,
                                implementation: ModChoiceImplementation::Unavailable,
                            });
                        }
                    }
                }
            }
            Err(error) => {
                choices.push(ApplicationModChoice {
                    description: ModDescription {
                        selection: ModSelection {
                            product: product.expectation.id.clone(),
                            id: "unavailable".to_owned(),
                        },
                        source: ProviderReference {
                            provider: qa_core::identity::ProviderId::new(
                                &product.expectation.family.to_string(),
                                "official",
                            ),
                            content: product.id.clone(),
                        },
                        title: product.expectation.title.clone(),
                        source_title: source_title.clone(),
                        purpose: ModPurpose::Addition,
                        requires: Vec::new(),
                        conflicts: Vec::new(),
                        availability: ModAvailability::Unavailable {
                            reason: error.to_string(),
                        },
                    },
                    implementation: ModChoiceImplementation::Unavailable,
                });
            }
        }
        match weapon_choices(&product.expectation.id) {
            Ok(entries) => {
                for entry in entries {
                    let Some(selection) = entry.selection else {
                        continue;
                    };
                    choices.push(ApplicationModChoice {
                        description: ModDescription {
                            selection: read_mod_selection(&entry.id)?,
                            source: selection.source.clone(),
                            title: entry.title,
                            source_title: source_title.clone(),
                            purpose: ModPurpose::Addition,
                            requires: Vec::new(),
                            conflicts: Vec::new(),
                            availability: match entry.unavailable {
                                None => ModAvailability::Available,
                                Some(reason) => ModAvailability::Unavailable { reason },
                            },
                        },
                        implementation: ModChoiceImplementation::WeaponBehavior { selection },
                    });
                }
            }
            Err(error) => {
                let mut id = "unavailable-weapon-behaviors".to_owned();
                while choices.iter().any(|choice: &ApplicationModChoice| {
                    choice.description.selection.product == product.expectation.id
                        && choice.description.selection.id == id
                }) {
                    id.push('+');
                }
                choices.push(ApplicationModChoice {
                    description: ModDescription {
                        selection: ModSelection {
                            product: product.expectation.id.clone(),
                            id,
                        },
                        source: ProviderReference {
                            provider: qa_core::identity::ProviderId::new(
                                &product.expectation.family.to_string(),
                                "official",
                            ),
                            content: product.id.clone(),
                        },
                        title: format!("{} weapon behaviors", product.expectation.title),
                        source_title: source_title.clone(),
                        purpose: ModPurpose::Addition,
                        requires: Vec::new(),
                        conflicts: Vec::new(),
                        availability: ModAvailability::Unavailable {
                            reason: error.to_string(),
                        },
                    },
                    implementation: ModChoiceImplementation::Unavailable,
                });
            }
        }
    }
    // Each trajectory adapter owns that launcher's path. Other gameplay registrations
    // compose independently through SessionMods rather than competing for this owner.
    let mut conflicts: Vec<Vec<ModSelection>> = vec![Vec::new(); choices.len()];
    for (index, choice) in choices.iter().enumerate() {
        let ModChoiceImplementation::WeaponBehavior { selection } = &choice.implementation else {
            continue;
        };
        for (other_index, other) in choices.iter().enumerate() {
            if other_index == index {
                continue;
            }
            if let ModChoiceImplementation::WeaponBehavior {
                selection: other_selection,
            } = &other.implementation
            {
                if other_selection.definition.role == selection.definition.role
                    || other_selection.definition.id == selection.definition.id
                {
                    conflicts[index].push(other.description.selection.clone());
                }
            }
        }
    }
    for (choice, extra) in choices.iter_mut().zip(conflicts) {
        choice.description.conflicts.extend(extra);
    }
    Ok(choices)
}

/// Apply enabled choices to a recipe.
pub fn apply_application_mods(
    recipe: &qa_content::contract::ExecutableRecipe,
    choices: &[ApplicationModChoice],
    enabled: &[ModSelection],
) -> Result<qa_content::contract::ExecutableRecipe, ModSelectionError> {
    let selection = ModSelectionSet::new(
        choices.iter().map(|choice| choice.description.clone()).collect(),
        enabled.to_vec(),
    )?;
    let mut selected = Vec::new();
    for enabled in selection.enabled() {
        let choice = choices
            .iter()
            .find(|choice| mod_selection_key(&choice.description.selection).ok() == mod_selection_key(enabled).ok())
            .ok_or_else(|| {
                ModSelectionError::Mod(format!(
                    "Selected mod is unavailable: {}",
                    mod_selection_key(enabled).unwrap_or_else(|_| "?".to_owned())
                ))
            })?;
        selected.push(choice);
    }
    if selected.is_empty() {
        return Ok(recipe.clone());
    }
    if selected
        .iter()
        .any(|choice| matches!(choice.implementation, ModChoiceImplementation::WeaponBehavior { .. }))
        && recipe.execution.iter().any(|module| {
            !matches!(module, qa_content::contract::ExecutionModule::Typescript { .. })
                && match module {
                    qa_content::contract::ExecutionModule::Typescript { role, .. }
                    | qa_content::contract::ExecutionModule::Qvm { role, .. }
                    | qa_content::contract::ExecutionModule::Native { role, .. } => {
                        *role == qa_content::contract::ModuleRole::ServerGame
                    }
                    qa_content::contract::ExecutionModule::Quakec { .. } => false,
                }
        })
    {
        return Err(ModSelectionError::Mod(
            "Selected projectile components require a shared weapon provider".to_owned(),
        ));
    }
    let mut recipe = recipe.clone();
    recipe.mods = selected
        .iter()
        .filter_map(|choice| match &choice.implementation {
            ModChoiceImplementation::Gameplay { selection } => Some(selection.clone()),
            _ => None,
        })
        .collect();
    let mut behaviors = Vec::new();
    for choice in &selected {
        match &choice.implementation {
            ModChoiceImplementation::Unavailable => {
                return Err(ModSelectionError::Mod(format!(
                    "Selected mod is unavailable: {}",
                    mod_selection_key(&choice.description.selection)?
                )));
            }
            ModChoiceImplementation::WeaponBehavior { selection } => {
                behaviors.push(selection.clone());
            }
            ModChoiceImplementation::Gameplay { .. } => {}
        }
    }
    recipe.weapon_behaviors = behaviors;
    Ok(recipe)
}

/// Preparation purpose: gameplay or presentation-only QVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModPreparePurpose {
    /// Prepare every selected mod.
    Gameplay,
    /// Prepare only QVM mods with presentations.
    Presentation,
}

/// One mod preparation request.
pub struct ModPrepareRequest<'a> {
    /// Mod description.
    pub description: ModDescription,
    /// Mod declaration.
    pub declaration: ModDeclaration,
    /// Declaration digest.
    pub declaration_digest: qa_content::contract::ContentDigest,
    /// Mounted content.
    pub mounts: &'a MountedContent,
}

/// Gameplay mod preparation (`simulation/quakec-mod.ts`, `qvm-mod.ts`, `native-mod.ts`).
pub trait GameplayModPreparer {
    /// Prepared mod.
    type Prepared;
    /// Prepare a QuakeC mod.
    fn prepare_quakec(&mut self, request: ModPrepareRequest<'_>) -> Result<Self::Prepared, ModSelectionError>;
    /// Prepare a QVM mod.
    fn prepare_qvm(&mut self, request: ModPrepareRequest<'_>) -> Result<Self::Prepared, ModSelectionError>;
    /// Prepare a native mod.
    fn prepare_native(&mut self, request: ModPrepareRequest<'_>) -> Result<Self::Prepared, ModSelectionError>;
}

/// Validate selections against installed declarations and prepare them.
pub fn prepare_application_mods<P: GameplayModPreparer>(
    catalog: &InstalledCatalog,
    recipe: &qa_content::contract::ExecutableRecipe,
    for_content: &dyn Fn(&ContentId) -> Result<MountedContent, ModSelectionError>,
    purpose: ModPreparePurpose,
    preparer: &mut P,
    read_quakec: &QuakecDeclarationReader,
) -> Result<Vec<P::Prepared>, ModSelectionError> {
    let mut prepared = Vec::new();
    let selections = &recipe.mods;
    let dependencies: HashSet<String> = selections
        .iter()
        .map(|selection| mod_selection_key(&selection.selection))
        .collect::<Result<_, _>>()?;
    let mut enabled = dependencies.clone();
    for selection in &recipe.weapon_behaviors {
        enabled.insert(mod_selection_key(&ModSelection {
            product: catalog
                .product(selection.source.content.as_str())?
                .expectation
                .id
                .clone(),
            id: selection.definition.id.clone(),
        })?);
    }
    for selection in selections {
        let product = catalog.require(&selection.selection.product)?;
        let mounted = for_content(&product.id)?;
        let declarations = discover_gameplay_mods(product, &mounted, read_quakec)?;
        let entry = declarations.iter().find(|entry| {
            let candidate = match entry {
                DiscoveredGameplayMod::Available(resolved) => &resolved.selection,
                DiscoveredGameplayMod::Unavailable(description) => &description.selection,
            };
            mod_selection_key(candidate).ok() == mod_selection_key(&selection.selection).ok()
        });
        if let Some(DiscoveredGameplayMod::Unavailable(description)) = entry {
            let reason = match &description.availability {
                ModAvailability::Unavailable { reason } => reason.clone(),
                ModAvailability::Available => "unavailable".to_owned(),
            };
            return Err(ModSelectionError::Mod(format!("{}: {reason}", description.title)));
        }
        let current = match entry {
            Some(DiscoveredGameplayMod::Available(resolved)) => Some(resolved),
            _ => None,
        };
        let Some(current) = current else {
            return Err(ModSelectionError::Mod(format!(
                "Selected mod differs from its installed declaration: {}",
                mod_selection_key(&selection.selection)?
            )));
        };
        if current.source.content != selection.source.content
            || current.source.provider != selection.source.provider
            || current.declaration_digest != selection.declaration_digest
            || current.declaration != selection.declaration
        {
            return Err(ModSelectionError::Mod(format!(
                "Selected mod differs from its installed declaration: {}",
                mod_selection_key(&selection.selection)?
            )));
        }
        for dependency in &current.requires {
            if !enabled.contains(&mod_selection_key(dependency)?) {
                return Err(ModSelectionError::Mod(format!(
                    "{} requires {}",
                    current.title,
                    mod_selection_key(dependency)?
                )));
            }
        }
        for conflict in &current.conflicts {
            if enabled.contains(&mod_selection_key(conflict)?) {
                return Err(ModSelectionError::Mod(format!(
                    "{} conflicts with {}",
                    current.title,
                    mod_selection_key(conflict)?
                )));
            }
        }
        let description = ModDescription {
            selection: current.selection.clone(),
            source: current.source.clone(),
            title: current.title.clone(),
            source_title: current.source_title.clone(),
            purpose: ModPurpose::Addition,
            // Projectile components initialize before the general operation registrations.
            requires: current
                .requires
                .iter()
                .filter(|dependency| {
                    mod_selection_key(dependency)
                        .ok()
                        .is_some_and(|key| dependencies.contains(&key))
                })
                .cloned()
                .collect(),
            conflicts: current.conflicts.clone(),
            availability: ModAvailability::Available,
        };
        if purpose == ModPreparePurpose::Presentation
            && !matches!(
                &current.declaration,
                ModDeclaration::Qvm(declaration) if declaration.presentation.is_some()
            )
        {
            continue;
        }
        let request = ModPrepareRequest {
            description,
            declaration: current.declaration.clone(),
            declaration_digest: current.declaration_digest.clone(),
            mounts: &mounted,
        };
        match &current.declaration {
            ModDeclaration::Quakec(_) => prepared.push(preparer.prepare_quakec(request)?),
            ModDeclaration::Qvm(_) => prepared.push(preparer.prepare_qvm(request)?),
            ModDeclaration::Native(_) => prepared.push(preparer.prepare_native(request)?),
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{
        CampaignSelection, CharacterSelection, ContentDigest, DopplerSelection, EnemySelection, EnvironmentSelection,
        EquipmentSelection, ExecutableRecipe, FrameOrdering, GrappleSelection, HandGrenadeSelection, LooseMount,
        MountId, MountIdentity, MountPlanId, PresentationSelection, RecipeId, ResolvedMap, ResolvedResourceReference,
        ResourceId, ResourceProvenance, ResourceResolution,
    };

    fn provider(content: &str) -> ProviderReference {
        ProviderReference {
            provider: qa_core::identity::ProviderId::new("q1", "test"),
            content: ContentId(content.to_owned()),
        }
    }

    fn recipe() -> ExecutableRecipe {
        let geometry = ResolvedResourceReference {
            id: ResourceId("resource:maps/e1m1.bsp".to_owned()),
            requested_path: "maps/e1m1.bsp".to_owned(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_owned()),
                        content: ContentId("q1".to_owned()),
                        generation: 1,
                    },
                    root_path: "/corpus".to_owned(),
                },
                member_path: "maps/e1m1.bsp".to_owned(),
            },
            digest: ContentDigest("sha256:00".to_owned()),
            byte_length: 0,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:1".to_owned()),
                rank: 0,
            },
        };
        ExecutableRecipe {
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            schema_version: 3,
            id: RecipeId("recipe:test:1".to_owned()),
            preset: RecipeId("recipe:test:1".to_owned()),
            map: ResolvedMap {
                geometry_content: ContentId("q1".to_owned()),
                geometry: geometry.clone(),
                entities: provider("q1"),
            },
            campaign: CampaignSelection::None,
            movement: provider("q1"),
            character: CharacterSelection {
                definition: provider("q1"),
                appearance: provider("q1"),
            },
            weapons: Vec::new(),
            equipment: EquipmentSelection {
                grapple: GrappleSelection::Disabled,
                hand_grenades: HandGrenadeSelection::Disabled,
            },
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Source,
                environment: EnvironmentSelection::AudioContent,
                assets: ContentId("q1".to_owned()),
                hud: provider("q1"),
                effects: provider("q1"),
                audio: provider("q1"),
            },
            engine_behavior: provider("q1"),
            combat: provider("q1"),
            inventory: provider("q1"),
            r#match: provider("q1"),
            transition: provider("q1"),
            execution: Vec::new(),
            mounts: ResolvedMountPlan {
                id: MountPlanId("mount-plan:test:1".to_owned()),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            resources: vec![geometry],
            timing: Vec::new(),
            ordering: FrameOrdering::Mixed { providers: Vec::new() },
        }
    }

    fn description(product: &str, id: &str) -> ModDescription {
        ModDescription {
            selection: ModSelection {
                product: product.to_owned(),
                id: id.to_owned(),
            },
            source: ProviderReference {
                provider: qa_core::identity::ProviderId::new("q1", "test"),
                content: qa_content::contract::ContentId("q1".to_owned()),
            },
            title: "Test".to_owned(),
            source_title: "Quake".to_owned(),
            purpose: ModPurpose::Addition,
            requires: Vec::new(),
            conflicts: Vec::new(),
            availability: ModAvailability::Available,
        }
    }

    #[test]
    fn empty_selection_returns_recipe_unchanged() {
        let recipe = recipe();
        let applied = apply_application_mods(&recipe, &[], &[]).expect("apply");
        assert_eq!(applied.mods, recipe.mods);
        assert_eq!(applied.weapon_behaviors, recipe.weapon_behaviors);
    }

    #[test]
    fn unknown_enabled_selection_errors() {
        let recipe = recipe();
        let enabled = [ModSelection {
            product: "id1".to_owned(),
            id: "ghost".to_owned(),
        }];
        let result = apply_application_mods(&recipe, &[], &enabled);
        assert!(result.is_err());
    }

    #[test]
    fn unavailable_choice_cannot_apply() {
        let recipe = recipe();
        let choice = ApplicationModChoice {
            description: description("id1", "broken"),
            implementation: ModChoiceImplementation::Unavailable,
        };
        let enabled = [ModSelection {
            product: "id1".to_owned(),
            id: "broken".to_owned(),
        }];
        let err = apply_application_mods(&recipe, &[choice], &enabled).expect_err("unavailable");
        assert!(err.to_string().contains("Selected mod is unavailable"));
    }
}
