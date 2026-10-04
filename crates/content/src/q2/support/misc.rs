//! Miscellaneous Q2-local shims for out-of-scope donor modules.
//!
//! Donor provenance (shapes only; owned by other lanes):
//! `src/core/common-parse.ts` (`parseQ2Token`),
//! `src/core/random/q2-rerelease.ts` (`Q2RereleaseRandomSource`),
//! `src/debug/shapes.ts` (`DebugLine`, `DebugShape`, `debugShapeLines`),
//! `src/text/world.ts` (`WorldTextInput`),
//! `src/world/gameplay/damage-modifier.ts`
//! (`applySourceDamageModifier`), `src/world/gameplay/pickups.ts`
//! (`previewPickupGrants`, `PickupGrantPlan`),
//! `src/world/gameplay/inventory.ts` (`inventoryGive`).

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, Vec3, Vec4};
use qa_core::numeric::float_to_wrapped_i32;

use crate::contract::{
    apply_source_damage_core, InventoryCountPolicy, InventoryEntry, ItemId, PickupAmmoGrant, PickupAmmoReceipt,
    PickupSupplyPreview, SourceCounterArithmetic,
};

use super::contracts::{DamageRequest, SourceDamageModifier};

// `src/core/common-parse.ts`.

/// Parse cursor over UTF-16 units (`LegacyParseState`).
///
/// Mirrors the `qa-net` cursor for the same donor; `qa-content` cannot
/// depend on `qa-net`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseState {
    /// Text units.
    pub data: Vec<u16>,
    /// Cursor position.
    pub index: usize,
}

impl ParseState {
    /// Build a cursor over text.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            data: text.encode_utf16().collect(),
            index: 0,
        }
    }

    fn unit(&self, index: usize) -> u16 {
        self.data.get(index).copied().unwrap_or(0)
    }
}

/// Default token ceiling (`Q2_TOKEN_MAX`).
pub const Q2_TOKEN_MAX: usize = 128;

/// Parse a token (`parseQ2Token`, `COM_Parse`).
///
/// Operates on UTF-16 units exactly like the donor, including the quirk
/// that an over-long unquoted word parses to the empty string.
pub fn parse_q2_token(state: &mut ParseState, max_token_chars: usize) -> String {
    loop {
        let mut unit = state.unit(state.index);
        while unit <= 32 {
            if unit == 0 {
                return String::new();
            }
            state.index += 1;
            unit = state.unit(state.index);
        }
        if unit == 47 && state.unit(state.index + 1) == 47 {
            while state.unit(state.index) != 0 && state.unit(state.index) != 10 {
                state.index += 1;
            }
            continue;
        }
        break;
    }
    let mut unit = state.unit(state.index);
    let mut token = Vec::new();
    if unit == 34 {
        state.index += 1;
        loop {
            unit = state.unit(state.index);
            state.index += 1;
            if unit == 34 || unit == 0 {
                return String::from_utf16_lossy(&token);
            }
            if token.len() < max_token_chars {
                token.push(unit);
            }
        }
    }
    loop {
        if token.len() < max_token_chars {
            token.push(unit);
        }
        state.index += 1;
        unit = state.unit(state.index);
        if unit <= 32 {
            break;
        }
    }
    if token.len() == max_token_chars {
        return String::new();
    }
    String::from_utf16_lossy(&token)
}

// `src/core/random/q2-rerelease.ts`.

/// Rerelease MT19937 random source (`Q2RereleaseRandomSource`).
///
/// The engine owns the stream; Q2 content only draws from it. One method
/// per donor overload.
pub trait Q2RereleaseRandomSource {
    /// Draw a raw word.
    fn next_uint32(&mut self) -> u32;
    /// Draw a unit float.
    fn float_unit(&mut self) -> f32;
    /// Draw a float below a maximum.
    fn float_max(&mut self, max_exclusive: f64) -> f32;
    /// Draw a float in a range.
    fn float_range(&mut self, min_inclusive: f64, max_exclusive: f64) -> f32;
    /// Draw a raw signed word.
    fn integer_any(&mut self) -> i32;
    /// Draw an integer below a maximum.
    fn integer_max(&mut self, max_exclusive: i32) -> i32;
    /// Draw an integer in a range.
    fn integer_range(&mut self, min_inclusive: i32, max_exclusive: i32) -> i32;
    /// Draw an inclusive millisecond bound.
    fn time_milliseconds(&mut self, min_inclusive: i64, max_inclusive: i64) -> i64;
}

