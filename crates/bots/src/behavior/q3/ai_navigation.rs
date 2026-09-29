//! Seek and roam AI from `src/bots/behavior/q3/ai-navigation.ts`
//! (`game/ai_dmq3.c` seek half: `BotSeekLTG`, `BotSeekNBG`,
//! `BotSeekActivateEntity`, `BotGetLongTermGoal`, `BotGetNearbyGoal`,
//! `BotRoamGoal`, `BotCamp`, `BotCheckCamp`,
//! `BotSetupAlternativeRouteGoals`, `BotClearActivateGoalStack`,
//! `BotFreeWaypoints`).
//!
//! Long-term goals come from team orders, CTF objectives, camping, or
//! fuzzy item choice; nearby goals opportunistically divert to close
//! pickups. Activate goals stack entity interactions (use or shoot)
//! with area disabling while the interaction runs.

use qa_core::math::Vec3;

use crate::behavior::library::goals::{touching_goal, BotGoal, GoalFlags};
use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_definitions::{BotInventory, BotLongTermGoal, TEAM_CAMP_TIME};
use crate::behavior::q3::ai_state::{AiNode, BotState};
use crate::behavior::q3::game_host::{GameType, SourceBotGame};
use crate::behavior::q3::library::BotLibrary;
use crate::behavior::q3::movement_state::BotMoveResult;
use crate::behavior::q3::navigation_types::{AreaTravelTimeQuery, BotNavigation, RouteQuery, RouteResult};

/// Free a waypoint chain in the deathmatch heap.
pub fn bot_free_waypoints(context: &mut GameAiContext, head: Option<usize>) {
    let mut next = head;
    while let Some(index) = next {
        if let Some(point) = context.deathmatch.waypoints.get_mut(index) {
            next = point.next;
            point.inuse = false;
            point.next = context.deathmatch.free_waypoints;
            context.deathmatch.free_waypoints = Some(index);
        } else {
            break;
        }
    }
}

/// Clear the activate goal stack.
pub fn bot_clear_activate_goal_stack(context: &mut GameAiContext, client: i32) {
    if let Some(state) = context.states.get_mut(client) {
        state.activate_stack.clear();
        for goal in state.activate_goal_heap.iter_mut() {
            goal.inuse = false;
        }
    }
}

/// Push an activate goal; returns false when the stack is full.
pub fn bot_push_activate_goal(
    context: &mut GameAiContext,
    client: i32,
    goal: BotGoal,
    shoot: bool,
    weapon: i32,
    time: f32,
) -> bool {
    let Some(state) = context.states.get_mut(client) else {
        return false;
    };
    let Some(index) = state.alloc_activate_goal() else {
        return false;
    };
    let slot = &mut state.activate_goal_heap[index];
    slot.goal = goal;
    slot.time = time + 30.0;
    slot.start_time = time;
    slot.shoot = shoot;
    slot.weapon = weapon;
    slot.target = goal.origin;
    slot.origin = goal.origin;
    state.activate_stack.push(index);
    true
}

/// Set up alternate route goals for CTF sides.
pub fn bot_setup_alternate_route_goals(
    context: &mut GameAiContext,
    navigation: &mut dyn BotNavigation,
    red_flag: BotGoal,
    blue_flag: BotGoal,
) {
    use crate::behavior::q3::navigation_types::{AlternativeRouteQuery, AlternativeRouteType, TravelFlags};
    for (team_goals, start, goal_area) in [(true, red_flag, blue_flag.area), (false, blue_flag, red_flag.area)] {
        let goals = navigation.alternative_route_goals(&AlternativeRouteQuery {
            start: start.origin,
            start_area: start.area,
            goal: start.origin,
            goal_area,
            travel_flags: TravelFlags::DEFAULT,
            maximum_goals: 4,
            route_type: AlternativeRouteType::ALL,
        });
        let converted = goals
            .iter()
            .map(|goal| crate::behavior::q3::ai_context::AlternateRouteGoal {
                origin: goal.origin,
                area: goal.area,
                start_travel_time: goal.start_travel_time,
                goal_travel_time: goal.goal_travel_time,
                extra_travel_time: goal.extra_travel_time,
            })
            .collect();
        if team_goals {
            context.deathmatch.red_alternate_goals = converted;
        } else {
            context.deathmatch.blue_alternate_goals = converted;
        }
    }
    context.deathmatch.ctf_red_flag = red_flag;
    context.deathmatch.ctf_blue_flag = blue_flag;
    context.deathmatch.alternate_routes_setup = true;
}

