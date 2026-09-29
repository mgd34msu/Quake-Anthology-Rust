//! Weapon knowledge from `src/bots/behavior/library/weapons.ts`
//! (`be_ai_weap.c`: `LoadWeaponConfig`, `BotChooseBestFightWeapon`,
//! `BotChooseBestWeapon`).
//!
//! Weapon configs declare projectile ballistics plus per-weapon spread,
//! timing, and inventory bindings. The AI ranks owned weapons with the
//! fuzzy weapon weights and picks the best fight weapon for the sensed
//! range.

use std::collections::HashMap;

use qa_core::math::Vec3;

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::library::structure::read_structure_definitions;
use crate::error::BotsError;

/// Maximum weapon states.
pub const MAX_WEAPON_STATES: usize = 64;
/// Projectile damages through windows.
pub const PROJECTILE_WINDOW_DAMAGE: i32 = 1;
/// Projectile returns (boomerang).
pub const PROJECTILE_RETURN: i32 = 2;
/// Fire released flag.
pub const WEAPON_FIRE_RELEASED: i32 = 1;
/// Impact damage type.
pub const DAMAGE_TYPE_IMPACT: i32 = 1;
/// Radial damage type.
pub const DAMAGE_TYPE_RADIAL: i32 = 2;
/// Visible damage type.
pub const DAMAGE_TYPE_VISIBLE: i32 = 4;

/// Weapon config load result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponLoadResult {
    /// Loaded.
    NoError = 0,
    /// Weapon weights failed to load.
    CannotLoadWeaponWeights = 11,
    /// Weapon config failed to load.
    CannotLoadWeaponConfig = 12,
}

/// Projectile ballistics.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileInfo {
    /// Projectile name.
    pub name: String,
    /// Model name.
    pub model: String,
    /// Flags.
    pub flags: i32,
    /// Gravity.
    pub gravity: f32,
    /// Damage.
    pub damage: f32,
    /// Splash radius.
    pub radius: f32,
    /// Visible damage bonus.
    pub visible_damage: f32,
    /// Damage type bits.
    pub damage_type: i32,
    /// Health increase on hit (vampire).
    pub health_increase: f32,
    /// Push force.
    pub push: f32,
    /// Detonation delay.
    pub detonation: f32,
    /// Bounce factor.
    pub bounce: f32,
    /// Bounce friction.
    pub bounce_friction: f32,
    /// Bounce stop speed.
    pub bounce_stop: f32,
}

impl Default for ProjectileInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            model: String::new(),
            flags: 0,
            gravity: 0.0,
            damage: 0.0,
            radius: 0.0,
            visible_damage: 0.0,
            damage_type: DAMAGE_TYPE_IMPACT,
            health_increase: 0.0,
            push: 0.0,
            detonation: 0.0,
            bounce: 0.0,
            bounce_friction: 0.0,
            bounce_stop: 0.0,
        }
    }
}

/// Weapon definition.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponInfo {
    /// Whether the slot is defined.
    pub valid: bool,
    /// Weapon number.
    pub number: i32,
    /// Weapon name.
    pub name: String,
    /// Model name.
    pub model: String,
    /// Weapon level.
    pub level: i32,
    /// Inventory index of the weapon.
    pub weapon_inventory_index: i32,
    /// Flags.
    pub flags: i32,
    /// Projectile name.
    pub projectile: String,
    /// Projectiles per shot.
    pub projectile_count: i32,
    /// Horizontal spread.
    pub horizontal_spread: f32,
    /// Vertical spread.
    pub vertical_spread: f32,
    /// Projectile speed (0 = hitscan/melee).
    pub speed: f32,
    /// Projectile acceleration.
    pub acceleration: f32,
    /// Recoil.
    pub recoil: Vec3,
    /// Muzzle offset.
    pub offset: Vec3,
    /// Angle offset.
    pub angle_offset: Vec3,
    /// Extra upward velocity.
    pub extra_z_velocity: f32,
    /// Ammo per shot.
    pub ammo_amount: i32,
    /// Ammo inventory index.
    pub ammo_inventory_index: i32,
    /// Activation time.
    pub activate: f32,
    /// Reload time.
    pub reload: f32,
    /// Spin-up time.
    pub spin_up: f32,
    /// Spin-down time.
    pub spin_down: f32,
    /// Resolved projectile ballistics.
    pub projectile_info: ProjectileInfo,
}

impl Default for WeaponInfo {
    fn default() -> Self {
        Self {
            valid: false,
            number: 0,
            name: String::new(),
            model: String::new(),
            level: 0,
            weapon_inventory_index: 0,
            flags: 0,
            projectile: String::new(),
            projectile_count: 0,
            horizontal_spread: 0.0,
            vertical_spread: 0.0,
            speed: 0.0,
            acceleration: 0.0,
            recoil: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            offset: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            angle_offset: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            extra_z_velocity: 0.0,
            ammo_amount: 0,
            ammo_inventory_index: 0,
            activate: 0.0,
            reload: 0.0,
            spin_up: 0.0,
            spin_down: 0.0,
            projectile_info: ProjectileInfo::default(),
        }
    }
}