// `src/debug/shapes.ts`.

/// Debug line segment (`DebugLine`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    /// Segment start.
    pub start: Vec3,
    /// Segment end.
    pub end: Vec3,
    /// Line color.
    pub color: Vec4,
    /// Whether depth testing applies.
    pub depth_test: bool,
}

/// Debug shape (`DebugShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugShape {
    /// Line segment.
    Line {
        /// Segment start.
        start: Vec3,
        /// Segment end.
        end: Vec3,
    },
    /// Point cross.
    Point {
        /// Cross origin.
        origin: Vec3,
        /// Cross size.
        size: f32,
    },
    /// Flat circle.
    Circle {
        /// Circle origin.
        origin: Vec3,
        /// Circle radius.
        radius: f32,
    },
    /// Sphere.
    Sphere {
        /// Sphere origin.
        origin: Vec3,
        /// Sphere radius.
        radius: f32,
    },
    /// Bounds box.
    Bounds {
        /// Box minimum.
        min: Vec3,
        /// Box maximum.
        max: Vec3,
    },
    /// Cylinder.
    Cylinder {
        /// Cylinder origin.
        origin: Vec3,
        /// Half height.
        half_height: f32,
        /// Cylinder radius.
        radius: f32,
    },
    /// Arrow with a cap.
    Arrow {
        /// Arrow start.
        start: Vec3,
        /// Arrow end.
        end: Vec3,
        /// Cap size.
        size: f32,
        /// Cap color.
        cap_color: Vec4,
    },
    /// Ray from an origin along a direction.
    Ray {
        /// Ray origin.
        origin: Vec3,
        /// Ray direction.
        direction: Vec3,
        /// Ray length.
        length: f32,
        /// Cap size.
        size: f32,
    },
}

/// Push one arrow's shaft and cap lines.
fn push_arrow(
    lines: &mut Vec<DebugLine>,
    start: Vec3,
    end: Vec3,
    size: f32,
    shaft_color: Vec4,
    cap_color: Vec4,
    depth_test: bool,
) {
    let delta = sub3(end, start);
    let length = length3(delta);
    let dir = normalize3(delta);
    let apex = if length > size {
        add3(start, scale3(dir, length - size))
    } else {
        end
    };
    if length > size {
        lines.push(DebugLine {
            start,
            end: apex,
            color: shaft_color,
            depth_test,
        });
    }
    let extent = if length > size { size } else { length };
    let tip = add3(apex, scale3(dir, extent));
    let rotated = Vec3 {
        x: dir.z,
        y: -dir.x,
        z: dir.y,
    };
    let right = normalize3(sub3(rotated, scale3(dir, dot3(rotated, dir))));
    for end in [
        tip,
        add3(apex, scale3(right, extent)),
        add3(apex, scale3(right, -extent)),
    ] {
        lines.push(DebugLine {
            start: apex,
            end,
            color: cap_color,
            depth_test,
        });
    }
}

/// Push one debug line segment.
fn push_line(lines: &mut Vec<DebugLine>, start: Vec3, end: Vec3, tint: Vec4, depth_test: bool) {
    lines.push(DebugLine {
        start,
        end,
        color: tint,
        depth_test,
    });
}

