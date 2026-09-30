//! Quake III presentation: prediction.
//!
//! Donor provenance: `src/content/q3/presentation/prediction.ts`.

use qa_core::math::{add3, length3, scale3, sub3, vec3, Bounds, Vec3};
use std::cell::RefCell;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Prediction (prediction.ts)
// ---------------------------------------------------------------------------

/// Solid brush-model encoding.
pub const SOLID_BMODEL: i32 = 0x00ff_ffff;

pub(crate) const CONTENTS_BODY: i32 = 0x0200_0000;

pub(crate) const MASK_PLAYERSOLID: i32 = 1 | 0x0001_0000 | CONTENTS_BODY;

/// User-command source (`CommandSource`).
pub trait CommandSource {
    /// Current command number.
    fn current_number(&self) -> i32;
    /// Read a command by number.
    fn read(&self, number: i32) -> PresentResult<Option<UserCommand>>;
}

/// `CL_GetUserCmd` command ring (`ClientCommandHistory`).
#[derive(Debug, Clone)]
pub struct ClientCommandHistory {
    commands: [UserCommand; 64],
    number: i32,
}

impl ClientCommandHistory {
    /// Empty history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: [UserCommand::default(); 64],
            number: 0,
        }
    }

    /// Append a command, returning its number.
    pub fn append(&mut self, command: &UserCommand) -> i32 {
        self.number = self.number.wrapping_add(1);
        self.commands[(self.number & 63) as usize] = *command;
        self.number
    }
}

impl Default for ClientCommandHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandSource for ClientCommandHistory {
    fn current_number(&self) -> i32 {
        self.number
    }

    fn read(&self, number: i32) -> PresentResult<Option<UserCommand>> {
        if number > self.number {
            return Err(state_msg(format!("CL_GetUserCmd: {number} >= {}", self.number)));
        }
        if number <= self.number.wrapping_sub(64) {
            return Ok(None);
        }
        Ok(Some(self.commands[(number & 63) as usize]))
    }
}

/// Prediction settings (`PredictionSettings`).
#[derive(Debug, Clone, Copy)]
pub struct PredictionSettings {
    /// Game type.
    pub game_type: GameType,
    /// DM flags.
    pub dm_flags: i32,
    /// Demo playback.
    pub demo_playback: bool,
    /// Prediction disabled.
    pub no_predict: bool,
    /// Synchronous clients.
    pub synchronous_clients: bool,
    /// Predict items.
    pub predict_items: bool,
    /// Fixed pmove.
    pub pmove_fixed: bool,
    /// Pmove milliseconds.
    pub pmove_msec: i32,
    /// Error-decay integer.
    pub error_decay_integer: i32,
    /// Error-decay value.
    pub error_decay_value: f32,
    /// Show miss level.
    pub show_miss: i32,
}

/// Prediction services (`PredictionHost`).
pub trait PredictionHost {
    /// Command timing.
    fn command_timing(&self) -> CommandTiming;
    /// Move the player.
    fn move_player(&mut self, ps: &mut PlayerState, command: &UserCommand, options: &mut PmoveOptions) -> PmoveResult;
    /// Update view angles.
    fn update_view_angles(&mut self, ps: &mut PlayerState, command: &UserCommand);
    /// Current command number.
    fn current_command_number(&mut self) -> i32;
    /// Read a command by number.
    fn read_command(&mut self, number: i32) -> PresentResult<Option<UserCommand>>;
    /// Current settings.
    fn settings(&self) -> PredictionSettings;
    /// Set pmove milliseconds.
    fn set_pmove_msec(&mut self, value: i32);
    /// Transition player state.
    fn transition_player_state(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        current: &PlayerState,
        previous: &mut PlayerState,
    ) -> PresentResult<()>;
    /// Predictable-event debug sink.
    fn event_debug(&self) -> Option<PredictableEventDebug>;
    /// Recipe pre-hook for predicted item touch; `false` skips the source.
    fn pre_predict_item(&mut self, state: &mut ClientGameState, entity_number: usize) -> bool;
    /// Recipe post-hook for predicted item touch.
    fn post_predict_item(&mut self, state: &mut ClientGameState, entity_number: usize);
    /// Warning print.
    fn warn(&mut self, message: &str);
}

