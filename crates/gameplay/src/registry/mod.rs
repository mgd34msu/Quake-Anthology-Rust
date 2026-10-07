mod generated;
pub use generated::SOURCE_COUNTS;
use qa_core::{
    names::NameTable,
    primitives::{CallbackId, ItemId, ModuleId, NameId, WeaponId},
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
    pub module: ModuleId,
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
    pub module: ModuleId,
    pub native: u16,
    pub ammo: Option<ItemId>,
    pub fire_name: NameId,
    /// Catalogue entries become playable when the module binds its rule recipe.
    pub recipe: Option<WeaponRecipe>,
}

struct ItemSource {
    module: u16,
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
    native_items: Box<[(ModuleId, u16, ItemId)]>,
    native_weapons: Box<[(ModuleId, u16, WeaponId)]>,
    classnames: Box<[(ModuleId, NameId, ItemId)]>,
}

// Cross-module ammo equivalence is authored data. Pickup rules select the
// target weapon's ammo entry; no map/movement family is consulted.
const AMMO_GROUPS: &[&[(u16, &str)]] = &[
    &[(1, "item_shells"), (2, "ammo_shells"), (3, "ammo_shells")],
    &[
        (1, "item_spikes"),
        (2, "ammo_bullets"),
        (3, "ammo_bullets"),
        (3, "ammo_nails"),
        (3, "ammo_belt"),
    ],
    &[
        (1, "item_rockets"),
        (2, "ammo_rockets"),
        (3, "ammo_rockets"),
    ],
    &[
        (1, "item_rockets"),
        (2, "ammo_grenades"),
        (3, "ammo_grenades"),
        (3, "ammo_mines"),
    ],
    &[
        (1, "item_cells"),
        (2, "ammo_cells"),
        (3, "ammo_cells"),
        (3, "ammo_lightning"),
        (3, "ammo_bfg"),
    ],
    &[(2, "ammo_slugs"), (3, "ammo_slugs")],
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
        let source_id = |module: u16, classname: &str| {
            sources
                .iter()
                .position(|item| {
                    item.module == module && item.classname.eq_ignore_ascii_case(classname)
                })
                .map(|index| ItemId(index as u32 + 1))
        };
        for (index, source) in sources.iter().enumerate() {
            let id = ItemId(index as u32 + 1);
            items.push(ItemDef {
                id,
                module: ModuleId(source.module),
                native: source.native,
                classname: name(source.classname)?,
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
                    module: ModuleId(source.module),
                    native: source.native_weapon,
                    ammo: if source.ammo.is_empty() {
                        None
                    } else {
                        Some(source_id(source.module, source.ammo).ok_or(RegistryError::Ammo)?)
                    },
                    fire_name: name(source.fire)?,
                    recipe: None,
                });
            }
        }
        let mut native_items: Vec<_> = items
            .iter()
            .map(|item| (item.module, item.native, item.id))
            .collect();
        native_items.sort_unstable_by_key(|item| (item.0.0, item.1));
        if native_items
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
        {
            return Err(RegistryError::DuplicateItem);
        }
        let mut native_weapons: Vec<_> = weapons
            .iter()
            .map(|weapon| (weapon.module, weapon.native, weapon.id))
            .collect();
        native_weapons.sort_unstable_by_key(|weapon| (weapon.0.0, weapon.1));
        if native_weapons
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
        {
            return Err(RegistryError::DuplicateWeapon);
        }
        let mut classnames: Vec<_> = items
            .iter()
            .filter(|item| item.classname != NameId(0))
            .map(|item| (item.module, item.classname, item.id))
            .collect();
        classnames.sort_unstable_by_key(|item| (item.0.0, item.1.0));
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
    pub fn native_item(&self, module: ModuleId, number: u16) -> Option<ItemId> {
        self.native_items
            .binary_search_by_key(&(module.0, number), |item| (item.0.0, item.1))
            .ok()
            .map(|index| self.native_items[index].2)
    }
    pub fn native_weapon(&self, module: ModuleId, number: u16) -> Option<WeaponId> {
        self.native_weapons
            .binary_search_by_key(&(module.0, number), |weapon| (weapon.0.0, weapon.1))
            .ok()
            .map(|index| self.native_weapons[index].2)
    }
    pub fn classname(&self, module: ModuleId, name: NameId) -> Option<ItemId> {
        self.classnames
            .binary_search_by_key(&(module.0, name.0), |item| (item.0.0, item.1.0))
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
