//! Quake III base/game: spawn.
//!
//! Donor provenance: `src/content/q3/base/game/spawn.ts`.

use qa_core::math::vec3;
use qa_core::math::Vec3;
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::numeric::*;
use crate::q3::base::game::state::*;

// ---------------------------------------------------------------------------
// spawn.ts: spawn variables and entity dispatch (g_spawn.c)
// ---------------------------------------------------------------------------

/// Maximum spawn variables (`MAX_SPAWN_VARS`).
pub const MAX_SPAWN_VARS: usize = 64;

/// Maximum spawn variable characters (`MAX_SPAWN_VARS_CHARS`).
pub const MAX_SPAWN_VARS_CHARS: usize = 4096;

/// Maximum token characters (`TOKEN_MAX` from `common-parse.ts`).
pub const TOKEN_MAX: usize = 1024;

/// Spawn pair (`SpawnPair`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPair {
    /// Key.
    pub key: String,
    /// Value.
    pub value: String,
}

/// Spawn value with presence (`SpawnValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnValue<T> {
    /// Present.
    pub present: bool,
    /// Value.
    pub value: T,
}

pub(crate) fn spawn_lower(value: &str) -> Vec<u8> {
    ascii_lower(value.as_bytes())
}

/// Spawn string with escape processing (`G_NewString`).
pub fn new_spawn_string(value: &str, memory: &mut GameMemory) -> Result<String, Q3GameError> {
    let bytes = latin1_bytes(value)?;
    let allocation = memory.allocate(bytes.len() + 1)?;
    {
        let out = memory.alloc_bytes_mut(&allocation);
        let mut output = 0usize;
        let mut index = 0usize;
        while index <= bytes.len() {
            let character = if index == bytes.len() { 0u8 } else { bytes[index] };
            if character == 92 && index < bytes.len() {
                index += 1;
                let next = if index < bytes.len() { bytes[index] } else { 0 };
                out[output] = if next == b'n' { 10 } else { 92 };
                output += 1;
            } else {
                out[output] = character;
                output += 1;
            }
            index += 1;
        }
    }
    memory.read_string(&allocation)
}

/// Ordered spawn variables (`SpawnVariables`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnVariables {
    /// Entries.
    pub entries: Vec<SpawnPair>,
    /// Character count.
    pub character_count: usize,
}

impl SpawnVariables {
    /// New variables with source-limit validation.
    pub fn new(entries: Vec<SpawnPair>) -> Result<Self, Q3GameError> {
        if entries.len() > MAX_SPAWN_VARS {
            return Err(range("G_ParseSpawnVars: MAX_SPAWN_VARS"));
        }
        let mut characters = 0usize;
        for pair in &entries {
            for value in [&pair.key, &pair.value] {
                let bytes = latin1_bytes(value)?;
                if bytes.len() >= TOKEN_MAX {
                    return Err(range("Spawn token exceeds MAX_TOKEN_CHARS"));
                }
                characters += bytes.len() + 1;
                if characters > MAX_SPAWN_VARS_CHARS {
                    return Err(range("G_AddSpawnVarToken: MAX_SPAWN_CHARS"));
                }
            }
        }
        Ok(Self {
            entries,
            character_count: characters,
        })
    }

    /// String value, first match (`string`).
    #[must_use]
    pub fn string(&self, key: &str, default_value: &str) -> SpawnValue<String> {
        let normalized = spawn_lower(key);
        match self.entries.iter().find(|pair| spawn_lower(&pair.key) == normalized) {
            Some(found) => SpawnValue {
                present: true,
                value: found.value.clone(),
            },
            None => SpawnValue {
                present: false,
                value: default_value.to_string(),
            },
        }
    }

    /// Integer value (`int`).
    pub fn int(&self, key: &str, default_value: &str) -> Result<SpawnValue<i32>, Q3GameError> {
        let found = self.string(key, default_value);
        Ok(SpawnValue {
            present: found.present,
            value: game_atoi(&found.value)?,
        })
    }

    /// Float value (`float`).
    pub fn float(&self, key: &str, default_value: &str) -> Result<SpawnValue<f32>, Q3GameError> {
        let found = self.string(key, default_value);
        Ok(SpawnValue {
            present: found.present,
            value: game_atof(&found.value)?,
        })
    }

