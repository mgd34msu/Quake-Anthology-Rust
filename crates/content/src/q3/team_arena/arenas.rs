//! Quake III team-arena: arenas.
//!
//! Donor provenance: `src/content/q3/team-arena/arenas.ts`.

use qa_core::math::{add3, angle_vectors, scale3, sub3, vec3, vector_to_angles, Vec3};
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::commands::*;
use crate::q3::team_arena::mirrors::*;
use crate::q3::team_arena::r#match::*;

// ---------------------------------------------------------------------------
// arenas.ts
// ---------------------------------------------------------------------------

/// Arena services (`ArenaHost`).
pub trait ArenaHost {
    /// Match runtime.
    fn match_runtime(&self) -> Rc<MatchRuntime>;
    /// Server world.
    fn world(&self) -> WorldRef;
    /// Cvar registry.
    fn cvars(&self) -> Rc<dyn CvarRegistry>;
    /// Config-string services.
    fn config(&self) -> Rc<dyn ConfigStrings>;
}

pub(crate) const ARENA_CONTENTS_SOLID: i32 = 1;

pub(crate) const ARENA_CONTENTS_PLAYERCLIP: i32 = 0x10000;

pub(crate) const ARENA_CONTENTS_BODY: i32 = 0x2000000;

pub(crate) const RANK_TIED_FLAG: i32 = 0x4000;

pub(crate) const ANIM_TOGGLEBIT: i32 = 128;

pub(crate) const TIMER_GESTURE: i32 = 34 * 66 + 50;

pub(crate) fn arena_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Arena entity has no source client pointer"),
    }
}

#[derive(Clone)]
pub(crate) struct ArenaInner {
    host: Rc<dyn ArenaHost>,
    podium1: RefCell<Option<EntityRef>>,
    podium2: RefCell<Option<EntityRef>>,
    podium3: RefCell<Option<EntityRef>>,
}

/// Arena runtime (`ArenaRuntime`).
#[derive(Clone)]
pub struct ArenaRuntime {
    inner: Rc<ArenaInner>,
}

