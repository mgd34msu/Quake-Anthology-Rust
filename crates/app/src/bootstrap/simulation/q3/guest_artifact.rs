//! Q3 guest artifact preparation: recipe checks plus bytecode loading.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/guest-artifact.ts`
//! (`assertQ3GuestRecipe`, `prepareQ3Game`, `PreparedQ3Game`,
//! `Q3GameExecution`).
//!
//! The donor is async; the Rust QVM port is fully synchronous, so preparation
//! is a plain function. Stock primary profiles (donor `compat/qvm/*`
//! builtins) have no Rust home, so callers inject them through
//! [`BuiltinPrimarySource`] (missing value, never duplicated here). The
//! `qa-guest` crate carries parallel QVM implementations; this module bridges
//! them mechanically (field-for-field projections, compiler-verified opcode
//! maps) instead of reimplementing any of them.

use std::cell::RefCell;

use qa_content::contract::{
    CampaignSelection, CharacterSelection, EnemySelection, ExecutableRecipe, ModuleRole, ProviderReference,
    Q3ApiIdentity, ResolvedExecutionModule, ResolvedResourceReference, ResourceProvenance,
};
use qa_content::mounts::{digest_bytes, MountedContent, ResourceRef};
use qa_content::q3::guest_items::{q3_guest_weapons, Q3GuestWeapon};
use qa_content::value::{parse_save_json, SaveJson as ContentSaveJson};
use qa_core::identity::ProviderId;
use qa_core::time::ClockProfile;
use qa_guest::core::contracts::{ContentDigest as GuestDigest, ModuleIdentity as GuestModuleIdentity};
use qa_guest::error::GuestError;
use qa_guest::qvm::artifacts::{resolve_qvm_artifact, ResolvedQvmArtifact};
use qa_guest::qvm::compatibility::{read_qvm_compatibility_declaration, QvmCompatibilityFile, QvmCompatibilityMounts};
use qa_guest::qvm::game_data::{
    self, AbiProfile as GameAbiProfile, ModuleIdentity as GameModuleIdentity, ProfileReader as GameProfileReader,
    ProfileValue as GameProfileValue, QvmArtifact as GameArtifact, QvmImage as GameImage,
    QvmInstruction as GameInstruction, QvmOpcode as GameOpcode, QvmRole as GameRole,
};
use qa_guest::qvm::image::{QvmInstruction as DecodedInstruction, QvmOpcode as DecodedOpcode, QvmOperand};
use qa_guest::qvm::item_catalog::{parse_qvm_item_layout, QvmItemAddress, QvmItemCount, QvmItemLayout};
use qa_guest::qvm::mod_provider::{
    self, ModuleId as ProviderModuleId, ProfileReader as ProviderProfileReader, QvmAbi,
    QvmArtifact as ProviderArtifact, QvmImage as ProviderImage, QvmInstruction as ProviderInstruction,
    QvmOpcode as ProviderOpcode, QvmRole as ProviderRole,
};
use qa_guest::qvm::primary_pickup_profile::{
    ItemTableFields, QvmItemLayout as PickupItemLayout, TableAddress, TableCount,
};
use qa_guest::qvm::primary_player_profile::QvmWeaponCatalogRow;
use qa_guest::qvm::primary_profile::{
    builtin_qvm_primary_profile, read_qvm_primary_profile, BuiltinPrimarySource, QvmPrimaryProfile,
    ResolvedResourceReference as PrimaryResourceReference,
};
use qa_guest::qvm::syscalls::{QvmAbiProfile, QvmRole as SyscallRole};
use qa_world::save::value::SaveJson;

/// QVM server-game execution (donor `Q3GameExecution`).
pub type Q3GameExecution = ResolvedExecutionModule;

/// Prepared Q3 server game, mirroring donor `PreparedQ3Game`.
///
/// The artifact is the loaded game-data form the runtime consumes; the Rust
/// module options take the resolved form, which preparation validates and
/// then decodes.
#[derive(Debug, Clone)]
pub struct PreparedQ3Game {
    /// Source execution.
    pub execution: Q3GameExecution,
    /// Loaded qagame artifact.
    pub artifact: GameArtifact,
    /// Artifact resource.
    pub resource: ResolvedResourceReference,
    /// Guest weapon catalog.
    pub weapons: Vec<Q3GuestWeapon>,
    /// Declared item layout, if any.
    pub items: Option<QvmItemLayout>,
    /// Primary player profile.
    pub primary: QvmPrimaryProfile,
}

