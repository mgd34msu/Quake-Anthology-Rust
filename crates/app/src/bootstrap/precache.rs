//! Application resource precache requests and preload.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/precache.ts`
//! (`characterResourceRequests`, `nativeQ2MonsterResources`,
//! `applicationResourceRequests`, `prepareApplicationResources`). The catalog resource
//! helpers, monster sources, transient sounds, entity and weapon types, and character
//! media lists are the ported content helpers; the recipe (`./content.ts`, out of
//! scope) arrives as [`PrecacheContent`], the simulation (`./simulation/runtime.ts`, out
//! of scope) through the [`PrecacheSimulation`] seam, and audio/effects through the
//! [`PrecacheAudio`]/[`PrecacheEffects`] seams. Q1 world enumeration arrives through the
//! [`Q1WorldPrecaches`] seam because Q1 services need a host; the donor's two filter
//! predicates are ported. Two documented folds: `nativeQ2MonsterResources` takes full
//! [`Q2Entity`] slices instead of the donor's classname/spawn pick (it reads the same
//! two fields), and the donor's `Bun.sleep(0)` yield between resources is a no-op in
//! sync code. The Q1 weapon/game identity check uses host-minted game ids standing in
//! for reference equality.

use std::collections::HashSet;

use qa_content::catalog::weapons::{q2_registered_weapon_resources, weapon_resources};
use qa_content::catalog::{equipment_resources, family_name, monster_resources, CatalogError, InstalledCatalog};
use qa_content::contract::{
    CharacterSelection, ContentId, EnemySelection, EquipmentSelection, GameFamily, ProviderReference, ResourceRequest,
    SourceEdition,
};
use qa_content::monsters::{monster_sources, MonsterSourceDefinition};
use qa_content::q2::base::player::resources::{Q2_CHARACTER_MODELS, Q2_CHARACTER_SOUNDS};
use qa_content::q2::foundation::effect_resources::{Q2_ROGUE_TRANSIENT_SOUNDS, Q2_TRANSIENT_SOUNDS};
use qa_content::q2::foundation::host::Q2Entity;
use qa_content::q2::foundation::weapons::types::Q2WeaponDefinition;
use qa_content::q2::rerelease::monsters::base_variants::medic::rerelease_medic_reinforcements;
use qa_content::q3::presentation::character_resources::Q3_CHARACTER_SOUNDS;
use qa_content::q3::presentation::players::CUSTOM_SOUND_NAMES;

/// Recipe and catalog reads (donor `ResourceContent`).
pub struct PrecacheContent<'a> {
    /// Installed catalog.
    pub catalog: &'a InstalledCatalog,
    /// Enemy selection.
    pub enemies: &'a EnemySelection,
    /// Equipment selection.
    pub equipment: &'a EquipmentSelection,
    /// Weapon providers.
    pub weapons: &'a [ProviderReference],
    /// Character selection.
    pub character: &'a CharacterSelection,
    /// Map entity provider.
    pub map_entities: &'a ProviderReference,
}

/// Q1 game precaches with a host-minted game identity.
#[derive(Debug, Clone)]
pub struct Q1GamePrecaches {
    /// Game identity (stands in for the donor's reference equality).
    pub game_id: u64,
    /// Precached models.
    pub models: Vec<String>,
    /// Precached sounds.
    pub sounds: Vec<String>,
}

/// QuakeC precache names.
#[derive(Debug, Clone)]
pub struct QuakecPrecaches {
    /// Precached models.
    pub models: Vec<String>,
    /// Precached sounds.
    pub sounds: Vec<String>,
}

/// Quake II precache data.
#[derive(Debug, Clone)]
pub struct Q2PrecacheData {
    /// Live entities.
    pub entities: Vec<Q2Entity>,
    /// Registered weapons.
    pub weapons: Vec<Q2WeaponDefinition>,
    /// Whether the game edition is rerelease.
    pub rerelease: bool,
}

