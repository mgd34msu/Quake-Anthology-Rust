//! Quake III application services: scene, resources, sound, draw, collision, cinematics.
//!
//! Port of `src/app/bootstrap/q3-client/services.ts` (`createApplicationQ3Services`). The donor is
//! async over the asset owner; here the media owner moves into the services value and every
//! engine callback arrives through injected sync seams. Callbacks shared by the scene, audio, and
//! draw targets live behind `Rc<RefCell<..>>` because the donor's closures borrow the service
//! owner.

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::audio::types::{AudioAudience, LoopSound as ClientLoopSound, PlaySound as ClientPlaySound, SoundAsset};
use qa_client::audio::SoundFamily;
use qa_client::media::audio::CinematicMixer;
use qa_client::media::presentation::ImageOperation;
use qa_client::render::types::{Rect as RenderRect, RenderCommand, RendererImage, TextureRect as RenderTextureRect};
use qa_client::text::draw2d::{CoordinateSpace, Draw2D, PictureAsset, Rect, TextDrawSink, TextureRect};
use qa_content::contract::ContentId;
use qa_content::q3::presentation::audio::{
    ClientSoundBank as PresentationSoundBank, LoopSound as ContentLoopSound, PlaySound as ContentPlaySound,
    Q3PresentationAudio, SoundOrigin as ContentSoundOrigin, StartSoundOptions,
};
use qa_content::q3::presentation::client::{Q3ClientSound, SourceSoundOptions, SourceSoundOrigin};
use qa_content::q3::presentation::ref_entity::RefModelEntity;
use qa_content::q3::presentation::resources::{
    Q3RendererResources, ResourceHandleOwner, ResourceWorld, ResourceWorldMap,
};
use qa_content::q3::presentation::scene::{
    Q3FogSelection, Q3PresentedScene, Q3SceneRecorder, Q3SceneTarget, Rect as SceneRect,
};
use qa_core::identity::{ActorId, ProviderId, SeatId};
use qa_core::math::{Axis, Vec3, Vec4};

use super::assets::{ApplicationQ3Assets, Q3AssetResourceHost, Q3RemapOverride, SharedPrint, SharedSoundBank};
use super::cinematics::{
    ApplicationQ3Cinematics, CinematicBytes, CinematicImages, CinematicMounts, SystemCinematicHost,
};
use super::collision::{Q3ClientCollision, Q3ClientMapQueries};

/// Presentation output callbacks (donor `output`).
pub trait Q3ServiceOutput {
    /// Publish a presented scene.
    fn scene(&mut self, scene: Q3PresentedScene);
    /// Emit a render command.
    fn command(&mut self, command: RenderCommand);
    /// Emit a text draw.
    fn text(&mut self, draw: Q3ServiceTextDraw);
    /// Emit a seat audio operation.
    fn audio(&mut self, operation: crate::bootstrap::audio::q3::Q3SeatAudioOperation);
    /// Move the listener.
    fn listener(&mut self, origin: Vec3, axis: Axis);
}

/// Shared presentation output.
pub type SharedServiceOutput = Rc<RefCell<Box<dyn Q3ServiceOutput>>>;

/// Handle-based 2D text draw (donor `MaterialTextDraw`; the resolved client type differs).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ServiceTextDraw {
    /// Viewing seat.
    pub seat: SeatId,
    /// Destination rectangle.
    pub rect: Rect,
    /// Source coordinates.
    pub uv: TextureRect,
    /// Draw color.
    pub color: Vec4,
    /// Material picture.
    pub picture: qa_client::text::draw2d::MaterialPicture,
}

/// Application audio surface behind background tracks (donor `ApplicationAudio`).
pub trait Q3ServiceAudio {
    /// Play a music spec for a content.
    fn play_music(&mut self, content: &ContentId, spec: &str);
}

/// Clip-model PVS rows behind the resource world.
pub trait Q3ServiceClip {
    /// PVS row for a cluster.
    fn cluster_pvs(&self, cluster: i32) -> Vec<u8>;
}

/// Resource world over an optional decoded map and clip.
pub struct Q3ServiceResourceWorld {
    /// Decoded world map.
    pub map: ResourceWorldMap,
    /// Clip models.
    pub clip: Box<dyn Q3ServiceClip>,
}

impl ResourceWorld for Q3ServiceResourceWorld {
    fn resource_map(&self) -> &ResourceWorldMap {
        &self.map
    }

    fn cluster_pvs_byte(&self, cluster: i32, offset: usize) -> u8 {
        self.clip.cluster_pvs(cluster).get(offset).copied().unwrap_or(0)
    }
}

/// Boxed collision queries.
pub struct Q3ServiceQueries(pub Box<dyn Q3ClientMapQueries>);

impl Q3ClientMapQueries for Q3ServiceQueries {
    fn trace_q3(&mut self, query: &super::collision::Q3MapTrace<'_>) -> super::collision::Q3MapTraceHit {
        self.0.trace_q3(query)
    }

    fn point_contents_q3(
        &mut self,
        point: Vec3,
        model: i32,
        origin: Vec3,
        angles: Vec3,
        curves: bool,
        player_curve_clip: bool,
    ) -> i32 {
        self.0
            .point_contents_q3(point, model, origin, angles, curves, player_curve_clip)
    }

