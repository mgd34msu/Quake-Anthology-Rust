//! Quake II multiview-demo presentation helpers.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-mvd-presentation.ts`
//! (`q2MvdLayout`, `q2MvdVisibility`). The extended-limits layout is literal;
//! classic streams reuse [`q2_application_layout`](super::q2_layout::q2_application_layout).
//! MVD visibility projects recorded entities through the admitted Q2 BSP.
//! The BSP scene owner lives outside this wave (the merge-time remote
//! presentation), so the scene surface is a local [`Q2MvdScene`] trait the
//! owner implements; the projection math below is a line-for-line port.
//! `qa-net`'s [`MvdVisibility`](qa_net::q2_svc::MvdVisibility) is infallible,
//! so the donor's scene-violation throws become panics with the donor text.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use qa_core::math::{vec3, Bounds};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::{EntityState, PlayerState};
use qa_net::q2_solid::{q2_solid_encoding, unpack_q2_solid};
use qa_net::q2_svc::{MvdChannel, MvdVisibility};
use qa_net::q2_variants::{MvdProfile, MvdProtocol};

use super::q2_layout::{q2_application_layout, Q2ApplicationLayout, Q2LayoutError};

/// Map an MVD stream protocol to its wire identity.
fn mvd_protocol_identity(protocol: &MvdProtocol) -> ProtocolIdentity {
    match *protocol {
        MvdProtocol::Classic => ProtocolIdentity::Q2Classic,
        MvdProtocol::Q2Pro { revision } => ProtocolIdentity::Q2Q2pro {
            revision: u32::from(revision),
        },
        MvdProtocol::Rerelease => ProtocolIdentity::Q2Rerelease,
    }
}

/// Configstring layout for an MVD stream (`q2MvdLayout`).
pub fn q2_mvd_layout(profile: &MvdProfile) -> Result<Q2ApplicationLayout, Q2LayoutError> {
    if !profile.extended || profile.rerelease {
        return q2_application_layout(mvd_protocol_identity(&profile.protocol));
    }
    Ok(Q2ApplicationLayout {
        models: 62,
        sounds: 8254,
        images: 10302,
        lights: 12350,
        items: 12606,
        player_skins: 12862,
        max_models: 8192,
        max_sounds: 2048,
        max_images: 2048,
        max_config_strings: 13630,
        map_checksum: 61,
        max_clients: 60,
        air_accelerate: 59,
        n64_physics: None,
    })
}

/// BSP box-leaf query result (`boxLeaves`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MvdBoxLeaves {
    /// Leaves touched by the box.
    pub leaves: Vec<usize>,
    /// Overflow top node, when the box exceeded the leaf budget.
    pub topnode: Option<usize>,
}

/// BSP branch child (`geometry.nodes` entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2MvdChild {
    /// Leaf child.
    Leaf(usize),
    /// Branch child.
    Node(usize),
}

/// Admitted Q2 BSP scene surface for MVD projection.
///
/// The donor calls these on `createSceneQueries`; the merge-time remote
/// presentation implements them over its world scene. Hosts must return a
/// stable scene object: portal states are re-applied only when the recorded
/// portal bits change.
pub trait Q2MvdScene {
    /// Whether the admitted geometry is a Q2 BSP.
    fn is_q2_bsp(&self) -> bool;
    /// Leaf count.
    fn leaf_count(&self) -> usize;
    /// Area portal numbers.
    fn area_portals(&self) -> Vec<u32>;
    /// Open or close an area portal.
    fn set_area_portal_state(&mut self, portal: u32, open: bool);
    /// Leaf containing a world point.
    fn point_leaf(&self, point: [f64; 3]) -> usize;
    /// Cluster of a leaf (`-1` when the leaf has none).
    fn leaf_cluster(&self, leaf: usize) -> i32;
    /// Area of a leaf.
    fn leaf_area(&self, leaf: usize) -> i32;
    /// Whether two areas connect through open portals.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
    /// Whether a cluster sees another cluster on a channel.
    fn cluster_visible(&self, from: i32, to: i32, channel: MvdChannel) -> bool;
    /// Area visibility bits for an area.
    fn area_bits(&self, area: i32) -> Vec<u8>;
    /// Leaves touched by a box, with the overflow top node.
    fn box_leaves(&self, bounds: &Bounds, max_leaves: usize) -> Q2MvdBoxLeaves;
    /// Inline model bounds.
    fn model_bounds(&self, index: u32) -> Bounds;
    /// Children of a branch node, if the node exists.
    fn node_children(&self, node: usize) -> Option<[Q2MvdChild; 2]>;
}

