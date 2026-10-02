//! Q3 guest area-portal ownership against the selected world.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/guest-world.ts`
//! (`Q3GuestWorld`, `Q3GuestTopology`).
//!
//! The scene surface mirrors donor `SharedSceneQueries`
//! (`src/world/collision/index.ts`): no Rust home exists yet, so the
//! topology trait and the geometry/clip mirrors below carry the exact donor
//! shapes this module reads (canonical home: the `qa-world` collision port;
//! unify post-merge).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::Bounds;
use qa_world::save::value::{arr, int, obj, str as save_str, SaveJson, SaveReader};
use qa_world::WorldError;

/// Mirror of donor `DecodedWorld["kind"]` (`src/contracts/scene.ts`)
/// (canonical home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GuestGeometryKind {
    /// Quake BSP (no area-portal graph).
    Q1Bsp,
    /// Quake II BSP (portal contributions).
    Q2Bsp,
    /// Quake III BSP (area-pair references).
    Q3Bsp,
}

impl Q3GuestGeometryKind {
    /// Donor wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Q1Bsp => "q1-bsp",
            Self::Q2Bsp => "q2-bsp",
            Self::Q3Bsp => "q3-bsp",
        }
    }

    /// Parse a donor wire spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "q1-bsp" => Some(Self::Q1Bsp),
            "q2-bsp" => Some(Self::Q2Bsp),
            "q3-bsp" => Some(Self::Q3Bsp),
            _ => None,
        }
    }
}

/// Mirror of one donor `Q2WorldGeometry` area portal row (canonical home:
/// the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3GuestGeometryPortal {
    /// Area on the far side.
    pub other_area: i32,
    /// Portal identifier.
    pub portal: i32,
}

/// Mirror of one donor `Q2WorldGeometry` area row (canonical home: the
/// `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3GuestGeometryArea {
    /// First portal index.
    pub portals_first: usize,
    /// Portal count.
    pub portals_count: usize,
}

/// Mirror of the donor `DecodedWorld` projection the guest reads (canonical
/// home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestGeometry {
    /// Geometry kind.
    pub kind: Q3GuestGeometryKind,
    /// Area table (Quake II maps).
    pub areas: Vec<Q3GuestGeometryArea>,
    /// Area portal table (Quake II maps).
    pub area_portals: Vec<Q3GuestGeometryPortal>,
    /// Leaf count (donor `geometry.leaves.length`).
    pub leaf_count: usize,
}

/// Mirror of donor `SourceClipModels["world"]` box-leaf query result
/// (`src/world/collision/q3/clip-models.ts`) (canonical home: the
/// `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestBoxLeafnums {
    /// Overlapping leaves.
    pub leaves: Vec<i32>,
    /// Last leaf.
    pub last_leaf: i32,
}

/// Mirror of the donor native Q3 clip world projection the guest reads
/// (canonical home: the `qa-world` collision port); unify post-merge.
pub trait Q3GuestNativeClipWorld {
    /// Leaves overlapping bounds, up to `limit`.
    fn box_leafnums(&self, bounds: &Bounds, limit: usize) -> Q3GuestBoxLeafnums;
    /// Restore native portal state from a legacy checkpoint value.
    fn restore_portal_checkpoint(&self, value: &SaveJson) -> Result<(), WorldError>;
}

/// Mirror of donor `Q3GuestTopology`
/// (`Pick<SharedSceneQueries, "geometry" | "adjustAreaPortalState" |
/// "adjustAreaPortalContribution" | "nativeQ3ClipModels">`) (canonical home:
/// the `qa-world` collision port); unify post-merge.
pub trait Q3GuestTopology {
    /// Selected world geometry.
    fn geometry(&self) -> Q3GuestGeometry;
    /// Open or close the portal between two Quake III areas.
    fn adjust_area_portal_state(&self, first: i32, second: i32, open: bool);
    /// Contribute to a Quake II portal.
    fn adjust_area_portal_contribution(&self, portal: i32, delta: i32);
    /// Native Q3 clip models, when the selected world is a Quake III map.
    fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>>;
}