    fn box_trace_q3(
        &mut self,
        mins: Vec3,
        maxs: Vec3,
        query: &super::collision::Q3MapTrace<'_>,
    ) -> super::collision::Q3MapTraceHit {
        self.0.box_trace_q3(mins, maxs, query)
    }
}

/// Boxed cinematic images.
pub struct Q3ServiceImages(pub Box<dyn CinematicImages>);

impl CinematicImages for Q3ServiceImages {
    fn allocate(&mut self, width: usize, height: usize, resource: &str) -> u32 {
        self.0.allocate(width, height, resource)
    }

    fn commit(&mut self, operation: ImageOperation) {
        self.0.commit(operation);
    }

    fn release(&mut self, image: u32) {
        self.0.release(image);
    }
}

/// Cinematic mounts over shared asset mounts.
struct Q3ServiceMounts {
    /// Shared asset seams.
    shared: super::assets::SharedAssetSeams,
    /// Source content.
    content: ContentId,
    /// Print callback.
    print: SharedPrint,
}

impl CinematicMounts for Q3ServiceMounts {
    fn open(&mut self, name: &str) -> Option<CinematicBytes> {
        match self.shared.borrow_mut().mounts.open(&self.content, name) {
            Ok(Some(opened)) => Some(CinematicBytes {
                bytes: opened.bytes,
                resource: opened.id,
            }),
            Ok(None) => None,
            Err(error) => {
                (self.print.borrow_mut())(&format!("Q3 cinematic {name} failed: {error}"));
                None
            }
        }
    }
}

/// Scene target over shared services (donor `Q3SceneRecorder` options).
struct Q3ServiceSceneTarget {
    /// Viewing seat.
    seat: SeatId,
    /// Viewport.
    viewport: SceneRect,
    /// Shared asset seams for fog.
    shared: super::assets::SharedAssetSeams,
    /// Print callback.
    print: SharedPrint,
    /// Presentation output.
    output: SharedServiceOutput,
}

impl Q3SceneTarget for Q3ServiceSceneTarget {
    fn seat(&self) -> SeatId {
        self.seat.clone()
    }

    fn viewport(&self) -> SceneRect {
        self.viewport
    }

    fn far_clip(&self) -> f32 {
        16384.0
    }

    fn near_clip(&self) -> f32 {
        4.0
    }

    fn rail(&self) -> qa_content::q3::presentation::scene::RailSettings {
        qa_content::q3::presentation::scene::RailSettings
    }

    fn fog_selections(&self) -> Vec<Q3FogSelection> {
        self.shared.borrow().world.fog_selections()
    }

    fn print(&mut self, text: &str) {
        (self.print.borrow_mut())(text);
    }

    fn actor(&self, _entity: &RefModelEntity) -> Option<ActorId> {
        None
    }

    fn publish(&mut self, scene: Q3PresentedScene) {
        self.output.borrow_mut().scene(scene);
    }
}

/// Convert a content sound origin into a client origin.
fn client_origin(origin: &ContentSoundOrigin) -> qa_client::audio::types::SoundOrigin {
    match origin {
        ContentSoundOrigin::Local => qa_client::audio::types::SoundOrigin::Local,
        ContentSoundOrigin::Fixed { position } => qa_client::audio::types::SoundOrigin::Fixed { position: *position },
        ContentSoundOrigin::Actor { actor } => qa_client::audio::types::SoundOrigin::Actor { actor: actor.clone() },
    }
}

/// Resolve a decoded client asset by bank name.
type SoundResolver = Rc<dyn Fn(&str) -> Option<SoundAsset>>;

/// Actor lookup.
type ActorLookup = Rc<RefCell<Box<dyn FnMut(i32) -> ActorId>>>;

/// Audio target over shared services (donor `Q3PresentationAudio` target).
struct Q3ServiceAudioTarget {
    /// Viewing seat.
    seat: SeatId,
    /// Presentation sound bank.
    sounds: SharedSoundBank,
    /// Resolve a decoded client asset by bank name.
    resolve_sound: SoundResolver,
    /// Actor lookup.
    actor_at: ActorLookup,
    /// Frame number.
    frame_number: Rc<dyn Fn() -> i32>,
    /// Guest owner override.
    owner: Option<ProviderId>,
    /// Presentation output.
    output: SharedServiceOutput,
}

impl qa_content::q3::presentation::audio::Q3AudioTarget for Q3ServiceAudioTarget {
    fn seat(&self) -> SeatId {
        self.seat.clone()
    }

    fn sounds(&self) -> Rc<RefCell<dyn PresentationSoundBank>> {
        self.sounds.clone()
    }

    fn actor(&self, source: i32) -> ActorId {
        (self.actor_at.borrow_mut())(source)
    }

    fn frame_number(&self) -> i32 {
        (self.frame_number)()
    }

    fn play(&mut self, sound: ContentPlaySound) {
        let name = sound.sound.pcm.borrow().name.clone();
        let Some(asset) = (self.resolve_sound)(&name) else {
            return;
        };
        let seat = sound.seat.clone();
        self.output
            .borrow_mut()
            .audio(crate::bootstrap::audio::q3::Q3SeatAudioOperation::Play {
                sound: ClientPlaySound {
                    family: SoundFamily::Q3,
                    sound: asset,
                    origin: client_origin(&sound.origin),
                    actor: sound.actor,
                    owner: self.owner.clone(),
                    channel: sound.channel,
                    volume: f64::from(sound.volume),
                    attenuation: f64::from(sound.attenuation),
                    audience: AudioAudience::Seat { seat },
                    delay_seconds: None,
                    server_milliseconds: None,
                },
            });
    }