/// MVD visibility host (`q2MvdVisibility` host argument).
pub trait Q2MvdSceneHost {
    /// Scene implementation.
    type Scene: Q2MvdScene;
    /// Borrow the admitted scene.
    fn scene(&mut self) -> &mut Self::Scene;
    /// Model path for a model index, if the model is known.
    fn model_path(&self, index: u16) -> Option<String>;
}

/// Whether a model path names an inline BSP model (`/^\*[0-9]+$/`).
fn inline_model(path: &str) -> Option<u32> {
    let number = path.strip_prefix('*')?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse::<u32>().ok()
}

/// MVD visibility projection over an admitted Q2 BSP (`q2MvdVisibility`).
pub struct Q2MvdVisibility<H: Q2MvdSceneHost> {
    profile: MvdProfile,
    host: RefCell<H>,
    applied_bits: RefCell<Option<Vec<u8>>>,
}

impl<H: Q2MvdSceneHost> Q2MvdVisibility<H> {
    /// Build a projection for a stream profile and scene host.
    pub fn new(profile: MvdProfile, host: H) -> Self {
        Self {
            profile,
            host: RefCell::new(host),
            applied_bits: RefCell::new(None),
        }
    }

    /// Player origin in world units.
    fn player_origin(&self, player: &PlayerState) -> [f64; 3] {
        if self.profile.rerelease {
            [
                f64::from(player.pmove.origin_f[0]),
                f64::from(player.pmove.origin_f[1]),
                f64::from(player.pmove.origin_f[2]),
            ]
        } else {
            [
                f64::from(player.pmove.origin[0]) / 8.0,
                f64::from(player.pmove.origin[1]) / 8.0,
                f64::from(player.pmove.origin[2]) / 8.0,
            ]
        }
    }

    /// Player eye position.
    fn view_origin(&self, player: &PlayerState) -> [f64; 3] {
        let point = self.player_origin(player);
        let extra = if self.profile.rerelease {
            f64::from(player.pmove.viewheight)
        } else {
            0.0
        };
        [
            point[0] + player.viewoffset[0],
            point[1] + player.viewoffset[1],
            point[2] + player.viewoffset[2] + extra,
        ]
    }
}