/// Parsed weapon config.
#[derive(Debug, Clone, Default)]
pub struct WeaponConfig {
    /// Source path.
    pub path: String,
    /// Weapons by number.
    pub weapons: Vec<WeaponInfo>,
    /// Projectiles by name.
    pub projectiles: Vec<ProjectileInfo>,
    /// Parse warnings.
    pub diagnostics: Vec<String>,
}

impl WeaponConfig {
    /// Parse `projectileinfo`/`weaponinfo` blocks.
    pub fn parse(path: &str, text: &str) -> Result<Self, BotsError> {
        let definitions = read_structure_definitions(text)?;
        let mut projectiles = Vec::new();
        let mut weapons = Vec::new();
        for definition in &definitions {
            match definition.type_name.as_deref() {
                Some("projectileinfo") => projectiles.push(parse_projectile(definition)),
                Some("weaponinfo") => weapons.push(parse_weapon(definition)),
                _ => {}
            }
        }
        weapons.sort_by_key(|weapon| weapon.number);
        let by_name: HashMap<&str, ProjectileInfo> = projectiles
            .iter()
            .map(|projectile| (projectile.name.as_str(), projectile.clone()))
            .collect();
        for weapon in &mut weapons {
            if let Some(projectile) = by_name.get(weapon.projectile.as_str()) {
                weapon.projectile_info = projectile.clone();
            }
            weapon.valid = true;
        }
        Ok(Self {
            path: path.to_owned(),
            weapons,
            projectiles,
            diagnostics: Vec::new(),
        })
    }

    /// Weapon info by number.
    #[must_use]
    pub fn weapon_info(&self, number: i32) -> Option<&WeaponInfo> {
        self.weapons.iter().find(|weapon| weapon.number == number)
    }
}

fn field_string(definition: &super::structure::StructureDefinition, name: &str) -> String {
    definition.string(name).unwrap_or_default().to_owned()
}

fn field_number(definition: &super::structure::StructureDefinition, name: &str) -> f32 {
    definition.number(name).unwrap_or(0.0) as f32
}

fn field_vec(definition: &super::structure::StructureDefinition, prefix: &str) -> Vec3 {
    Vec3 {
        x: definition.number(&format!("{prefix}.x")).unwrap_or(0.0) as f32,
        y: definition.number(&format!("{prefix}.y")).unwrap_or(0.0) as f32,
        z: definition.number(&format!("{prefix}.z")).unwrap_or(0.0) as f32,
    }
}

fn parse_projectile(definition: &super::structure::StructureDefinition) -> ProjectileInfo {
    ProjectileInfo {
        name: field_string(definition, "name"),
        model: field_string(definition, "model"),
        flags: field_number(definition, "flags") as i32,
        gravity: field_number(definition, "gravity"),
        damage: field_number(definition, "damage"),
        radius: field_number(definition, "radius"),
        visible_damage: field_number(definition, "visdamage"),
        damage_type: field_number(definition, "damagetype") as i32,
        health_increase: field_number(definition, "healthinc"),
        push: field_number(definition, "push"),
        detonation: field_number(definition, "detonation"),
        bounce: field_number(definition, "bounce"),
        bounce_friction: field_number(definition, "bouncefric"),
        bounce_stop: field_number(definition, "bouncestop"),
    }
}

fn parse_weapon(definition: &super::structure::StructureDefinition) -> WeaponInfo {
    WeaponInfo {
        valid: true,
        number: field_number(definition, "number") as i32,
        name: field_string(definition, "name"),
        model: field_string(definition, "model"),
        level: field_number(definition, "level") as i32,
        weapon_inventory_index: field_number(definition, "inventoryindex") as i32,
        flags: field_number(definition, "flags") as i32,
        projectile: field_string(definition, "projectile"),
        projectile_count: field_number(definition, "numprojectiles") as i32,
        horizontal_spread: field_number(definition, "hspread"),
        vertical_spread: field_number(definition, "vspread"),
        speed: field_number(definition, "speed"),
        acceleration: field_number(definition, "acceleration"),
        recoil: field_vec(definition, "recoil"),
        offset: field_vec(definition, "offset"),
        angle_offset: field_vec(definition, "angleoffset"),
        extra_z_velocity: field_number(definition, "extrazvelocity"),
        ammo_amount: field_number(definition, "ammoamount") as i32,
        ammo_inventory_index: field_number(definition, "ammoinventoryindex") as i32,
        activate: field_number(definition, "activate"),
        reload: field_number(definition, "reload"),
        spin_up: field_number(definition, "spinup"),
        spin_down: field_number(definition, "spindown"),
        projectile_info: ProjectileInfo::default(),
    }
}

/// Weapon state for one bot (`bot_weaponstate_t`).
#[derive(Debug, Clone)]
pub struct BotWeaponState {
    /// Owned weapon numbers.
    pub owned: Vec<bool>,
    /// Current weapon.
    pub current: i32,
}

