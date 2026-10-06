//! Native Q1 status bar: `gfx.wad` pictures, live-state feed, draw batches.
//!
//! WinQuake `sbar.c` presentation over [`NativeQ1HudFrame`] layout ops
//! (`qa_client::ui::hud::q1_native`). Pictures decode from the mounted
//! `gfx.wad` plus `gfx/palette.lmp` (the `menu_font.rs` open pattern)
//! and the `gfx/*.lmp` plates; the feed folds live simulation state
//! (combat health/armor, carried items/ammo, kill and secret counters,
//! client clock) into bar stats. Batches are NDC quads in op order
//! (the `windowed_menu.rs` menu-batch shape), uploaded once on the
//! first frame.

use std::collections::HashMap;

use qa_client::render::scene::resources::{rgba_image, SceneImageRegistry};
use qa_client::render::types::{
    BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch, ImageLevel,
    ImageResourceOperation, RenderState, RenderVertex, ResourceOwner, TextureBinding, TextureFilter, TextureSampling,
};
use qa_client::ui::hud::q1_native::{
    NativeQ1HudFrame, NativeQ1HudOperation, Q1SbarProduct, Q1_AMMO_LUMPS, Q1_ARMOR_LUMPS, Q1_HIPNOTIC_ITEM_LUMPS,
    Q1_HIPNOTIC_STEMS, Q1_ITEM_LUMPS, Q1_IT_CELLS, Q1_IT_GRENADE_LAUNCHER, Q1_IT_LIGHTNING, Q1_IT_NAILGUN, Q1_IT_NAILS,
    Q1_IT_ROCKETS, Q1_IT_ROCKET_LAUNCHER, Q1_IT_SHELLS, Q1_IT_SHOTGUN, Q1_IT_SUPER_NAILGUN, Q1_IT_SUPER_SHOTGUN,
    Q1_ROGUE_AMMO_LUMPS, Q1_ROGUE_ITEM_LUMPS, Q1_ROGUE_WEAPONS, Q1_SIGIL_LUMPS, Q1_STAT_ACTIVEWEAPON, Q1_STAT_AMMO,
    Q1_STAT_ARMOR, Q1_STAT_CELLS, Q1_STAT_FRAGS, Q1_STAT_HEALTH, Q1_STAT_MONSTERS, Q1_STAT_NAILS, Q1_STAT_ROCKETS,
    Q1_STAT_SECRETS, Q1_STAT_SHELLS, Q1_STAT_TOTALMONSTERS, Q1_STAT_TOTALSECRETS, Q1_STAT_WEAPON, Q1_WEAPON_STEMS,
};
use qa_client::ui::hud::q1_view_blend::{Q1EyeContents, Q1ViewBlends, Q1_FACE_PAIN_SECONDS};
use qa_content::images::indexed::decode_qpic;
use qa_content::images::palette::decode_palette;
use qa_content::images::wad::{decode_wad, OwnedWadArchive};
use qa_content::mounts::MountedContent;
use qa_core::math::{angle_vectors, vec2, vec4};

use super::simulation::native_q1_spawns::{Q1NativeBehaviors, Q1PlayerDamage};

/// Opaque bar plates (`Draw_Pic` keeps palette index 0 black).
const OPAQUE_LUMPS: [&str; 8] = [
    "sbar",
    "ibar",
    "scorebar",
    "r_invbar1",
    "r_invbar2",
    "disc",
    "gfx/complete.lmp",
    "gfx/ranking.lmp",
];

/// `conchars` dimensions (raw 128x128 lump, the `menu_font.rs` shape).
const CONCHARS_SIZE: u32 = 128;

/// Face lumps (`sbar.c:175-189`).
const FACE_LUMPS: [&str; 14] = [
    "face1",
    "face_p1",
    "face2",
    "face_p2",
    "face3",
    "face_p3",
    "face4",
    "face_p4",
    "face5",
    "face_p5",
    "face_invis",
    "face_invul2",
    "face_inv2",
    "face_quad",
];

