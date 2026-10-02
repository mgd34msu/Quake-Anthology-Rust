//! Q3 guest bot library over the shared selected navigation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/guest-bots.ts`.
//!
//! Missing siblings: `ApplicationBotNavigation` (`navigation.ts`, navigation
//! partition). The [`GuestBotSelectedNavigation`] seam exposes exactly the
//! donor's selected-navigation surface (area queries plus persistence); the
//! navigation partition implements it post-merge.
//!
//! Three adaptations are forced by missing `qa_bots`/`qa_guest`
//! capabilities (canonical homes noted; unify post-merge):
//!
//! * The elementary syscall surfaces (`BotLibraryHost`, `AasHost`,
//!   `BotMoveHost`) have no production implementations, so [`Q3GuestBots::syscall`]
//!   takes caller-provided hosts. The guest half of the split — entity
//!   observations, navigation callbacks, module services, lifecycle, and
//!   persistence — is implemented here.
//! * Library save-state has no Rust home, so checkpoints capture
//!   `library: null` and restore requires it (the `guest_runtime.rs`
//!   precedent for unported botlib state).
//! * The level-item scan in [`Q3GuestBots::load_map`] mirrors
//!   `initLevelItems` from donor `src/bots/behavior/library/goals.ts`
//!   (canonical home: `qa_bots::behavior::library::goals`); map-location
//!   and camp-spot info entities have no Rust storage and are skipped.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_bots::behavior::assets::BotSourceFiles;
use qa_bots::behavior::library::bsp_entities::{AasBspEntities, BspEntity};
use qa_bots::behavior::library::goals::GoalError;
use qa_bots::behavior::library::log::MemoryLogSink;
use qa_bots::behavior::library::memory::{BotMemoryKind, BotMemoryProvenance};
use qa_bots::behavior::library::weapons::WeaponLoadResult;
use qa_bots::behavior::q3::library::BotLibrary;
use qa_core::math::{Bounds, Vec3};
use qa_guest::qvm::bot_library_syscalls::{bot_library_syscall, BotLibraryHost, BotModuleServices};
use qa_guest::qvm::bot_navigation_records::{
    read_bot_entity_state, AasEntityInfo, BotEntityUpdate, QVM_BOT_ENTITY_STATE_BYTES,
};
use qa_guest::qvm::bot_navigation_syscalls::{bot_navigation_syscall, AasHost, BotMoveHost};
use qa_guest::qvm::client_collision_syscalls::{TraceRecord, TraceShape as GuestTraceShape};
use qa_guest::qvm::client_state::{CallKind, HostCall, QvmRole, SyscallMemory, WireUserCommand};
use qa_guest::qvm::server_game_syscalls::{ServerSpatialHost, ServerTraceQuery};
use qa_guest::qvm::shared_entity_record::QvmEntityCollisionModel;
use qa_guest::GuestError;
use qa_world::save::value::{arr, boolean, int, num, obj, str as json_str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::guest_records::Q3GuestRecords;
use super::guest_spatial::Q3GuestSpatial;

/// Selected-navigation area queries (donor `SelectedBotNavigation` surface).
pub trait GuestBotSelectedNavigation {
    /// Donor `navigation.pointArea`.
    fn point_area(&self, point: Vec3) -> i32;
    /// Donor `navigation.bestReachableArea`.
    fn best_reachable_area(&self, origin: Vec3, bounds: Bounds) -> (i32, Vec3);
    /// Donor `navigation.dropToFloor`.
    fn drop_to_floor(&self, origin: Vec3, bounds: Bounds) -> (Vec3, bool);
    /// Donor `navigation.bestReachableFromJumpPadArea`.
    fn best_reachable_from_jumppad_area(&self, origin: Vec3, bounds: Bounds) -> i32;
    /// Donor `navigation.checkpoint()`.
    fn checkpoint_navigation(&self) -> SaveJson;
    /// Donor `navigation.restoreCheckpoint(...)`.
    fn restore_navigation(&mut self, value: &SaveJson) -> Result<(), WorldError>;
}

/// Guest bot client callbacks (donor `options.clients` surface).
pub trait Q3GuestBotClients {
    /// Allocate a bot client slot.
    fn allocate_client(&mut self) -> i32;
    /// Free a bot client slot.
    fn free_client(&mut self, client: i32);
    /// Snapshot one entity for a client.
    fn snapshot_entity(&mut self, client: i32, sequence: i32) -> i32;
    /// Take a pending console message for a client.
    fn console_message(&mut self, client: i32) -> Option<String>;
    /// Deliver a user command.
    fn user_command(&mut self, client: i32, command: WireUserCommand);
}

/// Guest bot construction options (donor `Q3GuestBotOptions`).
pub struct Q3GuestBotOptions<'f, 'r, 's, S, C> {
    /// Bot source files.
    pub files: &'f dyn BotSourceFiles,
    /// Print diagnostics.
    pub print: Rc<dyn Fn(&str)>,
    /// Maximum bot clients.
    pub clients_count: usize,
    /// Debug logging.
    pub debug: bool,
    /// Selected navigation.
    pub selected: S,
    /// Guest records.
    pub records: &'r Q3GuestRecords,
    /// Guest spatial queries.
    pub spatial: &'s RefCell<Q3GuestSpatial>,
    /// BSP entity text.
    pub entities: String,
    /// Map name.
    pub map_name: String,
    /// Client callbacks.
    pub clients: C,
}