pub(crate) fn prediction_inventory(ps: &PlayerState) -> PresentResult<PlayerInventory<'_>> {
    let schema = stat_schema(ps.product);
    let team = ps.persistant.get(PersistentIndex::Team as i32)?;
    let persistent = match schema.persistent_powerup {
        Some(slot) => ps.stats.get(slot)?,
        None => 0,
    };
    if ps.product == Q3Product::MissionPack && schema.persistent_powerup.is_none() {
        return Err(state_msg("Missionpack prediction requires its stat schema"));
    }
    Ok(PlayerInventory::new(
        ps.product,
        ps.health()?,
        ps.stats.get(schema.armor)?,
        ps.stats.get(schema.max_health)?,
        ps.stats.get(schema.holdable_item)?,
        team,
        persistent,
        &ps.ammo,
        &ps.powerups,
    ))
}

pub(crate) fn interpolate_vector(a: Vec3, b: Vec3, fraction: f32) -> Vec3 {
    vec3(
        a.x + fraction * (b.x - a.x),
        a.y + fraction * (b.y - a.y),
        a.z + fraction * (b.z - a.z),
    )
}

pub(crate) fn lerp_angle(from: f32, to: f32, fraction: f32) -> f32 {
    let mut to = to;
    if to - from > 180.0 {
        to -= 360.0;
    }
    if to - from < -180.0 {
        to += 360.0;
    }
    from + fraction * (to - from)
}

/// Client prediction (`PredictionRuntime`).
pub struct PredictionRuntime<C, H> {
    /// Collision world.
    pub collision: C,
    /// Host services.
    pub host: H,
    command: UserCommand,
}

impl<C: CollisionWorld, H: PredictionHost> PredictionRuntime<C, H> {
    /// New runtime.
    #[must_use]
    pub fn new(collision: C, host: H) -> Self {
        Self {
            collision,
            host,
            command: UserCommand::default(),
        }
    }

    /// Trace against the world and solid entities.
    pub fn trace(
        &mut self,
        state: &ClientGameState,
        start: Vec3,
        end: Vec3,
        bounds: Bounds,
        skip_number: i32,
        mask: i32,
    ) -> PresentResult<MovementTrace> {
        let query = TraceQuery {
            start,
            end,
            shape: TraceShape::Box {
                mins: bounds.min,
                maxs: bounds.max,
            },
            mask,
            model_index: None,
        };
        let world_trace = self.collision.trace(&query);
        let mut result = MovementTrace::from_world(&world_trace);
        for entity_number in state.solid_entities.clone() {
            let number = i32::try_from(entity_number).map_err(|_| range_msg("Entity number outside int32"))?;
            let cent = state.entity_at(number)?;
            let entity = &cent.current_state;
            if entity.number == skip_number {
                continue;
            }
            let trace = if entity.solid == SOLID_BMODEL {
                let mut query = query;
                query.model_index = Some(entity.modelindex);
                self.collision.transformed_trace(
                    &query,
                    entity.modelindex,
                    evaluate_trajectory(&entity.pos, state.physics_time)?,
                    cent.lerp_angles,
                )
            } else {
                let x = (entity.solid & 255) as f32;
                let zd = ((entity.solid >> 8) & 255) as f32;
                let zu = (((entity.solid >> 16) & 255) - 32) as f32;
                self.collision
                    .box_trace(vec3(-x, -x, -zd), vec3(x, x, zu), &query, cent.lerp_origin)
            };
            if trace.solidity == TraceSolidity::AllSolid || trace.fraction < result.fraction {
                result = MovementTrace {
                    fraction: trace.fraction,
                    end: trace.end,
                    solidity: trace.solidity,
                    contact: trace.contact,
                    contents: trace.contents,
                    surface_flags: trace.surface_flags,
                    entity_num: entity.number,
                };
            } else if trace.solidity != TraceSolidity::Clear && result.solidity != TraceSolidity::AllSolid {
                result.solidity = TraceSolidity::StartSolid;
            }
            if result.solidity == TraceSolidity::AllSolid {
                return Ok(result);
            }
        }
        Ok(result)
    }

