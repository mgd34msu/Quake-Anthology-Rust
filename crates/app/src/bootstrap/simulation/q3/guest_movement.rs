//! Quake III guest movement projection for navigation prediction.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/guest-movement.ts`
//! (`guestMovementProjection`). Navigation reads public VM records at
//! use time; it never owns a second gameplay player, so the donor's
//! getters become live read methods on [`Q3MovementPredictionPlayer`].
//!
//! The players lane owns donor `../player-movement.ts`
//! (`locomotionTemplate`, `MovementPredictionPlayer`, `MovementPlayer`)
//! and `../players.ts` (`movementProfile`, `providerFamily`); the
//! template pieces this projection needs are mirrored here:
//! [`Q3GuestSourceMovement`] plus the q3-only template arm (character,
//! profile, standing bounds, numeric services). The mirror follows the
//! donor arm for arm: recipe timing selects the q3 movement profile and
//! anything else panics like the donor's selected-adapter throw.
//!
//! Two projections apply. `InventoryEntry` has no capacity word in Rust,
//! so ammo capacities (1/200) are dropped while counts are exact. The
//! donor's spectator literal (`ps.pmType === 1`, i.e. `PM_NOCLIP`) is
//! ported verbatim even though the native projection checks
//! `PM_SPECTATOR` — donor is arbiter; the divergence is reported.

use qa_bots::movement_contract::{MovementKind, MovementProfile};
use qa_content::contract::{ExecutableRecipe, GameFamily};
use qa_content::q3::base::shared::definitions::{stat_schema, Powerup, Product};
use qa_content::q3::foundation::arsenal::{q3_spawn_arsenal_runtime, Q3ArsenalRuntimeState, Q3_WEAPON_ITEMS};
use qa_core::identity::{ClientId, OwnedActor, ProviderId};
use qa_core::math::Bounds;
use qa_core::numeric::{NumericOps, Q3_BINARY32_PROFILE};
use qa_core::time::ClockProfile;
use qa_guest::qvm::player_record::QvmPlayerState;
use qa_world::movement::q3::constants::move_flags;
use qa_world::movement::q3::postures::Q3_SOURCE_STANDING_BOUNDS;
use qa_world::movement::q3::types::Q3MovementState;
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, InventoryEntry, MovementEnvironment, TraceHit, WeaponState,
};

use super::guest_runtime::{Q3ApplicationPlayer, Q3QvmServerGame};
use super::player_state::{item_tag, schema_max_health, schema_persistent_powerup};

/// World slot (donor `1022`).
const WORLD_SLOT: i32 = 1022;

/// Null slot (donor `1023`).
const NULL_SLOT: i32 = 1023;

/// Alive-client trace mask (donor `0x02010001`).
const TRACE_MASK: i32 = 0x0201_0001;

/// Dead-client trace mask (donor `0x10001`).
const DEAD_TRACE_MASK: i32 = 0x1_0001;

/// Source movement options behind prediction (donor
/// `ClientMovementOptions` literal in the `sourceMovement` getter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestSourceMovement {
    /// Trace contents mask.
    pub trace_mask: i32,
    /// Fixed step in milliseconds, when forced.
    pub fixed_msec: Option<i32>,
    /// Whether footsteps are muted.
    pub no_footsteps: bool,
    /// Whether the gauntlet hit.
    pub gauntlet_hit: bool,
    /// Debug level.
    pub debug_level: i32,
}

/// Provider family from a provider id (donor `providerFamily`).
fn provider_family(provider: &ProviderId) -> GameFamily {
    match provider.namespace.as_str() {
        "q1" => GameFamily::Q1,
        "q2" => GameFamily::Q2,
        "q3" => GameFamily::Q3,
        _ => panic!(
            "Unknown source provider family: {}:{}",
            provider.namespace, provider.name
        ),
    }
}

/// Live movement projection over one guest player (donor
/// `MovementPredictionPlayer` as built by `guestMovementProjection`).
pub struct Q3MovementPredictionPlayer<'a> {
    /// Connected client.
    pub client: ClientId,
    /// Player actor.
    pub actor: OwnedActor,
    /// Selected movement profile (always q3).
    pub profile: MovementProfile,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Provider family.
    pub character: GameFamily,
    /// Numeric services.
    pub numeric: NumericOps,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
    /// Movement speed multiplier.
    pub movement_speed_multiplier: f64,
    /// Guest game.
    guest: &'a Q3QvmServerGame,
    /// Projected player.
    player: Q3ApplicationPlayer,
    /// Product.
    product: Product,
    /// Arsenal provider.
    provider: ProviderId,
    /// Character provider.
    character_provider: ProviderId,
}