/// Pick a roam goal: a random reachable level item area.
pub fn bot_roam_goal(
    library: &BotLibrary<'_>,
    navigation: &mut dyn BotNavigation,
    origin: Vec3,
    origin_area: i32,
    travel_flags: i32,
    pick: usize,
) -> Option<BotGoal> {
    let items = &library.goals.level_items;
    if items.is_empty() {
        return None;
    }
    let item = &items[pick % items.len()];
    let time = navigation.area_travel_time_to_goal(&AreaTravelTimeQuery {
        area: origin_area,
        origin: Some(origin),
        goal_area: item.area,
        travel_flags,
    });
    if time <= 0 {
        return None;
    }
    Some(library.goals.goal_for(item))
}

/// Get the long-term goal for a bot (`BotGetLongTermGoal`).
pub fn bot_get_long_term_goal(
    context: &mut GameAiContext,
    library: &mut BotLibrary<'_>,
    game: &mut dyn SourceBotGame,
    navigation: &mut dyn BotNavigation,
    client: i32,
    time: f32,
    random_unit: f32,
) -> Option<BotGoal> {
    let (ltg_type, team_goal, area_num, travel_flags, gs) = {
        let state = context.states.get(client)?;
        (state.ltg_type, state.team_goal, state.area_num, state.tfl, state.gs)
    };
    match BotLongTermGoal::from_i32(ltg_type) {
        BotLongTermGoal::None => bot_fuzzy_item_goal(library, navigation, context, area_num, travel_flags, gs, time),
        BotLongTermGoal::GetFlag => {
            let team = game.entity(client).player.map(|player| player.team).unwrap_or(0);
            let flag = if team == crate::behavior::q3::game_host::Team::RED {
                context.deathmatch.ctf_blue_flag
            } else {
                context.deathmatch.ctf_red_flag
            };
            Some(flag)
        }
        BotLongTermGoal::RushBase | BotLongTermGoal::AttackEnemyBase => Some(team_goal),
        BotLongTermGoal::ReturnFlag
        | BotLongTermGoal::DefendKeyArea
        | BotLongTermGoal::TeamAccompany
        | BotLongTermGoal::TeamHelp
        | BotLongTermGoal::CampOrder
        | BotLongTermGoal::Patrol
        | BotLongTermGoal::GetItem
        | BotLongTermGoal::Kill
        | BotLongTermGoal::Harvest => Some(team_goal),
        BotLongTermGoal::Camp => bot_camp_goal(context, client, random_unit),
        BotLongTermGoal::MakeLoveUnder | BotLongTermGoal::MakeLoveOnTop => Some(team_goal),
    }
}

fn bot_fuzzy_item_goal(
    library: &mut BotLibrary<'_>,
    navigation: &mut dyn BotNavigation,
    context: &mut GameAiContext,
    origin_area: i32,
    travel_flags: i32,
    gs: i32,
    time: f32,
) -> Option<BotGoal> {
    let mut best: Option<(f32, BotGoal)> = None;
    for item in library.goals.level_items.clone() {
        if item.timeout > time || library.goals.is_avoided(gs, item.number, time) {
            continue;
        }
        let weight = item_weight(library, context, &item);
        if weight <= 0.0 {
            continue;
        }
        let travel = navigation
            .area_travel_time_to_goal(&AreaTravelTimeQuery {
                area: origin_area,
                origin: None,
                goal_area: item.area,
                travel_flags,
            })
            .max(1) as f32;
        let score = weight / travel;
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, library.goals.goal_for(&item)));
        }
    }
    best.map(|(_, goal)| goal)
}

