//! Scene material registrations and shader remaps.
//!
//! Donor provenance: `src/render/scene/material-registrations.ts`. The donor
//! spreads admission across provider/world registries with async remap
//! negotiation; this port keeps one synchronous table per process: shaders
//! are admitted with monotonic registration ids, snapshots define source
//! draw-sort ranks, and remaps redirect shader names to replacements with a
//! time offset. Name keys normalize exactly like the donor (`strip` then
//! lowercase).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::materials::compile::CompiledMaterial;
use crate::materials::material::normalize_shader_name;

static NEXT_REGISTRATION: AtomicU64 = AtomicU64::new(1);
static NEXT_WORLD: AtomicU64 = AtomicU64::new(1);
static TABLE: OnceLock<Mutex<MaterialRegistrationTable>> = OnceLock::new();

/// Serializes tests that mutate or observe process-wide remap state: the
/// table and its revision are shared across test threads, so an
/// interleaved publish or removal flips revision-sensitive assertions in
/// parallel tests.
#[cfg(test)]
pub(crate) static REMAP_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Hold process-wide remap state still for one test. Poisoning is
/// absorbed so one failing test cannot cascade into the others.
#[cfg(test)]
pub(crate) fn lock_remap_tests() -> std::sync::MutexGuard<'static, ()> {
    REMAP_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn table() -> &'static Mutex<MaterialRegistrationTable> {
    TABLE.get_or_init(|| Mutex::new(MaterialRegistrationTable::default()))
}

/// A shader remap: replacement material name plus shader-clock offset.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialRemap {
    /// Replacement material name.
    pub material: String,
    /// Shader-clock time offset in seconds.
    pub time_offset: f32,
}

/// Opaque shader registration handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderRegistration(u64);

/// Opaque world identity for per-lightmap shader bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderWorldIdentity(u64);

/// Mint a fresh world identity for shader bindings.
#[must_use]
pub fn alloc_world_identity() -> ShaderWorldIdentity {
    ShaderWorldIdentity(NEXT_WORLD.fetch_add(1, Ordering::SeqCst))
}

/// An admitted scene material: compiled shader plus its registration.
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredSceneMaterial {
    /// Registration handle.
    pub registration: ShaderRegistration,
    /// Compiled shader.
    pub compiled: CompiledMaterial,
}

impl std::ops::Deref for RegisteredSceneMaterial {
    type Target = CompiledMaterial;

    fn deref(&self) -> &Self::Target {
        &self.compiled
    }
}

/// The registration table behind [`current_remap`] and the global admission
/// functions. `Default` builds an empty table for isolated use.
#[derive(Debug, Default)]
pub struct MaterialRegistrationTable {
    materials: HashMap<u64, RegisteredSceneMaterial>,
    order: Vec<u64>,
    remaps: HashMap<String, MaterialRemap>,
    revision: u64,
}

impl MaterialRegistrationTable {
    /// Admit a compiled shader and return its registered handle.
    pub fn admit(&mut self, compiled: CompiledMaterial) -> RegisteredSceneMaterial {
        let id = NEXT_REGISTRATION.fetch_add(1, Ordering::SeqCst);
        let material = RegisteredSceneMaterial {
            registration: ShaderRegistration(id),
            compiled,
        };
        self.materials.insert(id, material.clone());
        self.order.push(id);
        material
    }

    /// Look up a registered material by handle.
    #[must_use]
    pub fn get(&self, registration: ShaderRegistration) -> Option<&RegisteredSceneMaterial> {
        self.materials.get(&registration.0)
    }

    /// Snapshot of admitted materials ordered by finished sort rank, then
    /// admission order. The index in this snapshot is the source draw-sort
    /// shader rank consumed by scene submissions.
    #[must_use]
    pub fn snapshot(&self) -> Vec<RegisteredSceneMaterial> {
        let mut materials: Vec<RegisteredSceneMaterial> = self
            .order
            .iter()
            .filter_map(|id| self.materials.get(id).cloned())
            .collect();
        materials.sort_by(|a, b| {
            a.finished
                .sort
                .cmp(&b.finished.sort)
                .then_with(|| a.registration.0.cmp(&b.registration.0))
        });
        materials
    }

    /// Draw-sort rank of a registration within [`snapshot`](Self::snapshot).
    #[must_use]
    pub fn rank_of(&self, registration: ShaderRegistration) -> Option<usize> {
        self.snapshot()
            .iter()
            .position(|material| material.registration == registration)
    }

    /// Publish a remap from `original` to `remap`.
    pub fn publish_remap(&mut self, original: &str, remap: MaterialRemap) {
        self.remaps.insert(remap_key(original), remap);
        self.revision += 1;
    }

    /// Drop every remap.
    pub fn clear_remaps(&mut self) {
        self.remaps.clear();
        self.revision += 1;
    }

    /// Drop the remap for one shader name, if present.
    pub fn remove_remap(&mut self, original: &str) {
        if self.remaps.remove(&remap_key(original)).is_some() {
            self.revision += 1;
        }
    }