fn provider_name(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

fn execution_parts(execution: &Q3GameExecution) -> Option<(&ProviderReference, &ResolvedResourceReference, u8)> {
    match execution {
        ResolvedExecutionModule::Qvm {
            owner,
            artifact,
            role: ModuleRole::ServerGame,
            api: Q3ApiIdentity::Qagame(version),
        } => Some((owner, artifact, *version)),
        _ => None,
    }
}

/// Assert a recipe carries exactly one native Q3 server game with native
/// movement and character, map-defined actors, and supported providers.
pub fn assert_q3_guest_recipe(recipe: &ExecutableRecipe, execution: &Q3GameExecution) {
    fn fail() -> ! {
        panic!(
            "Q3 bytecode requires one native Q3 server game, native movement and character, \
             map-defined actors and supported source providers without campaign"
        )
    }
    let owner = &recipe.map.entities;
    let same = |reference: &ProviderReference| reference == owner;
    let Some((execution_owner, _, _)) = execution_parts(execution) else {
        fail()
    };
    if !same(execution_owner)
        || recipe.execution.len() != 1
        || recipe.execution.first().is_none_or(|first| first != execution)
        || owner.provider != ProviderId::new("q3", "official")
        || !matches!(recipe.campaign, CampaignSelection::None)
        || recipe.movement.provider != ProviderId::new("q3", "movement")
        || recipe.movement.content != owner.content
    {
        fail()
    }
    let CharacterSelection { definition, appearance } = &recipe.character;
    if definition.provider != ProviderId::new("q3", "character")
        || definition.content != owner.content
        || appearance.content != owner.content
        || appearance.provider.namespace != "q3"
        || !appearance.provider.name.starts_with("model/")
        || recipe.weapons.len() != 1
        || [
            &recipe.engine_behavior,
            &recipe.combat,
            &recipe.inventory,
            &recipe.r#match,
            &recipe.transition,
        ]
        .into_iter()
        .any(|reference| !same(reference))
        || !matches!(recipe.enemies, EnemySelection::MapDefined)
        || recipe
            .timing
            .iter()
            .find(|timing| timing.provider == owner.provider)
            .is_none_or(|timing| !matches!(timing.clock, ClockProfile::Q3 { .. }))
    {
        fail()
    }
}

/// Compatibility-mount bridge over mounted content.
struct CompatMounts<'a> {
    mounts: &'a MountedContent,
    reference: RefCell<Option<ResolvedResourceReference>>,
}

impl QvmCompatibilityMounts for CompatMounts<'_> {
    fn open(&self, path: &str) -> Result<Option<QvmCompatibilityFile>, GuestError> {
        let opened = self
            .mounts
            .open(path, |_| true)
            .map_err(|error| GuestError::invalid(error.to_string()))?;
        let Some(opened) = opened else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&opened.bytes);
        let document = parse_save_json(&text).map_err(|error| GuestError::invalid(error.to_string()))?;
        let reference = opened.reference.requested_path.clone();
        *self.reference.borrow_mut() = Some(opened.reference);
        Ok(Some(QvmCompatibilityFile {
            json: content_to_world(&document),
            reference,
        }))
    }
}

fn content_to_world(value: &ContentSaveJson) -> SaveJson {
    match value {
        ContentSaveJson::Null => SaveJson::Null,
        ContentSaveJson::Bool(value) => SaveJson::Bool(*value),
        ContentSaveJson::Number(value) => SaveJson::Number(*value),
        ContentSaveJson::BigInt(value) => SaveJson::BigInt(*value),
        ContentSaveJson::Bytes(value) => SaveJson::Bytes(value.clone()),
        ContentSaveJson::String(value) => SaveJson::String(value.clone()),
        ContentSaveJson::Array(items) => SaveJson::Array(items.iter().map(content_to_world).collect()),
        ContentSaveJson::Object(members) => SaveJson::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), content_to_world(value)))
                .collect(),
        ),
    }
}

macro_rules! opcode_converter {
    ($name:ident, $from:ty, $to:ty, [$($variant:ident),*]) => {
        fn $name(opcode: $from) -> $to {
            match opcode {
                $(<$from>::$variant => <$to>::$variant,)*
            }
        }
    };
}

opcode_converter!(
    game_opcode,
    DecodedOpcode,
    GameOpcode,
    [
        OpUndef,
        OpIgnore,
        OpBreak,
        OpEnter,
        OpLeave,
        OpCall,
        OpPush,
        OpPop,
        OpConst,
        OpLocal,
        OpJump,
        OpEq,
        OpNe,
        OpLti,
        OpLei,
        OpGti,
        OpGei,
        OpLtu,
        OpLeu,
        OpGtu,
        OpGeu,
        OpEqf,
        OpNef,
        OpLtf,
        OpLef,
        OpGtf,
        OpGef,
        OpLoad1,
        OpLoad2,
        OpLoad4,
        OpStore1,
        OpStore2,
        OpStore4,
        OpArg,
        OpBlockCopy,
        OpSex8,
        OpSex16,
        OpNegi,
        OpAdd,
        OpSub,
        OpDivi,
        OpDivu,
        OpModi,
        OpModu,
        OpMuli,
        OpMulu,
        OpBand,
        OpBor,
        OpBxor,
        OpBcom,
        OpLsh,
        OpRshi,
        OpRshu,
        OpNegf,
        OpAddf,
        OpSubf,
        OpDivf,
        OpMulf,
        OpCvif,
        OpCvfi
    ]
);

