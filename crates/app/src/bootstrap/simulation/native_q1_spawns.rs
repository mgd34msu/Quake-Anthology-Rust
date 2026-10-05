//! Native Quake I map spawns for the reachable server.
//!
//! Stock spawn functions for the entities the walking skeleton simulates
//! for real — worldspawn, player starts, lights, func_door — wired to the
//! reachable [`Server`](qa_world::server::Server) through the native
//! touch/mover-think hooks. Generic `map:{classname}` spawns still cover
//! every other classname until their native behavior lands.
//!
//! qsrc: `progs106/doors.qc` (func_door spawn, LinkDoors, spawn_field,
//! door_touch, door_trigger_touch, door_fire, door_go_up, door_go_down,
//! door_hit_top, door_hit_bottom), `progs106/subs.qc:12` (SetMovedir),
//! `progs106/misc.qc:41` (light), `progs106/client.qc:555`
//! (info_player_start), `progs106/world.qc:172` (worldspawn),
//! `WinQuake/pr_edict.c:930` + `server.h:180` (skill/deathmatch
//! inhibition), `progs106/defs.qc:305` (key item bits).
//!
//! Skeleton scope notes (each lands with its system, not here):
//! sounds have no sim audio path yet, `SUB_UseTargets` needs the target
//! system, door messages need centerprint, door blocking needs the
//! pusher transaction, and shootable doors need damage routing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{angle_vectors, vec3, Bounds, Vec3};
use qa_world::movers::{use_mover, MoverKind, MoverPhase, MoverState, MoverTable};
use qa_world::server::{Server, ServerLogic};
use qa_world::session::Simulation;
use qa_world::spawn::{SpawnFields, SpawnRegistry, SpawnRequest};
use qa_world::triggers::{TouchContact, TriggerTable};
use qa_world::WorldError;

/// Stock spawnflag inhibition bits (`server.h:180-183`).
const SPAWNFLAG_NOT_EASY: i32 = 256;
/// Stock spawnflag inhibition bits (`server.h:180-183`).
const SPAWNFLAG_NOT_MEDIUM: i32 = 512;
/// Stock spawnflag inhibition bits (`server.h:180-183`).
const SPAWNFLAG_NOT_HARD: i32 = 1024;
/// Stock spawnflag inhibition bits (`server.h:180-183`).
const SPAWNFLAG_NOT_DEATHMATCH: i32 = 2048;

/// Door spawnflags (`doors.qc:1-6`).
const DOOR_START_OPEN: i32 = 1;
/// Door spawnflags (`doors.qc:1-6`).
const DOOR_DONT_LINK: i32 = 4;
/// Door spawnflags (`doors.qc:1-6`).
const DOOR_GOLD_KEY: i32 = 8;
/// Door spawnflags (`doors.qc:1-6`).
const DOOR_SILVER_KEY: i32 = 16;
/// Door spawnflags (`doors.qc:1-6`).
const DOOR_TOGGLE: i32 = 32;

/// Key item bits (`defs.qc:305-306`).
const IT_KEY1: u32 = 131_072;
/// Key item bits (`defs.qc:305-306`).
const IT_KEY2: u32 = 262_144;

/// Door trigger-field expansion in map units (`spawn_field`,
/// `doors.qc:273`: `setsize (trigger, t1 - '60 60 8', t2 + '60 60 8')`).
const FIELD_EXPAND: Vec3 = Vec3 {
    x: 60.0,
    y: 60.0,
    z: 8.0,
};

/// Whether stock skill/deathmatch inhibition removes an entity before its
/// spawn function runs (`ED_LoadFromFile`, `pr_edict.c:930-947`).
#[must_use]
pub fn q1_spawn_inhibited(spawnflags: i32, skill: u8, deathmatch: bool) -> bool {
    if deathmatch {
        return spawnflags & SPAWNFLAG_NOT_DEATHMATCH != 0;
    }
    match skill {
        0 => spawnflags & SPAWNFLAG_NOT_EASY != 0,
        1 => spawnflags & SPAWNFLAG_NOT_MEDIUM != 0,
        _ => spawnflags & SPAWNFLAG_NOT_HARD != 0,
    }
}

/// Outcome of the Q1 pre-spawn filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1PreSpawn {
    /// Run the spawn function.
    Spawn,
    /// Stock removes the entity before spawning, with the stock reason.
    Skip(String),
}

/// Q1 pre-spawn filter: skill/deathmatch inhibition, inert-light removal,
/// and static-visual removal. Everything else spawns.
#[must_use]
pub fn q1_pre_spawn(classname: &str, fields: &SpawnFields, skill: u8, deathmatch: bool) -> Q1PreSpawn {
    if q1_spawn_inhibited(fields.spawnflags, skill, deathmatch) {
        return Q1PreSpawn::Skip("inhibited by spawnflags for skill/mode".to_string());
    }
    match classname {
        // `light` without a targetname is inert (baked): `remove(self)`
        // (`misc.qc:41`). Targeted lights stay; style >= 32 arms `use`.
        "light" => {
            if fields.targetname.as_deref().is_some_and(|name| !name.is_empty()) {
                Q1PreSpawn::Spawn
            } else {
                Q1PreSpawn::Skip("inert baked light".to_string())
            }
        }
        // Static visuals leave the sim via `makestatic` (`misc.qc`).
        "light_globe"
        | "light_torch_small_walltorch"
        | "light_flame_large_yellow"
        | "light_flame_small_yellow"
        | "light_flame_small_white" => Q1PreSpawn::Skip("static visual (makestatic)".to_string()),
        _ => Q1PreSpawn::Spawn,
    }
}