    /// Vector value (`vector`).
    pub fn vector(&self, key: &str, default_value: &str) -> Result<SpawnValue<Vec3>, Q3GameError> {
        let found = self.string(key, default_value);
        Ok(SpawnValue {
            present: found.present,
            value: scan_game_vector(&found.value)?,
        })
    }
}

/// Spawn token (`Token`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnToken {
    /// Value.
    pub value: String,
    /// Line.
    pub line: usize,
    /// Column.
    pub column: usize,
    /// Quoted.
    pub quoted: bool,
}

/// Native-x86 spawn parser (`SpawnParser`).
#[derive(Debug, Clone)]
pub struct SpawnParser {
    text: Vec<u8>,
    name: String,
    offset: usize,
    line: usize,
    column: usize,
}

impl SpawnParser {
    /// New parser over spawn text.
    pub fn new(text: &str, name: &str) -> Result<Self, Q3GameError> {
        let cut = text.split('\0').next().unwrap_or("");
        latin1_bytes(cut)?;
        Ok(Self {
            text: cut.bytes().collect(),
            name: name.to_string(),
            offset: 0,
            line: 1,
            column: 1,
        })
    }

    fn error(&self, message: &str, token: Option<&SpawnToken>) -> Q3GameError {
        Q3GameError::Parse {
            source: self.name.clone(),
            line: token.map_or(self.line, |token| token.line),
            column: token.map_or(self.column, |token| token.column),
            message: message.to_string(),
        }
    }

    fn advance(&mut self) {
        if self.text.get(self.offset) == Some(&b'\n') {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        self.offset += 1;
    }

    fn token(&mut self) -> Result<Option<SpawnToken>, Q3GameError> {
        loop {
            while self.offset < self.text.len() {
                let code = self.text[self.offset];
                if code > 32 && code < 128 {
                    break;
                }
                self.advance();
            }
            if self.text[self.offset..].starts_with(b"//") {
                while self.offset < self.text.len() && self.text[self.offset] != b'\n' {
                    self.advance();
                }
            } else if self.text[self.offset..].starts_with(b"/*") {
                self.advance();
                self.advance();
                while self.offset < self.text.len() && !self.text[self.offset..].starts_with(b"*/") {
                    self.advance();
                }
                if self.offset < self.text.len() {
                    self.advance();
                    self.advance();
                }
            } else {
                break;
            }
        }
        if self.offset == self.text.len() {
            return Ok(None);
        }
        let line = self.line;
        let column = self.column;
        let quoted = self.text[self.offset] == b'"';
        let mut value = Vec::new();
        if quoted {
            self.advance();
        }
        while self.offset < self.text.len() {
            let code = self.text[self.offset];
            if quoted && code == b'"' {
                break;
            }
            if !quoted && (code <= 32 || code >= 128) {
                break;
            }
            value.push(code);
            self.advance();
            if value.len() >= TOKEN_MAX {
                let token = SpawnToken {
                    value: latin1_string(&value),
                    line,
                    column,
                    quoted,
                };
                return Err(self.error("Spawn token exceeds MAX_TOKEN_CHARS", Some(&token)));
            }
        }
        if quoted && self.offset < self.text.len() {
            self.advance();
        }
        Ok(Some(SpawnToken {
            value: latin1_string(&value),
            line,
            column,
            quoted,
        }))
    }

    /// Parse the next variable block (`next`).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<SpawnVariables>, Q3GameError> {
        let Some(opening) = self.token()? else { return Ok(None) };
        if !opening.value.starts_with('{') {
            return Err(self.error(
                &format!("G_ParseSpawnVars: found {} when expecting {{", opening.value),
                Some(&opening),
            ));
        }
        let mut entries = Vec::new();
        let mut characters = 0usize;
        loop {
            let Some(key) = self.token()? else {
                return Err(self.error("G_ParseSpawnVars: EOF without closing brace", None));
            };
            if key.value.starts_with('}') {
                return Ok(Some(SpawnVariables::new(entries)?));
            }
            let Some(value) = self.token()? else {
                return Err(self.error("G_ParseSpawnVars: EOF without closing brace", None));
            };
            if value.value.starts_with('}') {
                return Err(self.error("G_ParseSpawnVars: closing brace without data", Some(&value)));
            }
            if entries.len() == MAX_SPAWN_VARS {
                return Err(self.error("G_ParseSpawnVars: MAX_SPAWN_VARS", Some(&key)));
            }
            characters += key.value.len() + value.value.len() + 2;
            if characters > MAX_SPAWN_VARS_CHARS {
                return Err(self.error("G_AddSpawnVarToken: MAX_SPAWN_CHARS", Some(&value)));
            }
            entries.push(SpawnPair {
                key: key.value,
                value: value.value,
            });
        }
    }
}