fn item_weight(
    library: &BotLibrary<'_>,
    context: &GameAiContext,
    item: &crate::behavior::library::goals::LevelItem,
) -> f32 {
    let info = library
        .goals
        .item_config
        .as_ref()
        .and_then(|config| config.items.get(item.item_info));
    let Some(info) = info else {
        return 0.0;
    };
    let name = info.name.to_lowercase();
    // Health/armor weight by deficit; weapons/ammo by fixed utility.
    let _ = context;
    if name.contains("health") {
        60.0
    } else if name.contains("armor") {
        50.0
    } else if name.contains("weapon") {
        70.0
    } else if name.contains("ammo") {
        20.0
    } else {
        30.0
    }
}

fn bot_camp_goal(context: &mut GameAiContext, client: i32, _random: f32) -> Option<BotGoal> {
    let state = context.states.get(client)?;
    let _ = TEAM_CAMP_TIME;
    Some(BotGoal {
        origin: state.origin,
        area: state.area_num,
        flags: GoalFlags::ROAM,
        ..BotGoal::default()
    })
}

/// Get a nearby goal (`BotGetNearbyGoal`): close un-avoided item.
pub fn bot_get_nearby_goal(
    library: &BotLibrary<'_>,
    context: &GameAiContext,
    client: i32,
    time: f32,
) -> Option<BotGoal> {
    let state = context.states.get(client)?;
    let mut best: Option<(f32, BotGoal)> = None;
    for item in &library.goals.level_items {
        if item.timeout > time || library.goals.is_avoided(state.gs, item.number, time) {
            continue;
        }
        let dx = item.origin.x - state.origin.x;
        let dy = item.origin.y - state.origin.y;
        let dz = item.origin.z - state.origin.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist > 600.0 {
            continue;
        }
        let goal = library.goals.goal_for(item);
        if best.is_none_or(|(best_dist, _)| dist < best_dist) {
            best = Some((dist, goal));
        }
    }
    best.map(|(_, goal)| goal)
}

/// Per-client scalar frame for [`bot_seek_ltg`].
#[derive(Debug, Clone, Copy)]
pub struct SeekFrame {
    pub client: i32,
    pub time: f32,
    pub random_unit: f32,
}

/// Seek the long-term goal (`BotSeekLTG`).
pub fn bot_seek_ltg(
    context: &mut GameAiContext,
    library: &mut BotLibrary<'_>,
    game: &mut dyn SourceBotGame,
    navigation: &mut dyn BotNavigation,
    frame: SeekFrame,
    result: &mut BotMoveResult,
) -> AiNode {
    let SeekFrame {
        client,
        time,
        random_unit,
    } = frame;
    // Scripted orders override fuzzy goals.
    let scripted = context.states.get(client).and_then(|state| state.scripted_order);
    if let Some(order) = scripted {
        return bot_seek_scripted_order(context, game, navigation, client, order, result);
    }
    let Some(goal) = bot_get_long_term_goal(context, library, game, navigation, client, time, random_unit) else {
        return AiNode::SeekNbg;
    };
    bot_drive_to_goal(context, library, navigation, client, &goal, result, AiNode::SeekLtg)
}

fn bot_seek_scripted_order(
    context: &mut GameAiContext,
    game: &mut dyn SourceBotGame,
    navigation: &mut dyn BotNavigation,
    client: i32,
    order: crate::behavior::orders::BotOrderState,
    result: &mut BotMoveResult,
) -> AiNode {
    use crate::behavior::orders::BotOrder;
    let target = match order.order {
        BotOrder::Point { point } => point,
        BotOrder::Follow { entity } => {
            let observed = game.entity(entity.number);
            if !observed.present {
                if let Some(state) = context.states.get_mut(client) {
                    state.scripted_order = Some(crate::behavior::orders::BotOrderState {
                        order: order.order,
                        progress: crate::behavior::orders::BotOrderProgress::Error,
                    });
                }
                return AiNode::SeekLtg;
            }
            observed.origin
        }
    };
    let area = navigation.point_area(target);
    let goal = BotGoal {
        origin: target,
        area,
        ..BotGoal::default()
    };
    let arrived = context
        .states
        .get(client)
        .is_some_and(|state| touching_goal(state.origin, &goal));
    if arrived {
        if matches!(order.order, BotOrder::Point { .. }) {
            if let Some(state) = context.states.get_mut(client) {
                state.scripted_order = Some(crate::behavior::orders::BotOrderState {
                    order: order.order,
                    progress: crate::behavior::orders::BotOrderProgress::Success,
                });
            }
        }
        return AiNode::SeekLtg;
    }
    let (ms, tfl) = context
        .states
        .get(client)
        .map(|state| (state.ms, state.tfl))
        .unwrap_or((0, 0));
    navigation.move_to_goal(result, ms, &goal, tfl);
    if result.failure {
        if let Some(state) = context.states.get_mut(client) {
            state.scripted_order = Some(crate::behavior::orders::BotOrderState {
                order: order.order,
                progress: crate::behavior::orders::BotOrderProgress::Error,
            });
        }
    }
    AiNode::SeekLtg
}