opcode_converter!(
    provider_opcode,
    DecodedOpcode,
    ProviderOpcode,
    [
        OpUndef,
        OpIgnore,
        OpBreak,
        OpEnter,
        OpLeave,
        OpCall,
        OpPush,
        OpPop,
        OpConst,
        OpLocal,
        OpJump,
        OpEq,
        OpNe,
        OpLti,
        OpLei,
        OpGti,
        OpGei,
        OpLtu,
        OpLeu,
        OpGtu,
        OpGeu,
        OpEqf,
        OpNef,
        OpLtf,
        OpLef,
        OpGtf,
        OpGef,
        OpLoad1,
        OpLoad2,
        OpLoad4,
        OpStore1,
        OpStore2,
        OpStore4,
        OpArg,
        OpBlockCopy,
        OpSex8,
        OpSex16,
        OpNegi,
        OpAdd,
        OpSub,
        OpDivi,
        OpDivu,
        OpModi,
        OpModu,
        OpMuli,
        OpMulu,
        OpBand,
        OpBor,
        OpBxor,
        OpBcom,
        OpLsh,
        OpRshi,
        OpRshu,
        OpNegf,
        OpAddf,
        OpSubf,
        OpDivf,
        OpMulf,
        OpCvif,
        OpCvfi
    ]
);

fn game_instruction(instruction: &DecodedInstruction) -> GameInstruction {
    let (operand, operand_width) = match instruction.operand {
        QvmOperand::None => (0, 0),
        QvmOperand::Word(word) => (word, 4),
        QvmOperand::Byte(byte) => (i32::from(byte), 1),
    };
    GameInstruction {
        opcode: game_opcode(instruction.opcode),
        operand,
        operand_width,
        byte_offset: instruction.byte_offset,
    }
}

fn provider_instruction(instruction: &DecodedInstruction) -> ProviderInstruction {
    let (operand, operand_width) = match instruction.operand {
        QvmOperand::None => (0, 0),
        QvmOperand::Word(word) => (word, 4),
        QvmOperand::Byte(byte) => (i32::from(byte), 1),
    };
    ProviderInstruction {
        opcode: provider_opcode(instruction.opcode),
        operand,
        operand_width,
    }
}

macro_rules! profile_converter {
    ($name:ident, $to:ty) => {
        fn $name(value: &SaveJson) -> $to {
            match value {
                SaveJson::Null => <$to>::Null,
                SaveJson::Bool(value) => <$to>::Bool(*value),
                SaveJson::Number(value) => {
                    if value.fract() == 0.0 && *value >= i64::MIN as f64 && *value <= i64::MAX as f64 {
                        <$to>::Int(*value as i64)
                    } else {
                        <$to>::Float(*value)
                    }
                }
                SaveJson::BigInt(value) => i64::try_from(*value).map_or(<$to>::Float(*value as f64), <$to>::Int),
                SaveJson::Bytes(value) => <$to>::Bytes(value.clone()),
                SaveJson::String(value) => <$to>::Str(value.clone()),
                SaveJson::Array(items) => <$to>::Array(items.iter().map($name).collect()),
                SaveJson::Object(members) => <$to>::Record(
                    members
                        .iter()
                        .map(|(key, value)| (key.clone(), $name(value)))
                        .collect(),
                ),
            }
        }
    };
}

profile_converter!(game_profile, game_data::ProfileValue);
profile_converter!(provider_profile, mod_provider::ProfileValue);

fn pickup_layout(layout: &QvmItemLayout) -> PickupItemLayout {
    PickupItemLayout {
        address: match layout.address {
            QvmItemAddress::Direct(address) => TableAddress::Direct(address),
            QvmItemAddress::Global(global) => TableAddress::Global { global },
        },
        count: match layout.count {
            QvmItemCount::Direct(count) => TableCount::Direct(count),
            QvmItemCount::Global { global, maximum } => TableCount::Global { global, maximum },
        },
        live: layout.live_source,
        stride: layout.stride,
        fields: ItemTableFields {
            class_name: layout.fields.class_name,
            pickup_name: layout.fields.pickup_name,
            type_: layout.fields.item_type,
            tag: layout.fields.tag,
        },
        weapon_type: layout.weapon_type as usize,
        ammo_type: layout.ammo_type as usize,
    }
}