/// Simulation precache reads (donor `PrecacheSource`).
pub trait PrecacheSimulation {
    /// Q1 source precaches, when present.
    fn q1_precaches(&self) -> Option<Q1GamePrecaches>;
    /// QuakeC precache names, when present.
    fn quakec_precaches(&self) -> Option<QuakecPrecaches>;
    /// Q1 weapon source precaches, when present.
    fn q1_weapon_precaches(&self) -> Option<Q1GamePrecaches>;
    /// Quake II precache data, when present.
    fn q2_precache(&self) -> Option<Q2PrecacheData>;
    /// Quake II item resource paths for classnames.
    fn q2_item_resource_paths(&self, classnames: &HashSet<String>) -> Vec<String>;
    /// Quake II player skins, or empty without a source.
    fn q2_player_skins(&self) -> Vec<String>;
}

/// Q1 world precache enumeration (donor `precacheQ1World` with recording callbacks).
pub trait Q1WorldPrecaches {
    /// Worldspawn sounds.
    fn q1_world_sounds(&self) -> Vec<String>;
    /// Worldspawn models.
    fn q1_world_models(&self) -> Vec<String>;
}

/// Audio preload (donor `ApplicationAudio` subset).
pub trait PrecacheAudio {
    /// Preload a sound, with a player skin for `*` Quake II sounds.
    fn preload_sound(&mut self, content: &ContentId, path: &str, skin: Option<&str>) -> Result<(), String>;
    /// Preload character footsteps.
    fn preload_character_footsteps(&mut self) -> Result<(), String>;
}

/// Model preload (donor `ApplicationEffects` subset).
pub trait PrecacheEffects {
    /// Preload a model.
    fn preload_model(&mut self, content: &ContentId, path: &str) -> Result<(), String>;
}

/// Donor source edition text for monster source comparison.
fn source_edition_text(edition: SourceEdition) -> &'static str {
    match edition {
        SourceEdition::Classic => "classic",
        SourceEdition::Rerelease => "rerelease",
    }
}

/// Whether a path has a precache media extension (donor extension filter).
fn is_precache_media(path: &str) -> bool {
    matches!(
        path.rsplit('.').next().map(str::to_lowercase).as_deref(),
        Some("mdl" | "md2" | "md3" | "spr" | "sp2" | "wav" | "ogg")
    )
}

/// Whether a path is audio (donor wav/ogg test).
fn is_audio(path: &str) -> bool {
    matches!(
        path.rsplit('.').next().map(str::to_lowercase).as_deref(),
        Some("wav" | "ogg")
    )
}

/// Whether a Q1 world model is a player model (donor model regex).
fn is_q1_precache_model(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("progs/") else {
        return false;
    };
    let Some(stem) = rest.strip_suffix(".mdl").or_else(|| rest.strip_suffix(".spr")) else {
        return false;
    };
    matches!(
        stem,
        "player" | "eyes" | "h_player" | "gib1" | "gib2" | "gib3" | "s_bubble"
    )
}

/// Whether a Q1 world sound is a player sound (donor sound filter).
fn is_q1_precache_sound(path: &str) -> bool {
    path.starts_with("player/")
        || matches!(
            path,
            "misc/h2ohit1.wav"
                | "misc/outwater.wav"
                | "misc/r_tele1.wav"
                | "misc/r_tele2.wav"
                | "misc/r_tele3.wav"
                | "misc/r_tele4.wav"
                | "misc/r_tele5.wav"
        )
}

/// Character resource requests (donor `characterResourceRequests`).
pub fn character_resource_requests(
    content: &PrecacheContent,
    q1_world: &impl Q1WorldPrecaches,
) -> Result<Vec<ResourceRequest>, CatalogError> {
    let character = &content.character.definition.content;
    let family = content.catalog.product(character.as_str())?.expectation.family;
    let request = |path: String| ResourceRequest {
        content: character.clone(),
        path,
    };
    if family == GameFamily::Q3 {
        let sounds = Q3_CHARACTER_SOUNDS;
        let paths = [
            sounds.select_sound,
            sounds.gib_sound,
            sounds.tele_in_sound,
            sounds.tele_out_sound,
            sounds.respawn_sound,
            sounds.land_sound,
            sounds.watr_in_sound,
            sounds.watr_out_sound,
            sounds.watr_un_sound,
            sounds.jump_pad_sound,
        ];
        return Ok(paths
            .into_iter()
            .chain(CUSTOM_SOUND_NAMES)
            .map(|path| request(path.to_string()))
            .collect());
    }
    if family == GameFamily::Q2 {
        return Ok(Q2_CHARACTER_MODELS
            .iter()
            .map(|path| request(path.to_string()))
            .chain(Q2_CHARACTER_SOUNDS.iter().map(|path| {
                request(if path.starts_with('*') {
                    path.to_string()
                } else {
                    format!("sound/{path}")
                })
            }))
            .collect());
    }
    let mut paths = Vec::new();
    for path in q1_world.q1_world_sounds() {
        if is_q1_precache_sound(&path) {
            paths.push(format!("sound/{path}"));
        }
    }
    for path in q1_world.q1_world_models() {
        if is_q1_precache_model(&path) {
            paths.push(path);
        }
    }
    Ok(paths.into_iter().map(request).collect())
}

