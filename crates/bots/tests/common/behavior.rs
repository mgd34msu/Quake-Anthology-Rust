//! Shared synthetic fixtures for behavior integration tests.
#![allow(dead_code)]

use std::collections::HashMap;

use qa_bots::behavior::library::goals::BotGoal;
use qa_bots::behavior::q3::ai_state::BotUserCommand;
use qa_bots::behavior::q3::movement_state::{BotMoveResult, BotMoveStateStore};
use qa_bots::behavior::q3::navigation_types::{
    AlternativeGoal, AlternativeRouteQuery, AreaTravelTimeQuery, BotNavigation, BotNavigationArea, PredictRouteQuery,
    PredictedRoute, RouteQuery, RouteResult, RouteStopEvent,
};
use qa_core::identity::{ActorId, IdentityOwner};
use qa_core::math::{Bounds, Vec3};
use qa_world::client::ClientCommand;

/// Scripted navigation: fixed areas, instant routes, steering movement.
pub struct FakeNav {
    pub areas: HashMap<(i32, i32, i32), i32>,
    pub default_area: i32,
    pub positions: HashMap<i32, Vec3>,
    pub move_states: BotMoveStateStore,
    pub travel_time: i32,
    pub fail_moves: bool,
}

impl FakeNav {
    pub fn new() -> Self {
        Self {
            areas: HashMap::new(),
            default_area: 1,
            positions: HashMap::new(),
            move_states: BotMoveStateStore::new(),
            travel_time: 100,
            fail_moves: false,
        }
    }

    pub fn area_for(&self, origin: Vec3) -> i32 {
        let key = (origin.x as i32 / 64, origin.y as i32 / 64, origin.z as i32 / 64);
        self.areas.get(&key).copied().unwrap_or(self.default_area)
    }

    /// Teleport a move state (test driver for arrivals).
    pub fn teleport(&mut self, move_state: i32, origin: Vec3) {
        if let Some(state) = self.move_states.get_mut(move_state) {
            state.origin = origin;
        }
    }
}

impl Default for FakeNav {
    fn default() -> Self {
        Self::new()
    }
}

impl BotNavigation for FakeNav {
    fn ready(&self) -> bool {
        true
    }

    fn point_area(&self, origin: Vec3) -> i32 {
        self.area_for(origin)
    }

    fn reachability_area(&self, origin: Vec3, _client: i32) -> i32 {
        self.area_for(origin)
    }

    fn fuzzy_point_reachability_area(&self, origin: Vec3) -> i32 {
        self.area_for(origin)
    }

    fn area(&self, number: i32) -> BotNavigationArea {
        BotNavigationArea {
            contents: 0,
            flags: 0,
            presence_type: 0,
            cluster: 0,
            reachable_area_count: if number > 0 { 1 } else { 0 },
        }
    }

    fn trace_areas(&self, start: Vec3, end: Vec3, _maximum: usize) -> Vec<(i32, Vec3)> {
        vec![(self.area_for(start), start), (self.area_for(end), end)]
    }

    fn bbox_areas(&self, _bounds: &Bounds) -> Vec<i32> {
        vec![self.default_area]
    }

    fn set_area_enabled(&mut self, _area: i32, _enabled: bool) {}

    fn area_travel_time_to_goal(&mut self, query: &AreaTravelTimeQuery) -> i32 {
        if query.area == query.goal_area {
            1
        } else {
            self.travel_time
        }
    }

    fn route(&mut self, query: &RouteQuery) -> RouteResult {
        if query.area <= 0 || query.goal_area <= 0 {
            return RouteResult::Unreachable;
        }
        RouteResult::Found {
            travel_time: self.travel_time,
            next_reachability: 7,
        }
    }

    fn predict_route(&mut self, query: &PredictRouteQuery) -> PredictedRoute {
        PredictedRoute {
            succeeded: true,
            stop_event: RouteStopEvent::NONE,
            end_area: query.goal_area,
            end_contents: 0,
            end_travel_flags: query.travel_flags,
            end_position: query.origin,
            time: self.travel_time.min(query.maximum_time),
        }
    }

