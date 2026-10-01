//! Unified presentation frame vocabulary.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-types.ts`
//! (`UnifiedIdentityDecoder`, `UnifiedNativeCamera`, `UnifiedPresentationFrame`).
//!
//! The retained client resolves each wire reference through its own identity
//! ledger ([`UnifiedIdentityDecoder`]); the frame is the public state
//! projected for one authenticated player after audience filtering.
//!
//! Snapshot, scene, and event shapes mirror the out-of-scope donors
//! (`contracts/session.ts`, `contracts/scene.ts`, `../simulation/types.ts`)
//! following the `network::types` precedent: they carry exactly the fields
//! the unified codecs read and publish. Scalar fields decoded with
//! `finite()` stay `f64`; vectors reuse the `f32` [`qa_core::math`] types
//! with the same `as` casts as [`qa_world::save::shared`].

use qa_content::contract::ResolvedResourceReference;
use qa_core::identity::{ActorId, ClientId, SeatId, SessionId};
use qa_core::math::{Bounds, Vec3, Vec4};
use qa_core::time::{FrameContext, SourceTime};
use qa_world::inventory::InventoryEntry;
use qa_world::save::shared::{CharacterSelection, ProviderRef};

use super::types::{NativeModCameraView, PlayerUi, PlayerView, WorldText};
use super::unified_components::UnifiedComponentFrames;
use super::unified_event_codec::UnifiedSimulationEvent;
use super::unified_prediction::UnifiedPredictionProjection;
use crate::persistence::mods::ModIdentity;

/// Wire identity resolution for one retained unified client.
///
/// The client resolves each wire reference through its own ledger; slot and
/// generation arrive validated as non-negative wire integers.
pub trait UnifiedIdentityDecoder {
    /// Session the decoded frame belongs to.
    fn session(&self) -> SessionId;
    /// Resolve an actor reference.
    fn actor(&self, slot: u32, generation: u32) -> ActorId;
    /// Resolve a client reference.
    fn client(&self, slot: u32, generation: u32) -> ClientId;
    /// Resolve a seat reference.
    fn seat(&self, index: u32) -> SeatId;
    /// Resolve an already-declared unified resource id.
    fn resource_id(&self, id: &str) -> String;
}

/// Native camera carried directly on older frame versions.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeCamera {
    /// Presenting component owner.
    pub owner: qa_content::contract::PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: u64,
    /// Camera view.
    pub view: NativeModCameraView,
}

/// Snapshot actor row (donor `Snapshot['actors']` element).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedSnapshotActor {
    /// Actor handle.
    pub id: ActorId,
    /// Owning provider (`namespace:name`).
    pub owner: String,
    /// Definition id (`namespace:name`).
    pub definition: String,
}

/// Snapshot body state (donor `BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedBodyState {
    /// World origin.
    pub origin: Vec3,
    /// Angles in degrees.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
    /// Ground actor, if standing on one.
    pub ground: Option<ActorId>,
}

/// Snapshot body row (donor `BodySnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSnapshotBody {
    /// Actor handle.
    pub actor: ActorId,
    /// Body state.
    pub body: UnifiedBodyState,
}

/// Snapshot inventory row.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSnapshotInventory {
    /// Actor handle.
    pub actor: ActorId,
    /// Inventory entries.
    pub entries: Vec<InventoryEntry>,
}

/// Snapshot actor configuration row.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedActorConfiguration {
    /// Actor handle.
    pub actor: ActorId,
    /// Movement provider.
    pub movement: ProviderRef,
    /// Character selection.
    pub character: CharacterSelection,
    /// Weapon providers.
    pub weapons: Vec<ProviderRef>,
    /// Inventory provider.
    pub inventory: ProviderRef,
}

/// Negotiated scene world (donor `SceneSnapshot['world']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSceneWorld {
    /// World resource reference.
    pub resource: ResolvedResourceReference,
    /// Opaque local geometry handle the brush check compares by identity.
    pub geometry: String,
}

/// Decoded scene model (donor `DecodedModel`, unified subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedDecodedModel {
    /// Brush submodel of the negotiated world.
    BrushModel {
        /// Brush model number.
        model: i64,
        /// World geometry handle; must equal the scene world's geometry.
        world: String,
    },
    /// Any other decoded model.
    Other,
}

/// Entity animation pose (donor `ModelPose`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedModelPose {
    /// Frame-lerped pose.
    Frame {
        /// Current frame.
        frame: f64,
        /// Previous frame.
        previous_frame: f64,
        /// Inter-frame blend.
        back_lerp: f64,
    },
    /// Skeletal pose.
    Skeleton {
        /// Joints.
        joints: Vec<UnifiedSkeletonJoint>,
    },
}