/// Native Quake II monster resources (donor `nativeQ2MonsterResources`).
pub fn native_q2_monster_resources(
    content: &ContentId,
    source: Option<&MonsterSourceDefinition>,
    entities: &[Q2Entity],
) -> Vec<ResourceRequest> {
    let mut seen = HashSet::new();
    let mut classnames = Vec::new();
    for entity in entities {
        if seen.insert(entity.classname.clone()) {
            classnames.push(entity.classname.clone());
        }
    }
    if source.is_some_and(|source| source.edition == SourceEdition::Rerelease) {
        for entity in entities {
            if entity.classname == "monster_medic" || entity.classname == "monster_medic_commander" {
                for reinforcement in rerelease_medic_reinforcements(&entity.spawn.values) {
                    if seen.insert(reinforcement.classname.clone()) {
                        classnames.push(reinforcement.classname);
                    }
                }
            }
        }
    }
    classnames
        .into_iter()
        .flat_map(|classname| {
            source
                .and_then(|source| source.creatures.get(&classname))
                .map_or(Vec::new(), |creature| {
                    creature
                        .resources
                        .iter()
                        .map(|path| ResourceRequest {
                            content: content.clone(),
                            path: path.clone(),
                        })
                        .collect()
                })
        })
        .collect()
}

/// Append model and sound requests (donor `append`).
fn append_requests(requests: &mut Vec<ResourceRequest>, content: &ContentId, models: &[String], sounds: &[String]) {
    requests.extend(
        models
            .iter()
            .filter(|path| !path.is_empty() && !path.starts_with('*'))
            .map(|path| ResourceRequest {
                content: content.clone(),
                path: path.clone(),
            }),
    );
    requests.extend(
        sounds
            .iter()
            .filter(|path| !path.is_empty())
            .map(|path| ResourceRequest {
                content: content.clone(),
                path: if path.starts_with("sound/") || path.starts_with('*') {
                    path.clone()
                } else {
                    format!("sound/{path}")
                },
            }),
    );
}