/// Guest navigation trace result (donor navigation `trace` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct GuestBotTrace {
    /// Fraction reached.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Contents at the hit.
    pub contents: i32,
    /// Surface flags at the hit.
    pub surface_flags: i32,
    /// Trap solidity code (0 open, 1 all-solid, 2 start-solid).
    pub solidity: i32,
    /// Contact plane normal.
    pub plane_normal: Vec3,
    /// Contact plane distance.
    pub plane_distance: f32,
}

/// Guest entity model info (donor `modelInfo` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct GuestBotModelInfo {
    /// Entity number.
    pub entity: i32,
    /// Origin.
    pub origin: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Classname from BSP entities.
    pub classname: String,
}

/// Q3 guest bots (donor `Q3GuestBots`).
pub struct Q3GuestBots<'f, 'r, 's, S, C> {
    library: BotLibrary<'f>,
    files: &'f dyn BotSourceFiles,
    bsp_entities: AasBspEntities,
    observations: HashMap<i32, AasEntityInfo>,
    initialized: bool,
    loaded: bool,
    selected: S,
    records: &'r Q3GuestRecords,
    spatial: &'s RefCell<Q3GuestSpatial>,
    entities: String,
    map_name: String,
    clients: C,
    print: Rc<dyn Fn(&str)>,
    log_sink: MemoryLogSink,
}

impl<'f, 'r, 's, S: GuestBotSelectedNavigation, C: Q3GuestBotClients> Q3GuestBots<'f, 'r, 's, S, C> {
    /// Create the guest bot engine (donor constructor).
    pub fn new(options: Q3GuestBotOptions<'f, 'r, 's, S, C>) -> Self {
        Self {
            library: BotLibrary::new(options.files, options.clients_count, options.debug),
            files: options.files,
            bsp_entities: AasBspEntities::new(),
            observations: HashMap::new(),
            initialized: false,
            loaded: false,
            selected: options.selected,
            records: options.records,
            spatial: options.spatial,
            entities: options.entities,
            map_name: options.map_name,
            clients: options.clients,
            print: options.print,
            log_sink: MemoryLogSink::default(),
        }
    }