impl Q3MovementPredictionPlayer<'_> {
    /// Current player record.
    fn state(&self) -> QvmPlayerState {
        self.guest.records.player(self.player.source_entity)
    }

    /// Whether a powerup word is set.
    fn powerup(&self, slot: usize) -> bool {
        self.state().powerups.get(slot).copied().unwrap_or(0) != 0
    }

    /// Whether flight is granted.
    #[must_use]
    pub fn flight(&self) -> bool {
        self.powerup(Powerup::PwFlight as usize)
    }

    /// World gravity.
    #[must_use]
    pub fn world_gravity(&self) -> i32 {
        self.state().gravity
    }

    /// Source movement environment.
    #[must_use]
    pub fn source_environment(&self) -> MovementEnvironment {
        let ps = self.state();
        MovementEnvironment {
            client_outputs: None,
            speed_multiplier: None,
            pose: None,
            health: f64::from(ps.stats[0]),
            flight: self.flight(),
            haste: self.powerup(Powerup::PwHaste as usize),
            invulnerable: self.product == Product::Missionpack && self.powerup(Powerup::PwInvulnerability as usize),
            gravity_multiplier: 1.0,
        }
    }

    /// Source movement options.
    #[must_use]
    pub fn source_movement(&self) -> Q3GuestSourceMovement {
        let ps = self.state();
        let cvars = &self.guest.state.cvars;
        let fixed = cvars.borrow().variable_value("pmove_fixed");
        let msec = cvars.borrow().variable_value("pmove_msec") as i32;
        let dmflags = cvars.borrow().variable_value("dmflags") as i32;
        Q3GuestSourceMovement {
            trace_mask: if ps.stats[0] <= 0 { DEAD_TRACE_MASK } else { TRACE_MASK },
            fixed_msec: if fixed == 0.0 { None } else { Some(msec.clamp(8, 33)) },
            no_footsteps: dmflags != 0 && dmflags & 32 != 0,
            gauntlet_hit: false,
            debug_level: 0,
        }
    }

    /// Current entity bounds.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        let entity = self.guest.records.entity(self.player.source_entity);
        Bounds {
            min: entity.r.mins,
            max: entity.r.maxs,
        }
    }

    /// View height.
    #[must_use]
    pub fn view_height(&self) -> i32 {
        self.state().view_height
    }

    /// Current movement state.
    #[must_use]
    pub fn movement_state(&self) -> Q3MovementState {
        let ps = self.state();
        let ground = if ps.ground_entity_number == WORLD_SLOT {
            TraceHit::World { model: 0 }
        } else if ps.ground_entity_number == NULL_SLOT {
            TraceHit::None
        } else {
            TraceHit::Actor {
                actor: self.guest.records.actor(ps.ground_entity_number).id().clone(),
            }
        };
        Q3MovementState {
            command_time_milliseconds: ps.command_time_ms,
            movement_type: ps.movement_type,
            bob_cycle: ps.bob_cycle,
            movement_flags: ps.movement_flags,
            movement_time_milliseconds: ps.movement_time_ms,
            origin: ps.origin,
            velocity: ps.velocity,
            gravity: f64::from(ps.gravity),
            speed: f64::from(ps.speed),
            delta_angle_words: ps.delta_angle_words,
            movement_direction: ps.movement_direction,
            grapple_point: ps.grapple_point,
            flags: ps.flags,
            view_angles: ps.view_angles,
            view_height: f64::from(ps.view_height),
            ground,
            predictable_event_sequence: ps.event_sequence,
            jump_pad: if ps.jump_pad_entity == 0 {
                None
            } else {
                self.guest.records.reference(ps.jump_pad_entity)
            },
            movement_frame: ps.movement_frame_count,
            jump_pad_frame: ps.jump_pad_frame,
        }
    }

    /// Current arsenal.
    #[must_use]
    pub fn arsenal(&self) -> ArsenalState {
        if let Some(original) = self.guest.player_arsenal(&self.player.actor) {
            return original;
        }
        let ps = self.state();
        let schema = stat_schema(self.product);
        let weapons = schema.weapons();
        let mut ammo = Vec::new();
        for item in Q3_WEAPON_ITEMS.iter() {
            if self.product != Product::Missionpack && item.weapon as i32 >= 11 {
                continue;
            }
            let owned = ps.stats[weapons] & (1 << (item.weapon as i32)) != 0;
            ammo.push(InventoryEntry {
                item: item.item.clone(),
                count: f64::from(i32::from(owned)),
            });
            if let Some(ammo_item) = &item.ammo {
                ammo.push(InventoryEntry {
                    item: ammo_item.clone(),
                    count: f64::from(ps.ammo[item.weapon as usize]),
                });
            }
        }
        ArsenalState {
            provider: self.provider.clone(),
            active_weapon: qa_content::q3::foundation::arsenal::q3_weapon_item(ps.weapon)
                .map(|weapon| weapon.item.clone()),
            state: WeaponState::Q3 {
                source_weapon: ps.weapon,
                state: ps.weapon_state,
                time_milliseconds: ps.weapon_time_ms,
            },
            ammo,
        }
    }

    /// Current animation.
    #[must_use]
    pub fn animation(&self) -> ActorAnimationState {
        let ps = self.state();
        ActorAnimationState {
            provider: self.character_provider.clone(),
            state: AnimationState::Q3 {
                legs: ps.legs_animation,
                torso: ps.torso_animation,
                legs_timer_milliseconds: ps.legs_timer_ms,
                torso_timer_milliseconds: ps.torso_timer_ms,
            },
        }
    }

    /// Current q3 arsenal runtime.
    #[must_use]
    pub fn q3_arsenal(&self) -> Q3ArsenalRuntimeState {
        let ps = self.state();
        let schema = stat_schema(self.product);
        let holdable_item = ps.stats[schema.holdable_item()];
        Q3ArsenalRuntimeState {
            product: self.product,
            max_health: f64::from(ps.stats[schema_max_health(schema)]),
            spectator: ps.movement_type == 1,
            persistent_powerup_tag: schema_persistent_powerup(schema)
                .map(|slot| item_tag(self.product, ps.stats[slot]))
                .unwrap_or(0),
            holdable_item,
            holdable_tag: item_tag(self.product, holdable_item),
            respawned: ps.movement_flags & move_flags::RESPAWNED != 0,
            use_item_held: ps.movement_flags & move_flags::USE_ITEM_HELD != 0,
            event_sequence: ps.event_sequence,
            ..q3_spawn_arsenal_runtime(self.product, f64::from(ps.stats[0]), ps.event_sequence)
        }
    }
}