/// Lump names the bar needs for one product.
fn lump_names(product: Q1SbarProduct) -> Vec<String> {
    let mut names = vec![
        "sbar".to_string(),
        "ibar".to_string(),
        "scorebar".to_string(),
        "disc".to_string(),
        "conchars".to_string(),
        "num_minus".to_string(),
        "anum_minus".to_string(),
        "num_colon".to_string(),
        "num_slash".to_string(),
        "gfx/complete.lmp".to_string(),
        "gfx/inter.lmp".to_string(),
        "gfx/ranking.lmp".to_string(),
        "gfx/finale.lmp".to_string(),
    ];
    for digit in 0..10 {
        names.push(format!("num_{digit}"));
        names.push(format!("anum_{digit}"));
    }
    names.extend(FACE_LUMPS.iter().map(ToString::to_string));
    names.extend(Q1_AMMO_LUMPS.iter().map(ToString::to_string));
    names.extend(Q1_ARMOR_LUMPS.iter().map(ToString::to_string));
    names.extend(Q1_ITEM_LUMPS.iter().map(ToString::to_string));
    names.extend(Q1_SIGIL_LUMPS.iter().map(ToString::to_string));
    for stem in Q1_WEAPON_STEMS {
        names.push(format!("inv_{stem}"));
        names.push(format!("inv2_{stem}"));
        for flash in 1..=5 {
            names.push(format!("inva{flash}_{stem}"));
        }
    }
    if product == Q1SbarProduct::Hipnotic {
        names.extend(Q1_HIPNOTIC_ITEM_LUMPS.iter().map(ToString::to_string));
        for stem in Q1_HIPNOTIC_STEMS {
            names.push(format!("inv_{stem}"));
            names.push(format!("inv2_{stem}"));
            for flash in 1..=5 {
                names.push(format!("inva{flash}_{stem}"));
            }
        }
    }
    if product == Q1SbarProduct::Rogue {
        names.extend(["r_invbar1", "r_invbar2", "r_teambord"].iter().map(ToString::to_string));
        names.extend(Q1_ROGUE_WEAPONS.iter().map(ToString::to_string));
        names.extend(Q1_ROGUE_AMMO_LUMPS.iter().map(ToString::to_string));
        names.extend(Q1_ROGUE_ITEM_LUMPS.iter().map(ToString::to_string));
    }
    names.sort();
    names.dedup();
    names
}

/// Decode a `gfx/*.lmp` plate: little-endian width/height plus raw indices.
fn decode_lmp(bytes: &[u8], source: &str) -> Result<(u32, u32, Vec<u8>), String> {
    if bytes.len() < 8 {
        return Err(format!("{source} is too short for an LMP header"));
    }
    let width = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let height = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let count = width as usize * height as usize;
    if bytes.len() - 8 < count {
        return Err(format!("{source} ends inside its pixels"));
    }
    Ok((width, height, bytes[8..8 + count].to_vec()))
}

/// Expand indices through the palette; transparent lumps clear index 0.
fn expand_rgba(width: u32, height: u32, indices: &[u8], palette: &[u8], opaque: bool) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(indices.len() * 4);
    for index in indices {
        let base = usize::from(*index) * 3;
        pixels.push(palette[base]);
        pixels.push(palette[base + 1]);
        pixels.push(palette[base + 2]);
        pixels.push(if *index == 0 && !opaque { 0 } else { 255 });
    }
    debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
    pixels
}

/// One uploaded bar picture.
#[derive(Debug, Clone)]
pub struct Q1HudPicture {
    /// Backend image.
    pub image: qa_client::render::types::RendererImage,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// One decoded bar picture, ready to register.
#[derive(Debug, Clone)]
pub struct Q1DecodedPicture {
    /// Lump name.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Top-down RGBA texels.
    pub pixels: Vec<u8>,
}

/// Decode the bar pictures for `product` from mounted id1 data.
/// Missing lumps are skipped (their ops do not draw) so a partial
/// corpus still presents a bar; `gfx.wad` and its palette are
/// required.
pub fn decode_hud_pictures(
    mounts: &MountedContent,
    product: Q1SbarProduct,
) -> Result<(Vec<Q1DecodedPicture>, Vec<u8>), String> {
    let wad = mounts
        .open("gfx.wad", |_| true)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Q1 HUD requires gfx.wad".to_string())?;
    let palette_file = mounts
        .open("gfx/palette.lmp", |_| true)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Q1 HUD requires gfx/palette.lmp".to_string())?;
    let archive = decode_wad(&wad.bytes, "gfx.wad").map_err(|error| error.to_string())?;
    let palette = decode_palette(&palette_file.bytes, "gfx/palette.lmp")
        .map_err(|error| error.to_string())?
        .colors;
    let mut decoded = Vec::new();
    for name in lump_names(product) {
        let Some((width, height, indices)) = load_lump(mounts, &archive, &name) else {
            continue;
        };
        let opaque = OPAQUE_LUMPS.contains(&name.as_str());
        decoded.push(Q1DecodedPicture {
            name,
            width,
            height,
            pixels: expand_rgba(width, height, &indices, &palette, opaque),
        });
    }
    Ok((decoded, palette))
}

/// Native Q1 status bar: uploaded pictures plus the palette.
pub struct Q1NativeHud {
    pictures: HashMap<String, Q1HudPicture>,
    palette: Vec<u8>,
    white: qa_client::render::types::RendererImage,
    pending_uploads: Vec<ImageResourceOperation>,
    uploaded: bool,
    /// Content product selecting the lump set.
    pub product: Q1SbarProduct,
    /// Live view blends (damage/bonus/contents/powerup + kick).
    blends: Q1ViewBlends,
    /// Last consumed player-damage sequence number.
    last_damage_seq: u64,
    /// Last seen carried-items bits (pickup delta source).
    last_items: u32,
    /// Last seen shells/nails/rockets/cells (pickup delta source).
    last_ammo: [f64; 4],
    /// Per-item-bit pickup times for the bar flash window.
    item_gettime: [f32; 32],
    /// Pain-face hold for the bar (`face_anim_until`).
    face_anim_until: f32,
    /// Kick offsets for the next camera build (roll, pitch).
    pending_kick: (f32, f32),
    /// Whether a live snapshot has been taken yet.
    view_seen: bool,
    /// Last wall-clock frame time, milliseconds.
    last_wall_ms: Option<f64>,
}

impl Q1NativeHud {
    /// Register decoded pictures into a shared image registry (the
    /// windowed path: the world's registry, whose per-frame drain
    /// owns the uploads, so this HUD emits none itself).
    pub fn register(
        decoded: Vec<Q1DecodedPicture>,
        palette: Vec<u8>,
        white: qa_client::render::types::RendererImage,
        registry: &mut SceneImageRegistry,
        product: Q1SbarProduct,
    ) -> Result<Self, String> {
        let sampling = TextureSampling {
            repeat: false,
            filter: TextureFilter::Nearest,
        };
        let mut pictures = HashMap::new();
        for picture in decoded {
            let image = registry
                .register(
                    &format!("q1-hud:{}", picture.name),
                    rgba_image(ImageLevel {
                        width: picture.width,
                        height: picture.height,
                        pixels: picture.pixels,
                    }),
                    sampling,
                )
                .map_err(|error| error.to_string())?;
            pictures.insert(
                picture.name,
                Q1HudPicture {
                    image,
                    width: picture.width,
                    height: picture.height,
                },
            );
        }
        Ok(Self {
            pictures,
            palette,
            white,
            pending_uploads: Vec::new(),
            uploaded: true,
            product,
            blends: Q1ViewBlends::default(),
            last_damage_seq: 0,
            last_items: 0,
            last_ammo: [0.0; 4],
            item_gettime: [0.0; 32],
            face_anim_until: -1.0,
            pending_kick: (0.0, 0.0),
            view_seen: false,
            last_wall_ms: None,
        })
    }

