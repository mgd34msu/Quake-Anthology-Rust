//! Quake III team-arena: client spawn.
//!
//! Donor provenance: `src/content/q3/team-arena/client-spawn.ts`.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, sub3, vec3, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::numeric::game_atoi;
use crate::q3::base::game::spawn::SpawnVariables;
use crate::q3::base::game::state::{GameFlags, TeamState, MAX_GENTITIES};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::ServerEntityFlags;
use crate::q3::base::shared::player_state::{MoveFlags, PlayerAnimation, UserCommand};
use crate::q3::base::shared::trajectory::TrajectoryType;
use crate::q3::team_arena::client_effects::*;
use crate::q3::team_arena::client_events::*;
use crate::q3::team_arena::client_think::*;
use crate::q3::team_arena::support::*;

// ---------------------------------------------------------------------------
// client-spawn.ts
// ---------------------------------------------------------------------------

/// Spawn position and angles (`SpawnPose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnPose {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Spawn point with its entity (`SpawnPoint`).
#[derive(Clone)]
pub struct SpawnPoint {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Spawn entity.
    pub entity: EntityRef,
}

/// Spawn frame (`ClientSpawnFrame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSpawnFrame {
    /// Current time.
    pub time: i32,
    /// Game type code.
    pub game_type: i32,
    /// Inactivity limit.
    pub inactivity_seconds: i32,
    /// Intermission time.
    pub intermission_time: i32,
}

/// Client-spawn services (`ClientSpawnHost`).
pub trait ClientSpawnHost {
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Server world.
    fn world(&self) -> WorldRef;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
    /// Game random.
    fn random(&self) -> &GameRandom;
    /// Think runtime.
    fn think_runtime(&self) -> Rc<ClientThinkRuntime>;
    /// Current frame.
    fn frame(&self) -> ClientSpawnFrame;
    /// Latest user command for a client.
    fn user_command(&self, client_num: usize) -> UserCommand;
    /// Handicap cvar text for a client.
    fn handicap(&self, client_num: usize) -> String;
    /// Intermission pose.
    fn find_intermission_point(&self) -> SpawnPose;
    /// Move to intermission.
    fn move_to_intermission(&self, entity: &EntityRef);
    /// Telefag relief.
    fn kill_box(&self, entity: &EntityRef);
    /// Player death callback.
    fn player_die(&self) -> DieCallback;
    /// Body death callback.
    fn body_die(&self) -> DieCallback;
    /// Whether a selected player spawner is installed.
    fn has_selected_player(&self) -> bool;
    /// Selected player spawner.
    fn selected_player(&self, entity: &EntityRef, pose: &SpawnPose);
    /// Effect services.
    fn effects(&self) -> EffectsRef;
    /// Target services.
    fn targets(&self) -> Rc<dyn TargetsHost>;
}

/// Corpse queue size (`BODY_QUEUE_SIZE`).
pub const BODY_QUEUE_SIZE: usize = 8;

pub(crate) const SPAWN_CONTENTS_BODY: i32 = 0x2000000;

pub(crate) const CONTENTS_CORPSE: i32 = 0x4000000;

pub(crate) const CONTENTS_NODROP: u32 = 0x80000000;

pub(crate) const SPAWN_EF_DEAD: i32 = 1;

pub(crate) const EF_TELEPORT_BIT: i32 = 4;

pub(crate) const SPAWN_EF_KAMIKAZE: i32 = 0x200;

pub(crate) const SPAWN_EF_VOTED: i32 = 0x4000;

pub(crate) const SPAWN_EF_TEAMVOTED: i32 = 0x80000;

pub(crate) fn spawn_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Client spawning requires a client entity"),
    }
}

/// Deathmatch spawn-point keys (`spawnDeathmatchPoint`).
pub fn spawn_deathmatch_point(entity: &EntityRef, variables: &SpawnVariables) {
    if variables.int("nobots", "0").unwrap().value != 0 {
        entity.borrow_mut().flags |= GameFlags::NO_BOTS;
    }
    if variables.int("nohumans", "0").unwrap().value != 0 {
        entity.borrow_mut().flags |= GameFlags::NO_HUMANS;
    }
}