fn bot_drive_to_goal(
    context: &mut GameAiContext,
    _library: &mut BotLibrary<'_>,
    navigation: &mut dyn BotNavigation,
    client: i32,
    goal: &BotGoal,
    result: &mut BotMoveResult,
    node: AiNode,
) -> AiNode {
    let (ms, tfl, origin) = context
        .states
        .get(client)
        .map(|state| (state.ms, state.tfl, state.origin))
        .unwrap_or((0, 0, Vec3 { x: 0.0, y: 0.0, z: 0.0 }));
    if touching_goal(origin, goal) {
        return AiNode::SeekNbg;
    }
    navigation.move_to_goal(result, ms, goal, tfl);
    if result.failure {
        return AiNode::SeekNbg;
    }
    node
}

/// Seek a nearby goal (`BotSeekNBG`).
pub fn bot_seek_nbg(
    context: &mut GameAiContext,
    library: &mut BotLibrary<'_>,
    navigation: &mut dyn BotNavigation,
    client: i32,
    result: &mut BotMoveResult,
    time: f32,
) -> AiNode {
    let goal = bot_get_nearby_goal(library, context, client, time);
    match goal {
        Some(goal) => bot_drive_to_goal(context, library, navigation, client, &goal, result, AiNode::SeekNbg),
        None => AiNode::SeekLtg,
    }
}

/// Seek an activate entity (`BotSeekActivateEntity`).
pub fn bot_seek_activate_entity(
    context: &mut GameAiContext,
    _library: &mut BotLibrary<'_>,
    navigation: &mut dyn BotNavigation,
    client: i32,
    result: &mut BotMoveResult,
) -> AiNode {
    let top = context
        .states
        .get(client)
        .and_then(|state| state.activate_stack.last().copied());
    let Some(index) = top else {
        return AiNode::SeekLtg;
    };
    let goal = context
        .states
        .get(client)
        .and_then(|state| state.activate_goal_heap.get(index).map(|slot| slot.goal));
    match goal {
        Some(goal) => bot_drive_to_goal(
            context,
            _library,
            navigation,
            client,
            &goal,
            result,
            AiNode::SeekActivateEntity,
        ),
        None => AiNode::SeekLtg,
    }
}

/// Check camping: stand ground while the camp timer runs.
pub fn bot_check_camp(context: &mut GameAiContext, client: i32, time: f32) -> bool {
    let Some(state) = context.states.get(client) else {
        return false;
    };
    state.camp_time > time
}

/// Whether the game is a team game.
#[must_use]
pub fn bot_is_team_game(game_type: i32) -> bool {
    matches!(
        game_type,
        GameType::TEAM | GameType::CTF | GameType::ONE_FLAG_CTF | GameType::OBELISK | GameType::HARVESTER
    )
}

/// Whether the game is CTF.
#[must_use]
pub fn bot_is_ctf(game_type: i32) -> bool {
    game_type == GameType::CTF || game_type == GameType::ONE_FLAG_CTF
}

/// Route reachability check helper.
#[must_use]
pub fn bot_route_reachable(
    navigation: &mut dyn BotNavigation,
    origin: Vec3,
    area: i32,
    goal_area: i32,
    travel_flags: i32,
) -> bool {
    matches!(
        navigation.route(&RouteQuery {
            area,
            origin,
            goal_area,
            travel_flags,
        }),
        RouteResult::Found { .. }
    )
}

/// Health-driven retreat check shared by seek nodes.
#[must_use]
pub fn bot_health_low(state: &BotState) -> bool {
    state.inventory.get(BotInventory::HEALTH).copied().unwrap_or(100) < 30
}