    /// Load the bar pictures into a private registry (standalone and
    /// test use): uploads emit once through [`Q1NativeHud::uploads`].
    pub fn open(mounts: &MountedContent, product: Q1SbarProduct, owner: ResourceOwner) -> Result<Self, String> {
        let (decoded, palette) = decode_hud_pictures(mounts, product)?;
        let mut registry = SceneImageRegistry::new(owner);
        let white = registry
            .register(
                "q1-hud:white",
                rgba_image(ImageLevel {
                    width: 1,
                    height: 1,
                    pixels: vec![255, 255, 255, 255],
                }),
                TextureSampling {
                    repeat: false,
                    filter: TextureFilter::Nearest,
                },
            )
            .map_err(|error| error.to_string())?;
        let mut hud = Self::register(decoded, palette, white, &mut registry, product)?;
        hud.pending_uploads = registry.drain_operations();
        hud.uploaded = false;
        Ok(hud)
    }

    /// Picture count (loaded lumps).
    #[must_use]
    pub fn picture_count(&self) -> usize {
        self.pictures.len()
    }

    /// Whether a lump loaded.
    #[must_use]
    pub fn has_picture(&self, lump: &str) -> bool {
        self.pictures.contains_key(lump)
    }

    /// Upload operations, emitted once on the first frame.
    pub fn uploads(&mut self) -> Vec<ImageResourceOperation> {
        if self.uploaded {
            return Vec::new();
        }
        self.uploaded = true;
        std::mem::take(&mut self.pending_uploads)
    }

    /// Draw layout ops as NDC batches over a `width`x`height` viewport.
    /// Ops referencing missing lumps are skipped.
    pub fn draw(&self, ops: &[NativeQ1HudOperation], width: i32, height: i32) -> Vec<DrawBatch> {
        let quads = layout_quads(ops, &self.picture_sizes(), width, &self.palette);
        quads_to_batches(&quads, &self.quad_images(), width as f32, height as f32)
    }