    /// Current remap for a shader name, if any.
    #[must_use]
    pub fn current_remap(&self, name: &str) -> Option<MaterialRemap> {
        self.remaps.get(&remap_key(name)).cloned()
    }

    /// Count of remap publications; world scenes retain shadow caches while
    /// this is unchanged.
    #[must_use]
    pub const fn material_revision(&self) -> u64 {
        self.revision
    }
}

fn remap_key(name: &str) -> String {
    normalize_shader_name(name)
}

/// Current process-wide remap for a shader name, if any.
#[must_use]
pub fn current_remap(name: &str) -> Option<MaterialRemap> {
    table().lock().ok()?.current_remap(name)
}

/// Publish a process-wide remap from `original` to `remap`.
pub fn publish_remap(original: &str, remap: MaterialRemap) {
    if let Ok(mut table) = table().lock() {
        table.publish_remap(original, remap);
    }
}

/// Drop every process-wide remap.
pub fn clear_remaps() {
    if let Ok(mut table) = table().lock() {
        table.clear_remaps();
    }
}

/// Drop the process-wide remap for one shader name.
pub fn remove_remap(original: &str) {
    if let Ok(mut table) = table().lock() {
        table.remove_remap(original);
    }
}

/// Process-wide remap revision; world scenes retain shadow caches while
/// this is unchanged.
#[must_use]
pub fn material_revision() -> u64 {
    table().lock().map(|table| table.material_revision()).unwrap_or(0)
}

/// Admit a compiled shader into the process-wide table.
pub fn admit_material(compiled: CompiledMaterial) -> Option<RegisteredSceneMaterial> {
    table().lock().ok().map(|mut table| table.admit(compiled))
}

/// Snapshot the process-wide table ordered by finished sort rank.
#[must_use]
pub fn snapshot_materials() -> Vec<RegisteredSceneMaterial> {
    table().lock().map(|table| table.snapshot()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::compile::{
        default_shader_profile, RegisteredExplicitShader, RegisteredStage, RegistrationOutcome,
    };
    use crate::materials::finish::{finish_implicit_shader, FinishImplicitShaderInput, ImplicitShaderKind};
    use crate::materials::material::ShaderDefinition;
    use crate::materials::state::CullFace;

    fn compiled(name: &str, sort: i32) -> CompiledMaterial {
        let definition = ShaderDefinition {
            name: name.to_string(),
            stages: Vec::new(),
            surface_parms: Vec::new(),
            cull: CullFace::Back,
            sort: None,
            sky: None,
            fog: None,
            sun: None,
            deforms: Vec::new(),
            polygon_offset: false,
            no_mipmaps: false,
            no_picmip: false,
            entity_mergable: false,
            portal_range: 0.0,
            clamp_time: 0.0,
            warnings: Vec::new(),
            compiler_directives: Vec::new(),
        };
        let mut finished = finish_implicit_shader(&FinishImplicitShaderInput {
            name: name.to_string(),
            base_image: RegisteredStage::Missing,
            profile: default_shader_profile(),
            kind: ImplicitShaderKind::Default,
        })
        .expect("implicit finish");
        finished.sort = sort;
        let material = crate::materials::compile::shader_render_material(&definition).expect("view");
        CompiledMaterial {
            registered: RegisteredExplicitShader {
                definition,
                stages: Vec::new(),
                sky: None,
                outcome: RegistrationOutcome::Defined,
            },
            finished,
            material,
        }
    }

    #[test]
    fn snapshot_orders_by_finished_sort() {
        let mut table = MaterialRegistrationTable::default();
        let b = table.admit(compiled("b", 9));
        let a = table.admit(compiled("a", 2));
        let snapshot = table.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].registration, a.registration);
        assert_eq!(snapshot[1].registration, b.registration);
        assert_eq!(table.rank_of(a.registration), Some(0));
        assert_eq!(table.rank_of(b.registration), Some(1));
        assert_eq!(table.get(a.registration).unwrap().material.name, "a");
    }

    #[test]
    fn remap_keys_normalize_names() {
        let mut table = MaterialRegistrationTable::default();
        let before = table.material_revision();
        table.publish_remap(
            "Textures/Rock.TGA",
            MaterialRemap {
                material: "rock2".to_string(),
                time_offset: 1.5,
            },
        );
        assert!(table.material_revision() > before);
        let remap = table.current_remap("textures/rock").expect("remap");
        assert_eq!(remap.material, "rock2");
        assert_eq!(remap.time_offset, 1.5);
        table.clear_remaps();
        assert!(table.current_remap("textures/rock").is_none());
    }

    #[test]
    fn global_table_round_trips() {
        let _remap_lock = lock_remap_tests();
        publish_remap(
            "unit/global-remap-probe",
            MaterialRemap {
                material: "probe-target".to_string(),
                time_offset: 0.25,
            },
        );
        let remap = current_remap("unit/global-remap-probe.tga").expect("global remap");
        assert_eq!(remap.material, "probe-target");
        remove_remap("unit/global-remap-probe");
        assert!(current_remap("unit/global-remap-probe").is_none());
    }

    #[test]
    fn world_identities_are_unique() {
        assert_ne!(alloc_world_identity(), alloc_world_identity());
    }
}