/// Parse one spawn field (`G_ParseField`); false means no source field matched.
pub fn parse_spawn_field(
    key: &str,
    value: &str,
    entity: &mut GameEntity,
    memory: &mut GameMemory,
) -> Result<bool, Q3GameError> {
    let field = String::from_utf8_lossy(&spawn_lower(key)).into_owned();
    match field.as_str() {
        "classname" => entity.set_classname(Some(new_spawn_string(value, memory)?)),
        "model" => entity.model = Some(new_spawn_string(value, memory)?),
        "model2" => entity.model2 = Some(new_spawn_string(value, memory)?),
        "target" => entity.target = Some(new_spawn_string(value, memory)?),
        "targetname" => entity.targetname = Some(new_spawn_string(value, memory)?),
        "message" => entity.message = Some(new_spawn_string(value, memory)?),
        "team" => entity.team = Some(new_spawn_string(value, memory)?),
        "targetshadername" => entity.target_shader_name = Some(new_spawn_string(value, memory)?),
        "targetshadernewname" => entity.target_shader_new_name = Some(new_spawn_string(value, memory)?),
        "spawnflags" => entity.spawnflags = game_atoi(value)?,
        "count" => entity.count = game_atoi(value)?,
        "health" => entity.health = game_atoi(value)?,
        "dmg" => entity.damage = game_atoi(value)?,
        "speed" => entity.speed = game_atof(value)?,
        "wait" => entity.wait = game_atof(value)?,
        "random" => entity.random = game_atof(value)?,
        "origin" => entity.s.origin = scan_game_vector(value)?,
        "angles" => entity.s.angles = scan_game_vector(value)?,
        "angle" => entity.s.angles = vec3(0.0, game_atof(value)?, 0.0),
        "light" => {}
        _ => return Ok(false),
    }
    Ok(true)
}

/// Spawn handler (`SpawnHandler`).
pub type SpawnHandler =
    Rc<dyn Fn(&mut dyn Q3Driver, &mut dyn SpawnServices, usize, &SpawnVariables) -> Result<(), Q3GameError>>;

/// Spawn handler table.
#[derive(Default)]
pub struct SpawnHandlerTable {
    map: HashMap<String, SpawnHandler>,
}

impl SpawnHandlerTable {
    /// New table.
    #[must_use]
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    /// Insert a handler.
    pub fn insert(&mut self, classname: &str, handler: SpawnHandler) {
        self.map.insert(classname.to_string(), handler);
    }

    /// Look up a handler.
    #[must_use]
    pub fn get(&self, classname: &str) -> Option<SpawnHandler> {
        self.map.get(classname).cloned()
    }
}

impl std::fmt::Debug for SpawnHandlerTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<&str> = self.map.keys().map(String::as_str).collect();
        keys.sort_unstable();
        f.debug_struct("SpawnHandlerTable").field("handlers", &keys).finish()
    }
}

/// Spawn services (`SpawnContext` minus the pool, which the driver owns).
pub trait SpawnServices {
    /// Game memory.
    fn memory(&mut self) -> &mut GameMemory;
    /// Product.
    fn product(&self) -> Q3Product;
    /// Game type number.
    fn game_type(&self) -> i32;
    /// Handler table.
    fn handlers(&self) -> &SpawnHandlerTable;
    /// Spawn an item (`spawnItem`).
    fn spawn_item(
        &mut self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        item: usize,
        variables: &SpawnVariables,
    ) -> Result<(), Q3GameError>;
    /// Warn (`warn`).
    fn warn(&mut self, message: &str);
}

/// Spawn filter reason (`SpawnFilter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnFilter {
    /// Not single player.
    Notsingle,
    /// Not team.
    Notteam,
    /// Not free-for-all.
    Notfree,
    /// Not Team Arena.
    Notta,
    /// Not base Quake III.
    Notq3a,
    /// Game type list.
    Gametype,
}

impl SpawnFilter {
    /// Source key.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Notsingle => "notsingle",
            Self::Notteam => "notteam",
            Self::Notfree => "notfree",
            Self::Notta => "notta",
            Self::Notq3a => "notq3a",
            Self::Gametype => "gametype",
        }
    }
}