    /// Borrow the bot library (donor `library`).
    pub fn library(&self) -> &BotLibrary<'f> {
        &self.library
    }

    /// Borrow the BSP entities (donor `bspEntities`).
    pub fn bsp_entities(&self) -> &AasBspEntities {
        &self.bsp_entities
    }

    /// Whether setup succeeded (donor `initialized`).
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Whether a map is loaded (donor `loaded`).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Contents at a point (donor navigation `pointContents`).
    pub fn guest_point_contents(&self, point: Vec3, entity: i32) -> i32 {
        self.spatial.borrow_mut().point_contents(point, entity)
    }

    /// Trace the guest world (donor navigation `trace`).
    pub fn guest_trace(
        &self,
        start: Vec3,
        bounds: Option<Bounds>,
        end: Vec3,
        pass_entity: i32,
        contents: i32,
    ) -> GuestBotTrace {
        let (mins, maxs) = match bounds {
            None => (Vec3::default(), Vec3::default()),
            Some(bounds) => (bounds.min, bounds.max),
        };
        let result: TraceRecord = self.spatial.borrow_mut().trace(&ServerTraceQuery {
            start,
            end,
            mins,
            maxs,
            shape: GuestTraceShape::Capsule,
            pass_entity_num: pass_entity,
            mask: contents,
        });
        GuestBotTrace {
            fraction: result.fraction,
            end: result.end,
            entity_num: result.entity_num,
            contents: result.contents,
            surface_flags: result.surface_flags,
            solidity: if result.all_solid {
                1
            } else if result.start_solid {
                2
            } else {
                0
            },
            plane_normal: result.plane_normal,
            plane_distance: result.plane_distance,
        }
    }

    /// Model index for an observed entity (donor `entityModelIndex`).
    pub fn entity_model_index(&self, entity: i32) -> i32 {
        self.observations.get(&entity).map_or(0, |info| info.update.model_index)
    }

    /// Entity type for an observed entity (donor `entityType`).
    pub fn entity_type(&self, entity: i32) -> i32 {
        self.observations.get(&entity).map_or(0, |info| info.update.entity_type)
    }

    /// Weapon for an observed entity (donor `entityWeapon`).
    pub fn entity_weapon(&self, entity: i32) -> i32 {
        self.observations.get(&entity).map_or(0, |info| info.update.weapon)
    }

    /// Next observed entity after one (donor `nextEntity`).
    pub fn next_entity(&self, after: i32) -> Option<i32> {
        self.observations
            .keys()
            .filter(|number| **number > after)
            .min()
            .copied()
    }

    /// Model info for an entity slot (donor `modelInfo`).
    pub fn model_info(&self, slot: i32) -> Option<GuestBotModelInfo> {
        let info = self.observations.get(&slot)?;
        let entity = self.records.entity(slot);
        if !matches!(entity.r.model, QvmEntityCollisionModel::Inline { .. }) {
            return None;
        }
        Some(GuestBotModelInfo {
            entity: slot,
            origin: info.update.origin,
            bounds: Bounds {
                min: info.update.mins,
                max: info.update.maxs,
            },
            classname: self.model_classname(&entity.r.current_origin),
        })
    }

    /// Match a static BSP model by origin (donor `modelClassname`).
    ///
    /// The donor compares JS-formatted origin text; the port compares parsed
    /// origins, which agrees up to float formatting.
    fn model_classname(&self, origin: &Vec3) -> String {
        for entity in self.bsp_entities.entities() {
            if entity.value("classname") != "func_static" {
                continue;
            }
            if entity.value("origin").split_whitespace().count() == 0 {
                return "func_static".to_string();
            }
            if entity_vector(entity, "origin") == Some(*origin) {
                return "func_static".to_string();
            }
        }
        String::new()
    }

    /// Run one syscall through the bot traps (donor `syscall`).
    pub fn syscall(
        &mut self,
        call: &HostCall,
        memory: &mut SyscallMemory,
        library: &mut dyn BotLibraryHost,
        aas: &mut dyn AasHost,
        moves: &mut dyn BotMoveHost,
    ) -> Result<Option<i32>, GuestError> {
        if call.kind != CallKind::Engine || call.role != QvmRole::Qagame {
            return Ok(None);
        }
        if let Some(result) = bot_library_syscall(call, memory, library, self)? {
            return Ok(Some(result));
        }
        bot_navigation_syscall(call, memory, aas, moves)
    }

    /// Close the engine (donor `close`).
    pub fn close(&mut self) {
        self.shutdown_module();
    }

    fn setup_module(&mut self) -> i32 {
        if !self.library.is_initialized() && self.library.setup().is_err() {
            panic!("Bot behavior library is closed");
        }
        (self.print)("------- BotLib Initialization -------\n");
        let weapon_config = self.library.variables.string("weaponconfig", "weapons.c");
        let weapons = self.library.weapons.load_weapons(&weapon_config);
        if weapons != WeaponLoadResult::NoError {
            return weapons as i32;
        }
        let item_config = self.library.variables.string("itemconfig", "items.c");
        let goals = self.library.goals.load_item_config(&item_config);
        if goals != GoalError::NONE {
            return goals;
        }
        self.library.variables.string("droppedweight", "1000");
        self.initialized = true;
        0
    }

    fn shutdown_module(&mut self) -> i32 {
        self.initialized = false;
        self.loaded = false;
        self.library.shutdown(&mut self.log_sink);
        for (_, message) in std::mem::take(&mut self.log_sink.messages) {
            (self.print)(&message);
        }
        0
    }

    fn load_map_module(&mut self, name: &str) -> i32 {
        if name != self.map_name {
            panic!("Guest bots cannot switch maps without a full guest reset");
        }
        let entities = self.entities.clone();
        if self.bsp_entities.load(&entities).is_err() {
            return 1;
        }
        self.observations.clear();
        self.scan_level_items();
        self.loaded = true;
        0
    }

    /// Mirror of donor `initLevelItems` without info entities (see module docs).
    fn scan_level_items(&mut self) {
        self.library.goals.clear_level_items();
        let Some(config) = self.library.goals.item_config.clone() else {
            return;
        };
        let print = self.print.clone();
        for info in &config.items {
            if info.model_index == 0 {
                (print)(&format!("item {} has modelindex 0", info.classname));
            }
        }
        let count = self.bsp_entities.entities().len();
        for index in 1..=count {
            let row = self.bsp_entities.entities().get(index - 1).map(|entity| {
                (
                    entity.value("classname").to_string(),
                    entity.int("spawnflags"),
                    entity.value("origin").to_string(),
                    entity.int("notfree") != 0,
                    entity.int("notteam") != 0,
                    entity.int("notsingle") != 0,
                    entity.int("notbot") != 0,
                    entity.float("weight"),
                )
            });
            let Some((classname, spawnflags, origin_text, notfree, notteam, notsingle, notbot, botroam_weight)) = row
            else {
                continue;
            };
            if classname.is_empty() {
                continue;
            }
            let Some(info_index) = config.items.iter().position(|info| info.classname == classname) else {
                (print)(&format!("entity {classname} unknown item\r\n"));
                continue;
            };
            let info = &config.items[info_index];
            let Some(mut origin) = parse_vec3(&origin_text) else {
                (print)(&format!("item {classname} without origin\n"));
                continue;
            };
            let bounds = Bounds {
                min: info.mins,
                max: info.maxs,
            };
            let mut goal_area = 0;
            if spawnflags & 1 != 0 {
                let contents = self.guest_point_contents(origin, -1);
                if contents & 32 == 0 {
                    let trace = self.guest_trace(
                        origin,
                        Some(bounds),
                        Vec3 {
                            x: origin.x,
                            y: origin.y,
                            z: origin.z - 32.0,
                        },
                        -1,
                        1 | 0x10000,
                    );
                    if trace.fraction >= 1.0 {
                        goal_area = self.selected.best_reachable_from_jumppad_area(origin, bounds);
                        (print)(&format!(
                            "item {} reachable from jumppad area {goal_area}\r\n",
                            info.classname
                        ));
                        if goal_area == 0 {
                            continue;
                        }
                    }
                }
            }
            let mut flags = 0;
            if notfree {
                flags |= 1;
            }
            if notteam {
                flags |= 2;
            }
            if notsingle {
                flags |= 4;
            }
            if notbot {
                flags |= 8;
            }
            let mut weight = 0.0f32;
            if classname == "item_botroam" {
                flags |= 16;
                weight = botroam_weight;
            }
            if spawnflags & 1 == 0 {
                let (dropped, success) = self.selected.drop_to_floor(origin, bounds);
                origin = dropped;
                if !success {
                    (print)(&format!(
                        "{classname} in solid at ({:.1} {:.1} {:.1})\n",
                        origin.x, origin.y, origin.z
                    ));
                }
            }
            let (area, _) = if goal_area == 0 {
                self.selected.best_reachable_area(origin, bounds)
            } else {
                (goal_area, origin)
            };
            if area == 0 {
                (print)(&format!(
                    "{classname} not reachable for bots at ({:.1} {:.1} {:.1})\n",
                    origin.x, origin.y, origin.z
                ));
            }
            let number = self.library.goals.add_level_item(info_index, 0, origin, area, flags);
            if let Some(item) = self
                .library
                .goals
                .level_items
                .iter_mut()
                .find(|item| item.number == number)
            {
                item.weight = weight;
            }
        }
        (print)(&format!("found {} level items\n", self.library.goals.level_items.len()));
    }

    fn update_entity_module(&mut self, memory: &mut SyscallMemory, number: i32, word: i32) -> Result<(), GuestError> {
        if word == 0 {
            self.observations.remove(&number);
            return Ok(());
        }
        let view = memory.span(word, QVM_BOT_ENTITY_STATE_BYTES, 0)?;
        let bytes = memory.read_bytes(view.start, QVM_BOT_ENTITY_STATE_BYTES)?.to_vec();
        let state = read_bot_entity_state(&bytes)?;
        let time = self.library.time();
        let previous = self.observations.get(&number);
        let interval = previous.map_or(0.0, |info| time - info.last_update_time);
        let origin = previous.map_or(state.origin, |info| info.last_visible_origin);
        self.observations.insert(
            number,
            AasEntityInfo {
                valid: true,
                number,
                update: state,
                last_visible_origin: origin,
                last_update_time: time,
                update_interval: interval,
            },
        );
        Ok(())
    }
}

