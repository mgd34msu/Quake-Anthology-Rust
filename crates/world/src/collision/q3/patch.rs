//! Independent patch collision translated from id Software's `cm_patch.c`
//! and `cm_polylib.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/patch.ts`.
//!
//! Generation keeps the donor's binary64 grid math and rounds once at the
//! plane/bounds store. Source-hunk callers still observe the allocation
//! calls, but records decode eagerly: no Rust guest reads raw hunk cells,
//! so live `DataView` getters would only add borrow noise.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use qa_core::math::{dot3, length3, vec3, Bounds, Plane, Vec3};

use super::allocation::HunkAllocation;
use super::polylib::{CollisionWinding, CollisionWindingLibrary, CollisionWindingMemory, ON_EPSILON};
use crate::error::WorldError;

/// Bevel border plane reference inside a facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchBorder {
    /// Plane index.
    pub plane: usize,
    /// Border faces inward.
    pub inward: bool,
    /// Border skips automatic adjustment.
    pub no_adjust: bool,
}

/// One patch facet: a surface plane plus its bevel borders.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchFacet {
    /// Surface plane index.
    pub surface: usize,
    /// Bevel borders.
    pub borders: Vec<PatchBorder>,
}

/// Generated patch plane with its retained source signbits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatchPlane {
    /// Unit normal.
    pub normal: Vec3,
    /// Distance from the origin.
    pub distance: f32,
    /// Source sign bits.
    pub signbits: i32,
}

impl PatchPlane {
    fn as_plane(&self) -> Plane {
        Plane {
            normal: self.normal,
            distance: self.distance,
        }
    }
}

/// Collision records for one quadratic patch surface.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchCollide {
    /// Deduplicated planes.
    pub planes: Vec<PatchPlane>,
    /// Facets over the planes.
    pub facets: Vec<PatchFacet>,
    /// Grid bounds expanded by one unit.
    pub bounds: Bounds,
}

/// Sweep body for patch traces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatchShape {
    /// Point sweep with box offsets.
    Point {
        /// Box minimum corner.
        mins: Vec3,
        /// Box extents.
        extents: Vec3,
    },
    /// Box sweep.
    Box {
        /// Box minimum corner.
        mins: Vec3,
        /// Box extents.
        extents: Vec3,
    },
    /// Capsule sweep.
    Capsule {
        /// Box extents.
        extents: Vec3,
        /// Capsule radius.
        radius: f32,
        /// End-cap offset from the center.
        offset: Vec3,
    },
}

/// Patch generation allocation site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchAllocSite {
    /// Patch header (`CM_GeneratePatchCollide`).
    Generate,
    /// Facet records (`CM_PatchCollideFromGrid:facets`).
    Facets,
    /// Plane records (`CM_PatchCollideFromGrid:planes`).
    Planes,
}

impl PatchAllocSite {
    /// Source allocation site name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Generate => "CM_GeneratePatchCollide",
            Self::Facets => "CM_PatchCollideFromGrid:facets",
            Self::Planes => "CM_PatchCollideFromGrid:planes",
        }
    }
}

/// Source-hunk hook observing patch generation allocations.
pub trait PatchAllocator {
    /// Reserve `bytes` for `site`.
    fn allocate(&self, site: PatchAllocSite, bytes: usize) -> HunkAllocation;
}

/// Debug polygon sink: color, point count, points.
pub type CollisionDebugPolygon<'a> = dyn Fn(i32, usize, &[Vec3]) + 'a;

/// Host services borrowed by [`CollisionDebugSurface`].
#[derive(Clone)]
pub struct CollisionDebugHost {
    /// Shared cvar registry.
    pub cvars: Rc<RefCell<qa_core::cvar::CvarRegistry>>,
    /// Engine print.
    pub print: Rc<dyn Fn(&str)>,
    /// Developer print.
    pub developer_print: Rc<dyn Fn(&str)>,
    /// Optional winding zone.
    pub windings: Option<Rc<dyn CollisionWindingMemory>>,
}