/// Application resource requests (donor `applicationResourceRequests`).
pub fn application_resource_requests(
    content: &PrecacheContent,
    simulation: &impl PrecacheSimulation,
    q1_world: &impl Q1WorldPrecaches,
) -> Result<Vec<ResourceRequest>, CatalogError> {
    let native = content.map_entities.content.clone();
    let mut requests = character_resource_requests(content, q1_world)?;
    requests.extend(monster_resources(content.enemies)?);
    requests.extend(equipment_resources(content.equipment));
    requests.extend(weapon_resources(
        content.map_entities,
        content.weapons,
        content.catalog,
    )?);
    let q1 = simulation.q1_precaches();
    if let Some(q1) = &q1 {
        append_requests(&mut requests, &native, &q1.models, &q1.sounds);
    }
    if let Some(qc) = simulation.quakec_precaches() {
        append_requests(&mut requests, &native, &qc.models, &qc.sounds);
    }
    if let Some(q1_weapons) = simulation.q1_weapon_precaches() {
        if q1.as_ref().is_none_or(|q1| q1_weapons.game_id != q1.game_id) {
            let mut weapon_content = None;
            for weapon in content.weapons {
                if content.catalog.product(weapon.content.as_str())?.expectation.family == GameFamily::Q1 {
                    weapon_content = Some(weapon.content.clone());
                    break;
                }
            }
            if let Some(weapon_content) = weapon_content {
                append_requests(&mut requests, &weapon_content, &q1_weapons.models, &q1_weapons.sounds);
            }
        }
    }
    if let Some(q2) = simulation.q2_precache() {
        let classnames: HashSet<String> = q2.entities.iter().map(|entity| entity.classname.clone()).collect();
        let product = content.catalog.product(native.as_str())?.expectation.clone();
        let sources = monster_sources();
        let source = sources.iter().find(|source| {
            source.family.as_str() == family_name(product.family)
                && source_edition_text(source.edition) == product.edition
                && source.program.as_str() == product.campaign
        });
        requests.extend(native_q2_monster_resources(&native, source, &q2.entities));
        for path in simulation.q2_item_resource_paths(&classnames) {
            requests.push(ResourceRequest {
                content: native.clone(),
                path,
            });
        }
        for path in q2_registered_weapon_resources(&q2.weapons, q2.rerelease) {
            requests.push(ResourceRequest {
                content: native.clone(),
                path,
            });
        }
        let models: Vec<String> = q2
            .entities
            .iter()
            .flat_map(|entity| {
                [
                    entity.model.clone(),
                    entity.model2.clone(),
                    entity.model3.clone(),
                    entity.model4.clone(),
                ]
            })
            .collect();
        let sounds: Vec<String> = q2
            .entities
            .iter()
            .flat_map(|entity| [entity.noise.clone(), entity.sound.clone()])
            .collect();
        append_requests(&mut requests, &native, &models, &sounds);
    }
    let mut seen_contents = HashSet::new();
    let mut contents = Vec::new();
    for candidate in [native.clone()]
        .into_iter()
        .chain(requests.iter().map(|request| request.content.clone()))
    {
        if seen_contents.insert(candidate.clone()) {
            contents.push(candidate);
        }
    }
    for content_id in &contents {
        let product = content.catalog.product(content_id.as_str())?.expectation.clone();
        if product.family == GameFamily::Q2 {
            append_requests(
                &mut requests,
                content_id,
                &[],
                &Q2_TRANSIENT_SOUNDS
                    .iter()
                    .map(|path| path.to_string())
                    .collect::<Vec<_>>(),
            );
            if product.edition == "rerelease" || product.campaign == "rogue" {
                append_requests(
                    &mut requests,
                    content_id,
                    &[],
                    &Q2_ROGUE_TRANSIENT_SOUNDS
                        .iter()
                        .map(|path| path.to_string())
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for request in requests {
        if !is_precache_media(&request.path) {
            continue;
        }
        if seen.insert((request.content.clone(), request.path.clone())) {
            deduped.push(request);
        }
    }
    Ok(deduped)
}

/// Preload application resources (donor `prepareApplicationResources`).
pub fn prepare_application_resources(
    content: &PrecacheContent,
    simulation: &impl PrecacheSimulation,
    q1_world: &impl Q1WorldPrecaches,
    audio: &mut impl PrecacheAudio,
    effects: &mut impl PrecacheEffects,
    progress: &mut impl FnMut(&str),
    print: &mut impl FnMut(&str),
) -> Result<(), CatalogError> {
    let requests = application_resource_requests(content, simulation, q1_world)?;
    let q2_skins = simulation.q2_player_skins();
    for (index, request) in requests.iter().enumerate() {
        progress(&format!("Preparing resources {}/{}...", index + 1, requests.len()));
        let outcome = (|| -> Result<(), String> {
            if is_audio(&request.path) {
                let star_q2 = request.path.starts_with('*')
                    && content
                        .catalog
                        .product(request.content.as_str())
                        .map_err(|error| error.to_string())?
                        .expectation
                        .family
                        == GameFamily::Q2;
                if star_q2 {
                    let mut seen = HashSet::new();
                    let mut skins = Vec::new();
                    for skin in if q2_skins.is_empty() {
                        vec!["male".to_string()]
                    } else {
                        q2_skins.clone()
                    } {
                        if seen.insert(skin.clone()) {
                            skins.push(skin);
                        }
                    }
                    for skin in &skins {
                        let model = skin.split('/').next().filter(|part| !part.is_empty()).unwrap_or("male");
                        audio.preload_sound(&request.content, &request.path, Some(model))?;
                    }
                } else {
                    audio.preload_sound(&request.content, &request.path, None)?;
                }
            } else {
                effects.preload_model(&request.content, &request.path)?;
            }
            Ok(())
        })();
        if let Err(message) = outcome {
            print(&format!(
                "Optional resource preload skipped: {}/{}: {message}\n",
                request.content, request.path
            ));
        }
    }
    if let Err(message) = audio.preload_character_footsteps() {
        print(&format!("Optional character footsteps preload skipped: {message}\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::{GrappleSelection, HandGrenadeSelection};
    use qa_content::monsters::{MonsterCreature, MonsterFamily, MonsterProgram};
    use qa_core::identity::{IdentityOwner, ProviderId};

    struct StubSimulation {
        q1: Option<(u64, Vec<String>, Vec<String>)>,
        qc: Option<(Vec<String>, Vec<String>)>,
        q1_weapon: Option<(u64, Vec<String>, Vec<String>)>,
        q2: Option<Q2PrecacheData>,
        item_paths: Vec<String>,
        skins: Vec<String>,
    }

    impl PrecacheSimulation for StubSimulation {
        fn q1_precaches(&self) -> Option<Q1GamePrecaches> {
            self.q1.clone().map(|(game_id, models, sounds)| Q1GamePrecaches {
                game_id,
                models,
                sounds,
            })
        }
        fn quakec_precaches(&self) -> Option<QuakecPrecaches> {
            self.qc
                .clone()
                .map(|(models, sounds)| QuakecPrecaches { models, sounds })
        }
        fn q1_weapon_precaches(&self) -> Option<Q1GamePrecaches> {
            self.q1_weapon.clone().map(|(game_id, models, sounds)| Q1GamePrecaches {
                game_id,
                models,
                sounds,
            })
        }
        fn q2_precache(&self) -> Option<Q2PrecacheData> {
            self.q2.clone()
        }
        fn q2_item_resource_paths(&self, _classnames: &HashSet<String>) -> Vec<String> {
            self.item_paths.clone()
        }
        fn q2_player_skins(&self) -> Vec<String> {
            self.skins.clone()
        }
    }

    struct StubWorld {
        sounds: Vec<String>,
        models: Vec<String>,
    }

    impl Q1WorldPrecaches for StubWorld {
        fn q1_world_sounds(&self) -> Vec<String> {
            self.sounds.clone()
        }
        fn q1_world_models(&self) -> Vec<String> {
            self.models.clone()
        }
    }

    fn product(id: &str, family: GameFamily) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: "classic".to_string(),
                campaign: "baseq2".to_string(),
                title: id.to_string(),
                content_directory: id.to_string(),
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
        }
    }

    fn reference(content: &str) -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("test", "precache"),
            content: ContentId(content.to_string()),
        }
    }

    struct Fixture {
        catalog: InstalledCatalog,
        enemies: EnemySelection,
        equipment: EquipmentSelection,
        weapons: Vec<ProviderReference>,
        character: CharacterSelection,
        map_entities: ProviderReference,
    }

    impl Fixture {
        fn content(&self) -> PrecacheContent<'_> {
            PrecacheContent {
                catalog: &self.catalog,
                enemies: &self.enemies,
                equipment: &self.equipment,
                weapons: &self.weapons,
                character: &self.character,
                map_entities: &self.map_entities,
            }
        }
    }

    fn fixture(character: &str, native: &str) -> Fixture {
        let catalog = InstalledCatalog::new(
            "/corpus".to_string(),
            vec![
                product("q3:classic:baseq3:1", GameFamily::Q3),
                product("q2:classic:baseq2:1", GameFamily::Q2),
                product("q1:classic:id1:1", GameFamily::Q1),
            ],
            Vec::new(),
            1,
            None,
        )
        .unwrap();
        Fixture {
            catalog,
            enemies: EnemySelection::MapDefined,
            equipment: EquipmentSelection {
                grapple: GrappleSelection::Disabled,
                hand_grenades: HandGrenadeSelection::Disabled,
            },
            weapons: Vec::new(),
            character: CharacterSelection {
                definition: reference(character),
                appearance: reference(character),
            },
            map_entities: reference(native),
        }
    }

    fn simulation() -> StubSimulation {
        StubSimulation {
            q1: None,
            qc: None,
            q1_weapon: None,
            q2: None,
            item_paths: Vec::new(),
            skins: Vec::new(),
        }
    }

    fn world() -> StubWorld {
        StubWorld {
            sounds: Vec::new(),
            models: Vec::new(),
        }
    }

    #[test]
    fn character_q3_lists_sounds_and_custom_names() {
        let fixture = fixture("q3:classic:baseq3:1", "q3:classic:baseq3:1");
        let requests = character_resource_requests(&fixture.content(), &world()).unwrap();
        assert_eq!(requests.len(), 10 + CUSTOM_SOUND_NAMES.len());
        assert!(requests
            .iter()
            .all(|request| request.content.as_str() == "q3:classic:baseq3:1"));
        assert_eq!(requests[0].path, Q3_CHARACTER_SOUNDS.select_sound);
    }

    #[test]
    fn character_q2_prefixes_sounds() {
        let fixture = fixture("q2:classic:baseq2:1", "q2:classic:baseq2:1");
        let requests = character_resource_requests(&fixture.content(), &world()).unwrap();
        assert_eq!(requests.len(), Q2_CHARACTER_MODELS.len() + Q2_CHARACTER_SOUNDS.len());
        for (request, expected) in requests
            .iter()
            .zip(
                Q2_CHARACTER_MODELS
                    .iter()
                    .map(|path| path.to_string())
                    .chain(Q2_CHARACTER_SOUNDS.iter().map(|path| {
                        if path.starts_with('*') {
                            path.to_string()
                        } else {
                            format!("sound/{path}")
                        }
                    })),
            )
        {
            assert_eq!(request.path, expected);
        }
    }

    #[test]
    fn character_q1_filters_world_precaches() {
        let fixture = fixture("q1:classic:id1:1", "q1:classic:id1:1");
        let world = StubWorld {
            sounds: ["player/ax1.wav", "misc/h2ohit1.wav", "misc/other.wav"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            models: [
                "progs/player.mdl",
                "progs/eyes.spr",
                "progs/gib2.mdl",
                "progs/s_bubble.spr",
                "progs/h_player.mdl",
                "progs/other.mdl",
                "progs/player.md2",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        };
        let requests = character_resource_requests(&fixture.content(), &world).unwrap();
        let paths: Vec<&str> = requests.iter().map(|request| request.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "sound/player/ax1.wav",
                "sound/misc/h2ohit1.wav",
                "progs/player.mdl",
                "progs/eyes.spr",
                "progs/gib2.mdl",
                "progs/s_bubble.spr",
                "progs/h_player.mdl",
            ]
        );
    }

    #[test]
    fn native_monsters_add_rerelease_reinforcements() {
        use qa_content::monsters::MonsterSourceDefinition;
        use qa_core::identity::ProviderId as Pid;
        let owner = IdentityOwner::create("precache-monsters").unwrap();
        let actor = owner.actor(0, 1);
        let owned = owner.owned_actor(&actor, Pid::new("test", "precache")).unwrap();
        let spawn = |classname: &str, values: &[(&str, &str)]| {
            Q2Entity::new(
                owned.clone(),
                qa_content::q2::foundation::host::Q2SpawnFields {
                    ordinal: 0,
                    classname: classname.to_string(),
                    values: values
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect(),
                },
            )
        };
        let entities = vec![
            spawn("monster_soldier", &[]),
            spawn("monster_medic", &[("reinforcements", "monster_gladiator 2")]),
        ];
        let source = MonsterSourceDefinition {
            provider: Pid::new("test", "precache"),
            edition: SourceEdition::Rerelease,
            creatures: [
                (
                    "monster_soldier".to_string(),
                    MonsterCreature {
                        resources: vec!["s.md2".to_string()],
                    },
                ),
                (
                    "monster_gladiator".to_string(),
                    MonsterCreature {
                        resources: vec!["g.md2".to_string()],
                    },
                ),
            ]
            .into_iter()
            .collect(),
            family: MonsterFamily::Q2,
            program: MonsterProgram::Baseq2,
        };
        let requests =
            native_q2_monster_resources(&ContentId("q2:classic:baseq2:1".to_string()), Some(&source), &entities);
        let paths: Vec<&str> = requests.iter().map(|request| request.path.as_str()).collect();
        assert_eq!(paths, vec!["s.md2", "g.md2"]);
        let empty = native_q2_monster_resources(&ContentId("q2:classic:baseq2:1".to_string()), None, &entities);
        assert!(empty.is_empty());
    }

    #[test]
    fn application_requests_append_filter_and_dedupe() {
        let fixture = fixture("q1:classic:id1:1", "q1:classic:id1:1");
        let simulation = StubSimulation {
            q1: Some((
                1,
                vec!["progs/player.mdl".to_string(), "*skip".to_string(), String::new()],
                vec!["ax1.wav".to_string(), String::new(), "sound/keep.wav".to_string()],
            )),
            ..simulation()
        };
        let requests = application_resource_requests(&fixture.content(), &simulation, &world()).unwrap();
        let has = |content: &str, path: &str| {
            requests
                .iter()
                .any(|request| request.content.as_str() == content && request.path == path)
        };
        assert!(has("q1:classic:id1:1", "progs/player.mdl"));
        assert!(has("q1:classic:id1:1", "sound/ax1.wav"));
        assert!(has("q1:classic:id1:1", "sound/keep.wav"));
        assert!(!requests
            .iter()
            .any(|request| request.path.is_empty() || request.path == "*skip"));
        let mut seen = HashSet::new();
        for request in &requests {
            assert!(seen.insert((request.content.clone(), request.path.clone())));
        }
    }

    struct Audio {
        sounds: Vec<(String, String, Option<String>)>,
        footsteps: u32,
        fail: Option<String>,
    }

    struct Effects {
        models: Vec<(String, String)>,
    }

    impl PrecacheAudio for Audio {
        fn preload_sound(&mut self, content: &ContentId, path: &str, skin: Option<&str>) -> Result<(), String> {
            if self.fail.as_deref() == Some(path) {
                return Err("missing".to_string());
            }
            self.sounds
                .push((content.as_str().to_string(), path.to_string(), skin.map(str::to_string)));
            Ok(())
        }
        fn preload_character_footsteps(&mut self) -> Result<(), String> {
            self.footsteps += 1;
            Ok(())
        }
    }

    impl PrecacheEffects for Effects {
        fn preload_model(&mut self, content: &ContentId, path: &str) -> Result<(), String> {
            self.models.push((content.as_str().to_string(), path.to_string()));
            Ok(())
        }
    }

    #[test]
    fn prepare_preloads_and_reports_skips() {
        let fixture = fixture("q2:classic:baseq2:1", "q2:classic:baseq2:1");
        let simulation = StubSimulation {
            q1: Some((
                1,
                vec!["m.mdl".to_string()],
                vec!["*player/x.wav".to_string(), "bad.wav".to_string()],
            )),
            skins: vec!["male/grunt".to_string(), "male/grunt".to_string(), "female".to_string()],
            ..simulation()
        };
        let mut audio = Audio {
            sounds: Vec::new(),
            footsteps: 0,
            fail: Some("sound/bad.wav".to_string()),
        };
        let mut effects = Effects { models: Vec::new() };
        let mut progress = Vec::new();
        let mut printed = Vec::new();
        prepare_application_resources(
            &fixture.content(),
            &simulation,
            &world(),
            &mut audio,
            &mut effects,
            &mut |message| progress.push(message.to_string()),
            &mut |message| printed.push(message.to_string()),
        )
        .unwrap();
        assert_eq!(audio.footsteps, 1);
        assert!(effects.models.iter().any(|(_, path)| path == "m.mdl"));
        let star: Vec<Option<String>> = audio
            .sounds
            .iter()
            .filter(|(_, path, _)| path == "*player/x.wav")
            .map(|(_, _, skin)| skin.clone())
            .collect();
        assert_eq!(star, vec![Some("male".to_string()), Some("female".to_string())]);
        assert!(printed
            .iter()
            .any(|message| message.contains("bad.wav") && message.contains("missing")));
        assert!(progress
            .iter()
            .all(|message| message.starts_with("Preparing resources ")));
    }
}