fn parse_vec3(text: &str) -> Option<Vec3> {
    let parts: Vec<f32> = text.split_whitespace().filter_map(|part| part.parse().ok()).collect();
    if parts.len() != 3 {
        return None;
    }
    Some(Vec3 {
        x: parts[0],
        y: parts[1],
        z: parts[2],
    })
}

fn entity_vector(entity: &BspEntity, key: &str) -> Option<Vec3> {
    parse_vec3(entity.value(key))
}

impl<S: GuestBotSelectedNavigation, C: Q3GuestBotClients> BotModuleServices for Q3GuestBots<'_, '_, '_, S, C> {
    fn setup(&mut self) -> i32 {
        self.setup_module()
    }

    fn shutdown(&mut self) -> i32 {
        self.shutdown_module()
    }

    fn load_map(&mut self, name: &str) -> i32 {
        self.load_map_module(name)
    }

    fn update_entity(&mut self, memory: &mut SyscallMemory, number: i32, word: i32) -> i32 {
        // The trap return is ignored by the guest; ABI failures report -1.
        match self.update_entity_module(memory, number, word) {
            Ok(()) => 0,
            Err(error) => {
                (self.print)(&format!("guest bot entity update failed: {error}"));
                -1
            }
        }
    }

    fn snapshot_entity(&mut self, client: i32, sequence: i32) -> i32 {
        self.clients.snapshot_entity(client, sequence)
    }

    fn console_message(&mut self, client: i32) -> Option<String> {
        self.clients.console_message(client)
    }

    fn user_command(&mut self, client: i32, command: WireUserCommand) {
        self.clients.user_command(client, command);
    }

    fn allocate_client(&mut self) -> i32 {
        self.clients.allocate_client()
    }

    fn free_client(&mut self, client: i32) {
        self.clients.free_client(client);
    }
}

fn write_vec3(value: &Vec3) -> SaveJson {
    arr(vec![
        num(f64::from(value.x)),
        num(f64::from(value.y)),
        num(f64::from(value.z)),
    ])
}