/// Spawn dispatch route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnRoute {
    /// Item.
    Item,
    /// Handler.
    Handler,
}

/// Spawn outcome (`SpawnOutcome`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnOutcome {
    /// Dispatched.
    Dispatched {
        /// Route.
        route: SpawnRoute,
        /// Slot.
        slot: usize,
        /// Classname.
        classname: String,
    },
    /// Filtered.
    Filtered {
        /// Slot.
        slot: usize,
        /// Reason.
        reason: SpawnFilter,
    },
    /// Unknown classname.
    Unknown {
        /// Slot.
        slot: usize,
        /// Classname.
        classname: Option<String>,
    },
}

pub(crate) fn spawn_excluded(
    variables: &SpawnVariables,
    services: &mut dyn SpawnServices,
) -> Result<Option<SpawnFilter>, Q3GameError> {
    let game_type = services.game_type();
    if game_type == Q3GameType::SinglePlayer as i32 && variables.int("notsingle", "0")?.value != 0 {
        return Ok(Some(SpawnFilter::Notsingle));
    }
    let team_key = if game_type >= Q3GameType::Team as i32 {
        "notteam"
    } else {
        "notfree"
    };
    if variables.int(team_key, "0")?.value != 0 {
        return Ok(Some(if team_key == "notteam" {
            SpawnFilter::Notteam
        } else {
            SpawnFilter::Notfree
        }));
    }
    let product_key = if services.product() == Q3Product::Missionpack {
        "notta"
    } else {
        "notq3a"
    };
    if variables.int(product_key, "0")?.value != 0 {
        return Ok(Some(if product_key == "notta" {
            SpawnFilter::Notta
        } else {
            SpawnFilter::Notq3a
        }));
    }
    let gametype = variables.string("gametype", "");
    if gametype.present && (Q3GameType::Ffa as i32..Q3GameType::MaxGameType as i32).contains(&game_type) {
        let names = [
            "ffa",
            "tournament",
            "single",
            "team",
            "ctf",
            "oneflag",
            "obelisk",
            "harvester",
        ];
        let Some(name) = names.get(game_type as usize) else {
            return Err(range("No source gametype name"));
        };
        if !gametype.value.contains(name) {
            return Ok(Some(SpawnFilter::Gametype));
        }
    }
    Ok(None)
}

/// Spawn one entity from variables (`G_SpawnGEntityFromSpawnVars` / `G_CallSpawn`).
pub fn spawn_entity(
    variables: &SpawnVariables,
    driver: &mut dyn Q3Driver,
    services: &mut dyn SpawnServices,
) -> Result<SpawnOutcome, Q3GameError> {
    let slot = driver.pool().spawn_entity()?;
    for pair in &variables.entries {
        let parsed = {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure(format!("entity {slot} vanished after spawn")))?;
            parse_spawn_field(&pair.key, &pair.value, entity, services.memory())
        };
        if let Err(error) = parsed {
            if matches!(error, Q3GameError::Drop(_)) {
                return Err(error);
            }
            driver.pool().free_entity(slot);
            return Err(error);
        }
    }
    if let Some(reason) = spawn_excluded(variables, services)? {
        driver.pool().free_entity(slot);
        return Ok(SpawnOutcome::Filtered { slot, reason });
    }
    let origin = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.pos.base = origin;
        entity.r.current_origin = origin;
    }
    let classname = driver
        .pool()
        .entity(slot)
        .and_then(|entity| entity.classname_value().map(str::to_string));
    let Some(classname) = classname else {
        services.warn("G_CallSpawn: NULL classname\n");
        driver.pool().free_entity(slot);
        return Ok(SpawnOutcome::Unknown { slot, classname: None });
    };
    let mut item_index = None;
    for index in 0..driver.item_count() {
        if driver
            .item_at(index)
            .is_some_and(|item| item.class_name.as_deref() == Some(classname.as_str()))
        {
            item_index = Some(index);
            break;
        }
    }
    if let Some(item) = item_index {
        services.spawn_item(driver, slot, item, variables)?;
        return Ok(SpawnOutcome::Dispatched {
            route: SpawnRoute::Item,
            slot,
            classname,
        });
    }
    if let Some(handler) = services.handlers().get(&classname) {
        handler(driver, services, slot, variables)?;
        return Ok(SpawnOutcome::Dispatched {
            route: SpawnRoute::Handler,
            slot,
            classname,
        });
    }
    services.warn(&format!("{classname} doesn't have a spawn function\n"));
    driver.pool().free_entity(slot);
    Ok(SpawnOutcome::Unknown {
        slot,
        classname: Some(classname),
    })
}