    fn loop_sound(&mut self, sound: ContentLoopSound) {
        let name = sound.sound.pcm.borrow().name.clone();
        let Some(asset) = (self.resolve_sound)(&name) else {
            return;
        };
        let seat = sound.seat.clone();
        self.output
            .borrow_mut()
            .audio(crate::bootstrap::audio::q3::Q3SeatAudioOperation::Loop {
                sound: ClientLoopSound {
                    family: SoundFamily::Q3,
                    sound: asset,
                    origin: client_origin(&sound.origin),
                    actor: sound.actor,
                    owner: self.owner.clone(),
                    velocity: sound.velocity,
                    frame_number: sound.frame_number,
                    volume: f64::from(sound.volume),
                    attenuation: f64::from(sound.attenuation),
                    lifetime: if sound.persistent {
                        qa_client::audio::types::LoopLifetime::Persistent
                    } else {
                        qa_client::audio::types::LoopLifetime::Frame
                    },
                    audience: AudioAudience::Seat { seat },
                },
            });
    }

    fn update_actor(&mut self, actor: ActorId, position: Vec3) {
        self.output
            .borrow_mut()
            .audio(crate::bootstrap::audio::q3::Q3SeatAudioOperation::Position {
                actor,
                origin: position,
            });
    }

    fn stop_loop(&mut self, _seat: SeatId, actor: ActorId) {
        self.output
            .borrow_mut()
            .audio(crate::bootstrap::audio::q3::Q3SeatAudioOperation::StopLoop { actor });
    }
}

/// Cgame sound surface over presentation audio (donor `sound`).
pub struct Q3ServiceSound {
    /// Presentation audio.
    audio: Q3PresentationAudio,
    /// Presentation sound bank for handle resolution.
    bank: SharedSoundBank,
    /// Presentation output.
    output: SharedServiceOutput,
    /// Application audio for background tracks.
    music: Box<dyn Q3ServiceAudio>,
    /// Source content.
    content: ContentId,
}

impl Q3ServiceSound {
    /// Resolve a retail sound id into a bank handle.
    fn resolve(
        &self,
        pcm: Option<qa_content::q3::presentation::retail_snapshot::PcmSound>,
    ) -> Option<qa_content::q3::presentation::audio::PcmSound> {
        let pcm = pcm?;
        self.bank.borrow().sound_at_index(pcm.id as i32)
    }
}

type RetailPcmSound = qa_content::q3::presentation::retail_snapshot::PcmSound;

impl Q3ClientSound for Q3ServiceSound {
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, pcm: Option<RetailPcmSound>) {
        let pcm = self.resolve(pcm);
        self.audio.start_sound(origin, entity, channel, pcm);
    }

    fn start_source_sound(&mut self, pcm: Option<RetailPcmSound>, options: &SourceSoundOptions) {
        let pcm = self.resolve(pcm);
        self.audio.start_source_sound(
            pcm,
            &StartSoundOptions {
                entity: options.entity,
                origin: match options.origin {
                    SourceSoundOrigin::Local => qa_content::q3::presentation::audio::StartSoundOrigin::Local,
                    SourceSoundOrigin::Fixed(position) => {
                        qa_content::q3::presentation::audio::StartSoundOrigin::Fixed { position }
                    }
                    SourceSoundOrigin::Entity(entity) => {
                        qa_content::q3::presentation::audio::StartSoundOrigin::Entity { entity }
                    }
                },
                channel: options.channel,
                volume: options.volume as f32,
            },
        );
    }

    fn start_local_sound(&mut self, pcm: Option<RetailPcmSound>, channel: i32) {
        let pcm = self.resolve(pcm);
        self.audio.start_local_sound(pcm, channel);
    }

    fn start_background_track(
        &mut self,
        intro: &str,
        looping: &str,
    ) -> qa_content::q3::presentation::state::PresentResult<()> {
        self.music.play_music(&self.content, &format!("{intro} {looping}"));
        Ok(())
    }

    fn add_loop_sound(
        &mut self,
        entity: i32,
        origin: Vec3,
        velocity: Vec3,
        pcm: Option<RetailPcmSound>,
        real_loop: bool,
    ) {
        let pcm = self.resolve(pcm);
        self.audio.add_loop_sound(entity, origin, velocity, pcm, real_loop);
    }

    fn update_sound_position(&mut self, entity: i32, origin: Vec3) {
        self.audio.update_sound_position(entity, origin);
    }

    fn stop_looping_sound(&mut self, entity: i32) {
        self.audio.stop_looping_sound(entity);
    }

    fn clear_looping_sounds(&mut self, kill_all: bool) {
        self.output
            .borrow_mut()
            .audio(crate::bootstrap::audio::q3::Q3SeatAudioOperation::ClearLoops { kill_all });
    }

    fn set_listener(&mut self, _client: i32, origin: Vec3, axis: Axis) {
        self.output.borrow_mut().listener(origin, axis);
    }
}