/// Project a guest player for movement prediction (donor
/// `guestMovementProjection`).
#[must_use]
pub fn guest_movement_projection<'a>(
    guest: &'a Q3QvmServerGame,
    player: &Q3ApplicationPlayer,
    recipe: &ExecutableRecipe,
) -> Q3MovementPredictionPlayer<'a> {
    let timing = recipe
        .timing
        .iter()
        .find(|timing| timing.provider == recipe.movement.provider)
        .unwrap_or_else(|| panic!("Recipe has no timing for its movement provider"));
    if !matches!(timing.clock, ClockProfile::Q3 { .. }) {
        panic!("QVM movement projection requires its selected movement adapter");
    }
    let character = provider_family(&recipe.character.definition.provider);
    let product = if recipe.map.entities.content.as_str().contains(":missionpack:") {
        Product::Missionpack
    } else {
        Product::Baseq3
    };
    Q3MovementPredictionPlayer {
        client: player.client.clone(),
        actor: guest.records.actor(player.source_entity),
        profile: MovementProfile {
            kind: MovementKind::Q3,
            id: timing.provider.clone(),
            numeric: timing.numeric,
        },
        standing_bounds: if character == GameFamily::Q3 {
            Q3_SOURCE_STANDING_BOUNDS
        } else {
            Bounds {
                min: qa_core::math::vec3(-16.0, -16.0, -24.0),
                max: qa_core::math::vec3(16.0, 16.0, 32.0),
            }
        },
        character,
        numeric: NumericOps::select(timing.numeric)
            .unwrap_or_else(|_| NumericOps::select(Q3_BINARY32_PROFILE).expect("q3 numeric profile")),
        gravity_multiplier: 1.0,
        movement_speed_multiplier: 1.0,
        guest,
        player: player.clone(),
        product,
        provider: recipe
            .weapons
            .first()
            .map(|weapon| weapon.provider.clone())
            .unwrap_or_else(|| recipe.map.entities.provider.clone()),
        character_provider: recipe.character.definition.provider.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::guest_records::{
        Q3GuestActorSource, Q3GuestBodyBinding, Q3GuestLeafQuery, Q3GuestModelTraceQuery, Q3GuestPointTarget,
        Q3GuestRecordActors, Q3GuestRecordBodies, Q3GuestRecordHost, Q3GuestScene, Q3GuestTraceQuery,
    };
    use super::super::guest_runtime::{
        Q3ApplicationAdmission, Q3GuestCommon, Q3GuestConsoleCommands, Q3GuestInitialization, Q3GuestOutput,
        Q3GuestRuntimeOptions,
    };
    use super::super::guest_world::{Q3GuestGeometry, Q3GuestGeometryKind, Q3GuestNativeClipWorld, Q3GuestTopology};
    use super::super::host::Q3HostSettings;
    use super::*;
    use qa_content::contract::{
        create_mount_plan_id, CampaignSelection, CharacterSelection, ContentId, DopplerSelection, EnemySelection,
        EnvironmentSelection, EquipmentSelection, FrameOrdering, GrappleSelection, HandGrenadeSelection,
        PresentationSelection, ProviderReference, ProviderTiming, RecipeId, ResolvedMap, ResolvedMountPlan,
    };
    use qa_content::mounts::open_mount_plan;
    use qa_content::q3::base::shared::definitions::StatSchema;
    use qa_content::q3::base::world::ActorTraceResult;
    use qa_core::identity::{ActorId, IdentityOwner, SavedActorId};
    use qa_core::math::{vec3, Bounds, Vec3};
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use qa_guest::qvm::game_data::QvmRole as GameRole;
    use qa_guest::qvm::game_data::{AbiProfile as GameAbi, ModuleIdentity, QvmArtifact, QvmImage};
    use qa_guest::qvm::player_record::write_qvm_player_state;
    use qa_guest::qvm::shared_entity_record::{write_qvm_shared_entity, QvmSharedEntity};
    use qa_platform::files::writable::UserFileStore;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct FakeActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        sources: RefCell<HashMap<ActorId, Q3GuestActorSource>>,
    }

    impl FakeActors {
        fn new(name: &str) -> Self {
            Self {
                owner: IdentityOwner::create(name).unwrap(),
                owned: RefCell::new(HashMap::new()),
                sources: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3GuestRecordActors for FakeActors {
        fn assert_owned(&self, actor: &OwnedActor) {
            assert!(self.owner.owns_owned(actor));
        }
        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            let id = self.owner.actor(slot as u32, 1);
            let owned = self.owner.owned_actor(&id, provider.clone()).unwrap();
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.sources.borrow_mut().insert(
                id,
                Q3GuestActorSource {
                    provider: provider.clone(),
                    slot,
                },
            );
            owned
        }
        fn source_of(&self, actor: &ActorId) -> Option<Q3GuestActorSource> {
            self.sources.borrow().get(actor).cloned()
        }
        fn at_source(&self, _provider: &ProviderId, slot: usize) -> Option<OwnedActor> {
            let sources = self.sources.borrow();
            let id = sources
                .iter()
                .find(|(_, source)| source.slot == slot)
                .map(|(id, _)| id.clone())?;
            drop(sources);
            self.owned.borrow().get(&id).cloned()
        }
        fn owned_by(&self, provider: &ProviderId) -> Vec<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .filter(|actor| actor.owner() == provider)
                .cloned()
                .collect()
        }
        fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .find(|actor| SavedActorId::from(actor.id()) == *saved)
                .cloned()
        }
        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }
        fn release(&self, actor: &OwnedActor) {
            self.owned.borrow_mut().remove(actor.id());
            self.sources.borrow_mut().remove(actor.id());
        }
        fn session(&self) -> qa_core::identity::SessionId {
            self.owner.session().clone()
        }
    }

    struct FakeBodies;
    impl Q3GuestRecordBodies for FakeBodies {
        fn bind(&self, _actor: &OwnedActor, _binding: Q3GuestBodyBinding) {}
        fn unlink(&self, _actor: &OwnedActor) {}
        fn link(&self, _actor: &OwnedActor) {}
    }

    struct FakeScene;
    impl Q3GuestTopology for FakeScene {
        fn geometry(&self) -> Q3GuestGeometry {
            Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q3Bsp,
                areas: Vec::new(),
                area_portals: Vec::new(),
                leaf_count: 1,
            }
        }
        fn adjust_area_portal_state(&self, _first: i32, _second: i32, _open: bool) {}
        fn adjust_area_portal_contribution(&self, _portal: i32, _delta: i32) {}
        fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>> {
            None
        }
    }
    impl Q3GuestScene for FakeScene {
        fn bind_actor_collision(&self, _read: super::super::guest_records::Q3GuestCollisionReader) {}
        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> Q3GuestLeafQuery {
            Q3GuestLeafQuery {
                leaves: vec![0],
                topnode: None,
                overflow: false,
            }
        }
        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }
        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }
        fn trace(&self, _query: &Q3GuestTraceQuery) -> ActorTraceResult {
            panic!("unused");
        }
        fn point_contents(&self, _point: Vec3, _target: &Q3GuestPointTarget, _pass_actor: Option<&ActorId>) -> i32 {
            panic!("unused");
        }
        fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
            Vec::new()
        }
        fn geometry_trace(&self, _query: &Q3GuestModelTraceQuery) -> ActorTraceResult {
            panic!("unused");
        }
        fn model_bounds(&self, _model: i32) -> Bounds {
            panic!("unused");
        }
        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }
        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }
        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            true
        }
    }

    struct FakeClip;
    impl super::super::guest_spatial::Q3GuestClipModel for FakeClip {
        fn transformed_point_contents(&self, _point: Vec3, _origin: Vec3, _angles: Vec3) -> i32 {
            0
        }
        fn transformed_trace_solid(
            &self,
            _start: Vec3,
            _end: Vec3,
            _mask: i32,
            _shape: qa_world::movement::types::TraceShape,
            _origin: Vec3,
            _angles: Vec3,
        ) -> bool {
            false
        }
    }

    struct FakeOutput;
    impl Q3GuestOutput for FakeOutput {
        fn drop_client(&self, _slot: i32, _reason: &str) {}
        fn send_server_command(&self, _slot: i32, _text: &str) {}
        fn configstring(&self, _index: i32, _value: &str) {}
    }

    fn provider(namespace: &str, name: &str) -> ProviderId {
        ProviderId::new(namespace, name)
    }

    fn reference(provider: ProviderId, content: &str) -> ProviderReference {
        ProviderReference {
            provider,
            content: ContentId(content.to_string()),
        }
    }

    fn recipe() -> ExecutableRecipe {
        let movement = provider("q3", "movement");
        ExecutableRecipe {
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            schema_version: 3,
            id: RecipeId("recipe:q3:test".to_string()),
            preset: RecipeId("recipe:q3:base".to_string()),
            map: ResolvedMap {
                geometry_content: ContentId("q3:baseq3:maps:q3dm1".to_string()),
                geometry: qa_content::contract::ResolvedResourceReference {
                    id: qa_content::contract::ResourceId("maps/q3dm1.bsp".to_string()),
                    requested_path: "maps/q3dm1.bsp".to_string(),
                    provenance: qa_content::contract::ResourceProvenance::Loose {
                        mount: qa_content::contract::LooseMount {
                            identity: qa_content::contract::MountIdentity {
                                id: qa_content::contract::MountId("mount:test:1".to_string()),
                                content: ContentId("q3:baseq3:maps:q3dm1".to_string()),
                                generation: 1,
                            },
                            root_path: "/tmp".to_string(),
                        },
                        member_path: "maps/q3dm1.bsp".to_string(),
                    },
                    identity: qa_content::contract::ResourceIdentity {
                        mount_generation: 1,
                        member_index: 0,
                        byte_length: 128,
                        crc: 0,
                    },
                    byte_length: 128,
                    resolution: qa_content::contract::ResourceResolution::DefaultOrder {
                        plan: create_mount_plan_id("test", "1").unwrap(),
                        rank: 0,
                    },
                },
                entities: reference(provider("q3", "entities"), "q3:baseq3:entities:1"),
            },
            campaign: CampaignSelection::None,
            movement: reference(movement.clone(), "q3:baseq3:movement:1"),
            character: CharacterSelection {
                definition: reference(provider("q3", "character"), "q3:baseq3:character:1"),
                appearance: reference(provider("q3", "appearance"), "q3:baseq3:appearance:1"),
            },
            weapons: vec![reference(provider("q3", "weapons"), "q3:baseq3:weapons:1")],
            equipment: EquipmentSelection {
                grapple: GrappleSelection::Disabled,
                hand_grenades: HandGrenadeSelection::Disabled,
            },
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Disabled,
                environment: EnvironmentSelection::Disabled,
                assets: ContentId("q3:baseq3".to_string()),
                hud: reference(provider("q3", "hud"), "q3:baseq3:hud:1"),
                effects: reference(provider("q3", "effects"), "q3:baseq3:effects:1"),
                audio: reference(provider("q3", "audio"), "q3:baseq3:audio:1"),
            },
            engine_behavior: reference(provider("q3", "engine"), "q3:baseq3:engine:1"),
            combat: reference(provider("q3", "combat"), "q3:baseq3:combat:1"),
            inventory: reference(provider("q3", "inventory"), "q3:baseq3:inventory:1"),
            r#match: reference(provider("q3", "match"), "q3:baseq3:match:1"),
            transition: reference(provider("q3", "transition"), "q3:baseq3:transition:1"),
            execution: Vec::new(),
            mounts: ResolvedMountPlan {
                id: create_mount_plan_id("test", "1").unwrap(),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            resources: Vec::new(),
            timing: vec![ProviderTiming {
                provider: movement,
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

    struct Harness {
        guest: Q3QvmServerGame,
        owner: Rc<IdentityOwner>,
        actors: Rc<FakeActors>,
    }

    fn harness(name: &str) -> Harness {
        use super::super::server_state::{Q3ServerState, Q3ServerStateOptions};
        use qa_content::mounts::{MountedContent, OpenMountOptions};
        let owner = Rc::new(IdentityOwner::create(name).unwrap());
        let actors = Rc::new(FakeActors::new(name));
        let session_owner = IdentityOwner::create("q3-move-server").unwrap();
        let state = Q3ServerState::new(Q3ServerStateOptions {
            session: session_owner.session().clone(),
            settings: Q3HostSettings {
                game_type: 0,
                single_player: false,
                max_clients: 8,
                map_name: "q3dm1".to_string(),
                source_registry: None,
                source_archive: Vec::new(),
                cvars: Vec::new(),
            },
            now: Rc::new(|| 0),
            print: Rc::new(|_| {}),
            register_server_cvars: Rc::new(|registry, max_clients, map_name| {
                registry.register("sv_maxclients", &max_clients.to_string(), 0).unwrap();
                registry.register("mapname", map_name, 0).unwrap();
            }),
        });
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("move-test", "1").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        let mounts: MountedContent = open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("qa-move-{name}-{}", std::process::id()));
        let guest = Q3QvmServerGame::new(Q3GuestRuntimeOptions {
            artifact: QvmArtifact {
                module: ModuleIdentity {
                    id: "test:qagame".to_string(),
                    artifact_path: "test".to_string(),
                    digest: "test".to_string(),
                    revision: "1".to_string(),
                },
                role: GameRole::Qagame,
                abi_profile: None,
                image: QvmImage {
                    allocated_data_length: 65536,
                    ..Default::default()
                },
            },
            state,
            records: Q3GuestRecordHost {
                actors: actors.clone(),
                bodies: Rc::new(FakeBodies),
                scene: Rc::new(FakeScene),
                provider: ProviderId::new("q3", "guest-test"),
                collision: Rc::new(|_, _| {}),
                admit: None,
            },
            mounts,
            writable: UserFileStore::new(dir),
            max_clients: 8,
            dedicated: None,
            seed: 1,
            entity_text: String::new(),
            common: Q3GuestCommon {
                milliseconds: Rc::new(|| 0),
                real_time: Rc::new(|_| 0),
                commands: Q3GuestConsoleCommands {
                    execute_now: Rc::new(|_| {}),
                    insert: Rc::new(|_| {}),
                    append: Rc::new(|_| {}),
                },
            },
            now: Rc::new(|| 0),
            assert_current: Rc::new(|| {}),
            clip: Rc::new(|_, _| Rc::new(FakeClip)),
            before_disconnect: None,
            before_retire: None,
            source_restored: None,
            arsenal: None,
            client_changed: None,
            bot_command: None,
        })
        .unwrap();
        guest.game.data.locate(64, 16, 1024, 24576, 1024).unwrap();
        guest.state.cvars.borrow_mut().set("bot_enable", "0", true).unwrap();
        guest
            .initialize(Rc::new(FakeOutput), Q3GuestInitialization::New)
            .unwrap();
        Harness { guest, owner, actors }
    }

    fn admit(harness: &Harness, slot: u32) -> super::super::guest_runtime::Q3ApplicationPlayer {
        let client = harness.owner.client(slot, 1);
        let admission = harness.guest.connect(&client, "").unwrap();
        let Q3ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        player
    }

    fn write_ps(harness: &Harness, slot: i32, patch: impl FnOnce(&mut QvmPlayerState)) {
        let mut ps = harness.guest.records.player(slot);
        patch(&mut ps);
        let tables = harness.guest.game.data.checkpoint();
        let mut bytes = vec![0u8; tables.client_stride];
        write_qvm_player_state(&mut bytes, &ps, GameAbi::Modern).unwrap();
        harness
            .guest
            .game
            .module
            .memory()
            .write_bytes(tables.clients_word + slot as usize * tables.client_stride, &bytes)
            .unwrap();
    }

    fn write_entity(harness: &Harness, slot: i32, patch: impl FnOnce(&mut QvmSharedEntity)) {
        let mut entity = QvmSharedEntity::default();
        patch(&mut entity);
        let tables = harness.guest.game.data.checkpoint();
        let mut bytes = vec![0u8; tables.entity_stride];
        write_qvm_shared_entity(&mut bytes, &entity, GameAbi::Modern).unwrap();
        harness
            .guest
            .game
            .module
            .memory()
            .write_bytes(tables.entities_word + slot as usize * tables.entity_stride, &bytes)
            .unwrap();
    }

    #[test]
    fn projection_reads_live_records() {
        let harness = harness("move-live");
        let player = admit(&harness, 0);
        write_entity(&harness, 0, |entity| {
            entity.r.mins = vec3(-15.0, -15.0, -24.0);
            entity.r.maxs = vec3(15.0, 15.0, 32.0);
        });
        write_ps(&harness, 0, |ps| {
            ps.stats[0] = 125;
            ps.stats[6] = 100;
            ps.powerups[Powerup::PwFlight as usize] = 1;
            ps.gravity = 800;
            ps.view_height = 26;
            ps.weapon = 5;
            ps.weapon_state = 2;
            ps.weapon_time_ms = 300;
            ps.legs_animation = 7;
            ps.torso_animation = 9;
            ps.legs_timer_ms = 70;
            ps.torso_timer_ms = 90;
            ps.ground_entity_number = 1022;
            ps.movement_type = 0;
            ps.event_sequence = 4;
            ps.movement_frame_count = 11;
        });
        let projection = guest_movement_projection(&harness.guest, &player, &recipe());
        assert_eq!(projection.character, GameFamily::Q3);
        assert_eq!(projection.profile.kind, MovementKind::Q3);
        assert_eq!(projection.gravity_multiplier, 1.0);
        assert!(projection.flight());
        assert_eq!(projection.world_gravity(), 800);
        assert_eq!(projection.view_height(), 26);
        let environment = projection.source_environment();
        assert_eq!(environment.health, 125.0);
        assert!(environment.flight);
        assert!(!environment.haste);
        assert!(!environment.invulnerable);
        let movement = projection.source_movement();
        assert_eq!(movement.trace_mask, 0x0201_0001);
        assert_eq!(movement.fixed_msec, None);
        assert!(!movement.no_footsteps);
        let bounds = projection.bounds();
        assert_eq!(bounds.min, vec3(-15.0, -15.0, -24.0));
        assert_eq!(bounds.max, vec3(15.0, 15.0, 32.0));
        let state = projection.movement_state();
        assert!(matches!(state.ground, TraceHit::World { model: 0 }));
        assert_eq!(state.movement_frame, 11);
        assert_eq!(state.jump_pad, None);
        let animation = projection.animation();
        assert!(matches!(
            animation.state,
            AnimationState::Q3 {
                legs: 7,
                torso: 9,
                legs_timer_milliseconds: 70,
                torso_timer_milliseconds: 90
            }
        ));
        let arsenal = projection.q3_arsenal();
        assert_eq!(arsenal.max_health, 100.0);
        assert!(!arsenal.spectator);
    }

    #[test]
    fn dead_client_uses_dead_trace_mask_and_fixed_step() {
        let harness = harness("move-dead");
        let player = admit(&harness, 0);
        write_ps(&harness, 0, |ps| {
            ps.stats[0] = 0;
        });
        harness
            .guest
            .state
            .cvars
            .borrow_mut()
            .register("pmove_fixed", "0", 0)
            .unwrap();
        harness
            .guest
            .state
            .cvars
            .borrow_mut()
            .register("pmove_msec", "8", 0)
            .unwrap();
        harness
            .guest
            .state
            .cvars
            .borrow_mut()
            .register("dmflags", "0", 0)
            .unwrap();
        let projection = guest_movement_projection(&harness.guest, &player, &recipe());
        assert_eq!(projection.source_movement().trace_mask, 0x1_0001);
        harness
            .guest
            .state
            .cvars
            .borrow_mut()
            .set("pmove_fixed", "1", false)
            .unwrap();
        harness
            .guest
            .state
            .cvars
            .borrow_mut()
            .set("pmove_msec", "50", false)
            .unwrap();
        harness
            .guest
            .state
            .cvars
            .borrow_mut()
            .set("dmflags", "32", false)
            .unwrap();
        let movement = projection.source_movement();
        assert_eq!(movement.fixed_msec, Some(33));
        assert!(movement.no_footsteps);
    }

    #[test]
    fn ground_and_jump_pad_resolve_actors() {
        let harness = harness("move-ground");
        harness
            .actors
            .allocate_at_source(&ProviderId::new("q3", "guest-test"), 5, "pad");
        let player = admit(&harness, 0);
        write_ps(&harness, 0, |ps| {
            ps.ground_entity_number = 5;
            ps.jump_pad_entity = 5;
        });
        let projection = guest_movement_projection(&harness.guest, &player, &recipe());
        let state = projection.movement_state();
        let expected = harness.guest.records.actor(5).id().clone();
        assert!(matches!(state.ground, TraceHit::Actor { ref actor } if actor == &expected));
        assert_eq!(state.jump_pad, Some(expected));
        write_ps(&harness, 0, |ps| {
            ps.ground_entity_number = 1023;
            ps.jump_pad_entity = 0;
        });
        let state = projection.movement_state();
        assert!(matches!(state.ground, TraceHit::None));
        assert_eq!(state.jump_pad, None);
    }

    #[test]
    fn arsenal_counts_weapons_and_ammo() {
        use qa_content::q3::foundation::arsenal::q3_weapon_item;
        let harness = harness("move-arsenal");
        let player = admit(&harness, 0);
        let schema = match stat_schema(Product::Baseq3) {
            StatSchema::Base(layout) => layout,
            StatSchema::Missionpack(_) => panic!("baseq3"),
        };
        write_ps(&harness, 0, |ps| {
            ps.stats[schema.weapons as usize] = (1 << 5) | (1 << 1);
            ps.ammo[5] = 17;
            ps.weapon = 5;
        });
        let projection = guest_movement_projection(&harness.guest, &player, &recipe());
        let arsenal = projection.arsenal();
        assert_eq!(arsenal.provider, ProviderId::new("q3", "weapons"));
        assert_eq!(
            arsenal.active_weapon,
            q3_weapon_item(5).map(|weapon| weapon.item.clone())
        );
        let weapon_entry = arsenal
            .ammo
            .iter()
            .find(|entry| Some(&entry.item) == q3_weapon_item(5).map(|weapon| &weapon.item));
        assert_eq!(weapon_entry.map(|entry| entry.count), Some(1.0));
        let missing = arsenal
            .ammo
            .iter()
            .find(|entry| Some(&entry.item) == q3_weapon_item(6).map(|weapon| &weapon.item));
        assert_eq!(missing.map(|entry| entry.count), Some(0.0));
    }

    #[test]
    fn spectator_literal_matches_donor() {
        let harness = harness("move-spectator");
        let player = admit(&harness, 0);
        write_ps(&harness, 0, |ps| {
            ps.movement_type = 1;
        });
        let projection = guest_movement_projection(&harness.guest, &player, &recipe());
        assert!(projection.q3_arsenal().spectator);
        write_ps(&harness, 0, |ps| {
            ps.movement_type = 2;
        });
        assert!(!projection.q3_arsenal().spectator);
    }

    #[test]
    #[should_panic(expected = "QVM movement projection requires its selected movement adapter")]
    fn non_q3_timing_rejected() {
        let harness = harness("move-reject");
        let player = admit(&harness, 0);
        let mut bad = recipe();
        bad.timing[0].clock = ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: 30.0,
        };
        let _ = guest_movement_projection(&harness.guest, &player, &bad);
    }
}