    /// Fold one frame of live state into the view blends and return the
    /// combined `v_blend` rgba (or `None` without an admitted player).
    /// Consumes fresh `q1_t_damage` events (flash + kick + pain face),
    /// raises the bonus flash on pickup deltas (new item bits or ammo
    /// gains), and recomputes contents/powerup every frame, like stock's
    /// per-frame `V_CalcPowerupCshift`/`V_SetContentsColor`. `wall_dt` is
    /// real frame time for decay; `sim_now` is the client clock the bar
    /// compares pickup/face times against.
    pub fn update_view_state(
        &mut self,
        behaviors: &Q1NativeBehaviors,
        simulation: &qa_world::session::Simulation,
        eye_contents: Option<i32>,
        angles: [f32; 3],
        wall_dt: f32,
        sim_now: f32,
    ) -> Option<[f32; 4]> {
        let player = behaviors.player.as_ref()?;
        let items = behaviors.player_items;
        let ammo = &behaviors.player_ammo;
        let counts = [ammo.shells, ammo.nails, ammo.rockets, ammo.cells];
        if !self.view_seen {
            self.view_seen = true;
            self.last_damage_seq = behaviors.player_damage.seq;
            self.last_items = items;
            self.last_ammo = counts;
        }
        // Fresh damage events flash, kick, and hold the pain face.
        let event: &Q1PlayerDamage = &behaviors.player_damage;
        if event.seq != self.last_damage_seq {
            self.last_damage_seq = event.seq;
            let count = self.blends.damage(event.armor, event.blood);
            if let Some(from) = event.from {
                if let Some(body) = simulation.body_state(player) {
                    let origin = body.origin;
                    let axes = angle_vectors(qa_core::math::vec3(angles[0], angles[1], angles[2]));
                    self.blends.damage_kick(
                        [from[0] - origin.x, from[1] - origin.y, from[2] - origin.z],
                        [axes.forward.x, axes.forward.y, axes.forward.z],
                        [axes.right.x, axes.right.y, axes.right.z],
                        count,
                    );
                }
            }
            self.face_anim_until = sim_now + Q1_FACE_PAIN_SECONDS;
        }
        // Pickup deltas flash gold and open the bar item window. Weapon
        // switches only move the derived ammo-icon bit (not stored in
        // `player_items`), so they never flash.
        let fresh_items = items & !self.last_items;
        let ammo_grew = counts.iter().zip(self.last_ammo.iter()).any(|(now, was)| now > was);
        if fresh_items != 0 || ammo_grew {
            self.blends.bonus();
            let mut bits = fresh_items;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                if bit < 32 {
                    self.item_gettime[bit] = sim_now;
                }
                bits &= bits - 1;
            }
        }
        self.last_items = items;
        self.last_ammo = counts;
        self.blends.set_contents(
            eye_contents
                .map(Q1EyeContents::from_raw)
                .unwrap_or(Q1EyeContents::Empty),
        );
        self.blends.set_powerup(items);
        let wall_dt = wall_dt.clamp(0.0, 0.5);
        self.blends.tick(wall_dt);
        self.pending_kick = self.blends.kick_offsets(wall_dt);
        Some(self.blends.blend())
    }

    /// Fullscreen blend quad over a `width`x`height` viewport, drawn
    /// after the bar (stock tints through the palette, so the bar shifts
    /// too). Empty while the blend is clear.
    pub fn draw_blend(&self, rgba: [f32; 4], width: i32, height: i32) -> Vec<DrawBatch> {
        if rgba[3] <= 0.0 {
            return Vec::new();
        }
        let quads = [Q1HudQuad {
            rect: (0.0, 0.0, width as f32, height as f32),
            uv: (0.0, 0.0, 1.0, 1.0),
            color: (rgba[0], rgba[1], rgba[2], rgba[3]),
            texture: Q1QuadTexture::White,
        }];
        quads_to_batches(&quads, &self.quad_images(), width as f32, height as f32)
    }

    /// Kick offsets for the next camera build (roll, pitch).
    #[must_use]
    pub fn pending_kick(&self) -> (f32, f32) {
        self.pending_kick
    }

    /// Per-item-bit pickup times for the bar frame.
    #[must_use]
    pub fn item_gettime(&self) -> [f32; 32] {
        self.item_gettime
    }

    /// Pain-face hold for the bar frame.
    #[must_use]
    pub fn face_anim_until(&self) -> f32 {
        self.face_anim_until
    }

    /// Real frame delta in seconds since the last call (0 on the first).
    pub fn wall_dt(&mut self, time_ms: f64) -> f32 {
        let dt = self.last_wall_ms.map_or(0.0, |last| (time_ms - last) as f32);
        self.last_wall_ms = Some(time_ms);
        dt
    }

    fn picture_sizes(&self) -> HashMap<String, (u32, u32)> {
        self.pictures
            .iter()
            .map(|(name, picture)| (name.clone(), (picture.width, picture.height)))
            .collect()
    }

    fn quad_images(&self) -> HashMap<Q1QuadTexture, qa_client::render::types::RendererImage> {
        let mut images = HashMap::new();
        for (name, picture) in &self.pictures {
            images.insert(Q1QuadTexture::Picture(name.clone()), picture.image.clone());
        }
        images.insert(Q1QuadTexture::White, self.white.clone());
        if let Some(conchars) = self.pictures.get("conchars") {
            images.insert(Q1QuadTexture::Conchars, conchars.image.clone());
        }
        images
    }
}