/// 2D draw sink forwarding into presentation output (donor `TextCommandSink` callbacks).
struct Q3ServiceDrawSink {
    /// Viewing seat.
    seat: SeatId,
    /// Target rectangle.
    target: Rect,
    /// Presentation output.
    output: SharedServiceOutput,
    /// Resolve an image ordinal into a renderer image.
    image_at: Box<dyn FnMut(u32) -> RendererImage>,
    /// Current color.
    color: Vec4,
}

impl TextDrawSink for Q3ServiceDrawSink {
    fn seat(&self) -> &SeatId {
        &self.seat
    }

    fn target(&self) -> Rect {
        self.target
    }

    fn set_color(&mut self, color: Option<Vec4>) {
        let color = color.unwrap_or(Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        });
        self.color = color;
        self.output.borrow_mut().command(RenderCommand::SetColor(color));
    }

    fn stretch_pixels(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset) {
        let destination = Rect {
            x: rect.x + self.target.x,
            y: rect.y + self.target.y,
            width: rect.width,
            height: rect.height,
        };
        match picture {
            PictureAsset::Image(image) => {
                let Some((rect, uv)) = qa_client::text::draw2d::clip_picture(&destination, &uv, &self.target) else {
                    return;
                };
                let resolved = (self.image_at)(image.image);
                self.output.borrow_mut().command(RenderCommand::StretchPic {
                    rect: RenderRect {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    },
                    uv: RenderTextureRect {
                        s1: uv.s,
                        t1: uv.t,
                        s2: uv.s2,
                        t2: uv.t2,
                    },
                    image: resolved,
                });
            }
            PictureAsset::Material(material) => {
                self.output.borrow_mut().text(Q3ServiceTextDraw {
                    seat: self.seat.clone(),
                    rect: destination,
                    uv,
                    color: self.color,
                    picture: material,
                });
            }
        }
    }
}

/// Options for [`ApplicationQ3Services::create`] (donor `ApplicationQ3ServiceOptions`).
pub struct ApplicationQ3ServiceOptions<M: CinematicMixer> {
    /// Asset owner (moved into services).
    pub media: ApplicationQ3Assets,
    /// Application audio.
    pub audio: Box<dyn Q3ServiceAudio>,
    /// Cinematic images.
    pub images: Box<dyn CinematicImages>,
    /// Cinematic mixer.
    pub mixer: M,
    /// Viewing seat.
    pub seat: SeatId,
    /// 2D viewport.
    pub viewport: Rect,
    /// Collision queries.
    pub queries: Box<dyn Q3ClientMapQueries>,
    /// Collision settings.
    pub collision_settings: super::collision::CollisionMapSettings,
    /// Decoded world map with clip models.
    pub world: Option<Q3ServiceResourceWorld>,
    /// Resource handle owner.
    pub resource_handles: Option<ResourceHandleOwner>,
    /// Guest owner override.
    pub owner: Option<ProviderId>,
    /// Shader-remap override.
    pub remap_shader: Option<Q3RemapOverride>,
    /// Actor lookup.
    pub actor_at: Box<dyn FnMut(i32) -> ActorId>,
    /// Clock time.
    pub clock_now: Box<dyn FnMut() -> f64>,
    /// Frame number.
    pub clock_frame: Box<dyn Fn() -> i32>,
    /// System cinematics.
    pub system_cinematics: Option<Box<dyn SystemCinematicHost>>,
    /// Image ordinal resolution.
    pub image_at: Box<dyn FnMut(u32) -> RendererImage>,
    /// Presentation output.
    pub output: Box<dyn Q3ServiceOutput>,
}

/// Quake III application services (donor `createApplicationQ3Services` result).
pub struct ApplicationQ3Services<M: CinematicMixer> {
    /// Asset owner.
    pub media: ApplicationQ3Assets,
    /// Scene recorder.
    pub scene: Rc<RefCell<Q3SceneRecorder>>,
    /// Renderer resources.
    pub resources: Q3RendererResources<Q3AssetResourceHost, Q3ServiceResourceWorld>,
    /// Cgame sound surface.
    pub sound: Q3ServiceSound,
    /// 2D draw sink.
    draw_sink: Q3ServiceDrawSink,
    /// Collision queries.
    pub collision: Q3ClientCollision<Q3ServiceQueries>,
    /// Cinematics.
    pub cinematics: ApplicationQ3Cinematics<M, Q3ServiceImages>,
}