/// Register the native Q1 spawn functions, replacing the stub-table
/// entries for the same classnames. Door sizing/linking happens in the
/// post-spawn passes ([`build_q1_door`], [`link_q1_doors`]).
pub fn register_q1_spawns(registry: &mut SpawnRegistry) {
    registry.register(
        "worldspawn",
        Box::new(|_fields| {
            Ok(SpawnRequest {
                definition: "q1:worldspawn".to_string(),
                origin: None,
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
    for classname in [
        "info_player_start",
        "info_player_start2",
        "info_player_deathmatch",
        "info_player_coop",
        "light",
        "light_fluoro",
        "light_fluorospark",
        "func_door",
    ] {
        let definition = format!("q1:{classname}");
        registry.register(
            classname,
            Box::new(move |fields| {
                Ok(SpawnRequest {
                    definition: definition.clone(),
                    origin: Some(fields.origin),
                    combat: None,
                    grants: Vec::new(),
                })
            }),
        );
    }
    // Internal door trigger-field spawn (stock `spawn_field` actor).
    registry.register(
        "q1:door_field",
        Box::new(|fields| {
            Ok(SpawnRequest {
                definition: "q1:door_field".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
}

/// Move direction from spawn angles (`SetMovedir`, `subs.qc:12`): the
/// `(0,-1,0)`/`(0,-2,0)` spellings move up/down, anything else moves
/// along the forward vector.
#[must_use]
pub fn q1_movedir(angles: Vec3) -> Vec3 {
    if angles.x == 0.0 && angles.y == -1.0 && angles.z == 0.0 {
        return vec3(0.0, 0.0, 1.0);
    }
    if angles.x == 0.0 && angles.y == -2.0 && angles.z == 0.0 {
        return vec3(0.0, 0.0, -1.0);
    }
    angle_vectors(angles).forward
}

/// Parse one optional QC float field: missing or unparseable reads as
/// zero (stock `atof`), and zero selects the stock default.
fn q1_field_or(fields: &SpawnFields, key: &str, default: f64) -> f64 {
    let parsed = fields
        .extra
        .get(key)
        .and_then(|text| text.parse::<f64>().ok())
        .unwrap_or(0.0);
    if parsed == 0.0 {
        default
    } else {
        parsed
    }
}

/// Parsed func_door spawn parameters (`func_door`, `doors.qc:419-534`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1DoorParams {
    /// Brush-model size (`maxs - mins`), origin-independent.
    pub size: Vec3,
    /// Travel direction.
    pub movedir: Vec3,
    /// Travel speed, units/second (default 100).
    pub speed: f64,
    /// Wait at the top in seconds (default 3; -1 for key doors).
    pub wait: f64,
    /// Lip remaining when open (default 8).
    pub lip: f64,
    /// Key item bits required to open (0 for plain doors).
    pub items: u32,
    /// Toggle flag: stays open until fired again.
    pub toggle: bool,
    /// Spawn open instead of closed.
    pub start_open: bool,
    /// Never link to other doors (and, per stock, spawn no field).
    pub dont_link: bool,
    /// Shootable health (0 for touch doors).
    pub health: f64,
    /// Whether a targetname delays opening until fired.
    pub has_targetname: bool,
    /// Closed endpoint (always the closed position, even for
    /// start-open doors, so the think hook reads phases uniformly).
    pub pos1: Vec3,
    /// Open endpoint.
    pub pos2: Vec3,
}

/// Parse func_door parameters from spawn fields plus the `*N` brush-model
/// bounds in absolute map coords (`doors.qc:485-534`).
pub fn q1_door_params(fields: &SpawnFields, model: &Bounds) -> Result<Q1DoorParams, WorldError> {
    let size = vec3(
        model.max.x - model.min.x,
        model.max.y - model.min.y,
        model.max.z - model.min.z,
    );
    let movedir = q1_movedir(fields.angles);
    let speed = q1_field_or(fields, "speed", 100.0);
    let mut wait = q1_field_or(fields, "wait", 3.0);
    let lip = q1_field_or(fields, "lip", 8.0);
    let health = fields
        .extra
        .get("health")
        .and_then(|text| text.parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut items = 0u32;
    if fields.spawnflags & DOOR_SILVER_KEY != 0 {
        items |= IT_KEY1;
    }
    if fields.spawnflags & DOOR_GOLD_KEY != 0 {
        items |= IT_KEY2;
    }
    if items != 0 {
        wait = -1.0;
    }
    // `pos2 = pos1 + movedir * (|movedir . size| - lip)`.
    let dot = f64::from(movedir.x * size.x + movedir.y * size.y + movedir.z * size.z).abs();
    let travel = dot - lip;
    let pos1 = fields.origin;
    let pos2 = vec3(
        pos1.x + movedir.x * travel as f32,
        pos1.y + movedir.y * travel as f32,
        pos1.z + movedir.z * travel as f32,
    );
    Ok(Q1DoorParams {
        size,
        movedir,
        speed,
        wait,
        lip,
        items,
        toggle: fields.spawnflags & DOOR_TOGGLE != 0,
        start_open: fields.spawnflags & DOOR_START_OPEN != 0,
        dont_link: fields.spawnflags & DOOR_DONT_LINK != 0,
        health,
        has_targetname: fields.targetname.as_deref().is_some_and(|name| !name.is_empty()),
        pos1,
        pos2,
    })
}

/// Parse the `*N` brush-model index from a `model` field.
fn q1_model_index(fields: &SpawnFields) -> Option<usize> {
    fields.extra.get("model")?.strip_prefix('*')?.parse::<usize>().ok()
}

/// Live Q1 door gamecode state (the QC fields the think hook reads).
#[derive(Debug, Clone)]
pub struct Q1Door {
    /// Master door of the linked chain (`owner`).
    pub master: ActorId,
    /// Linked chain in enemy order, master first.
    pub peers: Vec<ActorId>,
    /// Key item bits required (0 for plain doors).
    pub items: u32,
    /// Wait at the top in seconds.
    pub wait: f64,
    /// Toggle flag.
    pub toggle: bool,
    /// Master-clock seconds until which `door_touch` stays throttled.
    pub touch_throttle_until: f64,
}

/// Door trigger-field gamecode state.
#[derive(Debug, Clone)]
pub struct Q1DoorField {
    /// Master door the field fires.
    pub master: ActorId,
    /// Master-clock seconds until which the field stays throttled.
    pub throttle_until: f64,
}

/// Live native Q1 gamecode state, shared between the spawn path and the
/// native hooks behind one [`Rc`]`<`[`RefCell`]`>`.
#[derive(Debug, Default)]
pub struct Q1NativeBehaviors {
    /// Door actors by id.
    pub doors: HashMap<ActorId, Q1Door>,
    /// Trigger-field actors by id.
    pub fields: HashMap<ActorId, Q1DoorField>,
    /// Admitted player opener (`None` on dedicated servers: with no
    /// player, doors stay shut).
    pub player: Option<ActorId>,
    /// Key item bits the player carries.
    pub player_keys: u32,
}

impl Q1NativeBehaviors {
    /// Empty behavior set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt the admitted player as the door opener.
    pub fn set_player(&mut self, player: Option<ActorId>) {
        self.player = player;
    }
}

/// A spawned func_door awaiting the [`link_q1_doors`] pass, in spawn order.
pub struct Q1PendingDoor {
    /// Spawned door actor.
    pub actor: OwnedActor,
    /// Parsed spawn parameters.
    pub params: Q1DoorParams,
    /// Absolute brush-model bounds.
    pub bounds: Bounds,
}

/// Finish a spawned func_door actor: size its body to the brush model,
/// register its mover, mark it touchable (`door_touch`), and record its
/// gamecode state. Returns the pending door for [`link_q1_doors`].
pub fn build_q1_door<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &OwnedActor,
    fields: &SpawnFields,
    models: &[Bounds],
) -> Result<Q1PendingDoor, WorldError> {
    let index = q1_model_index(fields)
        .filter(|index| *index < models.len())
        .ok_or_else(|| WorldError::BadSpawnFields(format!("func_door without brush model: {}", fields.classname)))?;
    let bounds = models[index];
    let params = q1_door_params(fields, &bounds)?;
    // Local body bounds rebase the absolute model bounds onto the spawn
    // origin (stock `setmodel` sizing).
    let local = Bounds {
        min: vec3(
            bounds.min.x - fields.origin.x,
            bounds.min.y - fields.origin.y,
            bounds.min.z - fields.origin.z,
        ),
        max: vec3(
            bounds.max.x - fields.origin.x,
            bounds.max.y - fields.origin.y,
            bounds.max.z - fields.origin.z,
        ),
    };
    server.simulation_mut().set_body_bounds(actor.id(), local)?;
    let origin = if params.start_open { params.pos2 } else { params.pos1 };
    server.simulation_mut().set_body_origin(actor.id(), origin)?;
    let mut mover = MoverState::new(MoverKind::Door, params.pos1, params.pos2, params.speed, params.wait);
    if params.start_open {
        mover.phase = MoverPhase::AtPos2;
    }
    server.movers_mut().insert(actor.id().clone(), mover);
    server.mark_trigger(actor.id())?;
    behaviors.doors.insert(
        actor.id().clone(),
        Q1Door {
            master: actor.id().clone(),
            peers: vec![actor.id().clone()],
            items: params.items,
            wait: params.wait,
            toggle: params.toggle,
            touch_throttle_until: 0.0,
        },
    );
    Ok(Q1PendingDoor {
        actor: actor.clone(),
        params,
        bounds,
    })
}

/// Whether two absolute brush bounds touch (`EntitiesTouching`,
/// `doors.qc:291`).
fn q1_bounds_touch(left: &Bounds, right: &Bounds) -> bool {
    left.min.x <= right.max.x
        && left.min.y <= right.max.y
        && left.min.z <= right.max.z
        && left.max.x >= right.min.x
        && left.max.y >= right.min.y
        && left.max.z >= right.min.z
}

/// Link spawned doors into master/peer chains and spawn trigger fields
/// (`LinkDoors` + `spawn_field`, `doors.qc:273-391`). Doors stay in spawn
/// order; touching doors chain through `enemy`. Stock quirks reproduced:
/// a `DONT_LINK` door still joins an earlier door's chain (the flag only
/// stops it from initiating one; a `DONT_LINK` chain starter stays solo
/// with no field), and shootable/targeted/key masters spawn no field.
/// One deliberate divergence: already-linked doors are skipped where
/// stock errors (`cross connected doors`), so a cross-connected map
/// loads instead of aborting the spawn.
pub fn link_q1_doors<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    pending: Vec<Q1PendingDoor>,
) -> Result<(), WorldError> {
    let mut linked: Vec<bool> = vec![false; pending.len()];
    for starte in 0..pending.len() {
        if linked[starte] {
            continue;
        }
        if pending[starte].params.dont_link {
            linked[starte] = true;
            continue;
        }
        // Walk the chain from the master, skipping non-touching doors.
        let mut chain = vec![starte];
        let mut current = starte;
        let mut candidate = starte + 1;
        while candidate < pending.len() {
            if !linked[candidate] && q1_bounds_touch(&pending[current].bounds, &pending[candidate].bounds) {
                linked[candidate] = true;
                chain.push(candidate);
                current = candidate;
            }
            candidate += 1;
        }
        linked[starte] = true;
        let master = pending[starte].actor.id().clone();
        // Propagate health/targetname down the chain (stock copies each
        // linked door's values onto the master as it links).
        let mut master_health = pending[starte].params.health;
        let mut master_targeted = pending[starte].params.has_targetname;
        for index in chain.iter().skip(1) {
            if pending[*index].params.health != 0.0 {
                master_health = pending[*index].params.health;
            }
            if pending[*index].params.has_targetname {
                master_targeted = true;
            }
        }
        let peers: Vec<ActorId> = chain.iter().map(|index| pending[*index].actor.id().clone()).collect();
        for index in &chain {
            if let Some(door) = behaviors.doors.get_mut(pending[*index].actor.id()) {
                door.master = master.clone();
                door.peers.clone_from(&peers);
            }
        }
        let master_items = behaviors.doors.get(&master).map_or(0, |door| door.items);
        if master_health != 0.0 || master_targeted || master_items != 0 {
            continue;
        }
        // Spawn the trigger field over the chained bounds.
        let mut mins = pending[chain[0]].bounds.min;
        let mut maxs = pending[chain[0]].bounds.max;
        for index in chain.iter().skip(1) {
            let bounds = &pending[*index].bounds;
            mins.x = mins.x.min(bounds.min.x);
            mins.y = mins.y.min(bounds.min.y);
            mins.z = mins.z.min(bounds.min.z);
            maxs.x = maxs.x.max(bounds.max.x);
            maxs.y = maxs.y.max(bounds.max.y);
            maxs.z = maxs.z.max(bounds.max.z);
        }
        let field_bounds = Bounds {
            min: vec3(
                mins.x - FIELD_EXPAND.x,
                mins.y - FIELD_EXPAND.y,
                mins.z - FIELD_EXPAND.z,
            ),
            max: vec3(
                maxs.x + FIELD_EXPAND.x,
                maxs.y + FIELD_EXPAND.y,
                maxs.z + FIELD_EXPAND.z,
            ),
        };
        let field_fields = SpawnFields {
            classname: "q1:door_field".to_string(),
            origin: vec3(0.0, 0.0, 0.0),
            ..SpawnFields::default()
        };
        let field = server.spawn_entity(&field_fields)?;
        server.simulation_mut().set_body_bounds(field.id(), field_bounds)?;
        server.mark_trigger(field.id())?;
        behaviors.fields.insert(
            field.id().clone(),
            Q1DoorField {
                master: master.clone(),
                throttle_until: 0.0,
            },
        );
    }
    Ok(())
}

/// Redirect a travelling mover toward the other endpoint, re-arming the
/// arrival think (stock `SUB_CalcMove` reversal; the engine's
/// [`use_mover`] leaves travelling movers untouched).
fn q1_redirect_mover(state: &mut MoverState, origin: Vec3, to_pos2: bool) {
    state.phase = if to_pos2 {
        MoverPhase::ToPos2
    } else {
        MoverPhase::ToPos1
    };
    let target = state.target();
    let delta = vec3(target.x - origin.x, target.y - origin.y, target.z - origin.z);
    let distance = f64::from(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
    if distance == 0.0 {
        state.phase = if to_pos2 {
            MoverPhase::AtPos2
        } else {
            MoverPhase::AtPos1
        };
        return;
    }
    if state.speed > 0.0 {
        state.next_think_seconds = state.local_time_seconds + distance / state.speed;
    }
}

/// Current body origin for a mover, defaulting to position 1 when the
/// body is unreadable (movers always have bodies; the fallback keeps a
/// corrupt table from panicking the tick).
fn q1_mover_origin(simulation: &Simulation, movers: &MoverTable, actor: &ActorId) -> Vec3 {
    simulation
        .body_state(actor)
        .map(|state| state.origin)
        .unwrap_or_else(|| movers.get(actor).map_or(vec3(0.0, 0.0, 0.0), |state| state.pos1))
}

/// Open one door (`door_go_up`, `doors.qc:77`): already-open doors reset
/// their wait, otherwise travel starts and targets fire (target firing
/// needs the target system).
fn q1_door_go_up(behaviors: &Q1NativeBehaviors, simulation: &Simulation, movers: &mut MoverTable, actor: &ActorId) {
    let Some(state) = movers.get_mut(actor) else {
        return;
    };
    match state.phase {
        MoverPhase::ToPos2 => return,
        MoverPhase::AtPos2 => {
            // Reset the top wait time.
            if let Some(door) = behaviors.doors.get(actor) {
                state.next_think_seconds = state.local_time_seconds + door.wait;
            }
            return;
        }
        MoverPhase::AtPos1 | MoverPhase::ToPos1 => {}
    }
    let origin = q1_mover_origin(simulation, movers, actor);
    if let Some(state) = movers.get_mut(actor) {
        if state.phase == MoverPhase::ToPos1 {
            q1_redirect_mover(state, origin, true);
        } else {
            use_mover(state, origin);
        }
    }
}

/// Close one door (`door_go_down`, `doors.qc:64`).
fn q1_door_go_down(simulation: &Simulation, movers: &mut MoverTable, actor: &ActorId) {
    let Some(state) = movers.get(actor) else {
        return;
    };
    match state.phase {
        MoverPhase::AtPos1 | MoverPhase::ToPos1 => return,
        MoverPhase::AtPos2 | MoverPhase::ToPos2 => {}
    }
    let origin = q1_mover_origin(simulation, movers, actor);
    if let Some(state) = movers.get_mut(actor) {
        if state.phase == MoverPhase::ToPos2 {
            q1_redirect_mover(state, origin, false);
        } else {
            use_mover(state, origin);
        }
    }
}

/// Fire a master door and its linked peers (`door_fire`, `doors.qc:104`):
/// toggle doors open at the top travel down instead.
fn q1_door_fire(behaviors: &Q1NativeBehaviors, simulation: &Simulation, movers: &mut MoverTable, master: &ActorId) {
    let Some(door) = behaviors.doors.get(master) else {
        return;
    };
    let peers = door.peers.clone();
    let toggle = door.toggle;
    if toggle
        && matches!(
            movers.get(master).map(|state| state.phase),
            Some(MoverPhase::ToPos2 | MoverPhase::AtPos2)
        )
    {
        for peer in &peers {
            q1_door_go_down(simulation, movers, peer);
        }
        return;
    }
    for peer in &peers {
        q1_door_go_up(behaviors, simulation, movers, peer);
    }
}

/// Native touch dispatch for Q1 doors: trigger fields fire
/// (`door_trigger_touch`, `doors.qc:160`) and door solids handle key
/// doors (`door_touch`, `doors.qc:196`). Non-player touches are ignored.
pub fn q1_native_touch(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    contact: &TouchContact,
) {
    let Some(player) = behaviors.player.clone() else {
        return;
    };
    if contact.other != player {
        return;
    }
    let now = simulation.frame().time.as_seconds_f64();
    if let Some(field) = behaviors.fields.get(&contact.trigger).cloned() {
        // `door_trigger_touch`: 1s refire, then fire the master.
        let throttled = behaviors
            .fields
            .get(&contact.trigger)
            .is_some_and(|field| now < field.throttle_until);
        if throttled {
            return;
        }
        if let Some(field) = behaviors.fields.get_mut(&contact.trigger) {
            field.throttle_until = now + 1.0;
        }
        q1_door_fire(behaviors, simulation, movers, &field.master);
        return;
    }
    if !behaviors.doors.contains_key(&contact.trigger) {
        return;
    }
    // `door_touch`: 2s master throttle, then key handling. Messages and
    // sounds need the centerprint/audio paths.
    let master = behaviors.doors.get(&contact.trigger).map(|door| door.master.clone());
    let Some(master) = master else {
        return;
    };
    let throttled = behaviors
        .doors
        .get(&master)
        .is_some_and(|door| now < door.touch_throttle_until);
    if throttled {
        return;
    }
    if let Some(door) = behaviors.doors.get_mut(&master) {
        door.touch_throttle_until = now + 2.0;
    }
    let items = behaviors.doors.get(&contact.trigger).map_or(0, |door| door.items);
    if items == 0 {
        return;
    }
    if behaviors.player_keys & items != items {
        return;
    }
    behaviors.player_keys &= !items;
    // Stock nulls the touch on the door and its chain next.
    triggers.unmark(&contact.trigger);
    let next = behaviors.doors.get(&contact.trigger).and_then(|door| {
        door.peers
            .iter()
            .position(|peer| peer == &contact.trigger)
            .and_then(|at| door.peers.get((at + 1) % door.peers.len()).cloned())
    });
    if let Some(next) = next {
        triggers.unmark(&next);
    }
    q1_door_fire(behaviors, simulation, movers, &master);
}

/// Native mover-think dispatch for Q1 doors: top arrival always rests
/// (toggle doors wait for a trigger; negative-wait key doors never
/// return — stock `door_hit_top` arms `nextthink = ltime + wait`, a past
/// instant the pusher never fires — and positive waits were armed by the
/// engine step), the top wait think closes the door, and bottom arrival
/// rests.
/// A think firing mid-travel without arrival re-arms the arrival think:
/// float dust between the armed arrival instant and the recomputed
/// remaining distance would otherwise consume the think and strand the
/// mover with no future think (the engine holds still without one).
pub fn q1_native_mover_think(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    actor: &ActorId,
    phase: MoverPhase,
    arrived: bool,
) {
    let Some(door) = behaviors.doors.get(actor) else {
        return;
    };
    let toggle = door.toggle;
    match (phase, arrived) {
        // `door_hit_top` (`progs106/doors.qc:48-56`): arrival at the top
        // never closes at once. Toggle doors return and wait for a
        // trigger; key doors (`wait -1 = never return`, `doors.qc:401`)
        // arm `nextthink = ltime - 1`, a past instant the pusher never
        // fires (`sv_phys.c:732` requires `thinktime > oldltime`), so
        // they rest at the top too; positive waits were armed by the
        // engine step and close through the wait think below.
        (MoverPhase::AtPos2, true) => {}
        // Top wait think: close, unless this is a toggle door (the
        // engine arms wait thinks for any positive wait; stock toggle
        // doors never schedule one).
        // Top wait think: close, unless this is a toggle door (the
        // engine arms wait thinks for any positive wait; stock toggle
        // doors never schedule one).
        (MoverPhase::AtPos2, false) => {
            if toggle {
                return;
            }
            q1_door_go_down(simulation, movers, actor);
        }
        // Mid-travel think without arrival: resume the interrupted
        // travel by re-arming the arrival think from the live origin.
        // The re-arm clamps to a future instant so a dust-exact arrival
        // (remaining distance zero) still schedules its completion
        // think — which arrives on the next step — instead of arming at
        // local time and freezing the mover with no future think.
        (MoverPhase::ToPos1 | MoverPhase::ToPos2, _) => {
            let origin = q1_mover_origin(simulation, movers, actor);
            if let Some(state) = movers.get_mut(actor) {
                let target = state.target();
                let delta = vec3(target.x - origin.x, target.y - origin.y, target.z - origin.z);
                let distance = f64::from(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
                if state.speed > 0.0 {
                    state.next_think_seconds = state.local_time_seconds + (distance / state.speed).max(1e-9);
                }
            }
        }
        _ => {}
    }
}

/// Install the native Q1 hooks on a server, sharing `behaviors` with the
/// spawn path. Spawn the map first, then install before the first tick.
pub fn install_q1_native<L: ServerLogic>(server: &mut Server<L>, behaviors: Rc<RefCell<Q1NativeBehaviors>>) {
    let touch_behaviors = Rc::clone(&behaviors);
    server.set_native_touch(Some(Box::new(move |simulation, movers, triggers, contact| {
        q1_native_touch(&mut touch_behaviors.borrow_mut(), simulation, movers, triggers, contact);
    })));
    server.set_native_mover_think(Some(Box::new(move |simulation, movers, actor, phase, arrived| {
        q1_native_mover_think(&mut behaviors.borrow_mut(), simulation, movers, actor, phase, arrived);
    })));
}

#[cfg(test)]
mod tests {
    use qa_core::time::SourceTime;
    use qa_world::body::BodyState;

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn door_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        let mut full = vec![("classname", "func_door")];
        full.extend_from_slice(pairs);
        SpawnFields::parse(&full).unwrap()
    }

    fn door_model() -> Bounds {
        Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(64.0, 64.0, 128.0),
        }
    }

    fn spawn_door(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
        models: &[Bounds],
    ) -> Q1PendingDoor {
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_door(server, behaviors, &actor, fields, models).unwrap()
    }

    fn spawn_player(server: &mut Server<qa_guest::server::GuestServerLogic>, origin: Vec3) -> OwnedActor {
        server
            .simulation_mut()
            .spawn(
                qa_core::identity::ProviderId::new("q1", "test"),
                "q1:test_player",
                Some(BodyState {
                    origin,
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-16.0, -16.0, -24.0),
                        max: vec3(16.0, 16.0, 32.0),
                    },
                    ground: None,
                }),
                None,
                Vec::new(),
            )
            .unwrap()
    }

    #[test]
    fn movedir_maps_angles_per_set_movedir() {
        assert_eq!(q1_movedir(vec3(0.0, -1.0, 0.0)), vec3(0.0, 0.0, 1.0));
        assert_eq!(q1_movedir(vec3(0.0, -2.0, 0.0)), vec3(0.0, 0.0, -1.0));
        assert_eq!(q1_movedir(vec3(0.0, 0.0, 0.0)), vec3(1.0, 0.0, 0.0));
        let yaw90 = q1_movedir(vec3(0.0, 90.0, 0.0));
        assert!(yaw90.x.abs() < 1e-6);
        assert!((f64::from(yaw90.y) - 1.0).abs() < 1e-6);
        assert_eq!(yaw90.z, 0.0);
    }

    #[test]
    fn inhibition_follows_skill_and_deathmatch() {
        assert!(q1_spawn_inhibited(SPAWNFLAG_NOT_EASY, 0, false));
        assert!(!q1_spawn_inhibited(SPAWNFLAG_NOT_EASY, 1, false));
        assert!(q1_spawn_inhibited(SPAWNFLAG_NOT_MEDIUM, 1, false));
        assert!(!q1_spawn_inhibited(SPAWNFLAG_NOT_MEDIUM, 2, false));
        assert!(q1_spawn_inhibited(SPAWNFLAG_NOT_HARD, 2, false));
        assert!(q1_spawn_inhibited(SPAWNFLAG_NOT_HARD, 3, false));
        assert!(!q1_spawn_inhibited(SPAWNFLAG_NOT_HARD, 1, false));
        assert!(q1_spawn_inhibited(SPAWNFLAG_NOT_DEATHMATCH, 1, true));
        assert!(!q1_spawn_inhibited(SPAWNFLAG_NOT_DEATHMATCH, 1, false));
        assert!(!q1_spawn_inhibited(0, 1, false));
    }

    #[test]
    fn pre_spawn_filters_lights_statics_and_inhibited() {
        let fields = SpawnFields::parse(&[("classname", "light")]).unwrap();
        assert!(matches!(q1_pre_spawn("light", &fields, 1, false), Q1PreSpawn::Skip(_)));
        let fields = SpawnFields::parse(&[("classname", "light"), ("targetname", "t1"), ("style", "32")]).unwrap();
        assert_eq!(q1_pre_spawn("light", &fields, 1, false), Q1PreSpawn::Spawn);
        for static_visual in [
            "light_globe",
            "light_torch_small_walltorch",
            "light_flame_large_yellow",
            "light_flame_small_yellow",
            "light_flame_small_white",
        ] {
            let fields = SpawnFields::parse(&[("classname", static_visual)]).unwrap();
            assert!(
                matches!(q1_pre_spawn(static_visual, &fields, 1, false), Q1PreSpawn::Skip(_)),
                "{static_visual}"
            );
        }
        let fields = SpawnFields::parse(&[("classname", "light_fluoro")]).unwrap();
        assert_eq!(q1_pre_spawn("light_fluoro", &fields, 1, false), Q1PreSpawn::Spawn);
        let fields = SpawnFields::parse(&[("classname", "monster_ogre"), ("spawnflags", "512")]).unwrap();
        assert!(matches!(
            q1_pre_spawn("monster_ogre", &fields, 1, false),
            Q1PreSpawn::Skip(_)
        ));
        assert_eq!(q1_pre_spawn("monster_ogre", &fields, 0, false), Q1PreSpawn::Spawn);
    }

    #[test]
    fn door_params_apply_stock_defaults_and_travel() {
        let fields = door_fields(&[("origin", "0 0 0"), ("angles", "0 90 0"), ("model", "*0")]);
        let params = q1_door_params(&fields, &door_model()).unwrap();
        assert_eq!(params.speed, 100.0);
        assert_eq!(params.wait, 3.0);
        assert_eq!(params.lip, 8.0);
        assert_eq!(params.items, 0);
        assert_eq!(params.pos1, vec3(0.0, 0.0, 0.0));
        // |movedir . size| - lip = 64 - 8 along +Y.
        assert!((f64::from(params.pos2.x)).abs() < 1e-4);
        assert!((f64::from(params.pos2.y) - 56.0).abs() < 1e-4);
        assert_eq!(params.pos2.z, 0.0);

        // Key doors take key bits and wait -1.
        let fields = door_fields(&[("spawnflags", "16"), ("model", "*0")]);
        let params = q1_door_params(&fields, &door_model()).unwrap();
        assert_eq!(params.items, IT_KEY1);
        assert_eq!(params.wait, -1.0);
        let fields = door_fields(&[("spawnflags", "8"), ("wait", "5"), ("model", "*0")]);
        let params = q1_door_params(&fields, &door_model()).unwrap();
        assert_eq!(params.items, IT_KEY2);
        assert_eq!(params.wait, -1.0);

        // Explicit nonzero values survive.
        let fields = door_fields(&[("speed", "400"), ("wait", "7"), ("lip", "4"), ("model", "*0")]);
        let params = q1_door_params(&fields, &door_model()).unwrap();
        assert_eq!(params.speed, 400.0);
        assert_eq!(params.wait, 7.0);
        assert_eq!(params.lip, 4.0);
        assert!(!params.toggle && !params.start_open && !params.dont_link);
        let fields = door_fields(&[("spawnflags", "33"), ("model", "*0")]);
        let params = q1_door_params(&fields, &door_model()).unwrap();
        assert!(params.toggle && params.start_open);
    }

    #[test]
    fn door_build_sizes_body_registers_mover_and_marks_touch() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let mut behaviors = Q1NativeBehaviors::new();
        let models = vec![door_model()];
        let fields = door_fields(&[("origin", "10 0 0"), ("angles", "0 90 0"), ("model", "*0")]);
        let actor = server.spawn_entity(&fields).unwrap();
        let pending = build_q1_door(&mut server, &mut behaviors, &actor, &fields, &models).unwrap();
        assert_eq!(pending.params.pos1, vec3(10.0, 0.0, 0.0));
        let body = server.simulation().body_state(actor.id()).unwrap();
        assert_eq!(body.origin, vec3(10.0, 0.0, 0.0));
        // Local bounds rebase the absolute model bounds onto the origin.
        assert_eq!(body.bounds.min, vec3(-10.0, 0.0, 0.0));
        assert_eq!(body.bounds.max, vec3(54.0, 64.0, 128.0));
        let mover = server.movers_mut().get(actor.id()).unwrap();
        assert_eq!(mover.phase, MoverPhase::AtPos1);
        assert_eq!(mover.speed, 100.0);
        assert_eq!(mover.wait_seconds, 3.0);
        assert!(server.triggers_mut().is_trigger(actor.id()));
        assert!(behaviors.doors.contains_key(actor.id()));
    }

    #[test]
    fn door_without_model_fails_to_build() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = door_fields(&[]);
        let actor = server.spawn_entity(&fields).unwrap();
        assert!(build_q1_door(&mut server, &mut behaviors, &actor, &fields, &[]).is_err());
    }

    #[test]
    fn link_chains_touching_doors_and_spawns_one_field() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let mut behaviors = Q1NativeBehaviors::new();
        // Two touching doors (shared face at x=64) plus one far door.
        let models = vec![
            door_model(),
            Bounds {
                min: vec3(64.0, 0.0, 0.0),
                max: vec3(128.0, 64.0, 128.0),
            },
            Bounds {
                min: vec3(1000.0, 0.0, 0.0),
                max: vec3(1064.0, 64.0, 128.0),
            },
        ];
        let first = spawn_door(&mut server, &mut behaviors, &door_fields(&[("model", "*0")]), &models);
        let second = spawn_door(&mut server, &mut behaviors, &door_fields(&[("model", "*1")]), &models);
        let solo = spawn_door(&mut server, &mut behaviors, &door_fields(&[("model", "*2")]), &models);
        link_q1_doors(&mut server, &mut behaviors, vec![first, second, solo]).unwrap();
        // Touching pair shares one master with a two-door chain; the far
        // door is its own master.
        let pair: Vec<&Q1Door> = behaviors.doors.values().filter(|door| door.peers.len() == 2).collect();
        assert_eq!(pair.len(), 2);
        assert_eq!(pair[0].master, pair[1].master);
        assert_eq!(pair[0].peers.len(), 2);
        let solos: Vec<&Q1Door> = behaviors.doors.values().filter(|door| door.peers.len() == 1).collect();
        assert_eq!(solos.len(), 1);
        // One field per chain, expanded 60/60/8 over the chained bounds.
        assert_eq!(behaviors.fields.len(), 2);
        let pair_field = behaviors
            .fields
            .iter()
            .find(|(_, field)| field.master == pair[0].master)
            .map(|(actor, _)| actor.clone())
            .unwrap();
        let bounds = server.simulation().body_state(&pair_field).unwrap().bounds;
        assert_eq!(bounds.min, vec3(-60.0, -60.0, -8.0));
        assert_eq!(bounds.max, vec3(188.0, 124.0, 136.0));
    }

    #[test]
    fn targeted_health_and_dont_link_doors_spawn_no_field() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let mut behaviors = Q1NativeBehaviors::new();
        let models = vec![door_model()];
        let targeted = spawn_door(
            &mut server,
            &mut behaviors,
            &door_fields(&[("model", "*0"), ("targetname", "t1")]),
            &models,
        );
        let healthy = spawn_door(
            &mut server,
            &mut behaviors,
            &door_fields(&[("model", "*0"), ("health", "25")]),
            &models,
        );
        let unlinked = spawn_door(
            &mut server,
            &mut behaviors,
            &door_fields(&[("model", "*0"), ("spawnflags", "4")]),
            &models,
        );
        // Identical bounds link all three into one chain (stock chains
        // first and decides the field from the master after), but the
        // targeted master spawns no field.
        link_q1_doors(&mut server, &mut behaviors, vec![targeted, healthy, unlinked]).unwrap();
        assert_eq!(behaviors.doors.len(), 3);
        assert!(behaviors.fields.is_empty());
        for door in behaviors.doors.values() {
            assert_eq!(door.peers.len(), 3);
        }
    }

    #[test]
    fn touch_opens_wait_closes_through_live_ticks() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        let models = vec![door_model()];
        let fields = door_fields(&[("angles", "0 90 0"), ("speed", "400"), ("wait", "0.2"), ("model", "*0")]);
        let pending = {
            let actor = server.spawn_entity(&fields).unwrap();
            build_q1_door(&mut server, &mut shared.borrow_mut(), &actor, &fields, &models).unwrap()
        };
        link_q1_doors(&mut server, &mut shared.borrow_mut(), vec![pending]).unwrap();
        let door = shared.borrow().doors.keys().next().unwrap().clone();
        assert_eq!(shared.borrow().fields.len(), 1);
        // Player starts inside the trigger field.
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 64.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        install_q1_native(&mut server, Rc::clone(&shared));
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos1);
        // First tick: the sweep touches the field and the door opens.
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::ToPos2);
        // Travel (56u at 400u/s) plus the 0.2s wait, with the player
        // held clear so the field does not refire.
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(32.0, 32.0, 500.0))
            .unwrap();
        for _ in 0..20 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos1);
    }

    #[test]
    fn key_door_denies_without_key_and_consumes_with_key() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        let models = vec![door_model()];
        let fields = door_fields(&[
            ("angles", "0 90 0"),
            ("speed", "4000"),
            ("spawnflags", "16"),
            ("model", "*0"),
        ]);
        let pending = {
            let actor = server.spawn_entity(&fields).unwrap();
            build_q1_door(&mut server, &mut shared.borrow_mut(), &actor, &fields, &models).unwrap()
        };
        link_q1_doors(&mut server, &mut shared.borrow_mut(), vec![pending]).unwrap();
        // Key doors spawn no field; the solid itself is the touch volume.
        assert!(shared.borrow().fields.is_empty());
        let door = shared.borrow().doors.keys().next().unwrap().clone();
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 64.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        install_q1_native(&mut server, Rc::clone(&shared));
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos1);
        // The deny touch fired (throttle armed) but spent no key.
        assert!(shared.borrow().doors.get(&door).unwrap().touch_throttle_until > 0.0);
        // Clear the volume for 2s so the throttle lapses (stock re-arms
        // on every processed touch, denies included), then bump the door
        // with the key carried: the touch consumes the key, nulls the
        // touch, and opens the door.
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(32.0, 32.0, 500.0))
            .unwrap();
        for _ in 0..22 {
            server.tick(SourceTime::Seconds(0.1)).unwrap();
        }
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(32.0, 32.0, 64.0))
            .unwrap();
        shared.borrow_mut().player_keys = IT_KEY1;
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(shared.borrow().player_keys, 0);
        assert!(!server.triggers_mut().is_trigger(&door));
        assert_ne!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos1);
    }

    #[test]
    fn key_door_stays_open_through_live_ticks() {
        // Stock key doors are `wait -1 = never return`
        // (`progs106/doors.qc:401,408`): `door_hit_top` arms `nextthink =
        // ltime - 1`, a past instant the pusher never fires
        // (`WinQuake/sv_phys.c:732`), so arrival at the top rests and live
        // ticks never close the door.
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        let models = vec![door_model()];
        let fields = door_fields(&[
            ("angles", "0 90 0"),
            ("speed", "4000"),
            ("spawnflags", "16"),
            ("model", "*0"),
        ]);
        let pending = {
            let actor = server.spawn_entity(&fields).unwrap();
            build_q1_door(&mut server, &mut shared.borrow_mut(), &actor, &fields, &models).unwrap()
        };
        link_q1_doors(&mut server, &mut shared.borrow_mut(), vec![pending]).unwrap();
        let door = shared.borrow().doors.keys().next().unwrap().clone();
        assert_eq!(shared.borrow().doors.get(&door).unwrap().wait, -1.0);
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 64.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        shared.borrow_mut().player_keys = IT_KEY1;
        install_q1_native(&mut server, Rc::clone(&shared));
        // Touch with the key carried consumes the key and opens the door.
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::ToPos2);
        // Travel (56u at 4000u/s) plus margin: arrival rests at the top.
        for _ in 0..10 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos2);
        // Thirty more ticks (1.5s, past any positive wait): still open,
        // with no creep back toward the bottom.
        for _ in 0..30 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos2);
    }

    #[test]
    fn toggle_door_rests_open_until_fired_again() {
        let mut server = test_server();
        register_q1_spawns(server.spawns_mut());
        let mut behaviors = Q1NativeBehaviors::new();
        let models = vec![door_model()];
        let fields = door_fields(&[("angles", "0 90 0"), ("spawnflags", "32"), ("model", "*0")]);
        let pending = spawn_door(&mut server, &mut behaviors, &fields, &models);
        let open_origin = pending.params.pos2;
        link_q1_doors(&mut server, &mut behaviors, vec![pending]).unwrap();
        let door = behaviors.doors.keys().next().unwrap().clone();
        let field = behaviors.fields.keys().next().unwrap().clone();
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 64.0));
        behaviors.set_player(Some(player.id().clone()));
        let contact = TouchContact {
            trigger: field,
            other: player.id().clone(),
        };
        {
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_native_touch(&mut behaviors, simulation, movers, triggers, &contact);
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::ToPos2);
        // Arrival at the top rests (no wait think scheduled). The body
        // travels with the faked phase so the return trip measures a
        // nonzero distance.
        server.movers_mut().get_mut(&door).unwrap().phase = MoverPhase::AtPos2;
        server.simulation_mut().set_body_origin(&door, open_origin).unwrap();
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_native_mover_think(&mut behaviors, simulation, movers, &door, MoverPhase::AtPos2, true);
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos2);
        // A stray wait think never closes a toggle door either.
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_native_mover_think(&mut behaviors, simulation, movers, &door, MoverPhase::AtPos2, false);
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::AtPos2);
        // Firing again travels down.
        reset_field_throttles(&mut behaviors);
        {
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_native_touch(&mut behaviors, simulation, movers, triggers, &contact);
        }
        assert_eq!(server.movers_mut().get(&door).unwrap().phase, MoverPhase::ToPos1);
    }

    /// Clear field throttles so a second direct touch fires in one test.
    fn reset_field_throttles(behaviors: &mut Q1NativeBehaviors) {
        for field in behaviors.fields.values_mut() {
            field.throttle_until = 0.0;
        }
    }
}