    fn alternative_route_goals(&mut self, query: &AlternativeRouteQuery) -> Vec<AlternativeGoal> {
        if query.maximum_goals <= 0 {
            return Vec::new();
        }
        vec![AlternativeGoal {
            origin: query.start,
            area: query.start_area,
            start_travel_time: 0,
            goal_travel_time: self.travel_time,
            extra_travel_time: 10,
        }]
    }

    fn move_to_goal(&mut self, result: &mut BotMoveResult, move_state: i32, goal: &BotGoal, _travel_flags: i32) {
        *result = BotMoveResult::default();
        if self.fail_moves {
            result.failure = true;
            return;
        }
        let origin = self
            .move_states
            .get(move_state)
            .map(|state| state.origin)
            .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let dir = Vec3 {
            x: goal.origin.x - origin.x,
            y: goal.origin.y - origin.y,
            z: 0.0,
        };
        let len = (dir.x * dir.x + dir.y * dir.y).sqrt();
        if len < 1.0 {
            result.move_direction = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        } else {
            result.move_direction = Vec3 {
                x: dir.x / len,
                y: dir.y / len,
                z: 0.0,
            };
            // Advance the move state toward the goal at 320 u/s per 100ms think.
            let step = 32.0f32.min(len);
            if let Some(state) = self.move_states.get_mut(move_state) {
                state.origin.x += dir.x / len * step;
                state.origin.y += dir.y / len * step;
            }
        }
        result.travel_type = qa_bots::behavior::TravelType::WALK;
    }

    fn move_in_direction(&mut self, _move_state: i32, _direction: Vec3, _speed: f32, _move_type: i32) -> bool {
        true
    }

    fn movement_view_target(
        &self,
        _move_state: i32,
        goal: &BotGoal,
        _travel_flags: i32,
        _look_ahead: f32,
    ) -> Option<Vec3> {
        Some(goal.origin)
    }

    fn predict_visible_position(&self, _origin: Vec3, _area: i32, goal: &BotGoal, _travel_flags: i32) -> Option<Vec3> {
        Some(goal.origin)
    }

    fn swimming(&self, _origin: Vec3) -> bool {
        false
    }

    fn presence_bounds(&self, _presence: i32) -> Bounds {
        Bounds {
            min: Vec3 {
                x: -15.0,
                y: -15.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 15.0,
                y: 15.0,
                z: 32.0,
            },
        }
    }
}

/// Scripted director host: admitted actors plus command encoding.
pub struct FakeHost {
    pub owner: IdentityOwner,
    pub bindings: HashMap<i32, (ActorId, u32)>,
    pub next_client: i32,
    pub time_ms: i32,
    pub printed: Vec<String>,
}

impl FakeHost {
    pub fn new() -> Self {
        Self {
            owner: IdentityOwner::create("behavior-test").unwrap(),
            bindings: HashMap::new(),
            next_client: 0,
            time_ms: 0,
            printed: Vec::new(),
        }
    }
}

impl Default for FakeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl qa_bots::behavior::SourceBotDirectorHost for FakeHost {
    fn allocate_client(&mut self) -> Option<i32> {
        let client = self.next_client;
        self.next_client += 1;
        let actor = self.owner.actor(client as u32, 1);
        self.bindings.insert(client, (actor, client as u32));
        Some(client)
    }

    fn actor(&self, client: i32) -> Option<(ActorId, u32)> {
        self.bindings.get(&client).cloned()
    }

    fn encode_command(&self, _client: i32, command: &BotUserCommand) -> ClientCommand {
        ClientCommand {
            family: qa_world::client::ClientFamily::Q3,
            buttons: command.buttons,
            impulse: command.weapon,
            forward_move: f64::from(command.forwardmove),
            side_move: 0.0,
            right_move: f64::from(command.rightmove),
            up_move: f64::from(command.upmove),
        }
    }