/// Q3 area-pair references resolve against the selected world's real portal
/// ownership.
pub struct Q3GuestWorld<S: Q3GuestTopology + ?Sized> {
    /// Selected world topology.
    pub scene: Rc<S>,
    references: RefCell<HashMap<(i32, i32), i32>>,
}

impl<S: Q3GuestTopology + ?Sized> Q3GuestWorld<S> {
    /// Wrap a scene topology.
    #[must_use]
    pub fn new(scene: Rc<S>) -> Self {
        Self {
            scene,
            references: RefCell::new(HashMap::new()),
        }
    }

    /// Open or close the portal between two areas, reference-counted.
    pub fn adjust_area_portal_state(&self, first: i32, second: i32, open: bool) {
        if first < 0 || second < 0 || first == second {
            return;
        }
        let geometry = self.scene.geometry();
        if geometry.kind == Q3GuestGeometryKind::Q1Bsp {
            return;
        }
        let key = (first.min(second), first.max(second));
        let previous = self.references.borrow().get(&key).copied().unwrap_or(0);
        let count = previous + if open { 1 } else { -1 };
        if count < 0 {
            panic!("Guest area portal reference underflow");
        }
        if geometry.kind == Q3GuestGeometryKind::Q3Bsp {
            self.scene.adjust_area_portal_state(first, second, open);
            self.references.borrow_mut().insert(key, count);
            return;
        }
        let area = geometry.areas.get(first as usize);
        if area.is_none() || geometry.areas.get(second as usize).is_none() {
            panic!("Guest area pair is outside the selected map");
        }
        let area = area.expect("checked area");
        self.references.borrow_mut().insert(key, count);
        for index in area.portals_first..area.portals_first + area.portals_count {
            let portal = geometry.area_portals.get(index);
            if portal.is_some_and(|portal| portal.other_area == second) {
                let portal = portal.expect("checked portal").portal;
                self.scene
                    .adjust_area_portal_contribution(portal, if open { 1 } else { -1 });
            }
        }
    }

    /// Capture the portal checkpoint value.
    #[must_use]
    pub fn capture_portal_checkpoint(&self) -> SaveJson {
        let mut references: Vec<((i32, i32), i32)> = self
            .references
            .borrow()
            .iter()
            .map(|(pair, count)| (*pair, *count))
            .collect();
        references.sort_by_key(|(pair, _)| *pair);
        obj(vec![
            ("kind", save_str(self.scene.geometry().kind.as_str())),
            (
                "references",
                arr(references
                    .into_iter()
                    .map(|((first, second), count)| {
                        obj(vec![
                            ("pair", save_str(&format!("{first}:{second}"))),
                            ("count", int(i64::from(count))),
                        ])
                    })
                    .collect()),
            ),
        ])
    }

    /// Release every retained portal reference.
    pub fn close(&self) {
        let references: Vec<((i32, i32), i32)> = self
            .references
            .borrow()
            .iter()
            .map(|(pair, count)| (*pair, *count))
            .collect();
        for ((first, second), count) in references {
            for _ in 0..count {
                self.adjust_area_portal_state(first, second, false);
            }
        }
        self.references.borrow_mut().clear();
    }