fn read_vec3(reader: SaveReader<'_>) -> Result<Vec3, WorldError> {
    let parts = reader.list(|entry| entry.finite())?;
    if parts.len() != 3 {
        return Err(reader.fail("expected a 3-component vector"));
    }
    Ok(Vec3 {
        x: parts[0] as f32,
        y: parts[1] as f32,
        z: parts[2] as f32,
    })
}

fn write_entity_update(update: &BotEntityUpdate) -> SaveJson {
    obj(vec![
        ("type", int(i64::from(update.entity_type))),
        ("flags", int(i64::from(update.flags))),
        ("origin", write_vec3(&update.origin)),
        ("oldOrigin", write_vec3(&update.old_origin)),
        ("angles", write_vec3(&update.angles)),
        ("mins", write_vec3(&update.mins)),
        ("maxs", write_vec3(&update.maxs)),
        ("groundEntity", int(i64::from(update.ground_entity))),
        ("solid", int(i64::from(update.solid))),
        ("modelIndex", int(i64::from(update.model_index))),
        ("modelIndex2", int(i64::from(update.model_index2))),
        ("frame", int(i64::from(update.frame))),
        ("event", int(i64::from(update.event))),
        ("eventParameter", int(i64::from(update.event_parameter))),
        ("powerups", int(i64::from(update.powerups))),
        ("weapon", int(i64::from(update.weapon))),
        ("legsAnimation", int(i64::from(update.legs_animation))),
        ("torsoAnimation", int(i64::from(update.torso_animation))),
    ])
}

fn read_entity_update(reader: SaveReader<'_>) -> Result<BotEntityUpdate, WorldError> {
    // Entity updates carry signed fields (ground entity -1); accept the full
    // integer range like the donor, which validates the shape only.
    let word = |reader: SaveReader<'_>| reader.integer(i64::MIN).map(|value| value as i32);
    Ok(BotEntityUpdate {
        entity_type: word(reader.field("type"))?,
        flags: word(reader.field("flags"))?,
        origin: read_vec3(reader.field("origin"))?,
        old_origin: read_vec3(reader.field("oldOrigin"))?,
        angles: read_vec3(reader.field("angles"))?,
        mins: read_vec3(reader.field("mins"))?,
        maxs: read_vec3(reader.field("maxs"))?,
        ground_entity: word(reader.field("groundEntity"))?,
        solid: word(reader.field("solid"))?,
        model_index: word(reader.field("modelIndex"))?,
        model_index2: word(reader.field("modelIndex2"))?,
        frame: word(reader.field("frame"))?,
        event: word(reader.field("event"))?,
        event_parameter: word(reader.field("eventParameter"))?,
        powerups: word(reader.field("powerups"))?,
        weapon: word(reader.field("weapon"))?,
        legs_animation: word(reader.field("legsAnimation"))?,
        torso_animation: word(reader.field("torsoAnimation"))?,
    })
}