impl<M: CinematicMixer> ApplicationQ3Services<M> {
    /// Build services over a media owner and injected seams.
    pub fn create(options: ApplicationQ3ServiceOptions<M>) -> Self {
        let output: SharedServiceOutput = Rc::new(RefCell::new(options.output));
        let print = options.media.print_shared();
        let target = Q3ServiceSceneTarget {
            seat: options.seat.clone(),
            viewport: SceneRect {
                x: options.viewport.x as i32,
                y: options.viewport.y as i32,
                width: options.viewport.width as i32,
                height: options.viewport.height as i32,
            },
            shared: options.media.shared(),
            print: print.clone(),
            output: output.clone(),
        };
        let scene = Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(target))));
        let mut host = options.media.resource_host(scene.clone());
        if let Some(remap) = options.remap_shader {
            host.set_remap_override(remap);
        }
        let resources = Q3RendererResources::with_world(
            host,
            options.world,
            options.resource_handles.unwrap_or(ResourceHandleOwner::Renderer),
        );
        let caches = options.media.sound_caches();
        let audio_target = Rc::new(RefCell::new(Q3ServiceAudioTarget {
            seat: options.seat.clone(),
            sounds: options.media.bank_shared(),
            resolve_sound: Rc::new(move |name: &str| caches.borrow().sounds.get(&name.to_lowercase()).cloned()),
            actor_at: Rc::new(RefCell::new(options.actor_at)),
            frame_number: options.clock_frame.into(),
            owner: options.owner.clone(),
            output: output.clone(),
        }));
        let sound = Q3ServiceSound {
            audio: Q3PresentationAudio::new(audio_target),
            bank: options.media.bank_shared(),
            output: output.clone(),
            music: options.audio,
            content: options.media.content().clone(),
        };
        let draw_sink = Q3ServiceDrawSink {
            seat: options.seat.clone(),
            target: options.viewport,
            output: output.clone(),
            image_at: options.image_at,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
        };
        let collision = Q3ClientCollision::new(Q3ServiceQueries(options.queries), options.collision_settings);
        let print_media = options.media.print_shared();
        let cinematics = ApplicationQ3Cinematics::new(
            Box::new(Q3ServiceMounts {
                shared: options.media.shared(),
                content: options.media.content().clone(),
                print: print_media.clone(),
            }),
            Q3ServiceImages(options.images),
            options.mixer,
            options.seat.clone(),
            options.clock_now,
            Box::new(move |line: &str| (print_media.borrow_mut())(line)),
            options.system_cinematics,
        );
        Self {
            media: options.media,
            scene,
            resources,
            sound,
            draw_sink,
            collision,
            cinematics,
        }
    }

    /// Build a 2D drawer over the service sink (donor `draw`).
    pub fn draw_2d(&mut self) -> Draw2D<'_> {
        Draw2D::new(&mut self.draw_sink, CoordinateSpace::Stretch640)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::media::audio::{AudioStreamTarget, StreamPcm};
    use qa_client::media::presentation::ImageOperation;
    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::scene::shaders::SceneShaderRegistry;
    use qa_client::render::scene::textures::{SceneAssetReader, SceneTextureLoader};
    use qa_client::render::types::{fresh_owner_identity, ImageSource, ResourceOwner};
    use qa_client::text::draw2d::{ImagePicture, MaterialPicture as DrawMaterialPicture};
    use qa_content::q3::presentation::audio::ClientSoundBank as AudioSoundBank;
    use qa_content::q3::presentation::ref_entity::{PresentResource, Q3DecodedModel};
    use qa_content::q3::presentation::resources::RendererResources;
    use qa_content::q3::presentation::retail_snapshot::{create_refdef, PcmSound as RetailPcmSound};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, vec4, Bounds};

    use super::super::assets::{
        ApplicationQ3AssetOptions, OpenedQ3Asset, Q3AssetCatalog, Q3AssetError, Q3AssetMode, Q3AssetModels,
        Q3AssetMounts, Q3AssetScope, Q3AssetWorld, Q3LoadedModel, Q3RemapOutcome,
    };
    use super::super::collision::{CollisionMapSettings, Q3MapTrace, Q3MapTraceHit};
    use crate::bootstrap::audio::q3::Q3SeatAudioOperation;
    use qa_content::q3::base::world::TraceContact;

    const CONTENT: &str = "q3:classic:baseq3:1";

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[128u8, 140, 128, 116]);
        bytes
    }

    #[derive(Default)]
    struct StubMounts {
        files: std::collections::HashMap<(String, String), OpenedQ3Asset>,
    }

    impl Q3AssetMounts for StubMounts {
        fn open(&mut self, content: &ContentId, path: &str) -> Result<Option<OpenedQ3Asset>, Q3AssetError> {
            Ok(self.files.get(&(content.to_string(), path.to_lowercase())).cloned())
        }

        fn resolve(&mut self, content: &ContentId, path: &str) -> bool {
            self.files.contains_key(&(content.to_string(), path.to_lowercase()))
        }

        fn catalog_archives(&self) -> Vec<(String, Vec<String>)> {
            Vec::new()
        }

        fn plan_archives(&self, _content: &ContentId) -> Vec<String> {
            Vec::new()
        }

        fn loose_roots(&self, _content: &ContentId) -> Vec<std::path::PathBuf> {
            Vec::new()
        }
    }

    #[derive(Default)]
    struct StubModels {
        models: std::collections::HashMap<(String, String), Q3LoadedModel>,
    }

    impl Q3AssetModels for StubModels {
        fn load_model(&mut self, content: &ContentId, path: &str) -> Result<Q3LoadedModel, Q3AssetError> {
            self.models
                .get(&(content.to_string(), path.to_string()))
                .cloned()
                .ok_or_else(|| Q3AssetError::Load(format!("missing model {path}")))
        }

        fn open_skin(&mut self, _content: &ContentId, _path: &str) -> Result<Option<Vec<u8>>, Q3AssetError> {
            Ok(None)
        }

        fn inline_bounds(
            &mut self,
            _world: &qa_content::q3::presentation::ref_entity::PresentWorld,
            _index: usize,
        ) -> Option<Bounds> {
            None
        }
    }

    struct StubCatalog;

    impl Q3AssetCatalog for StubCatalog {
        fn character_content(&self) -> ContentId {
            ContentId(CONTENT.to_string())
        }

        fn weapon_contents(&self) -> Vec<ContentId> {
            Vec::new()
        }

        fn family_of(&self, _content: &ContentId) -> qa_content::contract::GameFamily {
            qa_content::contract::GameFamily::Q3
        }
    }

    struct StubAssetWorld;

    impl Q3AssetWorld for StubAssetWorld {
        fn world_model_bounds(&self) -> Vec<Bounds> {
            Vec::new()
        }

        fn remap_shader(
            &mut self,
            _original: &str,
            _replacement: &str,
            _time_offset: f32,
            _current: &dyn Fn() -> bool,
        ) -> Q3RemapOutcome {
            Q3RemapOutcome::Applied
        }

        fn fog_selections(&self) -> Vec<Q3FogSelection> {
            Vec::new()
        }
    }

    struct FakeReader;

    impl SceneAssetReader for FakeReader {
        fn read(
            &self,
            _path: &str,
        ) -> Result<Option<qa_client::render::scene::textures::SceneAsset>, qa_client::render::error::RenderError>
        {
            Ok(None)
        }
    }

    fn shaders() -> SceneShaderRegistry {
        let session = IdentityOwner::create("q3-services-test")
            .expect("owner")
            .session()
            .clone();
        let owner = ResourceOwner::new(fresh_owner_identity(), session, 0);
        let images = SceneImageRegistry::new(owner);
        let loader = SceneTextureLoader::new(images, Box::new(FakeReader), None, None, 224).expect("loader");
        SceneShaderRegistry::with_defaults(loader)
    }

    fn media() -> ApplicationQ3Assets {
        let mut files = std::collections::HashMap::new();
        files.insert(
            (CONTENT.to_string(), "sound/test.wav".to_string()),
            OpenedQ3Asset {
                id: "resource:sound".to_string(),
                bytes: wav_bytes(),
            },
        );
        files.insert(
            (CONTENT.to_string(), "models/box.md3".to_string()),
            OpenedQ3Asset {
                id: "resource:box".to_string(),
                bytes: vec![7],
            },
        );
        let mut models = std::collections::HashMap::new();
        models.insert(
            (CONTENT.to_string(), "models/box.md3".to_string()),
            super::super::assets::Q3LoadedModel {
                model: super::super::assets::Q3AssetModel::Decoded(Q3DecodedModel::Framed { frames: Vec::new() }),
                resource: PresentResource::new("models/box.md3"),
            },
        );
        ApplicationQ3Assets::create(ApplicationQ3AssetOptions {
            content: ContentId(CONTENT.to_string()),
            mounts: Box::new(StubMounts { files }),
            models: Box::new(StubModels { models }),
            catalog: Box::new(StubCatalog),
            world: Box::new(StubAssetWorld),
            shaders: shaders(),
            print: Box::new(|_| {}),
            save_font_data: Box::new(|| false),
            user_root: None,
            scope: Q3AssetScope::Selected,
            mode: Q3AssetMode::GuestAsync,
        })
        .expect("media")
    }

    #[derive(Default)]
    struct StubOutput {
        scenes: usize,
        commands: Vec<RenderCommand>,
        texts: Vec<Q3ServiceTextDraw>,
        audio: Vec<String>,
        listeners: Vec<(Vec3, Axis)>,
    }

    impl Q3ServiceOutput for StubOutput {
        fn scene(&mut self, _scene: Q3PresentedScene) {
            self.scenes += 1;
        }

        fn command(&mut self, command: RenderCommand) {
            self.commands.push(command);
        }

        fn text(&mut self, draw: Q3ServiceTextDraw) {
            self.texts.push(draw);
        }

        fn audio(&mut self, operation: Q3SeatAudioOperation) {
            self.audio.push(match operation {
                Q3SeatAudioOperation::Play { .. } => "play".to_string(),
                Q3SeatAudioOperation::Loop { .. } => "loop".to_string(),
                Q3SeatAudioOperation::Position { .. } => "position".to_string(),
                Q3SeatAudioOperation::StopLoop { .. } => "stop".to_string(),
                Q3SeatAudioOperation::ClearLoops { .. } => "clear".to_string(),
                Q3SeatAudioOperation::ReleaseOwner => "release".to_string(),
            });
        }

        fn listener(&mut self, origin: Vec3, axis: Axis) {
            self.listeners.push((origin, axis));
        }
    }

    type SharedOutput = Rc<RefCell<StubOutput>>;

    struct OutputProxy(SharedOutput);

    impl Q3ServiceOutput for OutputProxy {
        fn scene(&mut self, scene: Q3PresentedScene) {
            self.0.borrow_mut().scene(scene);
        }

        fn command(&mut self, command: RenderCommand) {
            self.0.borrow_mut().command(command);
        }

        fn text(&mut self, draw: Q3ServiceTextDraw) {
            self.0.borrow_mut().text(draw);
        }

        fn audio(&mut self, operation: Q3SeatAudioOperation) {
            self.0.borrow_mut().audio(operation);
        }

        fn listener(&mut self, origin: Vec3, axis: Axis) {
            self.0.borrow_mut().listener(origin, axis);
        }
    }

    #[derive(Default)]
    struct StubAudio {
        music: Vec<String>,
    }

    impl Q3ServiceAudio for StubAudio {
        fn play_music(&mut self, _content: &ContentId, spec: &str) {
            self.music.push(spec.to_string());
        }
    }

    struct StubClip;

    impl Q3ServiceClip for StubClip {
        fn cluster_pvs(&self, cluster: i32) -> Vec<u8> {
            vec![cluster as u8, 0xFF]
        }
    }

    #[derive(Default)]
    struct StubImages;

    impl CinematicImages for StubImages {
        fn allocate(&mut self, _width: usize, _height: usize, _resource: &str) -> u32 {
            1
        }

        fn commit(&mut self, _operation: ImageOperation) {}

        fn release(&mut self, _image: u32) {}
    }

    #[derive(Default)]
    struct StubMixer;

    impl CinematicMixer for StubMixer {
        type StreamCheckpoint = ();

        fn queue_stream(&mut self, _target: &AudioStreamTarget, _pcm: &StreamPcm) {}

        fn stop_stream(&mut self, _id: &str) {}

        fn pause_stream(&mut self, _id: &str, _paused: bool) {}
    }

    #[derive(Default)]
    struct StubQueries {
        contents: i32,
    }

    impl Q3ClientMapQueries for StubQueries {
        fn trace_q3(&mut self, query: &Q3MapTrace<'_>) -> Q3MapTraceHit {
            Q3MapTraceHit {
                fraction: 1.0,
                end: query.end,
                all_solid: false,
                start_solid: false,
                contact: TraceContact::None,
                contents: 0,
                surface_flags: 0,
            }
        }

        fn point_contents_q3(
            &mut self,
            _point: Vec3,
            _model: i32,
            _origin: Vec3,
            _angles: Vec3,
            _curves: bool,
            _player_curve_clip: bool,
        ) -> i32 {
            self.contents
        }

        fn box_trace_q3(&mut self, _mins: Vec3, _maxs: Vec3, query: &Q3MapTrace<'_>) -> Q3MapTraceHit {
            self.trace_q3(query)
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("q3-services-owner").expect("owner")
    }

    fn services() -> (
        ApplicationQ3Services<StubMixer>,
        SharedOutput,
        Rc<RefCell<StubAudio>>,
        SeatId,
    ) {
        services_with(None)
    }

    fn services_with(
        remap_shader: Option<super::super::assets::Q3RemapOverride>,
    ) -> (
        ApplicationQ3Services<StubMixer>,
        SharedOutput,
        Rc<RefCell<StubAudio>>,
        SeatId,
    ) {
        let registry = owner();
        let seat = registry.seat(0);
        let actor_owner = registry.actor(0, 0);
        let output: SharedOutput = Rc::new(RefCell::new(StubOutput::default()));
        let audio = Rc::new(RefCell::new(StubAudio::default()));
        let audio_service = {
            let audio = audio.clone();
            struct Proxy(Rc<RefCell<StubAudio>>);
            impl Q3ServiceAudio for Proxy {
                fn play_music(&mut self, content: &ContentId, spec: &str) {
                    self.0.borrow_mut().play_music(content, spec);
                }
            }
            Box::new(Proxy(audio)) as Box<dyn Q3ServiceAudio>
        };
        let image_owner = ResourceOwner::new(fresh_owner_identity(), registry.session().clone(), 1);
        let services = ApplicationQ3Services::create(ApplicationQ3ServiceOptions {
            media: media(),
            audio: audio_service,
            images: Box::new(StubImages),
            mixer: StubMixer,
            seat: seat.clone(),
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
            queries: Box::new(StubQueries { contents: 3 }),
            collision_settings: CollisionMapSettings {
                no_curves: false,
                player_curve_clip: true,
            },
            world: Some(Q3ServiceResourceWorld {
                map: ResourceWorldMap {
                    entities: String::new(),
                    nodes: Vec::new(),
                    leaves: Vec::new(),
                    planes: Vec::new(),
                },
                clip: Box::new(StubClip),
            }),
            resource_handles: None,
            owner: None,
            remap_shader,
            actor_at: Box::new(move |_| actor_owner.clone()),
            clock_now: Box::new(|| 1000.0),
            clock_frame: Box::new(|| 7),
            system_cinematics: None,
            image_at: Box::new(move |ordinal| RendererImage {
                owner: image_owner.clone(),
                ordinal,
                source: ImageSource::Generated {
                    name: "test".to_string(),
                },
                width: 64,
                height: 64,
            }),
            output: Box::new(OutputProxy(output.clone())),
        });
        (services, output, audio, seat)
    }

    #[test]
    fn resources_register_through_media_host() {
        let (mut services, _, _, _) = services();
        let model = services
            .resources
            .register_model(Some("models/box.md3"))
            .expect("register");
        assert_eq!(services.resources.model_handle(&model).expect("handle"), 1);
    }

    #[test]
    fn scene_renders_publish_output() {
        let (mut services, output, _, _) = services();
        services.resources.clear_scene();
        services.resources.render_scene(&create_refdef());
        assert_eq!(output.borrow().scenes, 1);
        assert_eq!(services.scene.borrow().capture().models.len(), 0);
    }

    #[test]
    fn remap_override_replaces_world_remap() {
        let called = Rc::new(std::cell::Cell::new(false));
        let flag = called.clone();
        let (mut services, _, _, _) = services_with(Some(Box::new(
            move |original: &str, replacement: &str, offset: &str| {
                assert_eq!((original, replacement, offset), ("a", "b", "0.5"));
                flag.set(true);
                Ok(())
            },
        )));
        services.resources.remap_shader("a", "b", "0.5").expect("remap");
        assert!(called.get());
    }

    #[test]
    fn sounds_emit_play_operations() {
        let (mut services, output, _, _) = services();
        let pcm = services
            .media
            .bank_shared()
            .borrow_mut()
            .register_sound(Some("sound/test.wav"), false);
        assert!(pcm.is_some());
        let handle = services.media.bank_shared().borrow().index_for_sound(&pcm);
        assert!(handle > 0);
        services
            .sound
            .start_sound(None, 1, 2, Some(RetailPcmSound::new(handle as u32)));
        assert_eq!(output.borrow().audio, vec!["play".to_string()]);
        services.sound.clear_looping_sounds(true);
        services.sound.set_listener(
            0,
            vec3(1.0, 2.0, 3.0),
            [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
        );
        assert_eq!(output.borrow().audio.len(), 2);
        assert_eq!(output.borrow().listeners.len(), 1);
    }

    #[test]
    fn background_track_plays_music_spec() {
        let (mut services, _, audio, _) = services();
        services.sound.start_background_track("intro", "loop").expect("track");
        assert_eq!(audio.borrow().music, vec!["intro loop".to_string()]);
    }

    #[test]
    fn loops_update_and_stop() {
        let (mut services, output, _, _) = services();
        let pcm = services
            .media
            .bank_shared()
            .borrow_mut()
            .register_sound(Some("sound/test.wav"), false);
        let handle = services.media.bank_shared().borrow().index_for_sound(&pcm);
        services.sound.add_loop_sound(
            1,
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            Some(RetailPcmSound::new(handle as u32)),
            true,
        );
        services.sound.update_sound_position(1, vec3(4.0, 0.0, 0.0));
        services.sound.stop_looping_sound(1);
        assert_eq!(
            output.borrow().audio,
            vec!["loop".to_string(), "position".to_string(), "stop".to_string()]
        );
    }

    #[test]
    fn draw_forwards_commands_and_text() {
        let (mut services, output, _, _) = services();
        {
            let mut draw = services.draw_2d();
            draw.set_color(Some(vec4(1.0, 0.0, 0.0, 1.0)));
            draw.stretch_pic(
                Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 32.0,
                    height: 32.0,
                },
                qa_client::text::draw2d::TextureRect {
                    s: 0.0,
                    t: 0.0,
                    s2: 1.0,
                    t2: 1.0,
                },
                PictureAsset::Image(ImagePicture {
                    image: 9,
                    width: 64,
                    height: 64,
                }),
            );
            draw.stretch_pic(
                Rect {
                    x: 1000.0,
                    y: 0.0,
                    width: 32.0,
                    height: 32.0,
                },
                qa_client::text::draw2d::TextureRect {
                    s: 0.0,
                    t: 0.0,
                    s2: 1.0,
                    t2: 1.0,
                },
                PictureAsset::Image(ImagePicture {
                    image: 9,
                    width: 64,
                    height: 64,
                }),
            );
            draw.stretch_pic(
                Rect {
                    x: 20.0,
                    y: 20.0,
                    width: 16.0,
                    height: 16.0,
                },
                qa_client::text::draw2d::TextureRect {
                    s: 0.0,
                    t: 0.0,
                    s2: 1.0,
                    t2: 1.0,
                },
                PictureAsset::Material(DrawMaterialPicture { order: 2 }),
            );
        }
        assert_eq!(output.borrow().commands.len(), 2);
        assert_eq!(output.borrow().texts.len(), 1);
        assert_eq!(output.borrow().texts[0].rect.x, 20.0);
    }

    #[test]
    fn collision_delegates_to_queries() {
        use qa_content::q3::presentation::collision_host::CollisionWorld;
        let (mut services, _, _, _) = services();
        assert_eq!(services.collision.point_contents(vec3(0.0, 0.0, 0.0)), 3);
    }

    #[test]
    fn cinematics_prepare_missing_movies() {
        let (mut services, _, _, _) = services();
        assert!(services.cinematics.prepare_asset("missing.roq").is_err());
    }

    #[test]
    fn resource_world_reads_clip_rows() {
        let world = Q3ServiceResourceWorld {
            map: ResourceWorldMap {
                entities: String::new(),
                nodes: Vec::new(),
                leaves: Vec::new(),
                planes: Vec::new(),
            },
            clip: Box::new(StubClip),
        };
        assert_eq!(world.cluster_pvs_byte(5, 0), 5);
        assert_eq!(world.cluster_pvs_byte(5, 1), 0xFF);
        assert_eq!(world.cluster_pvs_byte(5, 9), 0);
    }
}