impl<H: Q2MvdSceneHost> MvdVisibility for Q2MvdVisibility<H> {
    fn entities(&self, entities: &[EntityState], player: &PlayerState, portal_bits: &[u8]) -> Vec<EntityState> {
        let mut host = self.host.borrow_mut();
        Self::apply_portals(&mut host, &self.applied_bits, portal_bits);
        let mut paths: HashMap<u16, Option<String>> = HashMap::new();
        for entity in entities {
            paths
                .entry(entity.modelindex)
                .or_insert_with(|| host.model_path(entity.modelindex));
        }
        let scene = &*host.scene();
        if !scene.is_q2_bsp() {
            panic!("MVD entity admission requires the admitted Q2 BSP");
        }
        let point = self.view_origin(player);
        let viewer = scene.point_leaf(point);
        let area = scene.leaf_area(viewer);
        let cluster = scene.leaf_cluster(viewer);
        let fat = scene.box_leaves(
            &Bounds {
                min: vec3(point[0] as f32 - 8.0, point[1] as f32 - 8.0, point[2] as f32 - 8.0),
                max: vec3(point[0] as f32 + 8.0, point[1] as f32 + 8.0, point[2] as f32 + 8.0),
            },
            64,
        );
        let fat_clusters: HashSet<i32> = fat.leaves.iter().map(|leaf| scene.leaf_cluster(*leaf)).collect();
        let encoding = q2_solid_encoding(mvd_protocol_identity(&self.profile.protocol), self.profile.extended)
            .expect("MVD profile requires a Quake II protocol");
        entities
            .iter()
            .filter(|entity| {
                let shadow = (entity.renderfx & 16384) != 0;
                let beam = (entity.renderfx & 128) != 0;
                if entity.number == 0
                    || (entity.modelindex == 0
                        && entity.effects == 0
                        && entity.sound == 0
                        && entity.event == 0
                        && !shadow)
                {
                    return false;
                }
                let mut bounds = match paths
                    .get(&entity.modelindex)
                    .and_then(|path| path.as_deref())
                    .and_then(inline_model)
                {
                    Some(index) => scene.model_bounds(index),
                    None if entity.solid != 0 && entity.solid != 31 => unpack_q2_solid(entity.solid, encoding),
                    None => Bounds {
                        min: vec3(0.0, 0.0, 0.0),
                        max: vec3(0.0, 0.0, 0.0),
                    },
                };
                if entity.solid == 31 && entity.angles.iter().any(|angle| *angle != 0.0) {
                    let radius = bounds
                        .min
                        .x
                        .abs()
                        .max(bounds.min.y.abs())
                        .max(bounds.min.z.abs())
                        .max(bounds.max.x.abs().max(bounds.max.y.abs()).max(bounds.max.z.abs()));
                    bounds = Bounds {
                        min: vec3(-radius, -radius, -radius),
                        max: vec3(radius, radius, radius),
                    };
                }
                let (x, y, z) = (entity.origin[0], entity.origin[1], entity.origin[2]);
                let linked = scene.box_leaves(
                    &Bounds {
                        min: vec3(
                            (x + f64::from(bounds.min.x) - 1.0) as f32,
                            (y + f64::from(bounds.min.y) - 1.0) as f32,
                            (z + f64::from(bounds.min.z) - 1.0) as f32,
                        ),
                        max: vec3(
                            (x + f64::from(bounds.max.x) + 1.0) as f32,
                            (y + f64::from(bounds.max.y) + 1.0) as f32,
                            (z + f64::from(bounds.max.z) + 1.0) as f32,
                        ),
                    },
                    128,
                );
                let mut first_area = 0;
                let mut second_area = 0;
                for leaf in &linked.leaves {
                    let next = scene.leaf_area(*leaf);
                    if next != 0 {
                        if first_area != 0 && first_area != next {
                            second_area = next;
                        } else {
                            first_area = next;
                        }
                    }
                }
                if !scene.areas_connected(area, first_area) && !scene.areas_connected(area, second_area) {
                    return false;
                }
                let clusters: Vec<i32> = {
                    let mut seen = HashSet::new();
                    linked
                        .leaves
                        .iter()
                        .map(|leaf| scene.leaf_cluster(*leaf))
                        .filter(|value| *value != -1 && seen.insert(*value))
                        .collect()
                };
                let admitted = |target: i32, kind: MvdChannel| match kind {
                    MvdChannel::Phs => scene.cluster_visible(cluster, target, MvdChannel::Phs),
                    MvdChannel::Pvs => fat_clusters
                        .iter()
                        .any(|source| scene.cluster_visible(*source, target, MvdChannel::Pvs)),
                };
                fn reachable(scene: &dyn Q2MvdScene, node: usize, admitted: &dyn Fn(i32) -> bool) -> bool {
                    let Some(children) = scene.node_children(node) else {
                        panic!("MVD entity topnode is outside the BSP");
                    };
                    children.iter().any(|child| match child {
                        Q2MvdChild::Leaf(leaf) => admitted(scene.leaf_cluster(*leaf)),
                        Q2MvdChild::Node(index) => reachable(scene, *index, admitted),
                    })
                }
                let in_mask = |kind: MvdChannel| {
                    if linked.leaves.len() < 128 && clusters.len() <= 16 {
                        return clusters.iter().any(|target| admitted(*target, kind));
                    }
                    let Some(topnode) = linked.topnode else {
                        panic!("MVD overflow entity has no BSP topnode");
                    };
                    reachable(scene, topnode, &|target| admitted(target, kind))
                };
                if !in_mask(if beam || entity.sound != 0 || shadow {
                    MvdChannel::Phs
                } else {
                    MvdChannel::Pvs
                }) {
                    return false;
                }
                let distance = (point[0] - x).hypot(point[1] - y).hypot(point[2] - z);
                if entity.sound != 0 {
                    let attenuation = entity.loop_attenuation;
                    let multiplier = if attenuation == -1.0 {
                        0.0
                    } else if attenuation > 0.0 && attenuation != 3.0 {
                        attenuation * 0.0006
                    } else {
                        0.003
                    };
                    if (distance - 80.0) * multiplier > 1.0
                        && (entity.modelindex == 0 || (!beam && !in_mask(MvdChannel::Pvs)))
                    {
                        return false;
                    }
                } else if entity.modelindex == 0 && !shadow && distance > 400.0 {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }

    fn visible(&self, leaf: u16, channel: MvdChannel, player: &PlayerState, portal_bits: &[u8]) -> bool {
        self.visible_leaf(usize::from(leaf), channel, player, portal_bits)
    }

    fn area_bits(&self, player: &PlayerState, portal_bits: &[u8]) -> Vec<u8> {
        let mut host = self.host.borrow_mut();
        Self::apply_portals(&mut host, &self.applied_bits, portal_bits);
        let scene = &*host.scene();
        if !scene.is_q2_bsp() {
            panic!("MVD visibility requires the admitted Q2 BSP");
        }
        let point = self.view_origin(player);
        scene.area_bits(scene.leaf_area(scene.point_leaf(point)))
    }

    fn sound_audible(&self, origin: [f64; 3], player: &PlayerState, portal_bits: &[u8]) -> bool {
        let leaf = self.host.borrow_mut().scene().point_leaf(origin);
        self.visible_leaf(leaf, MvdChannel::Phs, player, portal_bits)
    }

    fn sound_origin(&self, entity: &EntityState) -> [f64; 3] {
        let (x, y, z) = (entity.origin[0], entity.origin[1], entity.origin[2]);
        if entity.solid != 31 {
            return [x, y, z];
        }
        let host = self.host.borrow();
        let path = host.model_path(entity.modelindex);
        drop(host);
        let Some(index) = path.as_deref().and_then(inline_model) else {
            panic!("MVD brush sound has no inline model");
        };
        let bounds = self.host.borrow_mut().scene().model_bounds(index);
        [
            x + f64::from(bounds.min.x + bounds.max.x) / 2.0,
            y + f64::from(bounds.min.y + bounds.max.y) / 2.0,
            z + f64::from(bounds.min.z + bounds.max.z) / 2.0,
        ]
    }
}

impl<H: Q2MvdSceneHost> Q2MvdVisibility<H> {
    /// Apply recorded portal states unless the bits match the last call.
    ///
    /// `CM_SetPortalStates` opens portals beyond the supplied recorded byte
    /// count.
    fn apply_portals(host: &mut H, applied: &RefCell<Option<Vec<u8>>>, bits: &[u8]) {
        if !host.scene().is_q2_bsp() || applied.borrow().as_deref() == Some(bits) {
            return;
        }
        let portals: HashSet<u32> = host.scene().area_portals().into_iter().collect();
        for portal in portals {
            let open = bits
                .get((portal >> 3) as usize)
                .is_none_or(|value| value & (1u8 << (portal & 7)) != 0);
            host.scene().set_area_portal_state(portal, open);
        }
        *applied.borrow_mut() = Some(bits.to_vec());
    }

    /// Leaf visibility with the donor's range check.
    fn visible_leaf(&self, leaf: usize, channel: MvdChannel, player: &PlayerState, portal_bits: &[u8]) -> bool {
        let mut host = self.host.borrow_mut();
        Self::apply_portals(&mut host, &self.applied_bits, portal_bits);
        let scene = &*host.scene();
        if !scene.is_q2_bsp() || leaf >= scene.leaf_count() {
            panic!("MVD multicast leaf is outside the admitted BSP");
        }
        let viewer = scene.point_leaf(self.player_origin(player));
        let cluster = scene.leaf_cluster(viewer);
        cluster != -1
            && scene.areas_connected(scene.leaf_area(leaf), scene.leaf_area(viewer))
            && scene.cluster_visible(scene.leaf_cluster(leaf), cluster, channel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::q2::{PmoveState, MAX_STATS_STORAGE};
    use qa_net::q2_variants::MvdProtocol;

    struct FakeScene {
        portals: Vec<u32>,
        states: std::collections::HashMap<u32, bool>,
        clusters: Vec<i32>,
        areas: Vec<i32>,
        visible: bool,
        connected: bool,
        boxed: Vec<usize>,
    }

    impl FakeScene {
        fn new() -> Self {
            Self {
                portals: vec![3],
                states: std::collections::HashMap::new(),
                clusters: vec![0, 1],
                areas: vec![1, 1],
                visible: true,
                connected: true,
                boxed: vec![0],
            }
        }
    }

    impl Q2MvdScene for FakeScene {
        fn is_q2_bsp(&self) -> bool {
            true
        }

        fn leaf_count(&self) -> usize {
            self.clusters.len()
        }

        fn area_portals(&self) -> Vec<u32> {
            self.portals.clone()
        }

        fn set_area_portal_state(&mut self, portal: u32, open: bool) {
            self.states.insert(portal, open);
        }

        fn point_leaf(&self, _point: [f64; 3]) -> usize {
            0
        }

        fn leaf_cluster(&self, leaf: usize) -> i32 {
            self.clusters[leaf]
        }

        fn leaf_area(&self, leaf: usize) -> i32 {
            self.areas[leaf]
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            self.connected
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _channel: MvdChannel) -> bool {
            self.visible
        }

        fn area_bits(&self, _area: i32) -> Vec<u8> {
            vec![0b11]
        }

        fn box_leaves(&self, _bounds: &Bounds, _max_leaves: usize) -> Q2MvdBoxLeaves {
            Q2MvdBoxLeaves {
                leaves: self.boxed.clone(),
                topnode: None,
            }
        }

        fn model_bounds(&self, _index: u32) -> Bounds {
            Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            }
        }

        fn node_children(&self, _node: usize) -> Option<[Q2MvdChild; 2]> {
            None
        }
    }

    struct FakeHost {
        scene: FakeScene,
        models: std::collections::HashMap<u16, String>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                scene: FakeScene::new(),
                models: std::collections::HashMap::new(),
            }
        }
    }

    impl Q2MvdSceneHost for FakeHost {
        type Scene = FakeScene;

        fn scene(&mut self) -> &mut Self::Scene {
            &mut self.scene
        }

        fn model_path(&self, index: u16) -> Option<String> {
            self.models.get(&index).cloned()
        }
    }

    fn profile() -> MvdProfile {
        MvdProfile {
            revision: 2010,
            flags: 0,
            protocol: MvdProtocol::Classic,
            rerelease: false,
            extended: false,
            v2: false,
            fog: false,
            max_config_strings: 2080,
            max_clients_index: 30,
            max_entities: 1024,
        }
    }

    fn player() -> PlayerState {
        PlayerState {
            clientnum: 0,
            pmove: PmoveState {
                pm_type: 0,
                origin: [0, 0, 0],
                velocity: [0, 0, 0],
                pm_flags: 0,
                pm_time: 0,
                gravity: 0,
                delta_angles: [0, 0, 0],
                viewheight: 22,
                origin_f: [0.0, 0.0, 0.0],
                velocity_f: [0.0, 0.0, 0.0],
                delta_angles_f: [0.0, 0.0, 0.0],
                delta_angle_float: false,
            },
            viewangles: [0.0, 0.0, 0.0],
            viewoffset: [0.0, 0.0, 22.0],
            kick_angles: [0.0, 0.0, 0.0],
            gunangles: [0.0, 0.0, 0.0],
            gunoffset: [0.0, 0.0, 0.0],
            gunindex: 0,
            gunskin: 0,
            gunframe: 0,
            gunrate: 0,
            blend: [0.0, 0.0, 0.0, 0.0],
            damage_blend: [0.0, 0.0, 0.0, 0.0],
            fov: 90,
            rdflags: 0,
            stats: [0; MAX_STATS_STORAGE],
            team_id: 0,
            fog: Default::default(),
        }
    }

    #[test]
    fn mvd_layout_selects_classic_or_extended() {
        let classic = q2_mvd_layout(&profile()).expect("classic");
        assert_eq!(classic.models, 32);
        let mut extended = profile();
        extended.extended = true;
        let layout = q2_mvd_layout(&extended).expect("extended");
        assert_eq!(
            (
                layout.sounds,
                layout.images,
                layout.player_skins,
                layout.max_config_strings,
                layout.n64_physics
            ),
            (8254, 10302, 12862, 13630, None)
        );
        extended.rerelease = true;
        extended.protocol = MvdProtocol::Rerelease;
        let rerelease = q2_mvd_layout(&extended).expect("rerelease");
        assert_eq!(rerelease.models, 62);
    }

    #[test]
    fn visibility_applies_portal_bits_and_checks_range() {
        let view = Q2MvdVisibility::new(profile(), FakeHost::new());
        assert!(view.visible(1, MvdChannel::Pvs, &player(), &[0b1111]));
        assert_eq!(view.host.borrow().scene.states.get(&3), Some(&true));
        assert_eq!(view.area_bits(&player(), &[0b1111]), vec![0b11]);
    }

    #[test]
    #[should_panic(expected = "MVD multicast leaf is outside the admitted BSP")]
    fn visibility_rejects_outside_leaf() {
        let view = Q2MvdVisibility::new(profile(), FakeHost::new());
        view.visible(9, MvdChannel::Pvs, &player(), &[]);
    }

    #[test]
    fn entities_filter_world_and_admit_visible() {
        let view = Q2MvdVisibility::new(profile(), FakeHost::new());
        let world = EntityState {
            number: 0,
            ..EntityState::default()
        };
        let mut shown = EntityState {
            number: 7,
            ..EntityState::default()
        };
        shown.modelindex = 2;
        shown.origin = [10.0, 0.0, 0.0];
        let kept = view.entities(&[world, shown], &player(), &[]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].number, 7);
    }

    #[test]
    fn entities_drop_unconnected_areas() {
        let mut host = FakeHost::new();
        host.scene.connected = false;
        let view = Q2MvdVisibility::new(profile(), host);
        let mut shown = EntityState {
            number: 7,
            ..EntityState::default()
        };
        shown.modelindex = 2;
        assert!(view.entities(&[shown], &player(), &[]).is_empty());
    }

    #[test]
    fn sound_origin_centers_brush_models() {
        let mut host = FakeHost::new();
        host.models.insert(5, "*2".to_string());
        let view = Q2MvdVisibility::new(profile(), host);
        let point = EntityState {
            number: 3,
            solid: 0,
            ..EntityState::default()
        };
        assert_eq!(view.sound_origin(&point), [0.0, 0.0, 0.0]);
        let mut brush = EntityState {
            number: 4,
            solid: 31,
            ..EntityState::default()
        };
        brush.modelindex = 5;
        brush.origin = [10.0, 0.0, 0.0];
        assert_eq!(view.sound_origin(&brush), [10.0, 0.0, 0.0]);
        assert!(view.sound_audible([10.0, 0.0, 0.0], &player(), &[]));
    }

    #[test]
    #[should_panic(expected = "MVD brush sound has no inline model")]
    fn sound_origin_rejects_point_brushes() {
        let view = Q2MvdVisibility::new(profile(), FakeHost::new());
        let brush = EntityState {
            number: 4,
            solid: 31,
            ..EntityState::default()
        };
        view.sound_origin(&brush);
    }
}