/// Expand a debug shape into line segments (`debugShapeLines`).
pub fn debug_shape_lines(shape: &DebugShape, color: Vec4, depth_test: bool) -> Vec<DebugLine> {
    let mut lines: Vec<DebugLine> = Vec::new();
    match *shape {
        DebugShape::Line { start, end } => push_line(&mut lines, start, end, color, depth_test),
        DebugShape::Point { origin, size } => {
            let half = size * 0.5;
            for axis in [
                Vec3 {
                    x: half,
                    y: 0.0,
                    z: 0.0,
                },
                Vec3 {
                    x: 0.0,
                    y: half,
                    z: 0.0,
                },
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: half,
                },
            ] {
                push_line(&mut lines, sub3(origin, axis), add3(origin, axis), color, depth_test);
            }
        }
        DebugShape::Bounds { min, max } => {
            let corner = |i: i32, z: f32| Vec3 {
                x: if i > 1 { min.x } else { max.x },
                y: if (i + 1) % 4 > 1 { min.y } else { max.y },
                z,
            };
            for i in 0..4 {
                push_line(&mut lines, corner(i, min.z), corner(i, max.z), color, depth_test);
                for z in [min.z, max.z] {
                    push_line(&mut lines, corner(i, z), corner((i + 1) % 4, z), color, depth_test);
                }
            }
        }
        DebugShape::Circle { origin, radius } => {
            let count = (5.0 + radius / 8.0).min(16.0).trunc() as i32;
            let point = |i: i32, z: f32| {
                let angle = f64::from(i) * std::f64::consts::PI * 2.0 / f64::from(count);
                Vec3 {
                    x: origin.x + angle.cos() as f32 * radius,
                    y: origin.y + angle.sin() as f32 * radius,
                    z,
                }
            };
            for i in 0..count {
                push_line(
                    &mut lines,
                    point(i, origin.z),
                    point((i + 1) % count, origin.z),
                    color,
                    depth_test,
                );
            }
        }
        DebugShape::Cylinder {
            origin,
            half_height,
            radius,
        } => {
            let count = (5.0 + radius / 8.0).min(16.0).trunc() as i32;
            let point = |i: i32, z: f32| {
                let angle = f64::from(i) * std::f64::consts::PI * 2.0 / f64::from(count);
                Vec3 {
                    x: origin.x + angle.cos() as f32 * radius,
                    y: origin.y + angle.sin() as f32 * radius,
                    z,
                }
            };
            for i in 0..count {
                let bottom = origin.z - half_height;
                let top = origin.z + half_height;
                push_line(
                    &mut lines,
                    point(i, bottom),
                    point((i + 1) % count, bottom),
                    color,
                    depth_test,
                );
                push_line(
                    &mut lines,
                    point(i, top),
                    point((i + 1) % count, top),
                    color,
                    depth_test,
                );
                push_line(&mut lines, point(i, bottom), point(i, top), color, depth_test);
            }
        }
        DebugShape::Sphere { origin, radius } => {
            let stacks = (4.0 + radius / 32.0).min(10.0).trunc() as i32;
            let slices = (6.0 + radius / 32.0).min(16.0).trunc() as i32;
            let ring = |stack: i32, slice: i32| {
                let phi = std::f64::consts::PI * f64::from(stack + 1) / f64::from(stacks);
                let theta = std::f64::consts::PI * 2.0 * f64::from(slice) / f64::from(slices);
                add3(
                    origin,
                    scale3(
                        Vec3 {
                            x: (phi.sin() * theta.cos()) as f32,
                            y: (phi.sin() * theta.sin()) as f32,
                            z: phi.cos() as f32,
                        },
                        radius,
                    ),
                )
            };
            let north = add3(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: radius,
                },
            );
            let south = sub3(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: radius,
                },
            );
            for i in 0..slices {
                let next = (i + 1) % slices;
                push_line(&mut lines, north, ring(0, next), color, depth_test);
                push_line(&mut lines, ring(0, next), ring(0, i), color, depth_test);
                push_line(&mut lines, ring(0, i), north, color, depth_test);
                push_line(&mut lines, south, ring(stacks - 2, i), color, depth_test);
                push_line(
                    &mut lines,
                    ring(stacks - 2, i),
                    ring(stacks - 2, next),
                    color,
                    depth_test,
                );
                push_line(&mut lines, ring(stacks - 2, next), south, color, depth_test);
            }
            for j in 0..stacks - 2 {
                for i in 0..slices {
                    let next = (i + 1) % slices;
                    push_line(&mut lines, ring(j, i), ring(j, next), color, depth_test);
                    push_line(&mut lines, ring(j, next), ring(j + 1, next), color, depth_test);
                    push_line(&mut lines, ring(j + 1, next), ring(j + 1, i), color, depth_test);
                    push_line(&mut lines, ring(j + 1, i), ring(j, i), color, depth_test);
                }
            }
        }
        DebugShape::Arrow {
            start,
            end,
            size,
            cap_color,
        } => push_arrow(&mut lines, start, end, size, color, cap_color, depth_test),
        DebugShape::Ray {
            origin,
            direction,
            length,
            size,
        } => push_arrow(
            &mut lines,
            origin,
            add3(origin, scale3(direction, length)),
            size,
            color,
            color,
            depth_test,
        ),
    }
    lines
}