    /// Point contents with solid brush models.
    pub fn point_contents(&mut self, state: &ClientGameState, point: Vec3, pass_entity: i32) -> i32 {
        let mut contents = self.collision.point_contents(point);
        for entity_number in state.solid_entities.clone() {
            let Ok(number) = i32::try_from(entity_number) else {
                continue;
            };
            let Ok(cent) = state.entity_at(number) else { continue };
            let entity = &cent.current_state;
            if entity.number == pass_entity || entity.solid != SOLID_BMODEL || entity.modelindex == 0 {
                continue;
            }
            contents |=
                self.collision
                    .transformed_point_contents(point, entity.modelindex, entity.origin, entity.angles);
        }
        contents
    }

    /// Interpolate the player state between snapshots.
    pub fn interpolate_player_state(&mut self, state: &mut ClientGameState, grab_angles: bool) -> PresentResult<()> {
        let previous = state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("CG_InterpolatePlayerState requires cg.snap"))?;
        let mut out = previous.player_state.clone();
        if grab_angles {
            let current = self.host.current_command_number();
            let command = self.required_command(current)?;
            self.host.update_view_angles(&mut out, &command);
        }
        state.predicted_player_state = out;
        let next = state.next_snap.clone();
        let Some(next) = next else { return Ok(()) };
        if state.next_frame_teleport || next.server_time <= state.snap.as_ref().expect("snap").server_time {
            return Ok(());
        }
        let previous = state.snap.as_ref().expect("snap").clone();
        let fraction = state.time.wrapping_sub(previous.server_time) as f32
            / next.server_time.wrapping_sub(previous.server_time) as f32;
        let a = &previous.player_state;
        let b = &next.player_state;
        let cycle = if b.bob_cycle < a.bob_cycle {
            b.bob_cycle.wrapping_add(256)
        } else {
            b.bob_cycle
        };
        let ps = &mut state.predicted_player_state;
        ps.bob_cycle = qvm_float_to_int(a.bob_cycle as f32 + fraction * cycle.wrapping_sub(a.bob_cycle) as f32);
        ps.origin = interpolate_vector(a.origin, b.origin, fraction);
        ps.velocity = interpolate_vector(a.velocity, b.velocity, fraction);
        if !grab_angles {
            ps.viewangles = vec3(
                lerp_angle(a.viewangles.x, b.viewangles.x, fraction),
                lerp_angle(a.viewangles.y, b.viewangles.y, fraction),
                lerp_angle(a.viewangles.z, b.viewangles.z, fraction),
            );
        }
        Ok(())
    }

    /// Predicted item touch with the recipe hook.
    pub fn touch_item(
        &mut self,
        state: &mut ClientGameState,
        entity_number: usize,
        settings: &PredictionSettings,
    ) -> PresentResult<()> {
        if self.host.pre_predict_item(state, entity_number) {
            Self::source_touch_item(state, entity_number, settings)?;
            self.host.post_predict_item(state, entity_number);
        }
        Ok(())
    }

    fn source_touch_item(
        state: &mut ClientGameState,
        entity_number: usize,
        settings: &PredictionSettings,
    ) -> PresentResult<()> {
        let number = i32::try_from(entity_number).map_err(|_| range_msg("Entity number outside int32"))?;
        let time = state.time;
        let product = state.product;
        let entity = state.entity_at_mut(number)?.current_state.clone();
        let origin = state.predicted_player_state.origin;
        if !settings.predict_items
            || !player_touches_item(origin, &entity.pos, time)?
            || state.entity_at(number)?.misc_time == time
        {
            return Ok(());
        }
        let ps = &mut state.predicted_player_state;
        let ent = PickupEntity {
            model_index: entity.modelindex,
            model_index2: entity.modelindex2,
            generic1: entity.generic1,
        };
        let inventory = prediction_inventory(ps)?;
        if !can_item_be_grabbed(settings.game_type as i32, &ent, &inventory)? {
            return Ok(());
        }
        let item = *item_at(product, entity.modelindex)?;
        if product == Q3Product::MissionPack
            && settings.game_type == GameType::OneFctf
            && item.tag != Powerup::NeutralFlag as i32
        {
            return Ok(());
        }
        if settings.game_type == GameType::Ctf
            || (product == Q3Product::MissionPack && settings.game_type == GameType::Harvester)
        {
            let team = ps.persistant.get(PersistentIndex::Team as i32)?;
            if (team == Team::Red as i32 && item.tag == Powerup::RedFlag as i32)
                || (team == Team::Blue as i32 && item.tag == Powerup::BlueFlag as i32)
            {
                return Ok(());
            }
        }
        ps.add_event(EntityEvent::ItemPickup as i32, entity.modelindex)?;
        let entity = state.entity_at_mut(number)?;
        entity.current_state.e_flags |= 0x80;
        entity.misc_time = time;
        if item.item_type == ItemType::Weapon {
            let slot = stat_schema(product).weapons;
            let weapons = state.predicted_player_state.stats.get(slot)?;
            state
                .predicted_player_state
                .stats
                .set(slot, weapons | (1 << item.tag))?;
            if state.predicted_player_state.ammo.get(item.tag)? == 0 {
                state.predicted_player_state.ammo.set(item.tag, 1)?;
            }
        }
        Ok(())
    }

    /// Trigger prediction for items, teleporters, and jump pads.
    pub fn touch_trigger_prediction(
        &mut self,
        state: &mut ClientGameState,
        bounds: &Bounds,
        settings: &PredictionSettings,
    ) -> PresentResult<()> {
        if state.predicted_player_state.health()? <= 0 {
            return Ok(());
        }
        let spectator = state.predicted_player_state.pm_type == MoveType::Spectator as i32;
        if state.predicted_player_state.pm_type != MoveType::Normal as i32 && !spectator {
            return Ok(());
        }
        for entity_number in state.trigger_entities.clone() {
            let number = i32::try_from(entity_number).map_err(|_| range_msg("Entity number outside int32"))?;
            let entity = state.entity_at(number)?.current_state.clone();
            if entity.e_type == EntityType::Item as i32 && !spectator {
                self.touch_item(state, entity_number, settings)?;
                continue;
            }
            if entity.solid != SOLID_BMODEL || entity.modelindex == 0 {
                continue;
            }
            let origin = state.predicted_player_state.origin;
            let trace = self.collision.trace(&TraceQuery {
                start: origin,
                end: origin,
                shape: TraceShape::Box {
                    mins: bounds.min,
                    maxs: bounds.max,
                },
                mask: -1,
                model_index: Some(entity.modelindex),
            });
            if trace.solidity == TraceSolidity::Clear {
                continue;
            }
            if entity.e_type == EntityType::TeleportTrigger as i32 {
                state.hyperspace = true;
            } else if entity.e_type == EntityType::PushTrigger as i32 {
                let mut ps = state.take_predicted_player_state();
                touch_jump_pad(&mut ps, &entity)?;
                state.predicted_player_state = ps;
            }
        }
        let ps = &mut state.predicted_player_state;
        if ps.jumppad_frame != ps.pmove_framecount {
            ps.jumppad_frame = 0;
            ps.jumppad_ent = 0;
        }
        Ok(())
    }

    fn required_command(&mut self, number: i32) -> PresentResult<UserCommand> {
        self.host
            .read_command(number)?
            .ok_or_else(|| state_msg("Prediction command source violated its CMD_BACKUP window"))
    }

    /// Predict the player state.
    pub fn predict_player_state(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        let settings = self.host.settings();
        if state.snap.is_none() {
            return Err(state_msg("CG_PredictPlayerState requires cg.snap"));
        }
        state.hyperspace = false;
        if !state.valid_pps {
            state.valid_pps = true;
            let snap_ps = state.snap.as_ref().expect("snap").player_state.clone();
            state.predicted_player_state = snap_ps;
        }
        let snapshot_ps = state.snap.as_ref().expect("snap").player_state.clone();
        if settings.demo_playback || (snapshot_ps.pm_flags & MoveFlags::FOLLOW) != 0 {
            self.interpolate_player_state(state, false)?;
            return Ok(());
        }
        if settings.no_predict || settings.synchronous_clients {
            self.interpolate_player_state(state, true)?;
            return Ok(());
        }
        let mut mask = MASK_PLAYERSOLID;
        if state.predicted_player_state.pm_type == MoveType::Dead as i32
            || snapshot_ps.persistant.get(PersistentIndex::Team as i32)? == Team::Spectator as i32
        {
            mask &= !CONTENTS_BODY;
        }
        let old = state.predicted_player_state.clone();
        let current = self.host.current_command_number();
        let oldest = self.required_command(current.wrapping_sub(63))?;
        if oldest.server_time > snapshot_ps.command_time && oldest.server_time < state.time {
            if settings.show_miss != 0 {
                self.host.warn("exceeded PACKET_BACKUP on commands\n");
            }
            return Ok(());
        }
        let latest = self.required_command(current)?;
        let use_next = state.next_snap.is_some() && !state.next_frame_teleport && !state.this_frame_teleport;
        let selected = if use_next {
            state.next_snap.as_ref().expect("next snapshot").clone()
        } else {
            state.snap.as_ref().expect("snap").clone()
        };
        let mut ps = selected.player_state.clone();
        ps.set_event_debug(self.host.event_debug());
        state.physics_time = selected.server_time;
        if settings.pmove_msec < 8 {
            self.host.set_pmove_msec(8);
        } else if settings.pmove_msec > 33 {
            self.host.set_pmove_msec(33);
        }
        let settings = self.host.settings();
        let mut moved = false;
        let mut number = current.wrapping_sub(63);
        loop {
            if number > current {
                break;
            }
            if let Some(command) = self.host.read_command(number)? {
                self.command = command;
            }
            if settings.pmove_fixed {
                self.host.update_view_angles(&mut ps, &self.command);
            }
            if self.command.server_time > ps.command_time && self.command.server_time <= latest.server_time {
                if ps.command_time == old.command_time {
                    if state.this_frame_teleport {
                        state.predicted_error = vec3(0.0, 0.0, 0.0);
                        if settings.show_miss != 0 {
                            self.host.warn("PredictionTeleport\n");
                        }
                        state.this_frame_teleport = false;
                    } else {
                        let physics_time = state.physics_time;
                        let old_time = state.old_time;
                        let adjusted =
                            adjust_position_for_mover(state, ps.origin, ps.ground_entity_num, physics_time, old_time)?;
                        let delta = sub3(old.origin, adjusted);
                        let length = length3(delta);
                        if settings.show_miss != 0
                            && (old.origin.x != adjusted.x || old.origin.y != adjusted.y || old.origin.z != adjusted.z)
                        {
                            self.host.warn("prediction error\n");
                        }
                        if length > 0.1 {
                            if settings.show_miss != 0 {
                                self.host.warn(&format!("Prediction miss: {length:.6}\n"));
                            }
                            if settings.error_decay_integer != 0 {
                                let elapsed = state.time.wrapping_sub(state.predicted_error_time);
                                let mut fraction =
                                    (settings.error_decay_value - elapsed as f32) / settings.error_decay_value;
                                if fraction < 0.0 {
                                    fraction = 0.0;
                                }
                                if fraction > 0.0 && settings.show_miss != 0 {
                                    self.host.warn(&format!("Double prediction decay: {fraction:.6}\n"));
                                }
                                state.predicted_error = scale3(state.predicted_error, fraction);
                            } else {
                                state.predicted_error = vec3(0.0, 0.0, 0.0);
                            }
                            state.predicted_error = add3(delta, state.predicted_error);
                            state.predicted_error_time = state.old_time;
                        }
                    }
                }
                let original_server_time = self.command.server_time;
                if settings.pmove_fixed && self.host.command_timing() == CommandTiming::Q3 {
                    let stepped = self
                        .command
                        .server_time
                        .wrapping_add(settings.pmove_msec)
                        .wrapping_sub(1);
                    self.command.server_time = ((stepped as f64 / settings.pmove_msec as f64).trunc() as i32)
                        .wrapping_mul(settings.pmove_msec);
                }
                let result = {
                    let runtime = &mut *self;
                    let state_ref = &*state;
                    let collision = RefCell::new(&mut runtime.collision);
                    let trace_fn = |start: Vec3, end: Vec3, bounds: Bounds, skip: i32, mask: i32| {
                        Self::trace_with(state_ref, *collision.borrow_mut(), start, end, bounds, skip, mask)
                    };
                    let contents_fn =
                        |point: Vec3, pass: i32| Self::contents_with(state_ref, *collision.borrow_mut(), point, pass);
                    let mut options = PmoveOptions {
                        trace: Box::new(trace_fn),
                        point_contents: Box::new(contents_fn),
                        original_server_time,
                        trace_mask: mask,
                        fixed_msec: settings.pmove_fixed.then_some(settings.pmove_msec),
                        no_footsteps: (settings.dm_flags & 32) != 0,
                        gauntlet_hit: false,
                    };
                    runtime.host.move_player(&mut ps, &runtime.command, &mut options)
                };
                moved = true;
                state.predicted_player_state = ps;
                self.touch_trigger_prediction(state, &result.bounds, &settings)?;
                ps = state.take_predicted_player_state();
            }
            number = number.wrapping_add(1);
        }
        if settings.show_miss > 1 {
            self.host
                .warn(&format!("[{} : {}] ", self.command.server_time, state.time));
        }
        if !moved {
            state.predicted_player_state = ps;
            let mut old = old;
            let current_ps = state.predicted_player_state.clone();
            self.host
                .transition_player_state(state, static_state, &current_ps, &mut old)?;
            if settings.show_miss != 0 {
                self.host.warn("not moved\n");
            }
            return Ok(());
        }
        let physics_time = state.physics_time;
        let time = state.time;
        ps.origin = adjust_position_for_mover(state, ps.origin, ps.ground_entity_num, physics_time, time)?;
        if settings.show_miss != 0 && ps.event_sequence > old.event_sequence.wrapping_add(2) {
            self.host.warn("WARNING: dropped event\n");
        }
        state.predicted_player_state = ps;
        let mut old = old;
        let current_ps = state.predicted_player_state.clone();
        self.host
            .transition_player_state(state, static_state, &current_ps, &mut old)?;
        if settings.show_miss != 0 && state.event_sequence > state.predicted_player_state.event_sequence {
            self.host.warn("WARNING: double event\n");
            state.event_sequence = state.predicted_player_state.event_sequence;
        }
        Ok(())
    }

    fn trace_with(
        state: &ClientGameState,
        collision: &mut C,
        start: Vec3,
        end: Vec3,
        bounds: Bounds,
        skip_number: i32,
        mask: i32,
    ) -> MovementTrace {
        let query = TraceQuery {
            start,
            end,
            shape: TraceShape::Box {
                mins: bounds.min,
                maxs: bounds.max,
            },
            mask,
            model_index: None,
        };
        let world_trace = collision.trace(&query);
        let mut result = MovementTrace::from_world(&world_trace);
        for entity_number in state.solid_entities.iter().copied() {
            let Ok(number) = i32::try_from(entity_number) else {
                continue;
            };
            let Ok(cent) = state.entity_at(number) else { continue };
            let entity = &cent.current_state;
            if entity.number == skip_number {
                continue;
            }
            let Ok(evaluated) = evaluate_trajectory(&entity.pos, state.physics_time) else {
                continue;
            };
            let trace = if entity.solid == SOLID_BMODEL {
                let mut query = query;
                query.model_index = Some(entity.modelindex);
                collision.transformed_trace(&query, entity.modelindex, evaluated, cent.lerp_angles)
            } else {
                let x = (entity.solid & 255) as f32;
                let zd = ((entity.solid >> 8) & 255) as f32;
                let zu = (((entity.solid >> 16) & 255) - 32) as f32;
                collision.box_trace(vec3(-x, -x, -zd), vec3(x, x, zu), &query, cent.lerp_origin)
            };
            if trace.solidity == TraceSolidity::AllSolid || trace.fraction < result.fraction {
                result = MovementTrace {
                    fraction: trace.fraction,
                    end: trace.end,
                    solidity: trace.solidity,
                    contact: trace.contact,
                    contents: trace.contents,
                    surface_flags: trace.surface_flags,
                    entity_num: entity.number,
                };
            } else if trace.solidity != TraceSolidity::Clear && result.solidity != TraceSolidity::AllSolid {
                result.solidity = TraceSolidity::StartSolid;
            }
            if result.solidity == TraceSolidity::AllSolid {
                return result;
            }
        }
        result
    }

    fn contents_with(state: &ClientGameState, collision: &mut C, point: Vec3, pass_entity: i32) -> i32 {
        let mut contents = collision.point_contents(point);
        for entity_number in state.solid_entities.iter().copied() {
            let Ok(number) = i32::try_from(entity_number) else {
                continue;
            };
            let Ok(cent) = state.entity_at(number) else { continue };
            let entity = &cent.current_state;
            if entity.number == pass_entity || entity.solid != SOLID_BMODEL || entity.modelindex == 0 {
                continue;
            }
            contents |= collision.transformed_point_contents(point, entity.modelindex, entity.origin, entity.angles);
        }
        contents
    }
}