/// Load one lump's indices: `gfx/*.lmp` plates decode as LMP files,
/// `conchars` is raw 128x128, everything else is a qpic.
fn load_lump(mounts: &MountedContent, archive: &OwnedWadArchive, name: &str) -> Option<(u32, u32, Vec<u8>)> {
    if name.ends_with(".lmp") {
        let file = mounts.open(name, |_| true).ok()??;
        return decode_lmp(&file.bytes, name).ok();
    }
    let lump = archive.lumps.iter().find(|lump| lump.name == name)?;
    if name == "conchars" {
        if lump.bytes.len() != CONCHARS_SIZE as usize * CONCHARS_SIZE as usize {
            return None;
        }
        return Some((CONCHARS_SIZE, CONCHARS_SIZE, lump.bytes.clone()));
    }
    let image = decode_qpic(&lump.bytes, &format!("gfx.wad:{name}")).ok()?;
    Some((image.width, image.height, image.indices))
}

/// Current-ammo stat and ammo-icon bit from the active weapon
/// (`W_SetCurrentAmmo`, `weapons.qc:758-817`): the axe and unknown
/// weapons read 0 with no icon.
fn current_ammo(active_weapon: u32, shells: f64, nails: f64, rockets: f64, cells: f64) -> (i32, u32) {
    if active_weapon == Q1_IT_SHOTGUN || active_weapon == Q1_IT_SUPER_SHOTGUN {
        return (shells as i32, Q1_IT_SHELLS);
    }
    if active_weapon == Q1_IT_NAILGUN || active_weapon == Q1_IT_SUPER_NAILGUN {
        return (nails as i32, Q1_IT_NAILS);
    }
    if active_weapon == Q1_IT_GRENADE_LAUNCHER || active_weapon == Q1_IT_ROCKET_LAUNCHER {
        return (rockets as i32, Q1_IT_ROCKETS);
    }
    if active_weapon == Q1_IT_LIGHTNING {
        return (cells as i32, Q1_IT_CELLS);
    }
    (0, 0)
}

/// Fold live simulation state into a bar frame: combat health and Q1
/// armor points, carried items and ammo, frags, kill/secret counters,
/// and the client clock. The ammo-icon bits derive from the active
/// weapon when gamecode has not set them yet (the weapons slice owns
/// `W_SetCurrentAmmo`; the mapping is identical).
pub fn q1_frame_from_live(
    behaviors: &Q1NativeBehaviors,
    simulation: &qa_world::session::Simulation,
    levelname: &str,
    deathmatch: bool,
    product: Q1SbarProduct,
    item_gettime: [f32; 32],
    face_anim_until: f32,
) -> Option<NativeQ1HudFrame> {
    let player = behaviors.player.as_ref()?;
    let combat = simulation.combat_state(player).cloned().unwrap_or_default();
    let armor = match &combat.armor.regular {
        qa_world::combat::RegularArmor::Q1 { points, .. } => *points as i32,
        _ => 0,
    };
    let (ammo, ammo_bit) = current_ammo(
        behaviors.player_active_weapon,
        behaviors.player_ammo.shells,
        behaviors.player_ammo.nails,
        behaviors.player_ammo.rockets,
        behaviors.player_ammo.cells,
    );
    let mut stats = [0i32; 32];
    stats[Q1_STAT_HEALTH] = combat.health as i32;
    stats[Q1_STAT_FRAGS] = behaviors.player_frags;
    stats[Q1_STAT_WEAPON] = behaviors.player_active_weapon as i32;
    stats[Q1_STAT_AMMO] = ammo;
    stats[Q1_STAT_ARMOR] = armor;
    stats[Q1_STAT_SHELLS] = behaviors.player_ammo.shells as i32;
    stats[Q1_STAT_NAILS] = behaviors.player_ammo.nails as i32;
    stats[Q1_STAT_ROCKETS] = behaviors.player_ammo.rockets as i32;
    stats[Q1_STAT_CELLS] = behaviors.player_ammo.cells as i32;
    stats[Q1_STAT_ACTIVEWEAPON] = behaviors.player_active_weapon as i32;
    stats[Q1_STAT_TOTALSECRETS] = behaviors.total_secrets as i32;
    stats[Q1_STAT_TOTALMONSTERS] = behaviors.total_monsters as i32;
    stats[Q1_STAT_SECRETS] = behaviors.found_secrets as i32;
    stats[Q1_STAT_MONSTERS] = behaviors.killed_monsters as i32;
    Some(NativeQ1HudFrame {
        stats,
        items: behaviors.player_items | ammo_bit,
        item_gettime,
        time: simulation.frame().time.as_seconds_f64() as f32,
        face_anim_until,
        levelname: levelname.to_string(),
        deathmatch,
        maxclients: 1,
        viewentity: 1,
        showscores: false,
        scores: Vec::new(),
        product,
        teamplay: 0.0,
        completed_time: 0.0,
    })
}