    /// Restore a portal checkpoint value.
    pub fn restore_portal_checkpoint(&self, value: &SaveJson) -> Result<(), WorldError> {
        let reader = SaveReader::at(value, "q3.guest.foreign-portals");
        let native = self.scene.native_q3_clip_models();
        let legacy = reader.field("kind").value.is_none();
        if let (true, Some(native)) = (legacy, native.as_ref()) {
            self.close();
            native.restore_portal_checkpoint(value)?;
            let areas = reader
                .field("areas")
                .list(|area| Ok::<_, WorldError>(area.path().to_string()))?
                .len();
            let portals = reader.field("portals").list(|portal| portal.integer(0))?;
            for first in 0..areas {
                for second in (first + 1)..areas {
                    let count = portals.get(first * areas + second).copied().unwrap_or(0);
                    if count > 0 {
                        let count = i32::try_from(count).map_err(|_| reader.fail("Invalid guest portal reference"))?;
                        self.references
                            .borrow_mut()
                            .insert((first as i32, second as i32), count);
                    }
                }
            }
            return Ok(());
        }
        let kind = reader.field("kind").string()?;
        if Q3GuestGeometryKind::parse(&kind) != Some(self.scene.geometry().kind) {
            return Err(reader
                .field("kind")
                .fail(&format!("expected {}", self.scene.geometry().kind.as_str())));
        }
        let mut pairs = std::collections::HashSet::new();
        let mut entries = Vec::new();
        for entry in reader
            .field("references")
            .list(|cell| Ok::<_, WorldError>((cell.field("pair").string()?, cell.field("count").integer(0)?)))?
        {
            let (pair, count) = entry;
            if !pairs.insert(pair.clone()) {
                return Err(reader.fail("Invalid guest portal reference"));
            }
            let Some((first, second)) = pair.split_once(':') else {
                return Err(reader.fail("Invalid guest portal reference"));
            };
            if first.is_empty()
                || second.is_empty()
                || !first.bytes().all(|byte| byte.is_ascii_digit())
                || !second.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(reader.fail("Invalid guest portal reference"));
            }
            let first: i64 = first.parse().map_err(|_| reader.fail("Invalid guest portal pair"))?;
            let second: i64 = second.parse().map_err(|_| reader.fail("Invalid guest portal pair"))?;
            if first >= second {
                return Err(reader.fail("Invalid guest portal pair"));
            }
            let first = i32::try_from(first).map_err(|_| reader.fail("Invalid guest portal pair"))?;
            let second = i32::try_from(second).map_err(|_| reader.fail("Invalid guest portal pair"))?;
            let geometry = self.scene.geometry();
            if geometry.kind == Q3GuestGeometryKind::Q1Bsp
                || geometry.kind == Q3GuestGeometryKind::Q2Bsp
                    && (geometry.areas.get(first as usize).is_none() || geometry.areas.get(second as usize).is_none())
            {
                return Err(reader.fail("Guest portal checkpoint belongs to another area table"));
            }
            let count = i32::try_from(count).map_err(|_| reader.fail("Invalid guest portal reference"))?;
            if count < 0 {
                return Err(reader.fail("Invalid guest portal reference"));
            }
            entries.push((pair, count, first, second));
        }
        self.close();
        for (_pair, count, first, second) in entries {
            for _ in 0..count {
                self.adjust_area_portal_state(first, second, true);
            }
            if count == 0 {
                self.references.borrow_mut().insert((first, second), 0);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeScene {
        geometry: Q3GuestGeometry,
        states: RefCell<Vec<(i32, i32, bool)>>,
        contributions: RefCell<Vec<(i32, i32)>>,
    }

    impl Q3GuestTopology for FakeScene {
        fn geometry(&self) -> Q3GuestGeometry {
            self.geometry.clone()
        }

        fn adjust_area_portal_state(&self, first: i32, second: i32, open: bool) {
            self.states.borrow_mut().push((first, second, open));
        }

        fn adjust_area_portal_contribution(&self, portal: i32, delta: i32) {
            self.contributions.borrow_mut().push((portal, delta));
        }

        fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>> {
            None
        }
    }

    fn q3_scene() -> FakeScene {
        FakeScene {
            geometry: Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q3Bsp,
                areas: Vec::new(),
                area_portals: Vec::new(),
                leaf_count: 4,
            },
            states: RefCell::new(Vec::new()),
            contributions: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn q3_pairs_reference_count() {
        let scene = Rc::new(q3_scene());
        let world = Q3GuestWorld::new(scene.clone());
        world.adjust_area_portal_state(1, 2, true);
        world.adjust_area_portal_state(2, 1, true);
        world.adjust_area_portal_state(1, 2, false);
        assert_eq!(scene.states.borrow().len(), 3);
        let image = world.capture_portal_checkpoint();
        assert_eq!(image.get("kind").unwrap(), &save_str("q3-bsp"));
        world.close();
        assert_eq!(scene.states.borrow().len(), 4);
        assert!(scene
            .states
            .borrow()
            .iter()
            .all(|(first, second, _)| (*first, *second) == (1, 2) || (*first, *second) == (2, 1)));
    }

    #[test]
    fn degenerate_and_q1_pairs_are_ignored() {
        let scene = Rc::new(FakeScene {
            geometry: Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q1Bsp,
                areas: Vec::new(),
                area_portals: Vec::new(),
                leaf_count: 1,
            },
            states: RefCell::new(Vec::new()),
            contributions: RefCell::new(Vec::new()),
        });
        let world = Q3GuestWorld::new(scene.clone());
        world.adjust_area_portal_state(0, 0, true);
        world.adjust_area_portal_state(-1, 2, true);
        world.adjust_area_portal_state(1, 2, true);
        assert!(scene.states.borrow().is_empty());
        assert!(scene.contributions.borrow().is_empty());
    }

    #[test]
    fn q2_pairs_fan_out_to_portals() {
        let scene = Rc::new(FakeScene {
            geometry: Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q2Bsp,
                areas: vec![
                    Q3GuestGeometryArea {
                        portals_first: 0,
                        portals_count: 2,
                    },
                    Q3GuestGeometryArea {
                        portals_first: 2,
                        portals_count: 0,
                    },
                ],
                area_portals: vec![
                    Q3GuestGeometryPortal {
                        other_area: 1,
                        portal: 7,
                    },
                    Q3GuestGeometryPortal {
                        other_area: 3,
                        portal: 8,
                    },
                ],
                leaf_count: 2,
            },
            states: RefCell::new(Vec::new()),
            contributions: RefCell::new(Vec::new()),
        });
        let world = Q3GuestWorld::new(scene.clone());
        world.adjust_area_portal_state(0, 1, true);
        assert_eq!(scene.contributions.borrow().as_slice(), &[(7, 1)]);
        world.close();
        assert_eq!(scene.contributions.borrow().as_slice(), &[(7, 1), (7, -1)]);
    }

    #[test]
    #[should_panic(expected = "Guest area portal reference underflow")]
    fn underflow_panics() {
        let scene = Rc::new(q3_scene());
        let world = Q3GuestWorld::new(scene);
        world.adjust_area_portal_state(1, 2, false);
    }

    #[test]
    fn checkpoint_round_trip() {
        let scene = Rc::new(q3_scene());
        let world = Q3GuestWorld::new(scene.clone());
        world.adjust_area_portal_state(1, 2, true);
        world.adjust_area_portal_state(1, 2, true);
        let image = world.capture_portal_checkpoint();
        let fresh = Q3GuestWorld::new(scene.clone());
        fresh.restore_portal_checkpoint(&image).unwrap();
        assert_eq!(scene.states.borrow().len(), 4);
        let bad = obj(vec![("kind", save_str("q2-bsp")), ("references", arr(Vec::new()))]);
        assert!(fresh.restore_portal_checkpoint(&bad).is_err());
        let bad_pair = obj(vec![
            ("kind", save_str("q3-bsp")),
            (
                "references",
                arr(vec![obj(vec![("pair", save_str("2:1")), ("count", int(1))])]),
            ),
        ]);
        assert!(fresh.restore_portal_checkpoint(&bad_pair).is_err());
    }
}