/// Skeletal pose joint.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSkeletonJoint {
    /// Joint position.
    pub position: Vec3,
    /// Joint orientation.
    pub orientation: Vec4,
    /// Joint scale.
    pub scale: f64,
}

/// Scene entity transform.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedEntityTransform {
    /// World origin.
    pub origin: Vec3,
    /// Basis axes.
    pub axis: [Vec3; 3],
    /// Scale.
    pub scale: Vec3,
}

/// Scene entity render flags.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedEntityFlags {
    /// Source family.
    pub family: UnifiedSceneFamily,
    /// Flag bits.
    pub bits: f64,
}

/// Scene source family (`q1` / `q2` / `q3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedSceneFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Scene entity attachment.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSceneAttachment {
    /// Attachment tag.
    pub tag: String,
    /// Attached entity.
    pub entity: Box<UnifiedSceneEntity>,
}

/// Scene entity (donor `SceneEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSceneEntity {
    /// Owning actor, if any.
    pub actor: Option<ActorId>,
    /// Model resource reference.
    pub resource: ResolvedResourceReference,
    /// Decoded model.
    pub model: UnifiedDecodedModel,
    /// World transform.
    pub transform: UnifiedEntityTransform,
    /// Previous origin for interpolation.
    pub previous_origin: Vec3,
    /// Animation pose.
    pub pose: UnifiedModelPose,
    /// Skin index.
    pub skin: f64,
    /// Shader color.
    pub color: Vec4,
    /// Shader time.
    pub shader_time: SourceTime,
    /// Render flags.
    pub flags: UnifiedEntityFlags,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f64,
    /// Whole-object opacity.
    pub opacity: Option<f64>,
    /// Attachments.
    pub attachments: Vec<UnifiedSceneAttachment>,
}

/// Q2 light cone.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedLightCone {
    /// Cone direction.
    pub direction: Vec3,
    /// Cosine half-angle.
    pub cos_half_angle: f64,
}

/// Q2 light shadow mode.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedLightShadow {
    /// No shadow.
    None,
    /// Cast at a resolution.
    Cast {
        /// Shadow resolution.
        resolution: f64,
    },
}

/// Scene light profile (donor `SceneLight['profile']`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedLightProfile {
    /// Q1 light.
    Q1,
    /// Q2 scaled/coned light.
    Q2 {
        /// Intensity scale.
        scale: f64,
        /// Spot cone.
        cone: Option<UnifiedLightCone>,
        /// Shadow mode.
        shadow: UnifiedLightShadow,
    },
    /// Q3 light.
    Q3,
}

/// Scene light (donor `SceneLight`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSceneLight {
    /// World origin.
    pub origin: Vec3,
    /// Light color.
    pub color: Vec3,
    /// Radius in world units.
    pub radius: f64,
    /// Additive blending.
    pub additive: bool,
    /// Family profile.
    pub profile: UnifiedLightProfile,
}

/// Scene particle (donor `SceneParticle`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedSceneParticle {
    /// Palette-indexed particle.
    Indexed {
        /// World origin.
        origin: Vec3,
        /// World size.
        size: f64,
        /// Palette index.
        palette_index: f64,
        /// Opacity.
        alpha: f64,
    },
    /// True-color particle.
    Rgba {
        /// World origin.
        origin: Vec3,
        /// World size.
        size: f64,
        /// Shader color.
        color: Vec4,
        /// Billboard rotation.
        rotation: f64,
    },
}

/// Scene light style (donor `SceneLightStyle`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedSceneLightStyle {
    /// Q1 animated style.
    Q1 {
        /// Style index.
        style: f64,
        /// Light value.
        value: f64,
    },
    /// Q2 animated style.
    Q2 {
        /// Style index.
        style: f64,
        /// Style color.
        rgb: Vec3,
        /// White intensity.
        white: f64,
    },
}

/// Scene snapshot (donor `SceneSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSceneSnapshot {
    /// Session handle.
    pub session: SessionId,
    /// Scene time.
    pub time: SourceTime,
    /// Negotiated world.
    pub world: Option<UnifiedSceneWorld>,
    /// Scene entities.
    pub entities: Vec<UnifiedSceneEntity>,
    /// Dynamic lights.
    pub lights: Vec<UnifiedSceneLight>,
    /// Particles.
    pub particles: Vec<UnifiedSceneParticle>,
    /// Light styles.
    pub light_styles: Vec<UnifiedSceneLightStyle>,
    /// Area visibility bits.
    pub area_bits: Option<Vec<u8>>,
}