/// Quad texture key: a lump picture or the white fill image.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Q1QuadTexture {
    /// Lump picture.
    Picture(String),
    /// White fill image.
    White,
    /// `conchars` glyph atlas.
    Conchars,
}

/// One textured screen quad in viewport pixels.
#[derive(Debug, Clone, PartialEq)]
struct Q1HudQuad {
    /// Destination rect.
    rect: (f32, f32, f32, f32),
    /// Source UVs.
    uv: (f32, f32, f32, f32),
    /// Tint color.
    color: (f32, f32, f32, f32),
    /// Texture key.
    texture: Q1QuadTexture,
}

/// Lower layout ops to screen quads: pictures at natural size,
/// centered plates resolved against loaded sizes, conchars glyphs as
/// 8x8 atlas cells (spaces skipped, like stock's blank advance),
/// fills as white quads tinted with the palette color. Ops naming
/// missing lumps are skipped.
fn layout_quads(
    ops: &[NativeQ1HudOperation],
    sizes: &HashMap<String, (u32, u32)>,
    width: i32,
    palette: &[u8],
) -> Vec<Q1HudQuad> {
    let mut quads = Vec::new();
    for op in ops {
        match op {
            NativeQ1HudOperation::Picture { x, y, lump } | NativeQ1HudOperation::TransPicture { x, y, lump } => {
                let Some((w, h)) = sizes.get(lump) else {
                    continue;
                };
                quads.push(Q1HudQuad {
                    rect: (*x as f32, *y as f32, *w as f32, *h as f32),
                    uv: (0.0, 0.0, 1.0, 1.0),
                    color: (1.0, 1.0, 1.0, 1.0),
                    texture: Q1QuadTexture::Picture(lump.clone()),
                });
            }
            NativeQ1HudOperation::CenteredPicture { x_base, y, lump } => {
                let Some((w, h)) = sizes.get(lump) else {
                    continue;
                };
                quads.push(Q1HudQuad {
                    rect: (
                        (*x_base + (320 - *w as i32) / 2) as f32,
                        *y as f32,
                        *w as f32,
                        *h as f32,
                    ),
                    uv: (0.0, 0.0, 1.0, 1.0),
                    color: (1.0, 1.0, 1.0, 1.0),
                    texture: Q1QuadTexture::Picture(lump.clone()),
                });
            }
            NativeQ1HudOperation::ViewportCenteredPicture { y, lump } => {
                let Some((w, h)) = sizes.get(lump) else {
                    continue;
                };
                quads.push(Q1HudQuad {
                    rect: ((width - *w as i32) as f32 / 2.0, *y as f32, *w as f32, *h as f32),
                    uv: (0.0, 0.0, 1.0, 1.0),
                    color: (1.0, 1.0, 1.0, 1.0),
                    texture: Q1QuadTexture::Picture(lump.clone()),
                });
            }
            NativeQ1HudOperation::Char { x, y, code } => {
                if code & 127 == 32 || !sizes.contains_key("conchars") {
                    continue;
                }
                let column = (code & 15) as f32;
                let row = ((code >> 4) & 15) as f32;
                quads.push(Q1HudQuad {
                    rect: (*x as f32, *y as f32, 8.0, 8.0),
                    uv: (column / 16.0, row / 16.0, (column + 1.0) / 16.0, (row + 1.0) / 16.0),
                    color: (1.0, 1.0, 1.0, 1.0),
                    texture: Q1QuadTexture::Conchars,
                });
            }
            NativeQ1HudOperation::Fill {
                x,
                y,
                width: w,
                height: h,
                color,
            } => {
                let base = usize::from(*color) * 3;
                if base + 2 >= palette.len() {
                    continue;
                }
                quads.push(Q1HudQuad {
                    rect: (*x as f32, *y as f32, *w as f32, *h as f32),
                    uv: (0.0, 0.0, 1.0, 1.0),
                    color: (
                        f32::from(palette[base]) / 255.0,
                        f32::from(palette[base + 1]) / 255.0,
                        f32::from(palette[base + 2]) / 255.0,
                        1.0,
                    ),
                    texture: Q1QuadTexture::White,
                });
            }
        }
    }
    quads
}