// `src/text/world.ts`.

/// World text orientation (`WorldTextInput["orientation"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorldTextOrientation {
    /// Camera-facing billboard.
    Billboard,
    /// Fixed angles.
    Fixed {
        /// Fixed angles.
        angles: Vec3,
    },
}

/// World text font (`WorldTextInput["font"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorldTextFont {
    /// Classic font.
    Classic,
    /// Selected font.
    Selected,
}

/// World text submission (`WorldTextInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldTextInput {
    /// Text.
    pub text: String,
    /// Origin.
    pub origin: Vec3,
    /// Color.
    pub color: Vec4,
    /// Cell size.
    pub cell_size: f64,
    /// Cull factor against camera-forward depth.
    pub distance_cull_factor: Option<f64>,
    /// Orientation.
    pub orientation: WorldTextOrientation,
    /// Whether depth testing applies.
    pub depth_test: bool,
    /// Font.
    pub font: WorldTextFont,
}

// `src/world/gameplay/damage-modifier.ts`.

/// Apply a source damage modifier (`applySourceDamageModifier`).
///
/// Source kick stays independent; expired actor lifetimes use the
/// original world context.
pub fn apply_source_damage_modifier(
    request: DamageRequest,
    modifier: Option<&SourceDamageModifier>,
    is_live: &mut dyn FnMut(&ActorId) -> bool,
) -> DamageRequest {
    apply_source_damage_core(request, modifier, is_live)
}

// `src/world/gameplay/pickups.ts` + `src/world/gameplay/inventory.ts`.

/// Source-resolved grant plan (`PickupGrantPlan`).
#[derive(Debug, Clone, PartialEq)]
pub enum PickupGrantPlan {
    /// Weapon grant with ammo.
    Weapon {
        /// Weapon grants.
        weapons: Vec<PickupAmmoGrant>,
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
    },
    /// Ammo grant with weapon linkage.
    Ammo {
        /// Acceptance rule.
        acceptance: PickupAcceptance,
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
        /// Weapon linkage.
        weapons: PickupWeaponLink,
    },
}

/// Ammo acceptance rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupAcceptance {
    /// Any positive grant accepts.
    Positive,
    /// Any nonzero grant accepts.
    Nonzero,
}

/// Ammo-plan weapon linkage.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupWeaponLink {
    /// Grant weapons alongside ammo.
    Grant {
        /// Weapon grants.
        grants: Vec<PickupAmmoGrant>,
    },
    /// Weapons share the ammo pools.
    SharedAmmo {
        /// Shared weapon items.
        items: Vec<ItemId>,
    },
}

fn quantity(value: f64) -> f64 {
    if !value.is_finite() || value < 0.0 {
        panic!("Inventory quantity must be finite and nonnegative");
    }
    value
}

fn source_count(entry: &InventoryEntry, value: f64) -> f64 {
    if !value.is_finite() {
        panic!("Inventory counter must be finite");
    }
    match entry.count_policy.as_ref() {
        Some(InventoryCountPolicy::SourceCounter(arithmetic)) => match arithmetic {
            SourceCounterArithmetic::Binary32 => {
                let rounded = value as f32;
                if !rounded.is_finite() {
                    panic!("Inventory counter exceeds binary32 range");
                }
                f64::from(rounded)
            }
            SourceCounterArithmetic::Binary64 => value,
            SourceCounterArithmetic::Int32 => f64::from(float_to_wrapped_i32(value)),
        },
        _ => quantity(value),
    }
}