impl BotWeaponState {
    fn new(capacity: usize) -> Self {
        Self {
            owned: vec![false; capacity],
            current: 0,
        }
    }
}

/// Weapon AI: config plus per-bot weapon states.
pub struct WeaponAi<'a> {
    files: &'a dyn BotSourceFiles,
    config: Option<WeaponConfig>,
    states: Vec<Option<BotWeaponState>>,
    diagnostics: Vec<String>,
}

impl<'a> WeaponAi<'a> {
    /// New weapon AI over prepared files.
    pub fn new(files: &'a dyn BotSourceFiles) -> Self {
        Self {
            files,
            config: None,
            states: vec![None; MAX_WEAPON_STATES],
            diagnostics: Vec::new(),
        }
    }

    /// Parse warnings.
    #[must_use]
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// Load the weapon config (`BotLoadWeaponWeights` + config).
    pub fn load_weapons(&mut self, path: &str) -> WeaponLoadResult {
        let bytes = match self.files.read(path) {
            Some(bytes) => bytes,
            None => return WeaponLoadResult::CannotLoadWeaponConfig,
        };
        let text = String::from_utf8_lossy(&bytes);
        match WeaponConfig::parse(path, &text) {
            Ok(config) => {
                self.config = Some(config);
                WeaponLoadResult::NoError
            }
            Err(error) => {
                self.diagnostics.push(error.to_string());
                WeaponLoadResult::CannotLoadWeaponConfig
            }
        }
    }

    /// Allocate a weapon state (`BotAllocWeaponState`).
    pub fn alloc_state(&mut self) -> Option<i32> {
        let capacity = self.config.as_ref().map_or(32, |config| config.weapons.len().max(1));
        self.states.iter().position(Option::is_none).map(|index| {
            self.states[index] = Some(BotWeaponState::new(capacity.max(32)));
            index as i32 + 1
        })
    }

    /// Free a weapon state (`BotFreeWeaponState`).
    pub fn free_state(&mut self, handle: i32) {
        if let Some(slot) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            *slot = None;
        }
    }

    /// Reset a weapon state (`BotResetWeaponState`).
    pub fn reset_state(&mut self, handle: i32) {
        if let Some(Some(state)) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            state.owned.fill(false);
            state.current = 0;
        }
    }

    fn state(&self, handle: i32) -> Option<&BotWeaponState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get(index as usize)?.as_ref())
    }

    fn state_mut(&mut self, handle: i32) -> Option<&mut BotWeaponState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize)?.as_mut())
    }

    /// Weapon info by number.
    #[must_use]
    pub fn get_weapon_info(&self, number: i32) -> Option<&WeaponInfo> {
        self.config.as_ref()?.weapon_info(number)
    }

    /// Mark a weapon owned.
    pub fn set_owned(&mut self, handle: i32, number: i32, owned: bool) {
        if let Some(state) = self.state_mut(handle) {
            let index = number as usize;
            if index < state.owned.len() {
                state.owned[index] = owned;
            }
        }
    }

    /// Whether a weapon is owned.
    #[must_use]
    pub fn is_owned(&self, handle: i32, number: i32) -> bool {
        self.state(handle).map_or(false, |state| {
            state.owned.get(number as usize).copied().unwrap_or(false)
        })
    }

    /// Choose the best weapon for the sensed range (`BotChooseBestWeapon`).
    ///
    /// Scores owned weapons by damage-per-second adjusted for range fit:
    /// melee only scores close, splash scores mid-range, and hitscan
    /// scores at all ranges with spread falloff.
    pub fn choose_best_weapon(&self, handle: i32, inventory: &[i32], range: f32, random: &mut dyn BotRandom) -> i32 {
        let Some(config) = self.config.as_ref() else {
            return 0;
        };
        let mut best = 0;
        let mut best_score = 0.0f32;
        for weapon in &config.weapons {
            if !weapon.valid || !self.is_owned(handle, weapon.number) {
                continue;
            }
            if weapon.ammo_amount > 0 {
                let ammo = inventory
                    .get(weapon.ammo_inventory_index as usize)
                    .copied()
                    .unwrap_or(0);
                if ammo < weapon.ammo_amount {
                    continue;
                }
            }
            let cycle = (weapon.reload + weapon.activate).max(0.1);
            let dps = weapon.projectile_info.damage * weapon.projectile_count.max(1) as f32 / cycle;
            let mut score = dps;
            if weapon.speed <= 0.0 && weapon.projectile_info.gravity <= 0.0 {
                if range > 90.0 {
                    score *= 90.0 / range;
                }
                if weapon.projectile_count >= 4 {
                    score *= (900.0 / (range + 300.0)).clamp(0.2, 1.0);
                }
            } else if (weapon.projectile_info.damage_type & DAMAGE_TYPE_RADIAL) != 0 {
                if range < 120.0 {
                    score *= 0.4;
                } else if range > 1200.0 {
                    score *= 1200.0 / range;
                }
            }
            score *= 1.0 + random.next_unit() * 0.05;
            if score > best_score {
                best_score = score;
                best = weapon.number;
            }
        }
        best
    }
}