/// Prepare a Q3 server game from its execution and mounts.
pub fn prepare_q3_game(
    execution: &Q3GameExecution,
    mounts: &MountedContent,
    primary_source: &dyn BuiltinPrimarySource,
) -> Result<PreparedQ3Game, GuestError> {
    let Some((owner, reference, api_version)) = execution_parts(execution) else {
        return Err(GuestError::invalid(
            "Selected Q3 server artifact must provide a supported qagame ABI",
        ));
    };
    if reference.requested_path.to_lowercase() != "vm/qagame.qvm" {
        return Err(GuestError::invalid(
            "Selected Q3 server artifact must provide a supported qagame ABI",
        ));
    }
    let bytes = mounts
        .read(ResourceRef::Resolved(reference))
        .map_err(|error| GuestError::invalid(error.to_string()))?;
    let content_digest = digest_bytes(&bytes);
    let bridge = CompatMounts {
        mounts,
        reference: RefCell::new(None),
    };
    let (declaration, _) = read_qvm_compatibility_declaration(
        &bridge,
        &reference.requested_path,
        content_digest.as_str(),
        SyscallRole::Qagame,
    )?;
    let modern = declaration.profile == QvmAbiProfile::Modern;
    if api_version != if modern { 8 } else { 7 } {
        return Err(GuestError::invalid(
            "Selected QVM ABI differs from its saved or resolved recipe",
        ));
    }
    let (algorithm, value) = content_digest
        .as_str()
        .split_once(':')
        .ok_or_else(|| GuestError::invalid("Selected Q3 server artifact must provide a supported qagame ABI"))?;
    let identity = match &reference.provenance {
        ResourceProvenance::Archive { mount, .. } => &mount.identity,
        ResourceProvenance::Loose { mount, .. } => &mount.identity,
    };
    let module = GuestModuleIdentity::new(
        owner.provider.clone(),
        &reference.requested_path,
        GuestDigest::new(algorithm, value),
        &format!("{}:{}", identity.id.as_str(), identity.generation),
    );
    let resolved = resolve_qvm_artifact(&module, SyscallRole::Qagame, &bytes, Vec::new(), declaration.profile)?;
    let ResolvedQvmArtifact::Bytecode { image, .. } = resolved else {
        return Err(GuestError::invalid(
            "Selected Q3 guest artifact did not resolve to bytecode",
        ));
    };
    let game_abi = if modern {
        GameAbiProfile::Modern
    } else {
        GameAbiProfile::Legacy
    };
    let artifact = GameArtifact {
        module: GameModuleIdentity {
            id: provider_name(&owner.provider),
            artifact_path: reference.requested_path.clone(),
            digest: content_digest.as_str().to_string(),
            revision: format!("{}:{}", identity.id.as_str(), identity.generation),
        },
        role: GameRole::Qagame,
        abi_profile: Some(game_abi),
        image: GameImage {
            source: reference.requested_path.clone(),
            instructions: image.instructions.iter().map(game_instruction).collect(),
            code_offset: image.code_offset,
            code_length: image.code_length,
            data_length: image.data_length,
            literal_length: image.literal_length,
            bss_length: image.bss_length,
            allocated_data_length: image.allocated_data_length,
            initialized_data: image.initialized_data.clone(),
            data_mask: image.data_mask as usize,
        },
    };
    let items = declaration
        .primary
        .as_ref()
        .map(|primary| {
            let fields = primary.get("items").map_or(GameProfileValue::Undefined, game_profile);
            parse_qvm_item_layout(&GameProfileReader::new(&fields))
        })
        .transpose()?;
    let private_inventory = declaration.primary.as_ref().is_some_and(|primary| {
        primary
            .get("inventory")
            .and_then(|inventory| inventory.get("storage"))
            .is_some()
    });
    let weapons = q3_guest_weapons(&artifact, mounts, items, private_inventory)?;
    let provider_artifact = ProviderArtifact {
        module: ProviderModuleId {
            id: provider_name(&owner.provider),
            artifact_path: reference.requested_path.clone(),
            digest: content_digest.as_str().to_string(),
            revision: format!("{}:{}", identity.id.as_str(), identity.generation),
        },
        role: ProviderRole::Qagame,
        abi_profile: Some(if modern { QvmAbi::Modern } else { QvmAbi::Legacy }),
        image: ProviderImage {
            instructions: image.instructions.iter().map(provider_instruction).collect(),
            data_length: image.data_length,
            literal_length: image.literal_length,
            bss_length: image.bss_length,
            initialized_length: image.initialized_data.len(),
            allocated_data_length: image.allocated_data_length,
        },
    };
    let rows: Vec<QvmWeaponCatalogRow> = weapons
        .iter()
        .map(|weapon| QvmWeaponCatalogRow {
            weapon: weapon.weapon,
            item: weapon.item.clone(),
        })
        .collect();
    let primary = match declaration.primary.as_ref() {
        None => builtin_qvm_primary_profile(primary_source, &provider_artifact, &rows),
        Some(primary) => {
            let Some(items) = items else {
                return Err(GuestError::invalid("primary interface lost its declaration resource"));
            };
            let Some(resource) = bridge.reference.borrow().clone() else {
                return Err(GuestError::invalid("primary interface lost its declaration resource"));
            };
            let catalog = (!items.live_source).then_some(rows.as_slice());
            let value = provider_profile(primary);
            let pickup = pickup_layout(&items);
            let declaration = PrimaryResourceReference {
                id: resource.id.0.clone(),
                requested_path: resource.requested_path.clone(),
                identity: resource.identity.canonical(),
                byte_length: resource.byte_length as usize,
            };
            read_qvm_primary_profile(
                &ProviderProfileReader::new(&value),
                &provider_artifact,
                catalog,
                &pickup,
                declaration,
            )?
        }
    };
    Ok(PreparedQ3Game {
        execution: execution.clone(),
        artifact,
        resource: reference.clone(),
        weapons,
        items,
        primary,
    })
}

