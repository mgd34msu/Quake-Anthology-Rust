mod generated;
mod native_players;
pub use generated::SOURCE_COUNTS;
pub use native_players::NativePlayerBindings;
use qa_core::{
    names::NameTable,
    primitives::{CallbackId, ItemId, NameId, PlayerState, RuleSetId, WeaponId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Health,
    Armor,
    Ammo,
    Weapon,
    WeaponAmmo,
    Key,
    Powerup,
}

pub struct ItemDef {
    pub id: ItemId,
    pub source: RuleSetId,
    pub native: u16,
    pub classname: NameId,
    pub kind: ItemKind,
    pub amount: i32,
    /// Zero means the source pickup rules choose a limit from current state.
    pub maximum: i32,
    pub models: Box<[NameId]>,
    pub pickup_sound: NameId,
    pub label: NameId,
    pub pickup: NameId,
}

pub struct WeaponRecipe {
    pub fire: CallbackId,
    pub frames: &'static [u16],
}
pub struct WeaponDef {
    pub id: WeaponId,
    pub item: ItemId,
    pub source: RuleSetId,
    pub native: u16,
    pub ammo: Option<ItemId>,
    pub fire_name: NameId,
    /// Catalogue entries become playable when the module binds its rule recipe.
    pub recipe: Option<WeaponRecipe>,
}

struct ItemSource {
    source: RuleSetId,
    native: u16,
    classname: &'static str,
    kind: ItemKind,
    amount: i32,
    maximum: i32,
    models: &'static [&'static str],
    pickup_sound: &'static str,
    label: &'static str,
    ammo: &'static str,
    native_weapon: u16,
    pickup: &'static str,
    fire: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegistryError {
    Name,
    Ammo,
    DuplicateItem,
    DuplicateWeapon,
}

pub struct AmmoConversion {
    pub source: ItemId,
    pub target: ItemId,
    pub numerator: i32,
    pub denominator: i32,
}
pub struct Registry {
    pub items: Box<[ItemDef]>,
    pub weapons: Box<[WeaponDef]>,
    pub ammo_conversions: Box<[AmmoConversion]>,
    native_items: Box<[(RuleSetId, u16, ItemId)]>,
    native_weapons: Box<[(RuleSetId, u16, WeaponId)]>,
    classnames: Box<[(RuleSetId, NameId, ItemId)]>,
}

// Cross-module ammo equivalence is authored data. Pickup rules select the
// target weapon's ammo entry; no map/movement family is consulted.
const AMMO_GROUPS: &[&[(RuleSetId, &str)]] = &[
    &[
        (RuleSetId::Quake, "item_shells"),
        (RuleSetId::QuakeWorld, "item_shells"),
        (RuleSetId::Quake2, "ammo_shells"),
        (RuleSetId::Quake2Rerelease, "ammo_shells"),
        (RuleSetId::Quake3, "ammo_shells"),
    ],
    &[
        (RuleSetId::Quake, "item_spikes"),
        (RuleSetId::QuakeWorld, "item_spikes"),
        (RuleSetId::Quake2, "ammo_bullets"),
        (RuleSetId::Quake2Rerelease, "ammo_bullets"),
        (RuleSetId::Quake3, "ammo_bullets"),
        (RuleSetId::Quake3, "ammo_nails"),
        (RuleSetId::Quake3, "ammo_belt"),
    ],
    &[
        (RuleSetId::Quake, "item_rockets"),
        (RuleSetId::QuakeWorld, "item_rockets"),
        (RuleSetId::Quake2, "ammo_rockets"),
        (RuleSetId::Quake2Rerelease, "ammo_rockets"),
        (RuleSetId::Quake3, "ammo_rockets"),
    ],
    &[
        (RuleSetId::Quake, "item_rockets"),
        (RuleSetId::QuakeWorld, "item_rockets"),
        (RuleSetId::Quake2, "ammo_grenades"),
        (RuleSetId::Quake2Rerelease, "ammo_grenades"),
        (RuleSetId::Quake3, "ammo_grenades"),
        (RuleSetId::Quake3, "ammo_mines"),
    ],
    &[
        (RuleSetId::Quake, "item_cells"),
        (RuleSetId::QuakeWorld, "item_cells"),
        (RuleSetId::Quake2, "ammo_cells"),
        (RuleSetId::Quake2Rerelease, "ammo_cells"),
        (RuleSetId::Quake3, "ammo_cells"),
        (RuleSetId::Quake3, "ammo_lightning"),
        (RuleSetId::Quake3, "ammo_bfg"),
    ],
    &[
        (RuleSetId::Quake2, "ammo_slugs"),
        (RuleSetId::Quake2Rerelease, "ammo_slugs"),
        (RuleSetId::Quake3, "ammo_slugs"),
    ],
];

impl Registry {
    pub fn names_needed() -> impl Iterator<Item = &'static [u8]> {
        generated::ITEMS
            .iter()
            .flat_map(|item| {
                [
                    item.classname,
                    item.pickup_sound,
                    item.label,
                    item.ammo,
                    item.pickup,
                    item.fire,
                ]
                .into_iter()
                .chain(item.models.iter().copied())
            })
            .map(str::as_bytes)
    }

    pub fn load(names: &NameTable) -> Result<Self, RegistryError> {
        let sources = generated::ITEMS;
        let name = |value: &str| names.find(value.as_bytes()).ok_or(RegistryError::Name);
        let mut items = Vec::with_capacity(sources.len());
        let mut weapons = Vec::new();
        let source_names = sources
            .iter()
            .map(|item| name(item.classname))
            .collect::<Result<Vec<_>, _>>()?;
        let source_id = |provenance: RuleSetId, classname: &str| {
            let folded = names.find_folded(classname.as_bytes())?;
            sources
                .iter()
                .zip(&source_names)
                .position(|(item, &id)| {
                    item.source == provenance && names.folded(id) == Some(folded)
                })
                .map(|index| ItemId(index as u32 + 1))
        };
        for (index, source) in sources.iter().enumerate() {
            let id = ItemId(index as u32 + 1);
            items.push(ItemDef {
                id,
                source: source.source,
                native: source.native,
                classname: source_names[index],
                kind: source.kind,
                amount: source.amount,
                maximum: source.maximum,
                models: source
                    .models
                    .iter()
                    .map(|path| name(path))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
                pickup_sound: name(source.pickup_sound)?,
                label: name(source.label)?,
                pickup: name(source.pickup)?,
            });
            if matches!(source.kind, ItemKind::Weapon | ItemKind::WeaponAmmo) {
                weapons.push(WeaponDef {
                    id: WeaponId(weapons.len() as u32 + 1),
                    item: id,
                    source: source.source,
                    native: source.native_weapon,
                    ammo: if source.ammo.is_empty() {
                        None
                    } else {
                        Some(source_id(source.source, source.ammo).ok_or(RegistryError::Ammo)?)
                    },
                    fire_name: name(source.fire)?,
                    recipe: None,
                });
            }
        }
        let mut native_items: Vec<_> = items
            .iter()
            .map(|item| (item.source, item.native, item.id))
            .collect();
        native_items.sort_unstable_by_key(|item| (item.0 as u8, item.1));
        if native_items
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
        {
            return Err(RegistryError::DuplicateItem);
        }
        let mut native_weapons: Vec<_> = weapons
            .iter()
            .map(|weapon| (weapon.source, weapon.native, weapon.id))
            .collect();
        native_weapons.sort_unstable_by_key(|weapon| (weapon.0 as u8, weapon.1));
        if native_weapons
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
        {
            return Err(RegistryError::DuplicateWeapon);
        }
        let mut classnames: Vec<_> = items
            .iter()
            .filter(|item| item.classname != NameId(0))
            .map(|item| (item.source, item.classname, item.id))
            .collect();
        classnames.sort_unstable_by_key(|item| (item.0 as u8, item.1.0));
        let mut ammo_conversions = Vec::new();
        for group in AMMO_GROUPS {
            for &(source_module, source_name) in *group {
                let source = source_id(source_module, source_name).ok_or(RegistryError::Ammo)?;
                for &(target_module, target_name) in *group {
                    let target =
                        source_id(target_module, target_name).ok_or(RegistryError::Ammo)?;
                    ammo_conversions.push(AmmoConversion {
                        source,
                        target,
                        numerator: 1,
                        denominator: 1,
                    });
                }
            }
        }
        ammo_conversions.sort_unstable_by_key(|link| (link.source.0, link.target.0));
        ammo_conversions.dedup_by_key(|link| (link.source.0, link.target.0));
        Ok(Self {
            items: items.into_boxed_slice(),
            weapons: weapons.into_boxed_slice(),
            ammo_conversions: ammo_conversions.into_boxed_slice(),
            native_items: native_items.into_boxed_slice(),
            native_weapons: native_weapons.into_boxed_slice(),
            classnames: classnames.into_boxed_slice(),
        })
    }

    pub fn item(&self, id: ItemId) -> Option<&ItemDef> {
        id.0.checked_sub(1)
            .and_then(|index| self.items.get(index as usize))
    }
    pub fn weapon(&self, id: WeaponId) -> Option<&WeaponDef> {
        id.0.checked_sub(1)
            .and_then(|index| self.weapons.get(index as usize))
    }
    pub fn native_item(&self, source: RuleSetId, number: u16) -> Option<ItemId> {
        self.native_items
            .binary_search_by_key(&(source as u8, number), |item| (item.0 as u8, item.1))
            .ok()
            .map(|index| self.native_items[index].2)
    }
    pub fn native_weapon(&self, source: RuleSetId, number: u16) -> Option<WeaponId> {
        self.native_weapons
            .binary_search_by_key(&(source as u8, number), |weapon| (weapon.0 as u8, weapon.1))
            .ok()
            .map(|index| self.native_weapons[index].2)
    }
    pub fn native_weapon_number(&self, source: RuleSetId, id: WeaponId) -> Option<u16> {
        self.weapon(id)
            .filter(|weapon| weapon.source == source)
            .map(|weapon| weapon.native)
    }
    /// Native ordinals may be unknown while their raw source tail stays intact.
    pub fn project_player_weapon(&self, player: &mut PlayerState) {
        let Some(tail) = player.native else {
            return;
        };
        let Some(number) = tail.weapon_number else {
            return;
        };
        player.inventory.weapon = u16::try_from(number)
            .ok()
            .and_then(|number| self.native_weapon(tail.source, number))
            .unwrap_or_default();
    }
    /// Q3 usercmds carry one native weapon byte, with zero denoting NONE.
    pub fn command_weapon_number(&self, id: Option<WeaponId>) -> u8 {
        id.and_then(|id| self.native_weapon_number(RuleSetId::Quake3, id))
            .and_then(|number| u8::try_from(number).ok())
            .unwrap_or(0)
    }
    pub fn classname(&self, source: RuleSetId, name: NameId) -> Option<ItemId> {
        self.classnames
            .binary_search_by_key(&(source as u8, name.0), |item| (item.0 as u8, item.1.0))
            .ok()
            .map(|index| self.classnames[index].2)
    }
    pub fn ammo_conversion(&self, source: ItemId, target: ItemId) -> Option<&AmmoConversion> {
        self.ammo_conversions
            .binary_search_by_key(&(source.0, target.0), |link| (link.source.0, link.target.0))
            .ok()
            .map(|index| &self.ammo_conversions[index])
    }
    pub fn bind_weapon(&mut self, id: WeaponId, recipe: WeaponRecipe) -> bool {
        let Some(weapon) =
            id.0.checked_sub(1)
                .and_then(|index| self.weapons.get_mut(index as usize))
        else {
            return false;
        };
        weapon.recipe = Some(recipe);
        true
    }
}