    fn snapshot_entity(&self, _client: i32, sequence: i32) -> i32 {
        sequence
    }

    fn console_message(&self, _client: i32) -> Option<String> {
        None
    }

    fn point_contents(&self, _point: Vec3) -> i32 {
        0
    }

    fn print(&mut self, text: &str) {
        self.printed.push(text.to_owned());
    }

    fn time_ms(&self) -> i32 {
        self.time_ms
    }

    fn name_in_use(&self, _name: &str) -> bool {
        false
    }
}

/// Minimal item config for goal tests.
pub const TEST_ITEM_CONFIG: &str = r#"
iteminfo {
    number 1
    classname weapon_rocketlauncher
    name rocket
    model models/rocket.md3
    type 2
    index 8
    respawntime 30
}
iteminfo {
    number 2
    classname item_health_large
    name health
    model models/health.md3
    type 1
    index 29
    respawntime 35
}
"#;

/// Minimal bots.txt for catalog tests.
pub const TEST_BOTS_TXT: &str = r#"
{
    name Grunt
    funname grunt
    team red
}

{
    name Major
    funname major
    team blue
}
"#;

/// Minimal character file for AI setup tests.
pub const TEST_CHARACTER: &str = r#"
skill 1 {
    name Grunt
    aggression 0.3 0.0 1.0
    aim_accuracy 0.5
    reactiontime 0.4
    attack_skill 0.5
    view_factor 0.5
    view_maxchange 360
}
skill 5 {
    name Grunt
    aggression 0.9 0.0 1.0
    aim_accuracy 0.9
    reactiontime 0.1
    attack_skill 0.9
    view_factor 0.9
    view_maxchange 720
}
"#;

/// Minimal weapon config for weapon tests.
pub const TEST_WEAPON_CONFIG: &str = r#"
projectileinfo {
    name rocket
    model models/rocket.md3
    damage 100
    radius 120
    damagetype 2
    speed 900
}
weaponinfo {
    number 5
    name rocketlauncher
    projectile rocket
    numprojectiles 1
    speed 900
    reload 0.8
    ammoamount 1
    ammoinventoryindex 23
    inventoryindex 8
}
projectileinfo {
    name bullet
    damage 7
    damagetype 1
}
weaponinfo {
    number 2
    name machinegun
    projectile bullet
    numprojectiles 1
    reload 0.1
    ammoamount 1
    ammoinventoryindex 19
    inventoryindex 6
}
"#;

/// Minimal fuzzy weight config for weight tests.
pub const TEST_WEIGHT_CONFIG: &str = r#"
weight health {
    switch ( INVENTORY_HEALTH ) {
        case 0 : return 100 ;
        case 50 : return 50 ;
        case 100 : return 0 ;
        default : return 0 ;
    }
}
"#;

/// Minimal chat file for chat tests.
pub const TEST_CHAT_FILE: &str = r#"
random greetings {
    "hello"
    "hi there"
}
match {
    string hello
    variable 0
}
reply {
    priority 1
    message hi
}
chat kill {
    "got you"
}
"#;

/// Scripted fixtures bundle: files plus host/game/nav builders.
pub fn behavior_files() -> qa_bots::behavior::BotAssetFiles {
    let mut files = qa_bots::behavior::BotAssetFiles::new();
    files.add("botfiles/items.c", TEST_ITEM_CONFIG.as_bytes());
    files.add("scripts/bots.txt", TEST_BOTS_TXT.as_bytes());
    files.add("botfiles/bots.c", TEST_CHARACTER.as_bytes());
    files.add("botfiles/weapons.c", TEST_WEAPON_CONFIG.as_bytes());
    files.add("botfiles/fw.c", TEST_WEIGHT_CONFIG.as_bytes());
    files.add("botfiles/chat.c", TEST_CHAT_FILE.as_bytes());
    files
}