/// Pack quads into NDC batches, grouping consecutive same-texture
/// quads into runs so back-to-front op order survives (the
/// `windowed_menu.rs` batch shape: SrcAlpha blend, no depth test).
fn quads_to_batches(
    quads: &[Q1HudQuad],
    images: &HashMap<Q1QuadTexture, qa_client::render::types::RendererImage>,
    width: f32,
    height: f32,
) -> Vec<DrawBatch> {
    let mut batches = Vec::new();
    let mut run: Vec<&Q1HudQuad> = Vec::new();
    let mut run_texture: Option<&Q1QuadTexture> = None;
    let flush = |run: &mut Vec<&Q1HudQuad>, texture: &Q1QuadTexture, batches: &mut Vec<DrawBatch>| {
        if run.is_empty() {
            return;
        }
        let Some(image) = images.get(texture) else {
            run.clear();
            return;
        };
        let mut vertices = Vec::with_capacity(run.len() * 4);
        let mut indices = Vec::with_capacity(run.len() * 6);
        for quad in run.iter() {
            let base = vertices.len() as u32;
            let (x, y, w, h) = quad.rect;
            let left = 2.0 * x / width - 1.0;
            let right = 2.0 * (x + w) / width - 1.0;
            let top = 1.0 - 2.0 * y / height;
            let bottom = 1.0 - 2.0 * (y + h) / height;
            let (s, t, s2, t2) = quad.uv;
            let (r, g, b, a) = quad.color;
            for (vx, vy, us, vt) in [
                (left, top, s, t),
                (right, top, s2, t),
                (right, bottom, s2, t2),
                (left, bottom, s, t2),
            ] {
                vertices.push(RenderVertex {
                    position: vec4(vx, vy, 0.0, 1.0),
                    tex_coord: vec2(us, vt),
                    color: vec4(r, g, b, a),
                });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        run.clear();
        batches.push(DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices,
            texture: TextureBinding::BindImage(image.clone()),
            state: RenderState {
                blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                depth_test: DepthTest::Always,
                depth_write: false,
                alpha_test: qa_client::render::types::AlphaTest::None,
                cull: CullFace::None,
                depth_range: [0.0, 1.0],
                polygon_offset: None,
            },
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vertices),
        });
    };
    for quad in quads {
        if run_texture != Some(&quad.texture) {
            if let Some(texture) = run_texture {
                flush(&mut run, texture, &mut batches);
            }
            run_texture = Some(&quad.texture);
        }
        run.push(quad);
    }
    if let Some(texture) = run_texture {
        flush(&mut run, texture, &mut batches);
    }
    batches
}

/// Short level name for the scoreboard line: `maps/e1m1.bsp` trims to `e1m1`.
#[must_use]
pub fn level_short_name(map: &str) -> String {
    let base = map.rsplit('/').next().unwrap_or(map);
    base.strip_suffix(".bsp").unwrap_or(base).to_string()
}

/// Sbar product for a loaded content id.
#[must_use]
pub fn product_for_content(content: &str) -> Q1SbarProduct {
    if content.contains("rogue") {
        Q1SbarProduct::Rogue
    } else if content.contains("hipnotic") {
        Q1SbarProduct::Hipnotic
    } else {
        Q1SbarProduct::Id1
    }
}

#[cfg(test)]
mod tests {
    use qa_client::ui::hud::q1_native::Q1_IT_AXE;

    use super::*;

    fn test_palette() -> Vec<u8> {
        let mut palette = vec![0u8; 768];
        palette[0..3].copy_from_slice(&[0, 0, 0]);
        palette[3..6].copy_from_slice(&[255, 0, 0]);
        palette[6..9].copy_from_slice(&[0, 255, 0]);
        palette
    }

    #[test]
    fn lump_set_covers_the_bar() {
        let names = lump_names(Q1SbarProduct::Id1);
        for lump in [
            "sbar",
            "ibar",
            "scorebar",
            "face1",
            "face_p5",
            "face_quad",
            "sb_shells",
            "sb_armor3",
            "sb_key2",
            "sb_sigil4",
            "inv_shotgun",
            "inv2_lightng",
            "inva5_rlaunch",
            "num_0",
            "anum_9",
            "num_minus",
            "num_colon",
            "num_slash",
            "conchars",
            "disc",
            "gfx/complete.lmp",
            "gfx/inter.lmp",
            "gfx/ranking.lmp",
            "gfx/finale.lmp",
        ] {
            assert!(names.contains(&lump.to_string()), "missing {lump}");
        }
        assert!(!names.contains(&"r_lava".to_string()));
        assert!(!names.contains(&"inv_laser".to_string()));
        let hipnotic = lump_names(Q1SbarProduct::Hipnotic);
        assert!(hipnotic.contains(&"inv_laser".to_string()));
        assert!(hipnotic.contains(&"sb_wsuit".to_string()));
        let rogue = lump_names(Q1SbarProduct::Rogue);
        assert!(rogue.contains(&"r_lava".to_string()));
        assert!(rogue.contains(&"r_teambord".to_string()));
    }