/// Build the solid/trigger entity lists (`CG_BuildSolidList`).
pub fn build_solid_list(state: &mut ClientGameState) -> PresentResult<()> {
    state.solid_entities.clear();
    state.trigger_entities.clear();
    let use_next = state.next_snap.is_some() && !state.next_frame_teleport && !state.this_frame_teleport;
    let snapshot = if use_next {
        state.next_snap.as_ref()
    } else {
        state.snap.as_ref()
    };
    let Some(snapshot) = snapshot else {
        return Err(state_msg("CG_BuildSolidList requires a snapshot"));
    };
    let numbers: Vec<i32> = snapshot.entities.iter().map(|entry| entry.number).collect();
    for number in numbers {
        let entity = state.entity_at(number)?;
        let entity_type = entity.current_state.e_type;
        let next_solid = entity.next_state.solid;
        let index = usize::try_from(number).map_err(|_| range_msg("Entity number outside int32"))?;
        if entity_type == EntityType::Item as i32
            || entity_type == EntityType::PushTrigger as i32
            || entity_type == EntityType::TeleportTrigger as i32
        {
            state.trigger_entities.push(index);
        } else if next_solid != 0 {
            state.solid_entities.push(index);
        }
    }
    Ok(())
}

/// Adjust a position for its ground mover (`adjustPositionForMover`).
pub fn adjust_position_for_mover(
    state: &ClientGameState,
    input: Vec3,
    mover_num: i32,
    from_time: i32,
    to_time: i32,
) -> PresentResult<Vec3> {
    if mover_num <= 0 || mover_num >= ENTITYNUM_WORLD {
        return Ok(input);
    }
    let mover = state.entity_at(mover_num)?.current_state.clone();
    if mover.e_type != EntityType::Mover as i32 {
        return Ok(input);
    }
    let old_origin = evaluate_trajectory(&mover.pos, from_time)?;
    evaluate_trajectory(&mover.apos, from_time)?;
    let origin = evaluate_trajectory(&mover.pos, to_time)?;
    evaluate_trajectory(&mover.apos, to_time)?;
    Ok(add3(input, sub3(origin, old_origin)))
}