/// Inventory give transition (`InventoryGiveTransition`).
#[derive(Debug, Clone, PartialEq)]
pub enum InventoryGiveTransition {
    /// Count unchanged.
    Unchanged {
        /// Count given (zero).
        given: f64,
    },
    /// Entry written.
    Write {
        /// Updated entry.
        entry: InventoryEntry,
        /// Count given.
        given: f64,
    },
}

/// Give with source rounding (`inventoryGive`).
pub fn inventory_give(entry: &InventoryEntry, count: f64) -> InventoryGiveTransition {
    quantity(count);
    let given = count.min(0.0f64.max(entry.capacity - entry.count));
    if given == 0.0 {
        return InventoryGiveTransition::Unchanged { given: 0.0 };
    }
    let next = InventoryEntry {
        count: entry.count + source_count(entry, given),
        ..entry.clone()
    };
    let delta = next.count - entry.count;
    InventoryGiveTransition::Write {
        entry: next,
        given: delta,
    }
}

/// Preview pickup grants (`previewPickupGrants`).
pub fn preview_pickup_grants(inventory: &[InventoryEntry], plan: &PickupGrantPlan) -> PickupSupplyPreview {
    use std::collections::HashMap;

    let (weapons, ammo, acceptance) = match plan {
        PickupGrantPlan::Weapon { weapons, ammo } => (weapons.clone(), ammo.clone(), None),
        PickupGrantPlan::Ammo {
            acceptance,
            ammo,
            weapons,
        } => {
            let linked = match weapons {
                PickupWeaponLink::Grant { grants } => grants.clone(),
                PickupWeaponLink::SharedAmmo { items } => {
                    for item in items {
                        if !ammo.iter().any(|grant| &grant.item == item) {
                            panic!("Shared weapon {item} has no ammo grant");
                        }
                    }
                    Vec::new()
                }
            };
            (linked, ammo.clone(), Some((*acceptance, weapons)))
        }
    };
    let mut entries: HashMap<ItemId, InventoryEntry> = HashMap::new();
    for entry in inventory {
        entries.insert(entry.item.clone(), entry.clone());
    }
    for item in weapons
        .iter()
        .map(|grant| &grant.item)
        .chain(ammo.iter().map(|grant| &grant.item))
    {
        if !entries.contains_key(item) {
            panic!("Pickup destination {item} was not admitted");
        }
    }
    let mut give = |grant: &PickupAmmoGrant| -> PickupAmmoReceipt {
        let entry = entries
            .get(&grant.item)
            .unwrap_or_else(|| panic!("Pickup destination {} was not admitted", grant.item));
        let before = entry.count;
        match inventory_give(entry, grant.amount) {
            InventoryGiveTransition::Unchanged { given } => PickupAmmoReceipt {
                item: grant.item.clone(),
                before,
                given,
            },
            InventoryGiveTransition::Write { entry, given } => {
                entries.insert(grant.item.clone(), entry);
                PickupAmmoReceipt {
                    item: grant.item.clone(),
                    before,
                    given,
                }
            }
        }
    };
    match plan {
        PickupGrantPlan::Weapon { .. } => {
            let weapons = weapons.iter().map(&mut give).collect();
            let ammo = ammo.iter().map(&mut give).collect();
            PickupSupplyPreview {
                accepted: true,
                weapons,
                ammo,
            }
        }
        PickupGrantPlan::Ammo { .. } => {
            let (acceptance, link) = acceptance.expect("ammo plan");
            let ammo: Vec<PickupAmmoReceipt> = ammo.iter().map(&mut give).collect();
            let accepted = ammo.iter().any(|grant| match acceptance {
                PickupAcceptance::Positive => grant.given > 0.0,
                PickupAcceptance::Nonzero => grant.given != 0.0,
            });
            let weapons = if !accepted {
                Vec::new()
            } else {
                match link {
                    PickupWeaponLink::Grant { grants } => grants
                        .iter()
                        .map(|grant| {
                            ammo.iter()
                                .find(|receipt| receipt.item == grant.item)
                                .cloned()
                                .unwrap_or_else(|| give(grant))
                        })
                        .collect(),
                    PickupWeaponLink::SharedAmmo { items } => ammo
                        .iter()
                        .filter(|receipt| items.contains(&receipt.item))
                        .cloned()
                        .collect(),
                }
            };
            PickupSupplyPreview {
                accepted,
                ammo,
                weapons,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_bare_tokens() {
        let mut state = ParseState::new("{ \"a b\" // comment\nc }");
        assert_eq!(parse_q2_token(&mut state, 128), "{");
        assert_eq!(parse_q2_token(&mut state, 128), "a b");
        assert_eq!(parse_q2_token(&mut state, 128), "c");
        assert_eq!(parse_q2_token(&mut state, 128), "}");
        assert_eq!(parse_q2_token(&mut state, 128), "");
    }

    #[test]
    fn over_long_word_parses_empty() {
        let mut state = ParseState::new("abcdef");
        assert_eq!(parse_q2_token(&mut state, 3), "");
    }

    #[test]
    fn previews_weapon_and_ammo_grants() {
        let inventory = vec![
            InventoryEntry {
                item: "q2:weapon_shotgun".to_string(),
                count: 0.0,
                capacity: 1.0,
                count_policy: None,
            },
            InventoryEntry {
                item: "q2:ammo_shells".to_string(),
                count: 5.0,
                capacity: 100.0,
                count_policy: None,
            },
        ];
        let preview = preview_pickup_grants(
            &inventory,
            &PickupGrantPlan::Weapon {
                weapons: vec![PickupAmmoGrant {
                    item: "q2:weapon_shotgun".to_string(),
                    amount: 1.0,
                }],
                ammo: vec![PickupAmmoGrant {
                    item: "q2:ammo_shells".to_string(),
                    amount: 8.0,
                }],
            },
        );
        assert!(preview.accepted);
        assert_eq!(preview.weapons[0].given, 1.0);
        assert_eq!(preview.ammo[0].given, 8.0);
        let full = vec![InventoryEntry {
            item: "q2:ammo_shells".to_string(),
            count: 100.0,
            capacity: 100.0,
            count_policy: None,
        }];
        let denied = preview_pickup_grants(
            &full,
            &PickupGrantPlan::Ammo {
                acceptance: PickupAcceptance::Positive,
                ammo: vec![PickupAmmoGrant {
                    item: "q2:ammo_shells".to_string(),
                    amount: 8.0,
                }],
                weapons: PickupWeaponLink::SharedAmmo {
                    items: vec!["q2:ammo_shells".to_string()],
                },
            },
        );
        assert!(!denied.accepted);
    }

    #[test]
    fn damage_modifier_transforms_and_stamps_owner() {
        use qa_core::identity::{IdentityOwner, ProviderId};
        use qa_core::time::SourceTime;

        use crate::q2::support::contracts::{AttackCause, AttackProvenance, DamageDelivery};

        fn double(_: Option<&ActorId>, amount: f64) -> f64 {
            amount * 2.0
        }

        let owner = IdentityOwner::create("test").expect("owner");
        let attacker = owner.actor(7, 0);
        let dead = owner.actor(9, 0);
        let provider = ProviderId::new("q2", "test");
        let request = DamageRequest {
            attack: AttackProvenance {
                sequence: 1,
                time: SourceTime::Seconds(1.0),
                attacker: Some(attacker.clone()),
                inflictor: Some(dead),
                originating_projectile: None,
                weapon: None,
                weapon_provider: provider.clone(),
                damage_powerup_owner: None,
                combat_provider: provider.clone(),
                inventory_provider: provider.clone(),
                movement_provider: provider.clone(),
                cause: AttackCause::Q2 {
                    means_of_death: 1,
                    damage_flags: 0,
                    native: None,
                },
            },
            target: owner.actor(1, 0),
            amount: 10.0,
            knockback: 10.0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            delivery: DamageDelivery::Direct,
        };
        let modifier = SourceDamageModifier {
            owner: provider.clone(),
            transform: double,
        };
        let live_attacker = attacker.clone();
        let applied = apply_source_damage_modifier(request, Some(&modifier), &mut |actor| *actor == live_attacker);
        assert_eq!(applied.amount, 20.0);
        assert_eq!(applied.attack.attacker, Some(attacker));
        assert_eq!(applied.attack.inflictor, None);
        assert_eq!(applied.attack.damage_powerup_owner, Some(provider));
    }
}