/// Worldspawn music configstring.
pub const WORLDSPAWN_CS_MUSIC: i32 = 2;

/// Worldspawn message configstring.
pub const WORLDSPAWN_CS_MESSAGE: i32 = 3;

/// Worldspawn motd configstring.
pub const WORLDSPAWN_CS_MOTD: i32 = 4;

/// Worldspawn warmup configstring.
pub const WORLDSPAWN_CS_WARMUP: i32 = 5;

/// Worldspawn game configstring.
pub const WORLDSPAWN_CS_GAME: i32 = 20;

/// Worldspawn start-time configstring.
pub const WORLDSPAWN_CS_START_TIME: i32 = 21;

/// Worldspawn state (`WorldspawnContext` fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldspawnState {
    /// Start time.
    pub start_time: i32,
    /// Message of the day.
    pub motd: String,
    /// Restarted flag.
    pub restarted: i32,
    /// Warmup flag.
    pub do_warmup: i32,
    /// Warmup time (mutable).
    pub warmup_time: i32,
}

/// Spawn the world entity (`SP_worldspawn`).
pub fn spawn_world(
    variables: &SpawnVariables,
    driver: &mut dyn Q3Driver,
    world: &mut WorldspawnState,
) -> Result<(), Q3GameError> {
    if String::from_utf8_lossy(&spawn_lower(&variables.string("classname", "").value)) != "worldspawn" {
        return Err(failure("SP_worldspawn: The first entity isn't 'worldspawn'"));
    }
    driver.set_configstring(WORLDSPAWN_CS_GAME, "baseq3-1");
    driver.set_configstring(WORLDSPAWN_CS_START_TIME, &world.start_time.to_string());
    driver.set_configstring(WORLDSPAWN_CS_MUSIC, &variables.string("music", "").value);
    driver.set_configstring(WORLDSPAWN_CS_MESSAGE, &variables.string("message", "").value);
    driver.set_configstring(WORLDSPAWN_CS_MOTD, &world.motd.clone());
    driver.set_cvar("g_gravity", &variables.string("gravity", "800").value);
    driver.set_cvar("g_enableDust", &variables.string("enableDust", "0").value);
    driver.set_cvar("g_enableBreath", &variables.string("enableBreath", "0").value);
    let world_entity = driver
        .pool()
        .entity_mut(ENTITYNUM_WORLD)
        .ok_or_else(|| failure("SP_worldspawn: missing world entity"))?;
    world_entity.s.number = ENTITYNUM_WORLD as i32;
    world_entity.set_classname(Some("worldspawn".to_string()));
    driver.set_configstring(WORLDSPAWN_CS_WARMUP, "");
    if world.restarted != 0 {
        driver.set_cvar("g_restarted", "0");
        world.warmup_time = 0;
    } else if world.do_warmup != 0 {
        world.warmup_time = -1;
        driver.set_configstring(WORLDSPAWN_CS_WARMUP, "-1");
        driver.log("Warmup:\n");
    }
    Ok(())
}

/// Spawn report (`SpawnReport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnReport {
    /// World variables.
    pub world_variables: SpawnVariables,
    /// Outcomes.
    pub outcomes: Vec<SpawnOutcome>,
}

/// Parse and dispatch entities (`spawnEntities`).
pub fn spawn_entities(
    text: &str,
    driver: &mut dyn Q3Driver,
    services: &mut dyn SpawnServices,
    world: &mut WorldspawnState,
    source: &str,
) -> Result<SpawnReport, Q3GameError> {
    let mut parser = SpawnParser::new(text, source)?;
    let Some(world_variables) = parser.next()? else {
        return Err(failure("SpawnEntities: no entities"));
    };
    spawn_world(&world_variables, driver, world)?;
    let mut outcomes = Vec::new();
    while let Some(variables) = parser.next()? {
        outcomes.push(spawn_entity(&variables, driver, services)?);
    }
    Ok(SpawnReport {
        world_variables,
        outcomes,
    })
}