impl ArenaRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn ArenaHost>) -> Self {
        let runtime = Self {
            inner: Rc::new(ArenaInner {
                host,
                podium1: RefCell::new(None),
                podium2: RefCell::new(None),
                podium3: RefCell::new(None),
            }),
        };
        runtime.bind_save_callbacks();
        runtime
    }

    /// Checkpoint capture.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveValue {
        let slot = |podium: &RefCell<Option<EntityRef>>| -> SaveValue {
            match &*podium.borrow() {
                Some(entity) => SaveValue::Int(entity.borrow().slot as i64),
                None => SaveValue::Null,
            }
        };
        SaveValue::map(vec![
            ("podium1", slot(&self.inner.podium1)),
            ("podium2", slot(&self.inner.podium2)),
            ("podium3", slot(&self.inner.podium3)),
        ])
    }

    /// Checkpoint restore.
    pub fn restore_save_state(&self, value: &SaveValue) {
        let reader = SaveReader::new(value, "q3.arenas");
        let pool = self.inner.host.match_runtime().host.pool();
        let first = reader
            .field("podium1")
            .nullable(|entry| read_module_entity(entry, &pool));
        let second = reader
            .field("podium2")
            .nullable(|entry| read_module_entity(entry, &pool));
        let third = reader
            .field("podium3")
            .nullable(|entry| read_module_entity(entry, &pool));
        *self.inner.podium1.borrow_mut() = first;
        *self.inner.podium2.borrow_mut() = second;
        *self.inner.podium3.borrow_mut() = third;
    }

    /// Clear podium players (`resetPodiumPlayers`).
    pub fn reset_podium_players(&self) {
        *self.inner.podium1.borrow_mut() = None;
        *self.inner.podium2.borrow_mut() = None;
        *self.inner.podium3.borrow_mut() = None;
    }

    fn sorted(&self, index: usize) -> i32 {
        match self
            .inner
            .host
            .match_runtime()
            .host
            .state()
            .borrow()
            .sorted_clients
            .get(index)
        {
            Some(number) => *number,
            None => panic!("Arena rank index has no source storage"),
        }
    }

    fn score(&self, number: i32) -> i32 {
        self.inner
            .host
            .match_runtime()
            .host
            .pool()
            .client_at(number as usize)
            .borrow()
            .ps
            .persistant
            .get(persistent_index::SCORE as usize)
    }

    fn cvar_integer(&self, name: &str) -> i32 {
        match self.inner.host.cvars().get(name) {
            Some(cvar) => cvar.integer_value,
            None => panic!("Arena requires registered cvar {name}"),
        }
    }

    /// Postgame message (`updateTournamentInfo`).
    pub fn update_tournament_info(&self) {
        let runtime = self.inner.host.match_runtime();
        let pool = runtime.host.pool();
        let mut player: Option<EntityRef> = None;
        let mut player_client_num = 0;
        while player_client_num < pool.max_clients() {
            let entity = pool.at(player_client_num);
            if entity.borrow().inuse && entity.borrow().r.sv_flags & server_entity_flags::BOT == 0 {
                player = Some(entity);
                break;
            }
            player_client_num += 1;
        }
        let Some(player) = player else {
            return;
        };
        runtime.calculate_ranks();
        let ranked = pool.client_at(player_client_num);
        let non_spectators = runtime.host.state().borrow().num_non_spectator_clients;
        let mut message: String;
        if ranked.borrow().sess.session_team == team::SPECTATOR {
            message = game_format(
                if runtime.host.product() == Product::MissionPack {
                    "postgame %i %i 0 0 0 0 0 0 0 0 0 0 0"
                } else {
                    "postgame %i %i 0 0 0 0 0 0"
                },
                &[FormatArg::Int(non_spectators), FormatArg::Int(player_client_num as i32)],
                MAX_STRING_CHARS,
            );
        } else {
            let client = arena_client_of(&player);
            let accuracy = if client.borrow().accuracy_shots != 0 {
                client.borrow().accuracy_hits.wrapping_mul(100) / client.borrow().accuracy_shots
            } else {
                0
            };
            if runtime.host.product() == Product::MissionPack {
                let (won, score1, score2) = if runtime.host.settings().game_type >= game_type::CTF {
                    let score1 = runtime.host.team_scores().get(team::RED as usize);
                    let score2 = runtime.host.team_scores().get(team::BLUE as usize);
                    let won = if ranked.borrow().sess.session_team == team::RED {
                        score1 > score2
                    } else {
                        score2 > score1
                    };
                    (won, score1, score2)
                } else if Rc::ptr_eq(&ranked, &pool.client_at(self.sorted(0) as usize)) {
                    (true, self.score(self.sorted(0)), self.score(self.sorted(1)))
                } else {
                    (false, self.score(self.sorted(1)), self.score(self.sorted(0)))
                };
                let record = client.borrow();
                let perfect = if won && record.ps.persistant.get(persistent_index::KILLED as usize) == 0 {
                    1
                } else {
                    0
                };
                let time = runtime.host.state().borrow().time;
                message = game_format(
                    "postgame %i %i %i %i %i %i %i %i %i %i %i %i %i %i",
                    &[
                        FormatArg::Int(non_spectators),
                        FormatArg::Int(player_client_num as i32),
                        FormatArg::Int(accuracy),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::IMPRESSIVE_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::EXCELLENT_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::DEFEND_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::ASSIST_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::GAUNTLET_FRAG_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::SCORE as usize)),
                        FormatArg::Int(perfect),
                        FormatArg::Int(score1),
                        FormatArg::Int(score2),
                        FormatArg::Int(time),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::CAPTURES as usize)),
                    ],
                    MAX_STRING_CHARS,
                );
            } else {
                let record = client.borrow();
                let perfect = if ranked.borrow().ps.persistant.get(persistent_index::RANK as usize) == 0
                    && record.ps.persistant.get(persistent_index::KILLED as usize) == 0
                {
                    1
                } else {
                    0
                };
                message = game_format(
                    "postgame %i %i %i %i %i %i %i %i",
                    &[
                        FormatArg::Int(non_spectators),
                        FormatArg::Int(player_client_num as i32),
                        FormatArg::Int(accuracy),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::IMPRESSIVE_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::EXCELLENT_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::GAUNTLET_FRAG_COUNT as usize)),
                        FormatArg::Int(record.ps.persistant.get(persistent_index::SCORE as usize)),
                        FormatArg::Int(perfect),
                    ],
                    MAX_STRING_CHARS,
                );
            }
        }
        let message_length = message.len();
        for index in 0..non_spectators {
            let number = self.sorted(index as usize);
            let row = game_format(
                " %i %i %i",
                &[
                    FormatArg::Int(number),
                    FormatArg::Int(
                        pool.client_at(number as usize)
                            .borrow()
                            .ps
                            .persistant
                            .get(persistent_index::RANK as usize),
                    ),
                    FormatArg::Int(
                        pool.client_at(number as usize)
                            .borrow()
                            .ps
                            .persistant
                            .get(persistent_index::SCORE as usize),
                    ),
                ],
                32,
            );
            if message_length + row.len() + 1 >= MAX_STRING_CHARS {
                break;
            }
            // Source never increments msglen. A reached strcat overflow has no owned storage here.
            if message.len() + row.len() >= MAX_STRING_CHARS {
                panic!("Postgame strcat exceeds the source 1024-byte message buffer");
            }
            message += &row;
        }
        runtime.host.append_console_command(&message);
    }

    fn place_model(&self, body: &EntityRef, podium: &EntityRef, offset: Vec3) {
        let origin = self
            .inner
            .host
            .match_runtime()
            .host
            .state()
            .borrow()
            .intermission_origin;
        let angles = vector_to_angles(sub3(origin, podium.borrow().r.current_origin()));
        body.borrow_mut().s.apos.base = vec3(0.0, angles.y, 0.0);
        let axis = angle_vectors(body.borrow().s.apos.base);
        let mut placed = add3(podium.borrow().r.current_origin(), scale3(axis.forward, offset.x));
        placed = add3(placed, scale3(axis.right, offset.y));
        placed = add3(placed, scale3(axis.up, offset.z));
        set_origin(body, placed);
    }

    fn spawn_model_on_victory_pad(&self, pad: &EntityRef, offset: Vec3, entity: &EntityRef, place: i32) -> EntityRef {
        let runtime = self.inner.host.match_runtime();
        let pool = runtime.host.pool();
        let body = pool.spawn();
        let client = arena_client_of(entity);
        body.borrow_mut().bind_client_name(&client);
        body.borrow_mut().client = Some(client);
        body.borrow_mut().s = entity.borrow().s.clone();
        body.borrow_mut().s.e_type = entity_type::PLAYER;
        body.borrow_mut().s.e_flags = 0;
        body.borrow_mut().s.powerups = 0;
        body.borrow_mut().s.loop_sound = 0;
        let slot = body.borrow().slot as i32;
        body.borrow_mut().s.number = slot;
        body.borrow_mut().timestamp = runtime.host.state().borrow().time;
        body.borrow_mut().physics_object = true;
        body.borrow_mut().physics_bounce = 0;
        body.borrow_mut().s.event = 0;
        body.borrow_mut().s.pos.traj_type = trajectory_type::STATIONARY;
        write_ground(&body, Some(pool.at(ENTITYNUM_WORLD).borrow().actor.clone()), &pool);
        body.borrow_mut().s.legs_anim = player_animation::LEGS_IDLE;
        body.borrow_mut().s.torso_anim = player_animation::TORSO_STAND;
        if body.borrow().s.weapon == weapon::NONE {
            body.borrow_mut().s.weapon = weapon::MACHINEGUN;
        }
        if body.borrow().s.weapon == weapon::GAUNTLET {
            body.borrow_mut().s.torso_anim = player_animation::TORSO_STAND2;
        }
        body.borrow_mut().s.event = 0;
        body.borrow_mut().r.sv_flags = entity.borrow().r.sv_flags;
        // SVF_CAPSULE is encoded in r.model; source leaves the separate bmodel zero.
        body.borrow_mut().r.model = entity.borrow().r.model;
        body.borrow_mut().r.mins = entity.borrow().r.mins;
        body.borrow_mut().r.maxs = entity.borrow().r.maxs;
        body.borrow_mut().clipmask = ARENA_CONTENTS_SOLID | ARENA_CONTENTS_PLAYERCLIP;
        body.borrow_mut().r.contents = ARENA_CONTENTS_BODY;
        body.borrow_mut().r.owner_num = entity.borrow().r.owner_num;
        body.borrow_mut().takedamage = false;
        self.place_model(&body, pad, offset);
        // trap_LinkEntity replaces the copied source absmin/absmax with these bounds.
        self.inner.host.world().link(&body);
        body.borrow_mut().count = place;
        body
    }

    fn celebrate_stop(&self, player: &EntityRef) {
        let animation = if player.borrow().s.weapon == weapon::GAUNTLET {
            player_animation::TORSO_STAND2
        } else {
            player_animation::TORSO_STAND
        };
        let mut body = player.borrow_mut();
        body.s.torso_anim = ((body.s.torso_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | animation;
    }

    fn celebrate_start(&self, player: &EntityRef) {
        {
            let mut body = player.borrow_mut();
            body.s.torso_anim =
                ((body.s.torso_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | player_animation::TORSO_GESTURE;
        }
        let runtime = self.inner.host.match_runtime();
        player.borrow_mut().nextthink = runtime.host.state().borrow().time.wrapping_add(TIMER_GESTURE);
        let think = runtime
            .host
            .pool()
            .callbacks
            .resolve_think("q3.team-arena.arenas.celebrateStart.think");
        player.borrow_mut().think = Some(think);
        runtime.host.pool().add_event(player, entity_event::TAUNT, 0);
    }

    fn podium_origin(&self) -> Vec3 {
        let runtime = self.inner.host.match_runtime();
        let state = runtime.host.state();
        let record = state.borrow();
        let forward = angle_vectors(record.intermission_angle).forward;
        let view = record.intermission_origin;
        drop(record);
        // VectorMA expands the live engine cvar trap separately for each component.
        let mut origin = vec3(
            view.x + forward.x * self.cvar_integer("g_podiumDist") as f32,
            view.y + forward.y * self.cvar_integer("g_podiumDist") as f32,
            view.z + forward.z * self.cvar_integer("g_podiumDist") as f32,
        );
        let drop_distance = self.cvar_integer("g_podiumDrop");
        origin = vec3(origin.x, origin.y, origin.z - drop_distance as f32);
        origin
    }

    fn podium_placement_think(&self, podium: &EntityRef) {
        let runtime = self.inner.host.match_runtime();
        podium.borrow_mut().nextthink = runtime.host.state().borrow().time.wrapping_add(100);
        let origin = self.podium_origin();
        set_origin(podium, origin);
        if let Some(first) = self.inner.podium1.borrow().clone() {
            self.place_model(&first, podium, vec3(0.0, 0.0, 74.0));
        }
        if let Some(second) = self.inner.podium2.borrow().clone() {
            self.place_model(&second, podium, vec3(-10.0, 60.0, 54.0));
        }
        if let Some(third) = self.inner.podium3.borrow().clone() {
            self.place_model(&third, podium, vec3(-19.0, -60.0, 45.0));
        }
    }

    fn spawn_podium(&self) -> EntityRef {
        let runtime = self.inner.host.match_runtime();
        let pool = runtime.host.pool();
        let podium = pool.spawn();
        podium.borrow_mut().set_classname(Some("podium".to_string()));
        podium.borrow_mut().s.e_type = entity_type::GENERAL;
        let slot = podium.borrow().slot as i32;
        podium.borrow_mut().s.number = slot;
        podium.borrow_mut().clipmask = ARENA_CONTENTS_SOLID;
        podium.borrow_mut().r.contents = ARENA_CONTENTS_SOLID;
        podium.borrow_mut().s.modelindex = self
            .inner
            .host
            .config()
            .model_index("models/mapobjects/podium/podium4.md3");
        let origin = self.podium_origin();
        set_origin(&podium, origin);
        let view = runtime.host.state().borrow().intermission_origin;
        let yaw = vector_to_angles(sub3(view, podium.borrow().r.current_origin())).y;
        let base = podium.borrow().s.apos.base;
        podium.borrow_mut().s.apos.base = vec3(base.x, yaw, base.z);
        self.inner.host.world().link(&podium);
        let think = pool.callbacks.resolve_think("q3.team-arena.arenas.spawnPodium.think");
        podium.borrow_mut().think = Some(think);
        podium.borrow_mut().nextthink = runtime.host.state().borrow().time.wrapping_add(100);
        podium
    }

    /// Spawn victory-pad models (`spawnModelsOnVictoryPads`).
    pub fn spawn_models_on_victory_pads(&self) {
        self.reset_podium_players();
        let runtime = self.inner.host.match_runtime();
        let pool = runtime.host.pool();
        let podium = self.spawn_podium();
        let number = self.sorted(0);
        let player = self.spawn_model_on_victory_pad(
            &podium,
            vec3(0.0, 0.0, 74.0),
            &pool.at(number as usize),
            pool.client_at(number as usize)
                .borrow()
                .ps
                .persistant
                .get(persistent_index::RANK as usize)
                & !RANK_TIED_FLAG,
        );
        player.borrow_mut().nextthink = runtime.host.state().borrow().time.wrapping_add(2000);
        let think = pool
            .callbacks
            .resolve_think("q3.team-arena.arenas.spawnModelsOnVictoryPads.think");
        player.borrow_mut().think = Some(think);
        *self.inner.podium1.borrow_mut() = Some(player);
        let number = self.sorted(1);
        let player = self.spawn_model_on_victory_pad(
            &podium,
            vec3(-10.0, 60.0, 54.0),
            &pool.at(number as usize),
            pool.client_at(number as usize)
                .borrow()
                .ps
                .persistant
                .get(persistent_index::RANK as usize)
                & !RANK_TIED_FLAG,
        );
        *self.inner.podium2.borrow_mut() = Some(player);
        if runtime.host.state().borrow().num_non_spectator_clients > 2 {
            let number = self.sorted(2);
            let player = self.spawn_model_on_victory_pad(
                &podium,
                vec3(-19.0, -60.0, 45.0),
                &pool.at(number as usize),
                pool.client_at(number as usize)
                    .borrow()
                    .ps
                    .persistant
                    .get(persistent_index::RANK as usize)
                    & !RANK_TIED_FLAG,
            );
            *self.inner.podium3.borrow_mut() = Some(player);
        }
    }

    /// Abort the podium (`abortPodium`).
    pub fn abort_podium(&self) {
        let runtime = self.inner.host.match_runtime();
        if runtime.host.settings().game_type != game_type::SINGLE_PLAYER {
            return;
        }
        if let Some(first) = self.inner.podium1.borrow().clone() {
            first.borrow_mut().nextthink = runtime.host.state().borrow().time;
            let think = runtime
                .host
                .pool()
                .callbacks
                .resolve_think("q3.team-arena.arenas.celebrateStart.think");
            first.borrow_mut().think = Some(think);
        }
    }

    /// Register save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(&self) {
        let pool = self.inner.host.match_runtime().host.pool();
        let inner = self.inner.clone();
        pool.callbacks.intern_think(
            "q3.team-arena.arenas.celebrateStart.think",
            Rc::new(move |entity: &EntityRef| {
                Self { inner: inner.clone() }.celebrate_stop(entity);
            }),
        );
        let inner = self.inner.clone();
        pool.callbacks.intern_think(
            "q3.team-arena.arenas.spawnPodium.think",
            Rc::new(move |entity: &EntityRef| {
                Self { inner: inner.clone() }.podium_placement_think(entity);
            }),
        );
        let inner = self.inner.clone();
        pool.callbacks.intern_think(
            "q3.team-arena.arenas.spawnModelsOnVictoryPads.think",
            Rc::new(move |entity: &EntityRef| {
                Self { inner: inner.clone() }.celebrate_start(entity);
            }),
        );
    }
}