impl std::fmt::Debug for CollisionDebugHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollisionDebugHost")
            .field("windings", &self.windings.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct RecordedHit {
    patch: PatchCollide,
    facet: usize,
}

/// The common CM owner borrows this state across server and client
/// collision worlds.
#[derive(Debug)]
pub struct CollisionDebugSurface {
    /// Shared winding zone.
    pub windings: CollisionWindingLibrary,
    /// Total patch blocks generated.
    pub c_total_patch_blocks: Cell<i32>,
    /// Total patch surfaces (retained for the source ABI).
    pub c_total_patch_surfaces: Cell<i32>,
    /// Total patch edges (retained for the source ABI).
    pub c_total_patch_edges: Cell<i32>,
    hit: RefCell<Option<RecordedHit>>,
    debug_block: Cell<bool>,
    debug_block_points: RefCell<[Vec3; 4]>,
    point_update_registered: Cell<bool>,
    shape_update_registered: Cell<bool>,
    size_registered: Cell<bool>,
    host: CollisionDebugHost,
}

impl CollisionDebugSurface {
    /// Borrow host services.
    #[must_use]
    pub fn new(host: CollisionDebugHost) -> Self {
        Self {
            windings: CollisionWindingLibrary::new(host.windings.clone()),
            c_total_patch_blocks: Cell::new(0),
            c_total_patch_surfaces: Cell::new(0),
            c_total_patch_edges: Cell::new(0),
            hit: RefCell::new(None),
            debug_block: Cell::new(false),
            debug_block_points: RefCell::new([vec3(0.0, 0.0, 0.0); 4]),
            point_update_registered: Cell::new(false),
            shape_update_registered: Cell::new(false),
            size_registered: Cell::new(false),
            host,
        }
    }

    /// Print a winding through the host.
    pub fn print_winding(&self, winding: &CollisionWinding) -> Result<(), WorldError> {
        let host = &self.host;
        self.windings.pw(winding, &|text| (host.print)(text))
    }

    /// Engine print.
    pub fn print(&self, text: &str) {
        (self.host.print)(text);
    }

    /// Developer print.
    pub fn developer_print(&self, text: &str) {
        (self.host.developer_print)(text);
    }

    /// Forget the recorded debug surface on level change.
    pub fn clear_level_patches(&self) {
        *self.hit.borrow_mut() = None;
    }

    /// Record a patch impact for `r_debugSurface` drawing.
    pub fn record_trace(&self, patch: &PatchCollide, facet: usize, is_point: bool) -> Result<(), WorldError> {
        let registered = if is_point {
            &self.point_update_registered
        } else {
            &self.shape_update_registered
        };
        if !registered.get() {
            self.host
                .cvars
                .borrow_mut()
                .register("r_debugSurfaceUpdate", "1", 0)
                .map_err(|error| WorldError::BadCollisionRecord(error.to_string()))?;
            registered.set(true);
        }
        if self.cvar("r_debugSurfaceUpdate")?.integer_value != 0 {
            *self.hit.borrow_mut() = Some(RecordedHit {
                patch: patch.clone(),
                facet,
            });
        }
        Ok(())
    }

    /// Record a mixed-border block once, warning every time.
    pub fn record_mixed_border(&self, points: [Vec3; 4]) {
        self.developer_print("WARNING: CM_SetBorderInward: mixed plane sides\n");
        if self.debug_block.get() {
            return;
        }
        self.debug_block.set(true);
        *self.debug_block_points.borrow_mut() = points;
    }

    /// `CM_DrawDebugSurface`'s `r_debugSurface == 1` branch; the engine owns
    /// bot routing.
    pub fn draw(&self, draw_poly: &CollisionDebugPolygon<'_>) -> Result<(), WorldError> {
        let hit = self.hit.borrow().clone();
        let Some(hit) = hit else {
            return Ok(());
        };
        if !self.size_registered.get() {
            self.host
                .cvars
                .borrow_mut()
                .register("cm_debugSize", "2", 0)
                .map_err(|error| WorldError::BadCollisionRecord(error.to_string()))?;
            self.size_registered.set(true);
        }
        for (facet_index, facet) in hit.patch.facets.iter().enumerate() {
            let mut planes: Vec<PatchBorder> = facet.borders.clone();
            planes.push(PatchBorder {
                plane: facet.surface,
                inward: false,
                no_adjust: false,
            });
            for border in &planes {
                let original = patch_at(&hit.patch.planes, border.plane)?;
                let mut winding = Some(self.windings.base_for_plane(&self.debug_plane(
                    original,
                    border.inward,
                    1.0,
                )?)?);
                for clip in &planes {
                    if winding.is_none() {
                        break;
                    }
                    if clip.plane == border.plane {
                        continue;
                    }
                    let clip_plane = patch_at(&hit.patch.planes, clip.plane)?;
                    let chopped = self.windings.chop_in_place(
                        winding.take().expect("winding checked above"),
                        &self.debug_plane(clip_plane, !clip.inward, -1.0)?,
                        0.1f32,
                    )?;
                    winding = chopped;
                }
                if let Some(winding) = winding {
                    let color = if facet_index == hit.facet { 4 } else { 1 };
                    let points = winding.points()?;
                    draw_poly(color, points.len(), &points);
                    self.windings.free(winding)?;
                } else {
                    self.print("winding chopped away by border planes\n");
                }
            }
        }
        let points = *self.debug_block_points.borrow();
        draw_poly(2, 3, &[points[0], points[1], points[2]]);
        draw_poly(2, 3, &[points[2], points[3], points[0]]);
        Ok(())
    }

    fn debug_plane(&self, plane: &PatchPlane, flip: bool, direction: f32) -> Result<Plane, WorldError> {
        let normal = if flip {
            qa_core::math::sub3(vec3(0.0, 0.0, 0.0), plane.normal)
        } else {
            plane.normal
        };
        let corner = vec3(
            if normal.x > 0.0 { 15.0 } else { -15.0 },
            if normal.y > 0.0 { 15.0 } else { -15.0 },
            if normal.z > 0.0 { 28.0 } else { -28.0 },
        );
        let size = self.cvar("cm_debugSize")?.numeric_value;
        let base = if flip { -plane.distance } else { plane.distance };
        let distance = base + direction * size;
        let scaled = qa_core::math::scale3(normal, -1.0);
        let extra = patch_dot64(
            [f64::from(corner.x), f64::from(corner.y), f64::from(corner.z)],
            [f64::from(scaled.x), f64::from(scaled.y), f64::from(scaled.z)],
        );
        Ok(Plane {
            normal,
            distance: distance + direction * extra.abs() as f32,
        })
    }

    fn cvar(&self, name: &str) -> Result<qa_core::cvar::CvarSnapshot, WorldError> {
        self.host
            .cvars
            .borrow()
            .get(name)
            .ok_or_else(|| WorldError::BadCollisionRecord(format!("Collision debug cvar {name} is not registered")))
    }
}

/// Patch trace impact: fraction plus the source plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatchHit {
    /// Impact fraction.
    pub fraction: f32,
    /// Impact plane.
    pub plane: Plane,
}

fn patch_at<T>(values: &[T], index: usize) -> Result<&T, WorldError> {
    values
        .get(index)
        .ok_or_else(|| WorldError::BadCollisionRecord(format!("patch index {index} out of range")))
}

fn negate(plane: &PatchPlane) -> Plane {
    Plane {
        normal: qa_core::math::scale3(plane.normal, -1.0),
        distance: -plane.distance,
    }
}