/// Single-player start conversion (`spawnPlayerStart`).
pub fn spawn_player_start(entity: &EntityRef, variables: &SpawnVariables) {
    entity
        .borrow_mut()
        .set_classname(Some("info_player_deathmatch".to_string()));
    spawn_deathmatch_point(entity, variables);
}

/// Set the client view angle with QVM rounding (`setClientViewAngle`).
pub fn set_client_view_angle(entity: &EntityRef, angles: Vec3) {
    let client = spawn_client_of(entity);
    let command = client.borrow().pers.cmd.angles;
    let delta = |angle: f32, command: f32| -> i32 {
        let scaled = (angle * 65536.0) / 360.0;
        let short = qvm_float_to_int(scaled) & 65535;
        short.wrapping_sub(command as i32)
    };
    let mut record = client.borrow_mut();
    record.ps.delta_angles = vec3(
        delta(angles.x, command.x) as f32,
        delta(angles.y, command.y) as f32,
        delta(angles.z, command.z) as f32,
    );
    drop(record);
    entity.borrow_mut().s.angles = vec3(angles.x, angles.y, angles.z);
    let angles = entity.borrow().s.angles;
    client.borrow_mut().ps.viewangles = angles;
}

pub(crate) fn spawn_pose(entity: &EntityRef) -> SpawnPoint {
    let body = entity.borrow();
    SpawnPoint {
        origin: add3(body.s.origin, vec3(0.0, 0.0, 9.0)),
        angles: body.s.angles,
        entity: entity.clone(),
    }
}

/// Corpse queue state (`ClientSpawnState`).
#[derive(Clone, Default)]
pub struct ClientSpawnState {
    /// Corpse entities and rotation index.
    pub body_queue: Option<(Vec<EntityRef>, usize)>,
}

#[derive(Clone)]
pub(crate) struct ClientSpawnInner {
    host: Rc<dyn ClientSpawnHost>,
    state: RefCell<ClientSpawnState>,
}

/// Client-spawn runtime (`ClientSpawnRuntime`).
#[derive(Clone)]
pub struct ClientSpawnRuntime {
    inner: Rc<ClientSpawnInner>,
}