/// World snapshot (donor `Snapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSnapshot {
    /// Session handle.
    pub session: SessionId,
    /// Frame context.
    pub frame: FrameContext,
    /// Live actors.
    pub actors: Vec<UnifiedSnapshotActor>,
    /// Live bodies.
    pub bodies: Vec<UnifiedSnapshotBody>,
    /// Live inventories.
    pub inventories: Vec<UnifiedSnapshotInventory>,
    /// Actor configurations.
    pub configurations: Vec<UnifiedActorConfiguration>,
    /// Scene snapshot.
    pub scene: UnifiedSceneSnapshot,
}

/// Simulation output (donor `SimulationOutput`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedOutput {
    /// World snapshot.
    pub snapshot: UnifiedSnapshot,
    /// Simulation events.
    pub events: Vec<UnifiedSimulationEvent>,
}

/// Character team (donor `Q3CharacterView['team']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedCharacterTeam {
    /// Red team.
    Red,
    /// Blue team.
    Blue,
}

/// Character animation state (donor `Q3CharacterView['animation']`, `q3` only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnifiedCharacterAnimation {
    /// Legs animation word.
    pub legs: f64,
    /// Torso animation word.
    pub torso: f64,
    /// Legs timer in milliseconds.
    pub legs_timer_milliseconds: f64,
    /// Torso timer in milliseconds.
    pub torso_timer_milliseconds: f64,
}

/// Character view (donor `Q3CharacterView`).
///
/// A local mirror: `qa_world::movement::AnimationState` variant fields are
/// private to its crate, so the shared `Q3CharacterView` cannot be built
/// here; this carries exactly the fields the frame codec reads.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedCharacterView {
    /// Actor handle.
    pub actor: ActorId,
    /// World origin.
    pub origin: Vec3,
    /// Angles in degrees.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Movement direction.
    pub movement_direction: i64,
    /// Animation state.
    pub animation: UnifiedCharacterAnimation,
    /// Source flags.
    pub source_flags: i64,
    /// Powerups bitmask.
    pub powerups: i64,
    /// Team.
    pub team: Option<UnifiedCharacterTeam>,
    /// Shader color.
    pub color: Vec4,
    /// Visual scale.
    pub scale: Option<f64>,
    /// Opacity.
    pub opacity: Option<f64>,
}

/// Admitted player projection.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedFramePlayer {
    /// Player actor.
    pub actor: ActorId,
    /// Player camera view.
    pub view: PlayerView,
    /// Player HUD state.
    pub ui: PlayerUi,
}

/// Public state projected for one authenticated player (donor `UnifiedPresentationFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPresentationFrame {
    /// World epoch.
    pub epoch: u64,
    /// Component frames, when components are admitted.
    pub components: Option<UnifiedComponentFrames>,
    /// Legacy native camera (frame versions carrying one source camera).
    pub native_camera: Option<UnifiedNativeCamera>,
    /// Acknowledged input sequence.
    pub acknowledged_input: i64,
    /// Admitted-player movement projection.
    pub prediction: UnifiedPredictionProjection,
    /// Simulation output.
    pub output: UnifiedOutput,
    /// Presentation models.
    pub models: Vec<super::types::PresentationModel>,
    /// Character views.
    pub characters: Vec<UnifiedCharacterView>,
    /// World text.
    pub texts: Vec<WorldText>,
    /// Admitted player.
    pub player: UnifiedFramePlayer,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct Ledger {
        owner: IdentityOwner,
    }

    impl UnifiedIdentityDecoder for Ledger {
        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
        fn actor(&self, slot: u32, generation: u32) -> ActorId {
            self.owner.actor(slot, generation)
        }
        fn client(&self, slot: u32, generation: u32) -> ClientId {
            self.owner.client(slot, generation)
        }
        fn seat(&self, index: u32) -> SeatId {
            self.owner.seat(index)
        }
        fn resource_id(&self, id: &str) -> String {
            id.to_string()
        }
    }

    #[test]
    fn ledger_resolves_wire_references() {
        let ledger = Ledger {
            owner: IdentityOwner::create("test").unwrap(),
        };
        let actor = ledger.actor(3, 7);
        assert_eq!((actor.slot(), actor.generation()), (3, 7));
        assert_eq!(ledger.seat(1).index(), 1);
        assert_eq!(ledger.resource_id("resource:unified:x"), "resource:unified:x");
        assert_eq!(ledger.session(), *ledger.owner.session());
    }
}