    #[test]
    fn lmp_decodes_header_and_pixels() {
        let bytes = [4u8, 0, 0, 0, 2, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        let (width, height, indices) = decode_lmp(&bytes, "test.lmp").unwrap();
        assert_eq!((width, height), (4, 2));
        assert_eq!(indices, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(decode_lmp(&bytes[..7], "short").is_err());
        assert!(decode_lmp(&bytes[..10], "truncated").is_err());
    }

    #[test]
    fn transparency_clears_only_index_zero() {
        let palette = test_palette();
        let opaque = expand_rgba(2, 1, &[0, 1], &palette, true);
        assert_eq!(opaque, vec![0, 0, 0, 255, 255, 0, 0, 255]);
        let transparent = expand_rgba(2, 1, &[0, 1], &palette, false);
        assert_eq!(transparent, vec![0, 0, 0, 0, 255, 0, 0, 255]);
    }

    #[test]
    fn current_ammo_follows_the_active_weapon() {
        assert_eq!(current_ammo(Q1_IT_SHOTGUN, 25.0, 0.0, 0.0, 0.0), (25, Q1_IT_SHELLS));
        assert_eq!(current_ammo(Q1_IT_SUPER_SHOTGUN, 7.0, 0.0, 0.0, 0.0), (7, Q1_IT_SHELLS));
        assert_eq!(current_ammo(Q1_IT_NAILGUN, 0.0, 30.0, 0.0, 0.0), (30, Q1_IT_NAILS));
        assert_eq!(
            current_ammo(Q1_IT_SUPER_NAILGUN, 0.0, 31.0, 0.0, 0.0),
            (31, Q1_IT_NAILS)
        );
        assert_eq!(
            current_ammo(Q1_IT_GRENADE_LAUNCHER, 0.0, 0.0, 5.0, 0.0),
            (5, Q1_IT_ROCKETS)
        );
        assert_eq!(
            current_ammo(Q1_IT_ROCKET_LAUNCHER, 0.0, 0.0, 6.0, 0.0),
            (6, Q1_IT_ROCKETS)
        );
        assert_eq!(current_ammo(Q1_IT_LIGHTNING, 0.0, 0.0, 0.0, 40.0), (40, Q1_IT_CELLS));
        assert_eq!(current_ammo(Q1_IT_AXE, 25.0, 0.0, 0.0, 0.0), (0, 0));
        assert_eq!(current_ammo(0, 25.0, 0.0, 0.0, 0.0), (0, 0));
    }

    #[test]
    fn short_names_and_products() {
        assert_eq!(level_short_name("maps/e1m1.bsp"), "e1m1");
        assert_eq!(level_short_name("e2m3.bsp"), "e2m3");
        assert_eq!(level_short_name("start"), "start");
        assert_eq!(product_for_content("q1-classic-id1"), Q1SbarProduct::Id1);
        assert_eq!(product_for_content("q1-classic-hipnotic"), Q1SbarProduct::Hipnotic);
        assert_eq!(product_for_content("q1-classic-rogue"), Q1SbarProduct::Rogue);
    }

    #[test]
    fn quads_place_pictures_glyphs_and_fills() {
        let sizes = HashMap::from([
            ("sbar".to_string(), (320, 24)),
            ("num_1".to_string(), (24, 24)),
            ("conchars".to_string(), (128, 128)),
            ("gfx/ranking.lmp".to_string(), (160, 24)),
        ]);
        let ops = vec![
            NativeQ1HudOperation::Picture {
                x: 0,
                y: 176,
                lump: "sbar".to_string(),
            },
            NativeQ1HudOperation::TransPicture {
                x: 136,
                y: 176,
                lump: "num_1".to_string(),
            },
            NativeQ1HudOperation::TransPicture {
                x: 160,
                y: 176,
                lump: "missing".to_string(),
            },
            NativeQ1HudOperation::Char { x: 8, y: 180, code: 65 },
            NativeQ1HudOperation::Char {
                x: 16,
                y: 180,
                code: 32,
            },
            NativeQ1HudOperation::Fill {
                x: 194,
                y: 153,
                width: 28,
                height: 4,
                color: 1,
            },
            NativeQ1HudOperation::CenteredPicture {
                x_base: 160,
                y: 8,
                lump: "gfx/ranking.lmp".to_string(),
            },
        ];
        let quads = layout_quads(&ops, &sizes, 640, &test_palette());
        assert_eq!(quads.len(), 5);
        assert_eq!(quads[0].rect, (0.0, 176.0, 320.0, 24.0));
        assert_eq!(quads[1].rect, (136.0, 176.0, 24.0, 24.0));
        // 'A' (65): column 1, row 4 of conchars; the space is skipped.
        assert_eq!(quads[2].rect, (8.0, 180.0, 8.0, 8.0));
        assert_eq!(quads[2].uv, (1.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0, 5.0 / 16.0));
        assert_eq!(quads[2].texture, Q1QuadTexture::Conchars);
        assert_eq!(quads[3].rect, (194.0, 153.0, 28.0, 4.0));
        assert_eq!(quads[3].color, (1.0, 0.0, 0.0, 1.0));
        // Centered in the 320 playfield at x_base 160: 160 + (320-160)/2.
        assert_eq!(quads[4].rect, (240.0, 8.0, 160.0, 24.0));
    }
}