impl ClientSpawnRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn ClientSpawnHost>, state: ClientSpawnState) -> Self {
        if !Rc::ptr_eq(&host.think_runtime().host.pool(), &host.pool())
            || !Rc::ptr_eq(&host.think_runtime().host.world(), &host.world())
        {
            panic!("Client spawning and thinking must share entity storage and world");
        }
        let runtime = Self {
            inner: Rc::new(ClientSpawnInner {
                host,
                state: RefCell::new(state),
            }),
        };
        runtime.bind_save_callbacks();
        runtime
    }

    /// Host services.
    #[must_use]
    pub fn host(&self) -> Rc<dyn ClientSpawnHost> {
        self.inner.host.clone()
    }

    /// Checkpoint capture.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveValue {
        match &self.inner.state.borrow().body_queue {
            None => SaveValue::map(vec![("bodyQueue", SaveValue::Null)]),
            Some((entities, index)) => SaveValue::map(vec![(
                "bodyQueue",
                SaveValue::map(vec![
                    (
                        "entities",
                        SaveValue::List(
                            entities
                                .iter()
                                .map(|entity| SaveValue::Int(entity.borrow().slot as i64))
                                .collect(),
                        ),
                    ),
                    ("index", SaveValue::Int(*index as i64)),
                ]),
            )]),
        }
    }

    /// Checkpoint restore.
    pub fn restore_save_state(&self, value: &SaveValue) {
        let reader = SaveReader::new(value, "q3.clientSpawn");
        let queue = reader.field("bodyQueue").nullable(|entry| {
            let entities = entry
                .field("entities")
                .list(|item| read_module_entity(item, &self.inner.host.pool()));
            let index = entry.field("index").integer(0);
            (entities, index)
        });
        if let Some((entities, index)) = &queue {
            let unique: std::collections::HashSet<usize> = entities.iter().map(|entity| entity.borrow().slot).collect();
            if entities.len() != BODY_QUEUE_SIZE || *index as usize >= entities.len() || unique.len() != entities.len()
            {
                reader.fail("invalid body queue");
            }
        }
        self.inner.state.borrow_mut().body_queue = queue.map(|(entities, index)| (entities, index as usize));
    }

    fn owned_client(&self, entity: &EntityRef) -> ClientRef {
        let slot = entity.borrow().slot;
        let owned = match self.inner.host.pool().get(slot) {
            Some(owned) => Rc::ptr_eq(&owned, entity),
            None => false,
        };
        let linked = match entity.borrow().client.clone() {
            Some(client) => match slot < self.inner.host.pool().max_clients() {
                true => Rc::ptr_eq(&client, &self.inner.host.pool().client_at(slot)),
                false => false,
            },
            None => false,
        };
        if !owned || slot >= self.inner.host.pool().max_clients() || !linked {
            panic!("Client does not belong to the configured pool slot");
        }
        spawn_client_of(entity)
    }

    /// Whether a spawn point would telefrag (`spotWouldTelefrag`).
    #[must_use]
    pub fn spot_would_telefrag(&self, spot: &EntityRef) -> bool {
        let origin = spot.borrow().s.origin;
        let bounds = Bounds {
            min: add3(origin, vec3(-15.0, -15.0, -24.0)),
            max: add3(origin, vec3(15.0, 15.0, 32.0)),
        };
        self.inner
            .host
            .world()
            .area_actors(&bounds, MAX_GENTITIES)
            .iter()
            .any(|actor| self.inner.host.is_player(actor))
    }

    /// Nearest deathmatch spawn (`selectNearestDeathmatchSpawnPoint`).
    #[must_use]
    pub fn select_nearest_deathmatch_spawn_point(&self, from: Vec3) -> Option<EntityRef> {
        let mut spot: Option<EntityRef> = None;
        let mut nearest: Option<EntityRef> = None;
        let mut nearest_distance = 999_999.0f32;
        loop {
            spot = find_entity(
                &self.inner.host.pool(),
                spot.as_ref(),
                EntityStringField::Classname,
                Some("info_player_deathmatch"),
            );
            match spot.clone() {
                Some(point) => {
                    let distance = length3(sub3(point.borrow().s.origin, from));
                    if distance < nearest_distance {
                        nearest_distance = distance;
                        nearest = Some(point);
                    }
                }
                None => break,
            }
        }
        nearest
    }

    /// Random safe deathmatch spawn (`selectRandomDeathmatchSpawnPoint`).
    pub fn select_random_deathmatch_spawn_point(&self) -> Option<EntityRef> {
        let mut points = Vec::new();
        let mut spot: Option<EntityRef> = None;
        loop {
            spot = find_entity(
                &self.inner.host.pool(),
                spot.as_ref(),
                EntityStringField::Classname,
                Some("info_player_deathmatch"),
            );
            match spot.clone() {
                Some(point) => {
                    if self.spot_would_telefrag(&point) {
                        continue;
                    }
                    if points.len() == 128 {
                        panic!("SelectRandomDeathmatchSpawnPoint exceeds source 128-entry storage");
                    }
                    points.push(point);
                }
                None => break,
            }
        }
        if points.is_empty() {
            return find_entity(
                &self.inner.host.pool(),
                None,
                EntityStringField::Classname,
                Some("info_player_deathmatch"),
            );
        }
        let index = (self.inner.host.random().rand() as usize) % points.len();
        match points.get(index) {
            Some(chosen) => Some(chosen.clone()),
            None => panic!("Random spawn index invariant"),
        }
    }

    /// Spawn from the farthest integer half (`selectSpawnPoint`).
    pub fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint {
        let mut points: Vec<(EntityRef, f32)> = Vec::new();
        let mut spot: Option<EntityRef> = None;
        loop {
            spot = find_entity(
                &self.inner.host.pool(),
                spot.as_ref(),
                EntityStringField::Classname,
                Some("info_player_deathmatch"),
            );
            match spot.clone() {
                Some(point) => {
                    if self.spot_would_telefrag(&point) {
                        continue;
                    }
                    let distance = length3(sub3(point.borrow().s.origin, avoid));
                    match points.iter().position(|entry| distance > entry.1) {
                        Some(insertion) => {
                            points.insert(insertion, (point, distance));
                            if points.len() > 64 {
                                points.pop();
                            }
                        }
                        None => {
                            if points.len() < 64 {
                                points.push((point, distance));
                            }
                        }
                    }
                }
                None => break,
            }
        }
        if points.is_empty() {
            let fallback = find_entity(
                &self.inner.host.pool(),
                None,
                EntityStringField::Classname,
                Some("info_player_deathmatch"),
            );
            match fallback {
                Some(point) => return spawn_pose(&point),
                None => panic!("Couldn't find a spawn point"),
            }
        }
        let half = (points.len() / 2) as f32;
        let index = (self.inner.host.random().random() * half).trunc() as usize;
        match points.get(index) {
            Some(selected) => spawn_pose(&selected.0),
            None => panic!("Farthest spawn index invariant"),
        }
    }

    /// Initial spawn point (`selectInitialSpawnPoint`).
    pub fn select_initial_spawn_point(&self) -> SpawnPoint {
        let mut spot: Option<EntityRef> = None;
        loop {
            spot = find_entity(
                &self.inner.host.pool(),
                spot.as_ref(),
                EntityStringField::Classname,
                Some("info_player_deathmatch"),
            );
            match spot.clone() {
                Some(point) => {
                    if point.borrow().spawnflags & 1 != 0 {
                        break;
                    }
                }
                None => break,
            }
        }
        match spot {
            Some(point) if !self.spot_would_telefrag(&point) => spawn_pose(&point),
            _ => self.select_spawn_point(vec3(0.0, 0.0, 0.0)),
        }
    }

    /// Team spawn point (`selectTeamSpawnPoint`).
    pub fn select_team_spawn_point(&self, team_code: i32, state: i32) -> SpawnPoint {
        if team_code != Team::TeamRed as i32 && team_code != Team::TeamBlue as i32 {
            return self.select_spawn_point(vec3(0.0, 0.0, 0.0));
        }
        let side = if team_code == Team::TeamRed as i32 {
            "red"
        } else {
            "blue"
        };
        let role = if state == TeamState::Begin as i32 {
            "player"
        } else {
            "spawn"
        };
        let name = format!("team_CTF_{side}{role}");
        let mut points = Vec::new();
        let mut spot: Option<EntityRef> = None;
        loop {
            spot = find_entity(
                &self.inner.host.pool(),
                spot.as_ref(),
                EntityStringField::Classname,
                Some(name.as_str()),
            );
            match spot.clone() {
                Some(point) => {
                    if self.spot_would_telefrag(&point) {
                        continue;
                    }
                    points.push(point);
                    if points.len() == 32 {
                        break;
                    }
                }
                None => break,
            }
        }
        if points.is_empty() {
            let fallback = find_entity(
                &self.inner.host.pool(),
                None,
                EntityStringField::Classname,
                Some(name.as_str()),
            );
            return match fallback {
                Some(point) => spawn_pose(&point),
                None => self.select_spawn_point(vec3(0.0, 0.0, 0.0)),
            };
        }
        let index = (self.inner.host.random().rand() as usize) % points.len();
        match points.get(index) {
            Some(selected) => spawn_pose(selected),
            None => panic!("Team spawn index invariant"),
        }
    }

    /// Allocate the corpse queue (`initBodyQueue`).
    pub fn init_body_queue(&self) {
        if self.inner.state.borrow().body_queue.is_some() {
            panic!("Body queue is already initialized for this map");
        }
        let entities: Vec<EntityRef> = (0..BODY_QUEUE_SIZE)
            .map(|_| {
                let entity = self.inner.host.pool().spawn();
                let mut body = entity.borrow_mut();
                body.set_classname(Some("bodyque".to_string()));
                body.never_free = true;
                drop(body);
                entity
            })
            .collect();
        self.inner.state.borrow_mut().body_queue = Some((entities, 0));
    }

    /// Sink a corpse (`bodySink`).
    pub fn body_sink(&self, body_entity: &EntityRef) {
        let time = self.inner.host.frame().time;
        if time.wrapping_sub(body_entity.borrow().timestamp) > 6500 {
            self.inner.host.world().unlink(body_entity.borrow().slot as i32);
            body_entity.borrow_mut().physics_object = false;
            return;
        }
        body_entity.borrow_mut().nextthink = time.wrapping_add(100);
        let mut body = body_entity.borrow_mut();
        body.s.pos.base = add3(body.s.pos.base, vec3(0.0, 0.0, -1.0));
    }

    /// Copy a corpse into the queue (`copyToBodyQueue`).
    pub fn copy_to_body_queue(&self, entity: &EntityRef) -> Option<EntityRef> {
        let client = self.owned_client(entity);
        let time = self.inner.host.frame().time;
        self.inner.host.world().unlink(entity.borrow().slot as i32);
        let origin = entity.borrow().s.origin;
        if self.inner.host.world().point_contents(origin, -1) as u32 & CONTENTS_NODROP != 0 {
            return None;
        }
        let (body, product, kamikaze, velocity) = {
            let state = self.inner.state.borrow();
            let (entities, index) = match &state.body_queue {
                Some(queue) => queue,
                None => panic!("Body queue must be initialized before copying a corpse"),
            };
            let body = match entities.get(*index) {
                Some(body) => body.clone(),
                None => panic!("Body queue index invariant"),
            };
            let record = client.borrow();
            (
                body,
                record.ps.product,
                entity.borrow().s.e_flags & SPAWN_EF_KAMIKAZE != 0,
                record.ps.velocity,
            )
        };
        {
            let mut state = self.inner.state.borrow_mut();
            if let Some((_, index)) = state.body_queue.as_mut() {
                *index = (*index + 1) % BODY_QUEUE_SIZE;
            }
        }
        self.inner.host.world().unlink(body.borrow().slot as i32);
        {
            let source = entity.borrow().s.clone();
            let mut target = body.borrow_mut();
            target.s = source;
        }
        write_ground(&body, entity.borrow().ground.clone(), &self.inner.host.pool());
        body.borrow_mut().s.e_flags = SPAWN_EF_DEAD;
        if product == Product::Missionpack && kamikaze {
            body.borrow_mut().s.e_flags |= SPAWN_EF_KAMIKAZE;
            for index in 0..MAX_GENTITIES {
                let timer = self.inner.host.pool().at(index);
                let matches = {
                    let candidate = timer.borrow();
                    candidate.inuse
                        && candidate.classname() == Some("kamikaze timer".to_string())
                        && candidate
                            .activator
                            .as_ref()
                            .map(|activator| Rc::ptr_eq(activator, entity))
                            .unwrap_or(false)
                };
                if matches {
                    timer.borrow_mut().activator = Some(body.clone());
                    break;
                }
            }
        }
        {
            let mut target = body.borrow_mut();
            target.s.powerups = 0;
            target.s.loop_sound = 0;
            target.s.number = target.slot as i32;
            target.timestamp = time;
            target.physics_object = true;
            target.physics_bounce = 0;
            let grounded = target.ground.is_some();
            if grounded {
                target.s.pos.trajectory_type = TrajectoryType::TrStationary;
            } else {
                target.s.pos.trajectory_type = TrajectoryType::TrGravity;
                target.s.pos.time = time;
                target.s.pos.delta = velocity;
            }
            target.s.event = 0;
            let animation = target.s.legs_anim & !128;
            let dead = if animation == PlayerAnimation::BothDeath1 as i32
                || animation == PlayerAnimation::BothDead1 as i32
            {
                PlayerAnimation::BothDead1 as i32
            } else if animation == PlayerAnimation::BothDeath2 as i32 || animation == PlayerAnimation::BothDead2 as i32
            {
                PlayerAnimation::BothDead2 as i32
            } else {
                PlayerAnimation::BothDead3 as i32
            };
            target.s.torso_anim = dead;
            target.s.legs_anim = dead;
        }
        let (sv_flags, mins, maxs, number, health) = {
            let source = entity.borrow();
            (
                source.r.sv_flags,
                source.r.mins,
                source.r.maxs,
                source.s.number,
                source.health,
            )
        };
        {
            let mut target = body.borrow_mut();
            target.r.sv_flags = sv_flags;
            target.r.mins = mins;
            target.r.maxs = maxs;
            target.clipmask = 1 | 0x10000;
            target.r.contents = CONTENTS_CORPSE;
            target.r.owner_num = number;
            target.nextthink = time.wrapping_add(5000);
            target.takedamage = health > GIB_HEALTH;
            let settled = target.s.pos.base;
            target.r.set_current_origin(settled);
        }
        let think = self
            .inner
            .host
            .pool()
            .callbacks
            .resolve_think("q3.team-arena.client-spawn.copyToBodyQueue.think");
        let die = self.inner.host.pool().callbacks.resolve_die("q3.death.body");
        body.borrow_mut().think = Some(think);
        body.borrow_mut().die = Some(die);
        self.inner.host.world().link(&body);
        Some(body)
    }

    /// Spawn a client (`clientSpawn`).
    pub fn client_spawn(&self, entity: &EntityRef) {
        let client = self.owned_client(entity);
        let frame = self.inner.host.frame();
        let (spawn_origin, spawn_angles, spawn_entity) = self.pick_spawn(entity, &client, &frame);
        client.borrow_mut().pers.team_state.state = TeamState::Active as i32;
        entity.borrow_mut().s.e_flags &= !SPAWN_EF_KAMIKAZE;
        let flags =
            (client.borrow().ps.e_flags & (EF_TELEPORT_BIT | SPAWN_EF_VOTED | SPAWN_EF_TEAMVOTED)) ^ EF_TELEPORT_BIT;
        let product = client.borrow().ps.product;
        let ping = client.borrow().ps.ping;
        let persistant = client.borrow().ps.persistant.copy();
        let event_sequence = client.borrow().ps.event_sequence;
        let saved_pers = client.borrow().pers.clone();
        let saved_sess = client.borrow().sess.clone();
        let (accuracy_hits, accuracy_shots) = {
            let record = client.borrow();
            (record.accuracy_hits, record.accuracy_shots)
        };
        // The donor preserves the pers/sess/ps object graph while resetting
        // every other field; nothing else aliases those records here, so a
        // fresh client with the saved records restored is equivalent.
        *client.borrow_mut() = GameClient::new(product);
        {
            let mut record = client.borrow_mut();
            record.pers = saved_pers;
            record.sess = saved_sess;
            record.accuracy_hits = accuracy_hits;
            record.accuracy_shots = accuracy_shots;
            record.ps.ping = ping;
            for (index, value) in persistant.iter().enumerate() {
                record.ps.persistant.set(index, *value);
            }
            record.ps.event_sequence = event_sequence;
            let spawns = record.ps.persistant.get(PersistentIndex::PersSpawnCount as usize);
            record
                .ps
                .persistant
                .set(PersistentIndex::PersSpawnCount as usize, spawns.wrapping_add(1));
            let team_code = record.sess.session_team;
            record.ps.persistant.set(PersistentIndex::PersTeam as usize, team_code);
        }
        self.finish_client_spawn(
            entity,
            &client,
            &frame,
            flags,
            spawn_origin,
            spawn_angles,
            spawn_entity.as_ref(),
        );
    }

    fn pick_spawn(
        &self,
        entity: &EntityRef,
        client: &ClientRef,
        frame: &ClientSpawnFrame,
    ) -> (Vec3, Vec3, Option<EntityRef>) {
        if client.borrow().sess.session_team == Team::TeamSpectator as i32 {
            let pose = self.inner.host.find_intermission_point();
            return (pose.origin, pose.angles, None);
        }
        if frame.game_type >= GameType::GtCtf as i32 {
            let selected =
                self.select_team_spawn_point(client.borrow().sess.session_team, client.borrow().pers.team_state.state);
            return (selected.origin, selected.angles, Some(selected.entity));
        }
        let mut states = std::collections::HashSet::new();
        loop {
            let state = (self.inner.host.random().seed() & 0x7fff)
                | if client.borrow().pers.initial_spawn { 0x8000 } else { 0 };
            if !states.insert(state) {
                panic!("No permitted spawn point in the complete game RNG cycle");
            }
            let selected = if !client.borrow().pers.initial_spawn && client.borrow().pers.local_client {
                client.borrow_mut().pers.initial_spawn = true;
                self.select_initial_spawn_point()
            } else {
                self.select_spawn_point(client.borrow().ps.origin)
            };
            let bot = entity.borrow().r.sv_flags & ServerEntityFlags::Bot as i32 != 0;
            let blocked = if bot { GameFlags::NO_BOTS } else { GameFlags::NO_HUMANS };
            if selected.entity.borrow().flags & blocked != 0 {
                continue;
            }
            return (selected.origin, selected.angles, Some(selected.entity));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_client_spawn(
        &self,
        entity: &EntityRef,
        client: &ClientRef,
        frame: &ClientSpawnFrame,
        flags: i32,
        spawn_origin: Vec3,
        spawn_angles: Vec3,
        spawn_entity: Option<&EntityRef>,
    ) {
        client.borrow_mut().last_killed_client = -1;
        client.borrow_mut().air_out_time = frame.time.wrapping_add(12_000);
        let slot = entity.borrow().slot;
        let mut max_health = game_atoi(&self.inner.host.handicap(slot)).unwrap();
        if !(1..=100).contains(&max_health) {
            max_health = 100;
        }
        {
            let mut record = client.borrow_mut();
            record.pers.max_health = max_health;
            let max_health_slot = match stat_schema(record.ps.product) {
                StatSchema::Base(layout) => layout.max_health,
                StatSchema::Missionpack(layout) => layout.max_health,
            };
            record.ps.stats.set(max_health_slot as usize, max_health);
            record.ps.e_flags = flags;
        }
        write_ground(entity, None, &self.inner.host.pool());
        entity.borrow_mut().takedamage = true;
        self.inner.host.pool().activate_client(slot);
        entity.borrow_mut().set_classname(Some("player".to_string()));
        entity.borrow_mut().r.contents = SPAWN_CONTENTS_BODY;
        entity.borrow_mut().clipmask = 1 | 0x10000 | SPAWN_CONTENTS_BODY;
        let die = self.inner.host.pool().callbacks.resolve_die("q3.death.player");
        entity.borrow_mut().die = Some(die);
        entity.borrow_mut().waterlevel = 0;
        entity.borrow_mut().watertype = 0;
        entity.borrow_mut().flags = 0;
        let selected = self.inner.host.has_selected_player();
        if !selected {
            entity.borrow_mut().r.mins = vec3(-15.0, -15.0, -24.0);
            entity.borrow_mut().r.maxs = vec3(15.0, 15.0, 32.0);
        }
        let product = client.borrow().ps.product;
        client.borrow_mut().ps.client_num = slot as i32;
        client.borrow_mut().ps.ammo.set(Weapon::WpGauntlet as usize, -1);
        client.borrow_mut().ps.ammo.set(Weapon::WpGrapplingHook as usize, -1);
        if !selected {
            let weapons_slot = match stat_schema(product) {
                StatSchema::Base(layout) => layout.weapons,
                StatSchema::Missionpack(layout) => layout.weapons,
            };
            client.borrow_mut().ps.stats.set(
                weapons_slot as usize,
                (1 << Weapon::WpMachinegun as i32) | (1 << Weapon::WpGauntlet as i32),
            );
            client.borrow_mut().ps.ammo.set(
                Weapon::WpMachinegun as usize,
                if frame.game_type == GameType::GtTeam as i32 {
                    50
                } else {
                    100
                },
            );
            let grown = max_health.wrapping_add(25);
            entity.borrow_mut().health = grown;
            client.borrow_mut().ps.set_health(grown);
        }
        if selected {
            let pose = SpawnPose {
                origin: spawn_origin,
                angles: spawn_angles,
            };
            self.inner.host.selected_player(entity, &pose);
        }
        set_origin(entity, spawn_origin);
        client.borrow_mut().ps.origin = spawn_origin;
        client.borrow_mut().ps.pm_flags |= MoveFlags::Respawned as i32;
        let command = self.inner.host.user_command(slot);
        client.borrow_mut().pers.cmd = command;
        set_client_view_angle(entity, spawn_angles);
        if client.borrow().sess.session_team != Team::TeamSpectator as i32 {
            self.inner.host.kill_box(entity);
            self.inner.host.world().link(entity);
            if !selected {
                client.borrow_mut().ps.weapon = Weapon::WpMachinegun as i32;
            }
            client.borrow_mut().ps.weapon_state = WeaponState::WeaponReady as i32;
        }
        client.borrow_mut().ps.pm_flags |= MoveFlags::TimeKnockback as i32;
        client.borrow_mut().ps.pm_time = 100;
        client.borrow_mut().respawn_time = frame.time;
        client.borrow_mut().inactivity_time = frame.time.wrapping_add(frame.inactivity_seconds.wrapping_mul(1000));
        client.borrow_mut().latched_buttons = 0;
        client.borrow_mut().ps.torso_anim = PlayerAnimation::TorsoStand as i32;
        client.borrow_mut().ps.legs_anim = PlayerAnimation::LegsIdle as i32;
        if frame.intermission_time != 0 {
            self.inner.host.move_to_intermission(entity);
        } else {
            let activator = DamageParticipant::Entity(entity.clone());
            self.inner.host.targets().use_targets(spawn_entity, Some(&activator));
            if !selected {
                client.borrow_mut().ps.weapon = Weapon::WpGauntlet as i32;
                let weapons_slot = match stat_schema(product) {
                    StatSchema::Base(layout) => layout.weapons,
                    StatSchema::Missionpack(layout) => layout.weapons,
                };
                for weapon_tag in (1..weapon_count(product)).rev() {
                    if client.borrow().ps.stats.get(weapons_slot as usize) & (1 << weapon_tag) != 0 {
                        client.borrow_mut().ps.weapon = weapon_tag;
                        break;
                    }
                }
            }
        }
        client.borrow_mut().ps.command_time = frame.time.wrapping_sub(100);
        client.borrow_mut().pers.cmd.server_time = frame.time;
        let think_command = self.inner.host.user_command(slot);
        self.inner.host.think_runtime().client_think(slot, &think_command);
        if client.borrow().sess.session_team != Team::TeamSpectator as i32 {
            {
                let mut record = client.borrow_mut();
                let mut body = entity.borrow_mut();
                player_state_to_entity_state(&mut record.ps, &mut body.s, true);
            }
            entity.borrow_mut().r.set_current_origin(client.borrow().ps.origin);
            self.inner.host.world().link(entity);
        }
        client_end_frame(self.inner.host.effects().as_ref(), entity);
        {
            let mut record = client.borrow_mut();
            let mut body = entity.borrow_mut();
            player_state_to_entity_state(&mut record.ps, &mut body.s, true);
        }
    }

    /// Respawn an entity (`respawn`).
    pub fn respawn(&self, entity: &EntityRef) {
        self.copy_to_body_queue(entity);
        self.client_spawn(entity);
        let origin = spawn_client_of(entity).borrow().ps.origin;
        let temporary = self
            .inner
            .host
            .pool()
            .temp_entity(origin, EntityEvent::EvPlayerTeleportIn as i32);
        temporary.borrow_mut().s.client_num = entity.borrow().s.client_num;
    }

    /// Register save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(&self) {
        let inner = self.inner.clone();
        self.inner.host.pool().callbacks.intern_think(
            "q3.team-arena.client-spawn.copyToBodyQueue.think",
            Rc::new(move |entity: EntityRef| {
                let runtime = ClientSpawnRuntime { inner: inner.clone() };
                runtime.body_sink(&entity);
            }),
        );
        self.inner
            .host
            .pool()
            .callbacks
            .intern_die("q3.death.body", self.inner.host.body_die());
        self.inner
            .host
            .pool()
            .callbacks
            .intern_die("q3.death.player", self.inner.host.player_die());
    }
}

impl SpawnSelector for ClientSpawnRuntime {
    fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint {
        ClientSpawnRuntime::select_spawn_point(self, avoid)
    }
}