// Engine-side generation follows the native i386 gcc -O2 reference, not
// game QVM vector arithmetic: x87 keeps expression intermediates until
// float-array stores.
fn patch_dot64(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn patch_cross64(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize_patch_vector(value: [f64; 3]) -> [f64; 3] {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if length == 0.0 {
        value
    } else {
        [value[0] / length, value[1] / length, value[2] / length]
    }
}

fn curve_midpoint(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    [
        0.5 * (0.5 * (a[0] + b[0]) + 0.5 * (b[0] + c[0])),
        0.5 * (0.5 * (a[1] + b[1]) + 0.5 * (b[1] + c[1])),
        0.5 * (0.5 * (a[2] + b[2]) + 0.5 * (b[2] + c[2])),
    ]
}

fn lerp64(a: [f64; 3], b: [f64; 3], fraction: f64) -> [f64; 3] {
    [
        a[0] + fraction * (b[0] - a[0]),
        a[1] + fraction * (b[1] - a[1]),
        a[2] + fraction * (b[2] - a[2]),
    ]
}

fn store64(value: [f64; 3]) -> Vec3 {
    vec3(value[0] as f32, value[1] as f32, value[2] as f32)
}

fn load64(value: Vec3) -> [f64; 3] {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

fn from_points(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<([f64; 3], f32)> {
    let normal = normalize_patch_vector(patch_cross64(
        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
    ));
    if length3(store64(normal)) == 0.0 {
        return None;
    }
    Some((normal, patch_dot64(a, normal) as f32))
}

fn close_points(a: [f64; 3], b: [f64; 3], epsilon: f64) -> bool {
    (a[0] - b[0]).abs() <= epsilon && (a[1] - b[1]).abs() <= epsilon && (a[2] - b[2]).abs() <= epsilon
}

fn plane_equal(a: &PatchPlane, normal: [f64; 3], distance: f32) -> bool {
    let an = load64(a.normal);
    (an[0] - normal[0]).abs() < 0.0001
        && (an[1] - normal[1]).abs() < 0.0001
        && (an[2] - normal[2]).abs() < 0.0001
        && (f64::from(a.distance) - f64::from(distance)).abs() < 0.02
}

fn normal_signbits(normal: [f64; 3]) -> i32 {
    i32::from(normal[0] < 0.0) | (i32::from(normal[1] < 0.0) << 1) | (i32::from(normal[2] < 0.0) << 2)
}

fn subdivide(columns: &mut Vec<Vec<[f64; 3]>>) -> Result<(), WorldError> {
    let mut index = 0;
    while index + 2 < columns.len() {
        let (a, b, c) = (
            columns[index].clone(),
            columns[index + 1].clone(),
            columns[index + 2].clone(),
        );
        let needed = a.iter().enumerate().any(|(row, point)| {
            let mid = curve_midpoint(*point, b[row], c[row]);
            let flat = lerp64(*point, c[row], 0.5);
            let delta = [mid[0] - flat[0], mid[1] - flat[1], mid[2] - flat[2]];
            length3(store64(delta)) >= 16.0
        });
        if !needed {
            columns.remove(index + 1);
            index += 1;
            continue;
        }
        if columns.len() + 2 > 129 {
            return Err(WorldError::BadCollisionRecord(
                "collision subdivision exceeds MAX_GRID_SIZE".to_string(),
            ));
        }
        let first: Vec<[f64; 3]> = a
            .iter()
            .enumerate()
            .map(|(row, point)| lerp64(*point, b[row], 0.5))
            .collect();
        let last: Vec<[f64; 3]> = b
            .iter()
            .enumerate()
            .map(|(row, point)| lerp64(*point, c[row], 0.5))
            .collect();
        let mid: Vec<[f64; 3]> = a
            .iter()
            .enumerate()
            .map(|(row, point)| curve_midpoint(*point, b[row], c[row]))
            .collect();
        columns.splice(index + 1..index + 2, [first, mid, last]);
    }
    let mut index = 0;
    while index + 1 < columns.len() {
        let same = columns[index]
            .iter()
            .enumerate()
            .all(|(row, point)| close_points(*point, columns[index + 1][row], 0.1));
        if same {
            columns.remove(index + 1);
        } else {
            index += 1;
        }
    }
    Ok(())
}

struct PatchGen<'a> {
    columns: Vec<Vec<[f64; 3]>>,
    wrap_width: bool,
    wrap_height: bool,
    planes: Vec<PatchPlane>,
    facets: Vec<PatchFacet>,
    grid_planes: Vec<Vec<[i32; 2]>>,
    windings: &'a CollisionWindingLibrary,
    debug: Option<&'a CollisionDebugSurface>,
}

impl<'a> PatchGen<'a> {
    fn point(&self, x: usize, y: usize) -> Result<[f64; 3], WorldError> {
        Ok(*patch_at(patch_at(&self.columns, x)?, y)?)
    }

    fn find_plane(&mut self, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Result<i32, WorldError> {
        let Some((normal, distance)) = from_points(a, b, c) else {
            return Ok(-1);
        };
        for (index, existing) in self.planes.iter().enumerate() {
            let en = load64(existing.normal);
            if patch_dot64(normal, en) >= 0.0
                && [a, b, c]
                    .iter()
                    .all(|point| ((patch_dot64(*point, en) - f64::from(existing.distance)) as f32).abs() <= 0.1)
            {
                return Ok(index as i32);
            }
        }
        if self.planes.len() == 2048 {
            return Err(WorldError::BadCollisionRecord("MAX_PATCH_PLANES".to_string()));
        }
        let signbits = normal_signbits(normal);
        self.planes.push(PatchPlane {
            normal: store64(normal),
            distance,
            signbits,
        });
        Ok(self.planes.len() as i32 - 1)
    }

    fn find_plane2(&mut self, normal: [f64; 3], distance: f32) -> Result<PatchBorder, WorldError> {
        for (index, existing) in self.planes.iter().enumerate() {
            if plane_equal(existing, normal, distance) {
                return Ok(PatchBorder {
                    plane: index,
                    inward: false,
                    no_adjust: false,
                });
            }
            if plane_equal(existing, [-normal[0], -normal[1], -normal[2]], -distance) {
                return Ok(PatchBorder {
                    plane: index,
                    inward: true,
                    no_adjust: false,
                });
            }
        }
        if self.planes.len() == 2048 {
            return Err(WorldError::BadCollisionRecord("MAX_PATCH_PLANES".to_string()));
        }
        let signbits = normal_signbits(normal);
        self.planes.push(PatchPlane {
            normal: store64(normal),
            distance,
            signbits,
        });
        Ok(PatchBorder {
            plane: self.planes.len() - 1,
            inward: false,
            no_adjust: false,
        })
    }

    fn gp(&self, x: usize, y: usize, triangle: usize) -> Result<i32, WorldError> {
        Ok(patch_at(patch_at(&self.grid_planes, x)?, y)?[triangle])
    }

    fn edge_plane(&mut self, x: usize, y: usize, edge: usize) -> Result<i32, WorldError> {
        let (a, b, base, triangle): ([f64; 3], [f64; 3], [f64; 3], usize) = match edge {
            0 => (self.point(x, y)?, self.point(x + 1, y)?, self.point(x, y)?, 0),
            1 => (
                self.point(x + 1, y)?,
                self.point(x + 1, y + 1)?,
                self.point(x + 1, y)?,
                0,
            ),
            2 => (
                self.point(x + 1, y + 1)?,
                self.point(x, y + 1)?,
                self.point(x, y + 1)?,
                1,
            ),
            3 => (self.point(x, y + 1)?, self.point(x, y)?, self.point(x, y)?, 1),
            4 => (
                self.point(x + 1, y + 1)?,
                self.point(x, y)?,
                self.point(x + 1, y + 1)?,
                0,
            ),
            5 => (self.point(x, y)?, self.point(x + 1, y + 1)?, self.point(x, y)?, 1),
            _ => {
                return Err(WorldError::BadCollisionRecord("unknown patch edge".to_string()));
            }
        };
        let mut plane_index = self.gp(x, y, triangle)?;
        if plane_index == -1 {
            plane_index = self.gp(x, y, if triangle == 0 { 1 } else { 0 })?;
        }
        if plane_index == -1 {
            if let Some(debug) = self.debug {
                debug.print("WARNING: CM_GridPlane unresolvable\n");
            }
            return Ok(-1);
        }
        let normal = load64(patch_at(&self.planes, usize::try_from(plane_index).unwrap_or(usize::MAX))?.normal);
        let third = [
            base[0] + normal[0] * 4.0,
            base[1] + normal[1] * 4.0,
            base[2] + normal[2] * 4.0,
        ];
        self.find_plane(a, b, third)
    }

    fn make_facet(
        &mut self,
        surface: i32,
        raw_borders: &[i32],
        no_adjust: &[bool],
        vertices: &[[f64; 3]],
        block: [Vec3; 4],
    ) -> Result<(), WorldError> {
        let mut borders = Vec::new();
        for (border_index, index) in raw_borders.iter().enumerate() {
            let flag = *patch_at(no_adjust, border_index)?;
            if *index == -1 {
                borders.push(PatchBorder {
                    plane: usize::MAX,
                    inward: false,
                    no_adjust: flag,
                });
                continue;
            }
            let plane = patch_at(&self.planes, usize::try_from(*index).unwrap_or(usize::MAX))?;
            let pn = load64(plane.normal);
            let mut front = false;
            let mut back = false;
            for point in vertices {
                let d = (patch_dot64(*point, pn) - f64::from(plane.distance)) as f32;
                if d > 0.1 {
                    front = true;
                }
                if d < -0.1 {
                    back = true;
                }
            }
            if !front && !back {
                borders.push(PatchBorder {
                    plane: usize::MAX,
                    inward: false,
                    no_adjust: flag,
                });
                continue;
            }
            if front && back {
                if let Some(debug) = self.debug {
                    debug.record_mixed_border(block);
                }
            }
            borders.push(PatchBorder {
                plane: usize::try_from(*index).unwrap_or(usize::MAX),
                inward: front && !back,
                no_adjust: flag,
            });
        }
        if surface == -1 {
            return Ok(());
        }
        let surface_index = usize::try_from(surface).unwrap_or(usize::MAX);
        let mut winding = Some(
            self.windings
                .base_for_plane(&patch_at(&self.planes, surface_index)?.as_plane())?,
        );
        for border in &borders {
            if winding.is_none() {
                break;
            }
            // CM_ValidateFacet leaves this allocation live when a border is invalid.
            if border.plane == usize::MAX {
                return Ok(());
            }
            let plane = patch_at(&self.planes, border.plane)?;
            let clip = if border.inward { plane.as_plane() } else { negate(plane) };
            let owned = winding.take().expect("winding checked above");
            winding = self.windings.chop_in_place(owned, &clip, ON_EPSILON)?;
        }
        let Some(winding) = winding else {
            return Ok(());
        };
        let bounds = self.windings.bounds(&winding)?;
        self.windings.free(winding)?;
        let delta = load64(bounds.max);
        let min = load64(bounds.min);
        let span = [delta[0] - min[0], delta[1] - min[1], delta[2] - min[2]];
        if span[0] > 65535.0
            || span[1] > 65535.0
            || span[2] > 65535.0
            || min[0] >= 65535.0
            || min[1] >= 65535.0
            || min[2] >= 65535.0
            || delta[0] <= -65535.0
            || delta[1] <= -65535.0
            || delta[2] <= -65535.0
        {
            return Ok(());
        }
        // CM_AddFacetBevels starts a second winding after validation frees the first.
        let mut winding = Some(
            self.windings
                .base_for_plane(&patch_at(&self.planes, surface_index)?.as_plane())?,
        );
        for border in &borders {
            if winding.is_none() {
                break;
            }
            if border.plane == surface_index {
                continue;
            }
            let plane = patch_at(&self.planes, border.plane)?;
            let clip = if border.inward { plane.as_plane() } else { negate(plane) };
            let owned = winding.take().expect("winding checked above");
            winding = self.windings.chop_in_place(owned, &clip, ON_EPSILON)?;
        }
        let Some(winding) = winding else {
            self.facets.push(PatchFacet {
                surface: surface_index,
                borders,
            });
            return Ok(());
        };
        let bevel_bounds = self.windings.bounds(&winding)?;
        let winding_points = winding.points()?;
        let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let bb_min = load64(bevel_bounds.min);
        let bb_max = load64(bevel_bounds.max);
        for axis in axes {
            for direction in [-1.0, 1.0] {
                let normal = [axis[0] * direction, axis[1] * direction, axis[2] * direction];
                let extent = if direction == 1.0 { bb_max } else { bb_min };
                let distance = patch_dot64(normal, extent) as f32;
                if !bevel_duplicate(&self.planes, surface_index, &borders, normal, distance)? {
                    if borders.len() > 26 {
                        if let Some(debug) = self.debug {
                            debug.print("ERROR: too many bevels\n");
                        }
                    }
                    let border = self.find_plane2(normal, distance)?;
                    borders.push(border);
                }
            }
        }
        for j in 0..winding_points.len() {
            let point = patch_at(&winding_points, j)?;
            let next = patch_at(&winding_points, (j + 1) % winding_points.len())?;
            let difference = [
                f64::from(point.x) - f64::from(next.x),
                f64::from(point.y) - f64::from(next.y),
                f64::from(point.z) - f64::from(next.z),
            ];
            if length3(store64(difference)) < 0.5 {
                continue;
            }
            let mut edge = normalize_patch_vector(difference);
            for axis in axes {
                let component = patch_dot64(edge, axis);
                if (component - 1.0).abs() < 0.0001 {
                    edge = axis;
                    break;
                }
                if (component + 1.0).abs() < 0.0001 {
                    edge = [-axis[0], -axis[1], -axis[2]];
                    break;
                }
            }
            if edge[0].abs() == 1.0 || edge[1].abs() == 1.0 || edge[2].abs() == 1.0 {
                continue;
            }
            for axis in axes {
                for direction in [-1.0, 1.0] {
                    let scaled = [axis[0] * direction, axis[1] * direction, axis[2] * direction];
                    let raw = patch_cross64(edge, scaled);
                    if length3(store64(raw)) < 0.5 {
                        continue;
                    }
                    let normal = normalize_patch_vector(raw);
                    let anchor = load64(*patch_at(&winding_points, j)?);
                    let distance = patch_dot64(anchor, normal) as f32;
                    let over = winding_points.iter().any(|p| {
                        let q = load64(*p);
                        ((patch_dot64(q, normal) - f64::from(distance)) as f32) > 0.1
                    });
                    if over || bevel_duplicate(&self.planes, surface_index, &borders, normal, distance)? {
                        continue;
                    }
                    if borders.len() > 26 {
                        if let Some(debug) = self.debug {
                            debug.print("ERROR: too many bevels\n");
                        }
                    }
                    let border = self.find_plane2(normal, distance)?;
                    for existing in &borders {
                        if existing.plane == border.plane {
                            if let Some(debug) = self.debug {
                                debug.print("WARNING: bevel plane already used\n");
                            }
                        }
                    }
                    let plane = patch_at(&self.planes, border.plane)?.as_plane();
                    let clip = if border.inward { plane } else { negate_plane(&plane) };
                    let copy = self.windings.copy(&winding)?;
                    let clipped = self.windings.chop_in_place(copy, &clip, ON_EPSILON)?;
                    if clipped.is_none() {
                        if let Some(debug) = self.debug {
                            debug.developer_print("WARNING: CM_AddFacetBevels... invalid bevel\n");
                        }
                        continue;
                    }
                    self.windings.free(clipped.expect("checked above"))?;
                    borders.push(border);
                }
            }
        }
        self.windings.free(winding)?;
        borders.push(PatchBorder {
            plane: surface_index,
            inward: true,
            no_adjust: false,
        });
        if borders.len() > 27 {
            return Err(WorldError::BadCollisionRecord(
                "collision patch exceeds facet border storage".to_string(),
            ));
        }
        self.facets.push(PatchFacet {
            surface: surface_index,
            borders,
        });
        Ok(())
    }
}

fn bevel_duplicate(
    planes: &[PatchPlane],
    surface: usize,
    borders: &[PatchBorder],
    normal: [f64; 3],
    distance: f32,
) -> Result<bool, WorldError> {
    for index in std::iter::once(surface).chain(borders.iter().map(|border| border.plane)) {
        let existing = patch_at(planes, index)?;
        if plane_equal(existing, normal, distance)
            || plane_equal(existing, [-normal[0], -normal[1], -normal[2]], -distance)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn negate_plane(plane: &Plane) -> Plane {
    Plane {
        normal: qa_core::math::scale3(plane.normal, -1.0),
        distance: -plane.distance,
    }
}

/// Generate patch collision records for a `width` x `height` control grid.
pub fn generate_patch_collide(
    width: i32,
    height: i32,
    points: &[Vec3],
    debug: Option<&CollisionDebugSurface>,
    allocate: Option<&dyn PatchAllocator>,
) -> Result<PatchCollide, WorldError> {
    if width <= 2 || height <= 2 {
        return Err(WorldError::BadCollisionRecord(format!(
            "CM_GeneratePatchFacets: bad parameters: ({width}, {height}, managed points)"
        )));
    }
    if width % 2 == 0 || height % 2 == 0 {
        return Err(WorldError::BadCollisionRecord(
            "CM_GeneratePatchFacets: even sizes are invalid for quadratic meshes".to_string(),
        ));
    }
    if width > 129 || height > 129 {
        return Err(WorldError::BadCollisionRecord(
            "CM_GeneratePatchFacets: source is > MAX_GRID_SIZE".to_string(),
        ));
    }
    if points.len() != usize::try_from(width * height).unwrap_or(usize::MAX) {
        return Err(WorldError::BadCollisionRecord(
            "collision patch requires a complete control grid".to_string(),
        ));
    }
    for point in points {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(WorldError::BadCollisionRecord(
                "nonfinite patch control point".to_string(),
            ));
        }
    }
    let width = usize::try_from(width).unwrap_or(0);
    let height = usize::try_from(height).unwrap_or(0);
    let mut columns: Vec<Vec<[f64; 3]>> = (0..width)
        .map(|x| (0..height).map(|y| load64(points[y * width + x])).collect())
        .collect();
    let wrapped = |columns: &[Vec<[f64; 3]>]| -> Result<bool, WorldError> {
        let first = patch_at(columns, 0)?;
        let last = patch_at(columns, columns.len() - 1)?;
        let mut same = true;
        for (y, point) in first.iter().enumerate() {
            if !close_points(*point, *patch_at(last, y)?, 0.1) {
                same = false;
                break;
            }
        }
        Ok(same)
    };
    let wrap_height = wrapped(&columns)?;
    subdivide(&mut columns)?;
    let old = columns;
    columns = Vec::with_capacity(height);
    for y in 0..height {
        let mut row = Vec::with_capacity(old.len());
        for column in &old {
            row.push(*patch_at(column, y)?);
        }
        columns.push(row);
    }
    let wrap_width = wrapped(&columns)?;
    subdivide(&mut columns)?;
    let width = columns.len();
    let height = patch_at(&columns, 0)?.len();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for column in &columns {
        for point in column {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
    }
    if let Some(debug) = debug {
        debug.c_total_patch_blocks.set(
            debug
                .c_total_patch_blocks
                .get()
                .wrapping_add(((width - 1) * (height - 1)) as i32),
        );
    }
    let fallback;
    let windings = match debug {
        Some(debug) => &debug.windings,
        None => {
            fallback = CollisionWindingLibrary::new(None);
            &fallback
        }
    };
    let mut gen = PatchGen {
        columns,
        wrap_width,
        wrap_height,
        planes: Vec::new(),
        facets: Vec::new(),
        grid_planes: Vec::new(),
        windings,
        debug,
    };
    let mut grid_planes = Vec::with_capacity(width - 1);
    for x in 0..width - 1 {
        let mut row = Vec::with_capacity(height - 1);
        for y in 0..height - 1 {
            let first = gen.find_plane(gen.point(x, y)?, gen.point(x + 1, y)?, gen.point(x + 1, y + 1)?)?;
            let second = gen.find_plane(gen.point(x + 1, y + 1)?, gen.point(x, y + 1)?, gen.point(x, y)?)?;
            row.push([first, second]);
        }
        grid_planes.push(row);
    }
    gen.grid_planes = grid_planes;
    for x in 0..width - 1 {
        for y in 0..height - 1 {
            let first = gen.gp(x, y, 0)?;
            let second = gen.gp(x, y, 1)?;
            let mut top = if y > 0 {
                gen.gp(x, y - 1, 1)?
            } else if gen.wrap_height {
                gen.gp(x, height - 2, 1)?
            } else {
                -1
            };
            let mut bottom = if y < height - 2 {
                gen.gp(x, y + 1, 0)?
            } else if gen.wrap_height {
                gen.gp(x, 0, 0)?
            } else {
                -1
            };
            let mut left = if x > 0 {
                gen.gp(x - 1, y, 0)?
            } else if gen.wrap_width {
                gen.gp(width - 2, y, 0)?
            } else {
                -1
            };
            let mut right = if x < width - 2 {
                gen.gp(x + 1, y, 1)?
            } else if gen.wrap_width {
                gen.gp(0, y, 1)?
            } else {
                -1
            };
            let (no_top, no_bottom, no_left, no_right) =
                (top == first, bottom == second, left == second, right == first);
            if top == -1 || top == first {
                top = gen.edge_plane(x, y, 0)?;
            }
            if bottom == -1 || bottom == second {
                bottom = gen.edge_plane(x, y, 2)?;
            }
            if left == -1 || left == second {
                left = gen.edge_plane(x, y, 3)?;
            }
            if right == -1 || right == first {
                right = gen.edge_plane(x, y, 1)?;
            }
            if gen.facets.len() == 1024 {
                return Err(WorldError::BadCollisionRecord("MAX_FACETS".to_string()));
            }
            let block = [
                store64(gen.point(x, y)?),
                store64(gen.point(x + 1, y)?),
                store64(gen.point(x + 1, y + 1)?),
                store64(gen.point(x, y + 1)?),
            ];
            if first == second {
                if first != -1 {
                    gen.make_facet(
                        first,
                        &[top, right, bottom, left],
                        &[no_top, no_right, no_bottom, no_left],
                        &[
                            gen.point(x, y)?,
                            gen.point(x + 1, y)?,
                            gen.point(x + 1, y + 1)?,
                            gen.point(x, y + 1)?,
                        ],
                        block,
                    )?;
                }
            } else {
                let third = if second != -1 {
                    second
                } else if bottom != -1 {
                    bottom
                } else {
                    gen.edge_plane(x, y, 4)?
                };
                gen.make_facet(
                    first,
                    &[top, right, third],
                    &[no_top, no_right, false],
                    &[gen.point(x, y)?, gen.point(x + 1, y)?, gen.point(x + 1, y + 1)?],
                    block,
                )?;
                if gen.facets.len() == 1024 {
                    return Err(WorldError::BadCollisionRecord("MAX_FACETS".to_string()));
                }
                let third = if first != -1 {
                    first
                } else if top != -1 {
                    top
                } else {
                    gen.edge_plane(x, y, 5)?
                };
                gen.make_facet(
                    second,
                    &[bottom, left, third],
                    &[no_bottom, no_left, false],
                    &[gen.point(x + 1, y + 1)?, gen.point(x, y + 1)?, gen.point(x, y)?],
                    block,
                )?;
            }
        }
    }
    if let Some(allocate) = allocate {
        allocate.allocate(PatchAllocSite::Generate, 40);
        allocate.allocate(PatchAllocSite::Facets, gen.facets.len() * 320);
        for facet in &gen.facets {
            if facet.borders.len() > 26 {
                return Err(WorldError::BadCollisionRecord(
                    "Patch border outside source facet allocation".to_string(),
                ));
            }
        }
        allocate.allocate(PatchAllocSite::Planes, gen.planes.len() * 20);
    }
    Ok(PatchCollide {
        planes: gen.planes,
        facets: gen.facets,
        bounds: Bounds {
            min: store64([min[0] - 1.0, min[1] - 1.0, min[2] - 1.0]),
            max: store64([max[0] + 1.0, max[1] + 1.0, max[2] + 1.0]),
        },
    })
}

fn box_offset(plane: &PatchPlane, normal: Vec3, mins: Vec3, extents: Vec3) -> Result<f64, WorldError> {
    let signbits = plane.signbits;
    if !(0..8).contains(&signbits) {
        return Err(WorldError::BadCollisionRecord(
            "CM patch plane signbits outside trace offsets".to_string(),
        ));
    }
    let corner = [
        f64::from(if signbits & 1 != 0 { extents.x } else { mins.x }),
        f64::from(if signbits & 2 != 0 { extents.y } else { mins.y }),
        f64::from(if signbits & 4 != 0 { extents.z } else { mins.z }),
    ];
    // cm_trace.c's eight centered-box corners, indexed by the original
    // plane's stored signs.
    Ok(patch_dot64(corner, load64(normal)))
}

fn expanded(source: &PatchPlane, shape: &PatchShape, border: Option<&PatchBorder>) -> Result<Plane, WorldError> {
    let plane = if border.is_some_and(|border| border.inward) {
        negate(source)
    } else {
        source.as_plane()
    };
    match shape {
        PatchShape::Point { .. } => Ok(plane),
        PatchShape::Capsule { radius, offset, .. } => {
            let grown = f64::from(*radius) + f64::from(dot3(plane.normal, *offset).abs());
            Ok(Plane {
                normal: plane.normal,
                distance: (f64::from(plane.distance) + grown) as f32,
            })
        }
        PatchShape::Box { mins, extents } => {
            let offset = box_offset(source, plane.normal, *mins, *extents)?;
            let grown = if border.is_none() { -offset } else { offset.abs() };
            Ok(Plane {
                normal: plane.normal,
                distance: (f64::from(plane.distance) + grown) as f32,
            })
        }
    }
}

/// Test whether a position shape sits inside a patch facet.
pub fn position_in_patch(patch: &PatchCollide, start: Vec3, shape: &PatchShape) -> Result<bool, WorldError> {
    if matches!(shape, PatchShape::Point { .. }) {
        return Ok(false);
    }
    for facet in &patch.facets {
        let surface = expanded(patch_at(&patch.planes, facet.surface)?, shape, None)?;
        if dot3(start, surface.normal) > surface.distance {
            continue;
        }
        let mut inside = true;
        for border in &facet.borders {
            let plane = expanded(patch_at(&patch.planes, border.plane)?, shape, Some(border))?;
            if dot3(start, plane.normal) > plane.distance {
                inside = false;
                break;
            }
        }
        if inside {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Trace a sweep against patch facets, stopping at `max_fraction`.
pub fn trace_patch(
    patch: &PatchCollide,
    start: Vec3,
    end: Vec3,
    shape: &PatchShape,
    max_fraction: f32,
    debug: Option<&CollisionDebugSurface>,
) -> Result<Option<PatchHit>, WorldError> {
    if let PatchShape::Point { mins, extents } = shape {
        struct Relation {
            front: bool,
            intersection: f32,
        }
        let mut relationships = Vec::with_capacity(patch.planes.len());
        for plane in &patch.planes {
            let offset = box_offset(plane, plane.normal, *mins, *extents)?;
            let d1 = (f64::from(dot3(start, plane.normal)) - f64::from(plane.distance) + offset) as f32;
            let d2 = (f64::from(dot3(end, plane.normal)) - f64::from(plane.distance) + offset) as f32;
            let crossing = if d1 == d2 {
                99999.0
            } else {
                (f64::from(d1) / (f64::from(d1) - f64::from(d2))) as f32
            };
            relationships.push(Relation {
                front: d1 > 0.0,
                intersection: if crossing <= 0.0 { 99999.0 } else { crossing },
            });
        }
        let mut result: Option<PatchHit> = None;
        let mut fraction = max_fraction;
        for (facet_index, facet) in patch.facets.iter().enumerate() {
            let surface = patch_at(&relationships, facet.surface)?;
            if !surface.front || surface.intersection > fraction {
                continue;
            }
            let mut valid = true;
            for border in &facet.borders {
                let side = patch_at(&relationships, border.plane)?;
                let ok = if side.front != border.inward {
                    side.intersection <= surface.intersection
                } else {
                    side.intersection >= surface.intersection
                };
                if !ok {
                    valid = false;
                    break;
                }
            }
            if !valid {
                continue;
            }
            if let Some(debug) = debug {
                debug.record_trace(patch, facet_index, true)?;
            }
            let plane = patch_at(&patch.planes, facet.surface)?;
            let offset = box_offset(plane, plane.normal, *mins, *extents)?;
            let d1 = (f64::from(dot3(start, plane.normal)) - f64::from(plane.distance) + offset) as f32;
            let d2 = (f64::from(dot3(end, plane.normal)) - f64::from(plane.distance) + offset) as f32;
            fraction = 0.0f32.max(((f64::from(d1) - 0.125) / (f64::from(d1) - f64::from(d2))) as f32);
            result = Some(PatchHit {
                fraction,
                plane: plane.as_plane(),
            });
        }
        return Ok(result);
    }
    let mut result: Option<PatchHit> = None;
    let mut fraction = max_fraction;
    for (facet_index, facet) in patch.facets.iter().enumerate() {
        let mut enter = -1.0f32;
        let mut leave = 1.0f32;
        let mut hit_index = -1i32;
        let mut best: Option<Plane> = None;
        let mut clip = |plane: Plane, index: i32| -> bool {
            let d1 = dot3(start, plane.normal) - plane.distance;
            let d2 = dot3(end, plane.normal) - plane.distance;
            if d1 > 0.0 && (d2 >= 0.125 || d2 >= d1) {
                return false;
            }
            if d1 <= 0.0 && d2 <= 0.0 {
                return true;
            }
            if d1 > d2 {
                let crossed = 0.0f32.max((d1 - 0.125) / (d1 - d2));
                if crossed > enter {
                    enter = crossed;
                    hit_index = index;
                    best = Some(match shape {
                        PatchShape::Capsule { offset, .. } => Plane {
                            normal: plane.normal,
                            distance: plane.distance - dot3(plane.normal, *offset).abs(),
                        },
                        _ => plane,
                    });
                }
            } else {
                leave = leave.min(1.0f32.min((d1 + 0.125) / (d1 - d2)));
            }
            true
        };
        let surface = expanded(patch_at(&patch.planes, facet.surface)?, shape, None)?;
        if !clip(surface, -1) {
            continue;
        }
        let mut valid = true;
        for (index, border) in facet.borders.iter().enumerate() {
            let plane = patch_at(&patch.planes, border.plane)?;
            let grown = expanded(plane, shape, Some(border))?;
            #[allow(clippy::cast_possible_wrap)]
            if !clip(grown, index as i32) {
                valid = false;
                break;
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let last = facet.borders.len() as i32 - 1;
        if valid && hit_index != last && enter < leave && enter >= 0.0 && enter < fraction {
            if let Some(best) = best {
                if let Some(debug) = debug {
                    debug.record_trace(patch, facet_index, false)?;
                }
                fraction = enter;
                result = Some(PatchHit { fraction, plane: best });
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::cvar::CvarRegistry;

    fn flat_grid() -> Vec<Vec3> {
        let mut points = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                points.push(vec3((x - 1) as f32 * 64.0, (y - 1) as f32 * 64.0, 0.0));
            }
        }
        points
    }

    fn curved_grid() -> Vec<Vec3> {
        let mut points = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                let dx = (x - 2) as f32 * 32.0;
                let dy = (y - 2) as f32 * 32.0;
                points.push(vec3(dx, dy, 64.0 - (dx * dx + dy * dy) / 256.0));
            }
        }
        points
    }

    fn debug_host() -> CollisionDebugHost {
        CollisionDebugHost {
            cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
            print: Rc::new(|_| {}),
            developer_print: Rc::new(|_| {}),
            windings: None,
        }
    }

    #[test]
    fn generation_validates_its_grid() {
        let grid = flat_grid();
        let error = generate_patch_collide(2, 3, &grid, None, None).expect_err("narrow grid");
        assert_eq!(
            error.to_string(),
            "CM_GeneratePatchFacets: bad parameters: (2, 3, managed points)"
        );
        let error = generate_patch_collide(4, 3, &grid, None, None).expect_err("even grid");
        assert_eq!(
            error.to_string(),
            "CM_GeneratePatchFacets: even sizes are invalid for quadratic meshes"
        );
        let error = generate_patch_collide(131, 3, &grid, None, None).expect_err("huge grid");
        assert_eq!(error.to_string(), "CM_GeneratePatchFacets: source is > MAX_GRID_SIZE");
        let error = generate_patch_collide(3, 3, &grid[..8], None, None).expect_err("short grid");
        assert_eq!(error.to_string(), "collision patch requires a complete control grid");
        let mut bad = grid.clone();
        bad[0] = vec3(f32::NAN, 0.0, 0.0);
        let error = generate_patch_collide(3, 3, &bad, None, None).expect_err("NaN grid");
        assert_eq!(error.to_string(), "nonfinite patch control point");
    }

    #[test]
    fn flat_patch_matches_donor_records() {
        let patch = generate_patch_collide(3, 3, &flat_grid(), None, None).expect("generate");
        assert_eq!(patch.planes.len(), 5);
        assert_eq!(patch.facets.len(), 1);
        assert_eq!(patch.bounds.min, vec3(-65.0, -65.0, -1.0));
        assert_eq!(patch.bounds.max, vec3(65.0, 65.0, 1.0));
    }

    #[test]
    fn flat_patch_traces_match_donor() {
        let patch = generate_patch_collide(3, 3, &flat_grid(), None, None).expect("generate");
        let point = PatchShape::Point {
            mins: vec3(0.0, 0.0, 0.0),
            extents: vec3(0.0, 0.0, 0.0),
        };
        let hit = trace_patch(&patch, vec3(0.0, 0.0, 10.0), vec3(0.0, 0.0, -10.0), &point, 1.0, None)
            .expect("trace")
            .expect("point hits the sheet");
        assert_eq!(hit.fraction, 0.49375f32);
        assert_eq!(hit.plane.normal, vec3(0.0, 0.0, 1.0));
        assert_eq!(hit.plane.distance, 0.0);
        let shape = PatchShape::Box {
            mins: vec3(-8.0, -8.0, -8.0),
            extents: vec3(8.0, 8.0, 8.0),
        };
        let hit = trace_patch(&patch, vec3(0.0, 0.0, 20.0), vec3(0.0, 0.0, -20.0), &shape, 1.0, None)
            .expect("trace")
            .expect("box hits the sheet");
        assert_eq!(hit.fraction, 0.296875f32);
        assert_eq!(hit.plane.normal, vec3(0.0, 0.0, 1.0));
        assert_eq!(hit.plane.distance, 8.0);
        let gated = trace_patch(&patch, vec3(0.0, 0.0, 20.0), vec3(0.0, 0.0, -20.0), &shape, 0.1, None).expect("trace");
        assert!(gated.is_none());
        let capsule = PatchShape::Capsule {
            extents: vec3(8.0, 8.0, 16.0),
            radius: 8.0,
            offset: vec3(0.0, 0.0, 8.0),
        };
        let hit = trace_patch(&patch, vec3(0.0, 0.0, 20.0), vec3(0.0, 0.0, -20.0), &capsule, 1.0, None)
            .expect("trace")
            .expect("capsule hits the sheet");
        assert_eq!(hit.fraction, 0.09687499701976776f64 as f32);
        assert_eq!(hit.plane.distance, 8.0);
    }

    #[test]
    fn flat_patch_positions_match_donor() {
        let patch = generate_patch_collide(3, 3, &flat_grid(), None, None).expect("generate");
        let point = PatchShape::Point {
            mins: vec3(0.0, 0.0, 0.0),
            extents: vec3(0.0, 0.0, 0.0),
        };
        assert!(!position_in_patch(&patch, vec3(0.0, 0.0, 0.0), &point).expect("position"));
        let shape = PatchShape::Box {
            mins: vec3(-8.0, -8.0, -8.0),
            extents: vec3(8.0, 8.0, 8.0),
        };
        assert!(position_in_patch(&patch, vec3(0.0, 0.0, 0.0), &shape).expect("position"));
    }

    #[test]
    fn curved_patch_matches_donor_records() {
        let patch = generate_patch_collide(5, 5, &curved_grid(), None, None).expect("generate");
        assert_eq!(patch.planes.len(), 28);
        assert_eq!(patch.facets.len(), 4);
        assert_eq!(patch.bounds.min, vec3(-65.0, -65.0, 31.0));
        assert_eq!(patch.bounds.max, vec3(65.0, 65.0, 65.0));
        let shape = PatchShape::Box {
            mins: vec3(-4.0, -4.0, -4.0),
            extents: vec3(4.0, 4.0, 4.0),
        };
        let hit = trace_patch(&patch, vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), &shape, 1.0, None)
            .expect("trace")
            .expect("box hits the dome");
        assert_eq!(hit.fraction, 0.15937568247318268f64 as f32);
    }

    #[test]
    fn allocator_observes_source_sites() {
        struct Ledger {
            sites: RefCell<Vec<(PatchAllocSite, usize)>>,
        }
        impl PatchAllocator for Ledger {
            fn allocate(&self, site: PatchAllocSite, bytes: usize) -> HunkAllocation {
                self.sites.borrow_mut().push((site, bytes));
                HunkAllocation {
                    kind: super::super::allocation::HunkKind::Permanent,
                    byte_offset: 0,
                    byte_length: bytes,
                    bytes: vec![0; bytes],
                }
            }
        }
        let ledger = Ledger {
            sites: RefCell::new(Vec::new()),
        };
        let patch = generate_patch_collide(3, 3, &flat_grid(), None, Some(&ledger)).expect("generate");
        assert_eq!(patch.planes.len(), 5);
        assert_eq!(
            *ledger.sites.borrow(),
            [
                (PatchAllocSite::Generate, 40),
                (PatchAllocSite::Facets, 320),
                (PatchAllocSite::Planes, 100),
            ]
        );
    }

    #[test]
    fn debug_surface_records_and_draws() {
        let host = debug_host();
        let debug = CollisionDebugSurface::new(host.clone());
        let patch = generate_patch_collide(3, 3, &flat_grid(), Some(&debug), None).expect("generate");
        assert_eq!(debug.c_total_patch_blocks.get(), 1);
        let shape = PatchShape::Box {
            mins: vec3(-8.0, -8.0, -8.0),
            extents: vec3(8.0, 8.0, 8.0),
        };
        trace_patch(
            &patch,
            vec3(0.0, 0.0, 20.0),
            vec3(0.0, 0.0, -20.0),
            &shape,
            1.0,
            Some(&debug),
        )
        .expect("trace");
        let calls = RefCell::new(Vec::new());
        debug
            .draw(&|color, count, points| {
                calls.borrow_mut().push((color, count, points.len()));
            })
            .expect("draw");
        assert!(!calls.borrow().is_empty());
        assert!(calls.borrow().iter().any(|(color, _, _)| *color == 4));
        debug.clear_level_patches();
        calls.borrow_mut().clear();
        debug
            .draw(&|color, count, points| {
                calls.borrow_mut().push((color, count, points.len()));
            })
            .expect("draw");
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn debug_surface_rejects_unknown_cvars() {
        let debug = CollisionDebugSurface::new(debug_host());
        let error = debug.cvar("r_debugSurfaceUpdate").expect_err("unregistered debug cvar");
        assert_eq!(
            error.to_string(),
            "Collision debug cvar r_debugSurfaceUpdate is not registered"
        );
    }

    #[test]
    fn flat_sheet_point_trace_matches_donor() {
        let mut points = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                points.push(vec3(100.0 + x as f32 * 64.0, (y - 1) as f32 * 64.0, 0.0));
            }
        }
        let collide = generate_patch_collide(3, 3, &points, None, None).expect("collide");
        assert_eq!(collide.facets.len(), 1);
        let hit = trace_patch(
            &collide,
            vec3(164.0, 0.0, 100.0),
            vec3(164.0, 0.0, -100.0),
            &PatchShape::Point {
                mins: vec3(0.0, 0.0, 0.0),
                extents: vec3(0.0, 0.0, 0.0),
            },
            1.0,
            None,
        )
        .expect("trace")
        .expect("hit");
        assert_eq!(hit.fraction, 0.49937498569488525f64 as f32);
        assert_eq!(hit.plane.normal, vec3(0.0, 0.0, 1.0));
        assert_eq!(hit.plane.distance, 0.0);
    }
}