#[cfg(test)]
mod tests {
    use qa_content::contract::ContentMount;
    use qa_content::contract::{
        CampaignSelection, CharacterSelection, ContentId, EnemySelection, EquipmentSelection, ExecutionModule,
        FrameOrdering, MountId, MountIdentity, MountPlanId, PresentationSelection, ProviderReference, ProviderTiming,
        Q3ApiIdentity, RecipeId, ResolvedMap, ResolvedMountPlan, ResolvedResourceReference, ResourceId,
        ResourceProvenance, ResourceResolution,
    };
    use qa_content::mounts::open_mount_plan;
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use qa_guest::qvm::mod_provider::{InputPointerKind, QvmModInputPointer, QvmRegionEvaluation};
    use qa_guest::qvm::mod_weapon_stage::{
        DispatcherHead, QvmItemCapacity, QvmItemField, QvmItemStorage, QvmWeaponActor, QvmWeaponDispatcherDefinition,
        SelectionValue, StageRequest, StageSelection,
    };
    use qa_guest::qvm::primary_inventory_profile::QvmInventoryProfile;
    use qa_guest::qvm::primary_pickup_profile::{FunctionCalls, GateProfile, PickupFields, QvmPickupProfile};
    use qa_guest::qvm::primary_player_profile::{
        ArmorDefinition, CombatCallbacks, CombatFields, CombatReactions, CombatState, CombatStateFlags,
        CombatTeamState, DamageFactor, DelayPlayer, DropAmmo, DropProfile, GiveProfile, InputEntries, NamedGrant,
        PowerupOffsets, QvmCombatCall, QvmCombatMass, QvmDamageFlags, QvmEquipmentMovementProfile, QvmInputDefinition,
        QvmPrimaryCombatProfile, QvmPrimaryWeaponProfile, QvmReactionCall, RegionRef, TeleportProfile, TorsoAnimation,
        WaterLevel, WeaponAvailability,
    };

    use super::*;

    fn provider(namespace: &str, name: &str) -> ProviderId {
        ProviderId::new(namespace, name)
    }

    fn reference(provider: ProviderId, content: &str) -> ProviderReference {
        ProviderReference {
            provider,
            content: ContentId(content.to_string()),
        }
    }

    fn owner() -> ProviderReference {
        reference(provider("q3", "official"), "q3:baseq3:q3dm1:1")
    }

    fn execution() -> Q3GameExecution {
        ExecutionModule::Qvm {
            owner: owner(),
            artifact: ResolvedResourceReference {
                id: ResourceId("resource:q3:qagame".to_string()),
                requested_path: "vm/qagame.qvm".to_string(),
                provenance: ResourceProvenance::Loose {
                    mount: qa_content::contract::LooseMount {
                        identity: MountIdentity {
                            id: MountId("mount:q3:base".to_string()),
                            content: ContentId("q3:baseq3".to_string()),
                            generation: 1,
                        },
                        root_path: "/tmp".to_string(),
                    },
                    member_path: "vm/qagame.qvm".to_string(),
                },
                identity: qa_content::contract::ResourceIdentity {
                    mount_generation: 1,
                    member_index: 0,
                    byte_length: 64,
                    crc: 0,
                },
                byte_length: 64,
                resolution: ResourceResolution::DefaultOrder {
                    plan: MountPlanId("mount-plan:q3:1".to_string()),
                    rank: 0,
                },
            },
            role: ModuleRole::ServerGame,
            api: Q3ApiIdentity::Qagame(8),
        }
    }