impl<S: GuestBotSelectedNavigation, C: Q3GuestBotClients> Q3GuestBots<'_, '_, '_, S, C> {
    /// Capture the checkpoint (donor `checkpoint`).
    pub fn checkpoint(&self) -> SaveJson {
        let Some(assets) = self.files.provenance() else {
            panic!("Guest bot persistence requires mounted asset provenance");
        };
        let capture = self.library.memory.checkpoint();
        let memory = arr(capture
            .image()
            .allocations
            .iter()
            .map(|allocation| {
                obj(vec![
                    (
                        "kind",
                        json_str(match allocation.kind {
                            BotMemoryKind::Heap => "heap",
                            BotMemoryKind::Hunk => "hunk",
                        }),
                    ),
                    (
                        "bytes",
                        arr(allocation.bytes.iter().map(|byte| int(i64::from(*byte))).collect()),
                    ),
                    (
                        "provenance",
                        allocation.provenance.as_ref().map_or(SaveJson::Null, |provenance| {
                            obj(vec![
                                ("file", json_str(&provenance.file)),
                                ("line", int(i64::from(provenance.line))),
                                ("label", json_str(&provenance.label)),
                            ])
                        }),
                    ),
                ])
            })
            .collect());
        let bsp = arr(self
            .bsp_entities
            .checkpoint()
            .iter()
            .map(|entity| {
                obj(vec![(
                    "pairs",
                    arr(entity
                        .epairs
                        .iter()
                        .map(|(key, value)| obj(vec![("key", json_str(key)), ("value", json_str(value))]))
                        .collect()),
                )])
            })
            .collect());
        let mut observations: Vec<(&i32, &AasEntityInfo)> = self.observations.iter().collect();
        observations.sort_by_key(|(number, _)| **number);
        let observations = arr(observations
            .iter()
            .map(|(number, info)| {
                obj(vec![
                    ("number", int(i64::from(**number))),
                    ("valid", boolean(info.valid)),
                    ("update", write_entity_update(&info.update)),
                    ("lastVisibleOrigin", write_vec3(&info.last_visible_origin)),
                    ("lastUpdateTime", num(f64::from(info.last_update_time))),
                    ("updateInterval", num(f64::from(info.update_interval))),
                ])
            })
            .collect());
        obj(vec![
            ("version", int(1)),
            ("assets", json_str(&assets)),
            ("initialized", boolean(self.initialized)),
            ("loaded", boolean(self.loaded)),
            ("memory", memory),
            ("library", SaveJson::Null),
            ("bsp", bsp),
            ("navigation", self.selected.checkpoint_navigation()),
            ("observations", observations),
        ])
    }

    /// Restore a checkpoint (donor `restore`).
    pub fn restore(&mut self, value: &SaveJson) -> Result<(), WorldError> {
        let reader = SaveReader::new(value);
        reader.field("version").literal_i64(1)?;
        let assets = self.files.provenance();
        let saved = reader.field("assets").string()?;
        if assets.as_ref() != Some(&saved) {
            return Err(reader.field("assets").fail("Guest bot assets differ from saved source"));
        }
        let navigation = reader.field("navigation").value.cloned().unwrap_or(SaveJson::Null);
        self.selected.restore_navigation(&navigation)?;
        let allocations = reader.field("memory").list(|entry| {
            let kind = entry.field("kind").choice_str(&["heap", "hunk"])?;
            let bytes = entry
                .field("bytes")
                .list(|byte| byte.integer(0).map(|byte| byte as u8))?;
            let provenance = entry.field("provenance").nullable(|provenance| {
                Ok(BotMemoryProvenance {
                    file: provenance.field("file").string()?,
                    line: provenance.field("line").integer(0)? as u32,
                    label: provenance.field("label").string()?,
                })
            })?;
            Ok(qa_bots::behavior::library::memory::BotMemoryCheckpointAllocation {
                kind: if kind == "heap" {
                    BotMemoryKind::Heap
                } else {
                    BotMemoryKind::Hunk
                },
                bytes,
                provenance,
            })
        })?;
        self.library
            .memory
            .restore(&qa_bots::behavior::library::memory::BotMemoryCheckpoint { allocations });
        let entities = reader.field("bsp").list(|entry| {
            let epairs: Vec<(String, String)> = entry
                .field("pairs")
                .list(|pair| Ok((pair.field("key").string()?, pair.field("value").string()?)))?;
            let map = epairs.iter().cloned().collect();
            Ok(BspEntity { epairs, map })
        })?;
        self.bsp_entities.restore(&entities);
        if !matches!(reader.field("library").value, None | Some(SaveJson::Null)) {
            return Err(reader
                .field("library")
                .fail("Saved guest bot library state is unavailable"));
        }
        let mut observations = HashMap::new();
        for entry in reader.field("observations").list(|entry| {
            Ok((
                entry.field("number").integer(0)? as i32,
                entry.field("valid").boolean()?,
                read_entity_update(entry.field("update"))?,
                read_vec3(entry.field("lastVisibleOrigin"))?,
                entry.field("lastUpdateTime").finite()? as f32,
                entry.field("updateInterval").finite()? as f32,
            ))
        })? {
            if observations.contains_key(&entry.0) {
                return Err(reader.fail("Duplicate guest bot observation"));
            }
            observations.insert(
                entry.0,
                AasEntityInfo {
                    valid: entry.1,
                    number: entry.0,
                    update: entry.2,
                    last_visible_origin: entry.3,
                    last_update_time: entry.4,
                    update_interval: entry.5,
                },
            );
        }
        self.observations = observations;
        self.initialized = reader.field("initialized").boolean()?;
        self.loaded = reader.field("loaded").boolean()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::assets::BotAssetFiles;
    use qa_bots::behavior::library::goals::{ItemConfig, ItemInfo};
    use qa_core::cmd::Dialect;
    use qa_core::cvar::CvarRegistry;
    use qa_core::identity::ActorId;
    use qa_core::identity::{IdentityOwner, OwnedActor, ProviderId, SavedActorId, SessionId};
    use qa_guest::qvm::game_data::{AbiProfile, QvmGameData, QvmSharedMemory};

    use super::super::guest_records::{
        Q3GuestActorCollision, Q3GuestActorSource, Q3GuestBodyBinding, Q3GuestLeafQuery, Q3GuestModelTraceQuery,
        Q3GuestPointTarget, Q3GuestRecordActors, Q3GuestRecordBodies, Q3GuestRecordHost, Q3GuestScene,
        Q3GuestTraceQuery,
    };
    use super::super::guest_spatial::{Q3GuestClipFactory, Q3GuestClipModel};
    use super::super::guest_world::{Q3GuestGeometry, Q3GuestGeometryKind, Q3GuestNativeClipWorld, Q3GuestTopology};
    use qa_content::q3::base::world::{ActorTraceHit, ActorTraceResult, TraceContact, TraceSolidity};
    use qa_world::movement::types::TraceShape;

    struct StubSelected;

    impl GuestBotSelectedNavigation for StubSelected {
        fn point_area(&self, _point: Vec3) -> i32 {
            7
        }
        fn best_reachable_area(&self, origin: Vec3, _bounds: Bounds) -> (i32, Vec3) {
            (7, origin)
        }
        fn drop_to_floor(&self, origin: Vec3, _bounds: Bounds) -> (Vec3, bool) {
            (origin, true)
        }
        fn best_reachable_from_jumppad_area(&self, _origin: Vec3, _bounds: Bounds) -> i32 {
            7
        }
        fn checkpoint_navigation(&self) -> SaveJson {
            obj(vec![])
        }
        fn restore_navigation(&mut self, _value: &SaveJson) -> Result<(), WorldError> {
            Ok(())
        }
    }

    struct StubClients;

    impl Q3GuestBotClients for StubClients {
        fn allocate_client(&mut self) -> i32 {
            0
        }
        fn free_client(&mut self, _client: i32) {}
        fn snapshot_entity(&mut self, _client: i32, _sequence: i32) -> i32 {
            0
        }
        fn console_message(&mut self, _client: i32) -> Option<String> {
            None
        }
        fn user_command(&mut self, _client: i32, _command: WireUserCommand) {}
    }

    struct FakeActors {
        owner: IdentityOwner,
    }

    impl Q3GuestRecordActors for FakeActors {
        fn assert_owned(&self, _actor: &OwnedActor) {}

        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            self.owner
                .owned_actor(&self.owner.actor(slot as u32, 1), provider.clone())
                .unwrap()
        }

        fn source_of(&self, _actor: &ActorId) -> Option<Q3GuestActorSource> {
            None
        }
        fn at_source(&self, _provider: &ProviderId, _slot: usize) -> Option<OwnedActor> {
            None
        }
        fn owned_by(&self, _provider: &ProviderId) -> Vec<OwnedActor> {
            Vec::new()
        }
        fn resolve_saved(&self, _saved: &SavedActorId) -> Option<OwnedActor> {
            None
        }
        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }
        fn release(&self, _actor: &OwnedActor) {}
        fn session(&self) -> SessionId {
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
        fn bind_actor_collision(&self, _read: Rc<dyn Fn(&ActorId) -> Option<Q3GuestActorCollision>>) {}
        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> Q3GuestLeafQuery {
            Q3GuestLeafQuery {
                leaves: Vec::new(),
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
            ActorTraceResult {
                fraction: 1.0,
                end: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                hit: ActorTraceHit::None,
                contact: TraceContact::None,
                solidity: TraceSolidity::Clear,
                contents: 0,
                surface_flags: 0,
            }
        }
        fn point_contents(&self, _point: Vec3, _target: &Q3GuestPointTarget, _pass_actor: Option<&ActorId>) -> i32 {
            0
        }
        fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
            Vec::new()
        }
        fn geometry_trace(&self, _query: &Q3GuestModelTraceQuery) -> ActorTraceResult {
            ActorTraceResult {
                fraction: 1.0,
                end: Vec3::default(),
                hit: ActorTraceHit::None,
                contact: TraceContact::None,
                solidity: TraceSolidity::Clear,
                contents: 0,
                surface_flags: 0,
            }
        }
        fn model_bounds(&self, _model: i32) -> Bounds {
            Bounds {
                min: Vec3::default(),
                max: Vec3::default(),
            }
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

    impl Q3GuestClipModel for FakeClip {
        fn transformed_point_contents(&self, _point: Vec3, _origin: Vec3, _angles: Vec3) -> i32 {
            0
        }
        fn transformed_trace_solid(
            &self,
            _start: Vec3,
            _end: Vec3,
            _mask: i32,
            _shape: TraceShape,
            _origin: Vec3,
            _angles: Vec3,
        ) -> bool {
            false
        }
    }

    fn records_and_spatial() -> (Rc<Q3GuestRecords>, Q3GuestSpatial) {
        let scene: Rc<dyn Q3GuestScene> = Rc::new(FakeScene);
        let memory = QvmSharedMemory::new(65536).unwrap();
        let data = QvmGameData::new(memory, AbiProfile::Modern);
        data.set_client_count(2).unwrap();
        data.locate(64, 8, 560, 8192, 560).unwrap();
        let records = Q3GuestRecords::open(
            data,
            Q3GuestRecordHost {
                actors: Rc::new(FakeActors {
                    owner: IdentityOwner::create("guest-bots").unwrap(),
                }),
                bodies: Rc::new(FakeBodies),
                scene: scene.clone(),
                provider: ProviderId::new("q3", "guest-test"),
                collision: Rc::new(|_, _| {}),
                admit: None,
            },
        );
        let cvars = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        cvars.borrow_mut().register("cm_noCurves", "0", 0).unwrap();
        cvars.borrow_mut().register("cm_playerCurveClip", "1", 0).unwrap();
        let clip: Q3GuestClipFactory = Rc::new(|_, _| Rc::new(FakeClip));
        let spatial = Q3GuestSpatial::new(records.clone(), scene, cvars, clip);
        (records, spatial)
    }

    fn bot<'a>(
        files: &'a BotAssetFiles,
        records: &'a Q3GuestRecords,
        spatial: &'a RefCell<Q3GuestSpatial>,
        entities: &str,
    ) -> Q3GuestBots<'a, 'a, 'a, StubSelected, StubClients> {
        Q3GuestBots::new(Q3GuestBotOptions {
            files,
            print: Rc::new(|_| {}),
            clients_count: 4,
            debug: false,
            selected: StubSelected,
            records,
            spatial,
            entities: entities.to_string(),
            map_name: "q3dm1".to_string(),
            clients: StubClients,
        })
    }

    #[test]
    fn setup_loads_configs_and_marks_initialized() {
        let mut files = BotAssetFiles::new();
        files.add(
            "weapons.c",
            b"projectileinfo { name gauntlet_hit; damage 50; }\nweaponinfo { number 1; name gauntlet; projectile gauntlet_hit; numprojectiles 1; speed 0; reload 0.4; ammoamount 0; }\n",
        );
        files.add(
            "items.c",
            b"iteminfo { classname item_health; name health; modelindex 1; }\n",
        );
        let (records, spatial) = records_and_spatial();
        let spatial = RefCell::new(spatial);
        let mut bots = bot(&files, &records, &spatial, "");
        assert_eq!(bots.setup_module(), 0);
        assert!(bots.is_initialized());
        assert_eq!(bots.library.variables.string("droppedweight", ""), "1000");
    }

    #[test]
    fn setup_reports_missing_weapon_config() {
        let files = BotAssetFiles::new();
        let (records, spatial) = records_and_spatial();
        let spatial = RefCell::new(spatial);
        let mut bots = bot(&files, &records, &spatial, "");
        assert_eq!(bots.setup_module(), WeaponLoadResult::CannotLoadWeaponConfig as i32);
        assert!(!bots.is_initialized());
    }

    #[test]
    fn load_map_scans_level_items() {
        let files = BotAssetFiles::new();
        let (records, spatial) = records_and_spatial();
        let spatial = RefCell::new(spatial);
        let mut bots = bot(
            &files,
            &records,
            &spatial,
            "{\n\"classname\" \"item_health\"\n\"origin\" \"10 20 30\"\n}\n",
        );
        bots.library.goals.item_config = Some(ItemConfig {
            path: "items.c".to_string(),
            items: vec![ItemInfo {
                classname: "item_health".to_string(),
                name: "health".to_string(),
                model: String::new(),
                model_index: 1,
                item_type: 0,
                index: 0,
                respawn_time: 30.0,
                mins: Vec3 {
                    x: -15.0,
                    y: -15.0,
                    z: -15.0,
                },
                maxs: Vec3 {
                    x: 15.0,
                    y: 15.0,
                    z: 15.0,
                },
                number: 0,
            }],
            diagnostics: Vec::new(),
        });
        assert_eq!(bots.load_map_module("q3dm1"), 0);
        assert!(bots.is_loaded());
        assert_eq!(bots.library.goals.level_items.len(), 1);
        assert_eq!(bots.library.goals.level_items[0].area, 7);
    }

    #[test]
    fn observations_round_trip_through_checkpoints() {
        let files = BotAssetFiles::new();
        let (records, spatial) = records_and_spatial();
        let spatial = RefCell::new(spatial);
        let mut bots = bot(&files, &records, &spatial, "");
        bots.observations.insert(
            3,
            AasEntityInfo {
                valid: true,
                number: 3,
                update: BotEntityUpdate {
                    entity_type: 2,
                    flags: 0,
                    origin: Vec3::default(),
                    angles: Vec3::default(),
                    old_origin: Vec3::default(),
                    mins: Vec3::default(),
                    maxs: Vec3::default(),
                    ground_entity: -1,
                    solid: 0,
                    model_index: 1,
                    model_index2: 0,
                    frame: 0,
                    event: 0,
                    event_parameter: 0,
                    powerups: 0,
                    weapon: 5,
                    legs_animation: 0,
                    torso_animation: 0,
                },
                last_visible_origin: Vec3::default(),
                last_update_time: 1.0,
                update_interval: 0.5,
            },
        );
        assert_eq!(bots.entity_model_index(3), 1);
        assert_eq!(bots.entity_type(3), 2);
        assert_eq!(bots.entity_weapon(3), 5);
        assert_eq!(bots.next_entity(2), Some(3));
        assert_eq!(bots.next_entity(3), None);
        let checkpoint = bots.checkpoint();
        let mut restored = bot(&files, &records, &spatial, "");
        restored.restore(&checkpoint).unwrap();
        assert_eq!(restored.observations.len(), 1);
        assert_eq!(restored.checkpoint(), checkpoint);
    }

    #[test]
    fn restore_rejects_foreign_assets() {
        let files = BotAssetFiles::new();
        let (records, spatial) = records_and_spatial();
        let spatial = RefCell::new(spatial);
        let mut bots = bot(&files, &records, &spatial, "");
        let checkpoint = bots.checkpoint();
        let SaveJson::Object(members) = checkpoint.clone() else {
            panic!("object");
        };
        let members = members
            .into_iter()
            .map(|(key, value)| {
                if key == "assets" {
                    (key, json_str("other"))
                } else {
                    (key, value)
                }
            })
            .collect();
        assert!(bots.restore(&SaveJson::Object(members)).is_err());
    }
}