    fn recipe(execution: Q3GameExecution) -> ExecutableRecipe {
        ExecutableRecipe {
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            schema_version: 3,
            id: RecipeId("recipe:q3:test".to_string()),
            preset: RecipeId("recipe:q3:base".to_string()),
            map: ResolvedMap {
                geometry_content: ContentId("q3:baseq3".to_string()),
                geometry: match &execution {
                    ExecutionModule::Qvm { artifact, .. } => artifact.clone(),
                    _ => panic!("test execution"),
                },
                entities: owner(),
            },
            campaign: CampaignSelection::None,
            movement: reference(provider("q3", "movement"), "q3:baseq3:q3dm1:1"),
            character: CharacterSelection {
                definition: reference(provider("q3", "character"), "q3:baseq3:q3dm1:1"),
                appearance: reference(provider("q3", "model/sarge"), "q3:baseq3:q3dm1:1"),
            },
            weapons: vec![reference(provider("q3", "weapons"), "q3:baseq3:q3dm1:1")],
            equipment: EquipmentSelection {
                grapple: qa_content::contract::GrappleSelection::Disabled,
                hand_grenades: qa_content::contract::HandGrenadeSelection::Disabled,
            },
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: qa_content::contract::DopplerSelection::Disabled,
                environment: qa_content::contract::EnvironmentSelection::Disabled,
                assets: ContentId("q3:baseq3".to_string()),
                hud: owner(),
                effects: owner(),
                audio: owner(),
            },
            engine_behavior: owner(),
            combat: owner(),
            inventory: owner(),
            r#match: owner(),
            transition: owner(),
            execution: vec![execution],
            mounts: ResolvedMountPlan {
                id: MountPlanId("mount-plan:q3:1".to_string()),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            resources: Vec::new(),
            timing: vec![ProviderTiming {
                provider: provider("q3", "official"),
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 100.0,
                    fixed_movement_milliseconds: None,
                },
                numeric: Q3_BINARY32_PROFILE,
            }],
            ordering: FrameOrdering::Native {
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 100.0,
                    fixed_movement_milliseconds: None,
                },
            },
        }
    }

    #[test]
    fn valid_recipe_passes() {
        let execution = execution();
        assert_q3_guest_recipe(&recipe(execution.clone()), &execution);
    }

    #[test]
    #[should_panic(expected = "Q3 bytecode requires one native Q3 server game")]
    fn campaign_rejected() {
        let execution = execution();
        let mut recipe = recipe(execution.clone());
        recipe.campaign = CampaignSelection::Campaign {
            mission: owner(),
            gamecode: owner(),
        };
        assert_q3_guest_recipe(&recipe, &execution);
    }

    #[test]
    #[should_panic(expected = "Q3 bytecode requires one native Q3 server game")]
    fn foreign_movement_rejected() {
        let execution = execution();
        let mut recipe = recipe(execution.clone());
        recipe.movement = reference(provider("q2", "movement"), "q3:baseq3:q3dm1:1");
        assert_q3_guest_recipe(&recipe, &execution);
    }

    #[test]
    #[should_panic(expected = "Q3 bytecode requires one native Q3 server game")]
    fn replaced_enemies_rejected() {
        let execution = execution();
        let mut recipe = recipe(execution.clone());
        recipe.enemies = EnemySelection::Replace {
            default: qa_content::contract::MonsterSelectionTarget::MapDefined,
            by_classname: std::collections::HashMap::new(),
        };
        assert_q3_guest_recipe(&recipe, &execution);
    }

    #[test]
    #[should_panic(expected = "Q3 bytecode requires one native Q3 server game")]
    fn mismatched_execution_rejected() {
        let execution = execution();
        let mut other = execution.clone();
        if let ExecutionModule::Qvm { api, .. } = &mut other {
            *api = Q3ApiIdentity::Qagame(7);
        }
        assert_q3_guest_recipe(&recipe(execution), &other);
    }

    struct FakePrimarySource;

    impl BuiltinPrimarySource for FakePrimarySource {
        fn builtin_input(&self, artifact: &mod_provider::QvmArtifact) -> QvmInputDefinition {
            QvmInputDefinition {
                module: ProviderModuleId {
                    id: artifact.module.id.clone(),
                    artifact_path: artifact.module.artifact_path.clone(),
                    digest: artifact.module.digest.clone(),
                    revision: artifact.module.revision.clone(),
                },
                entity_stride: 560,
                client_stride: 560,
                client_pointer: 0,
                intermission: Vec::new(),
                movement_modes: None,
                entries: InputEntries {
                    client_think: 0,
                    run_client: 0,
                    client_spawn: 0,
                    move_: 0,
                    slice: 0,
                },
            }
        }

        fn builtin_weapons(
            &self,
            _artifact: &mod_provider::QvmArtifact,
            _catalog: &[QvmWeaponCatalogRow],
        ) -> QvmPrimaryWeaponProfile {
            fixture_weapons()
        }

        fn builtin_inventory(&self, _artifact: &mod_provider::QvmArtifact) -> QvmInventoryProfile {
            fixture_inventory()
        }

        fn builtin_pickups(&self, _artifact: &mod_provider::QvmArtifact) -> QvmPickupProfile {
            fixture_pickups()
        }

        fn builtin_combat(&self, _artifact: &mod_provider::QvmArtifact) -> QvmPrimaryCombatProfile {
            fixture_combat()
        }
    }

    const FIXTURE_ENTITY: usize = 1024;
    const FIXTURE_CLIENT: usize = 2048;
    const FIXTURE_POINTER: usize = 100;

    fn fixture_module() -> ProviderModuleId {
        ProviderModuleId {
            id: "test:game".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:game".to_string(),
            revision: "1".to_string(),
        }
    }

    fn fixture_region() -> QvmRegionEvaluation {
        QvmRegionEvaluation {
            entry: 1,
            join: 2,
            inputs: Vec::new(),
            result: None,
        }
    }

    fn fixture_weapons() -> QvmPrimaryWeaponProfile {
        QvmPrimaryWeaponProfile {
            module: fixture_module(),
            match_: None,
            abi_profile: QvmAbi::Modern,
            equipment_movement: QvmEquipmentMovementProfile {
                move_: 10,
                slice: 20,
                duck: 30,
                movement_global: 40,
                locomotion: RegionRef { entry: 1, join: 2 },
                mins: 50,
                maxs: 60,
                body_trace: None,
            },
            entity_stride: FIXTURE_ENTITY,
            client_stride: FIXTURE_CLIENT,
            client_pointer: FIXTURE_POINTER,
            stage: QvmWeaponDispatcherDefinition {
                dispatcher: DispatcherHead {
                    entry: 1,
                    actor: QvmWeaponActor {
                        record: "entity".to_string(),
                        pointer: QvmModInputPointer {
                            kind: InputPointerKind::Argument { index: 0 },
                            indirections: Vec::new(),
                            offset: 0,
                        },
                    },
                },
                predicates: Vec::new(),
                settled: Vec::new(),
                selection: StageSelection {
                    field: QvmItemField {
                        record: "client".to_string(),
                        offset: 8,
                    },
                    values: vec![SelectionValue {
                        value: 3,
                        item: "weapon_rocket".to_string(),
                    }],
                },
                request: StageRequest {
                    entry: 1,
                    argument: 0,
                    accepted: Vec::new(),
                },
            },
            damage_factor: DamageFactor {
                entry: 1,
                result: 2,
                stop: RegionRef { entry: 1, join: 2 },
            },
            equipment_contexts: Vec::new(),
            delay: fixture_region(),
            delay_player: DelayPlayer {
                movement_global: 1,
                player_offset: 2,
            },
            teleport: TeleportProfile {
                entry: 1,
                region: fixture_region(),
                objectives: fixture_region(),
                spawn: 2,
                view: 3,
            },
            max_health: 4,
            persistent_max_health: 8,
            availability: WeaponAvailability {
                movement_type: 1,
                excluded: Vec::new(),
                health: 2,
                team: 3,
                spectator_team: 0,
                flags: 4,
                respawn_flag: 1,
            },
            powerups: PowerupOffsets {
                quad: 1,
                haste: 2,
                flight: 3,
            },
            torso_animation: TorsoAnimation {
                entry: 1,
                attack: 2,
                melee: 3,
            },
            water_level: WaterLevel {
                entity_offset: 1,
                movement_offset: 2,
            },
            drop: DropProfile {
                entry: 1,
                argument: 0,
                weapon: 2,
                ammo: DropAmmo::Inventory,
                region: RegionRef { entry: 1, join: 2 },
            },
            give: GiveProfile {
                entry: 1,
                argument: 0,
                weapons: 2,
                ammo: 3,
                named: NamedGrant {
                    entry: 1,
                    join: 2,
                    name: 3,
                    item: 4,
                },
            },
        }
    }

    fn fixture_inventory() -> QvmInventoryProfile {
        QvmInventoryProfile::Private {
            module: fixture_module(),
            abi_profile: QvmAbi::Modern,
            entity_stride: FIXTURE_ENTITY,
            client_stride: FIXTURE_CLIENT,
            image: ProviderImage {
                instructions: Vec::new(),
                data_length: 0,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 0,
                allocated_data_length: 0,
            },
            storage: vec![QvmItemStorage::counter(
                QvmItemField {
                    record: "client".to_string(),
                    offset: 8,
                },
                "weapon_rocket".to_string(),
                QvmItemCapacity::Constant(10),
            )],
        }
    }

    fn fixture_layout() -> PickupItemLayout {
        PickupItemLayout {
            address: TableAddress::Direct(100),
            count: TableCount::Direct(8),
            live: false,
            stride: 64,
            fields: ItemTableFields {
                class_name: 0,
                pickup_name: 4,
                type_: 8,
                tag: 12,
            },
            weapon_type: 2,
            ammo_type: 4,
        }
    }

    fn fixture_pickups() -> QvmPickupProfile {
        QvmPickupProfile {
            module: fixture_module(),
            abi_profile: QvmAbi::Modern,
            entity_stride: FIXTURE_ENTITY,
            client_stride: FIXTURE_CLIENT,
            fields: PickupFields {
                inuse: 0,
                client: FIXTURE_POINTER,
                health: 4,
                item: 8,
                count: 12,
                flags: 16,
            },
            dropped_flag: 1,
            items: fixture_layout(),
            touch: 1,
            gate: GateProfile {
                entry: 1,
                calls: Vec::new(),
                item_argument: 0,
                player_argument: 1,
            },
            targets: FunctionCalls {
                entry: 1,
                calls: Vec::new(),
            },
            free: 2,
            objective_types: Vec::new(),
            grants: Vec::new(),
        }
    }

    fn fixture_combat() -> QvmPrimaryCombatProfile {
        let call = QvmCombatCall::damage(0, 1, 2, 3, 4, 5, 6, 7, Vec::new());
        QvmPrimaryCombatProfile {
            damage_call: call.clone(),
            module: fixture_module(),
            abi_profile: QvmAbi::Modern,
            entity_stride: FIXTURE_ENTITY,
            client_stride: FIXTURE_CLIENT,
            fields: CombatFields {
                inuse: 0,
                health: 4,
                takedamage: 8,
                parent: 12,
                client: FIXTURE_POINTER,
            },
            callbacks: CombatCallbacks {
                allocate: 1,
                free: 2,
                damage: 3,
            },
            armor: ArmorDefinition {
                check_armor: 1,
                call,
                points_stat: 2,
                protection: 0.5,
                tiers: None,
            },
            reactions: CombatReactions {
                flags: 1,
                pain: 2,
                die: 3,
                pain_call: QvmReactionCall {
                    arguments: 2,
                    target: 0,
                    amount: 1,
                },
                die_call: QvmReactionCall {
                    arguments: 2,
                    target: 0,
                    amount: 1,
                },
            },
            grapple_damage_method: 0,
            state: CombatState {
                health_stat: 1,
                team: CombatTeamState {
                    persistent_stat: 2,
                    values: Vec::new(),
                },
                flags: CombatStateFlags {
                    notarget: 1,
                    invulnerable: 2,
                    no_knockback: 4,
                },
                mass: QvmCombatMass::Constant(100.0),
            },
            damage_flags: QvmDamageFlags {
                radius: 1,
                no_armor: 2,
                no_knockback: 4,
                no_protection: 8,
                no_team_protection: 16,
            },
        }
    }

    fn qvm_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1272_1444u32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&32i32.to_le_bytes());
        bytes.extend_from_slice(&5i32.to_le_bytes());
        bytes.extend_from_slice(&37i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.push(4);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn prepare_rejects_non_server_artifact() {
        let dir = std::env::temp_dir().join("sim-q3-artifact-reject");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:q3:test".to_string()),
            mounts: vec![ContentMount::Loose(qa_content::contract::LooseMount {
                identity: MountIdentity {
                    id: MountId("mount:q3:test".to_string()),
                    content: ContentId("q3:baseq3".to_string()),
                    generation: 1,
                },
                root_path: dir.to_string_lossy().to_string(),
            })],
            default_order: vec![MountId("mount:q3:test".to_string())],
            prefix_orders: Vec::new(),
        };
        let mounts = open_mount_plan(
            &plan,
            qa_content::mounts::OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .unwrap();
        let bad = ExecutionModule::Qvm {
            owner: owner(),
            artifact: match execution() {
                ExecutionModule::Qvm { artifact, .. } => artifact,
                _ => panic!("test execution"),
            },
            role: ModuleRole::ClientGame,
            api: Q3ApiIdentity::Qagame(8),
        };
        let error = prepare_q3_game(&bad, &mounts, &FakePrimarySource).unwrap_err();
        assert!(error.to_string().contains("supported qagame ABI"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_loads_builtin_stock_game() {
        let dir = std::env::temp_dir().join("sim-q3-artifact-stock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("vm")).unwrap();
        let bytes = qvm_bytes();
        std::fs::write(dir.join("vm/qagame.qvm"), &bytes).unwrap();
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:q3:test".to_string()),
            mounts: vec![ContentMount::Loose(qa_content::contract::LooseMount {
                identity: MountIdentity {
                    id: MountId("mount:q3:test".to_string()),
                    content: ContentId("q3:baseq3".to_string()),
                    generation: 1,
                },
                root_path: dir.to_string_lossy().to_string(),
            })],
            default_order: vec![MountId("mount:q3:test".to_string())],
            prefix_orders: Vec::new(),
        };
        let mounts = open_mount_plan(
            &plan,
            qa_content::mounts::OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .unwrap();
        let mut execution = execution();
        if let ExecutionModule::Qvm { artifact, .. } = &mut execution {
            artifact.identity = qa_content::contract::ResourceIdentity {
                mount_generation: 1,
                member_index: qa_content::archive::crc32("vm/qagame.qvm".as_bytes()),
                byte_length: bytes.len() as u64,
                crc: qa_content::archive::crc32(&bytes),
            };
            artifact.byte_length = bytes.len() as u64;
            artifact.provenance = ResourceProvenance::Loose {
                mount: qa_content::contract::LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:q3:test".to_string()),
                        content: ContentId("q3:baseq3".to_string()),
                        generation: 1,
                    },
                    root_path: dir.to_string_lossy().to_string(),
                },
                member_path: "vm/qagame.qvm".to_string(),
            };
        }
        let prepared = prepare_q3_game(&execution, &mounts, &FakePrimarySource).unwrap();
        assert_eq!(prepared.weapons.len(), 10);
        assert!(prepared.items.is_none());
        assert!(prepared.primary.input.is_some());
        assert_eq!(prepared.artifact.image.instructions.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
