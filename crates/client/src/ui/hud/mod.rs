//! Shared per-seat HUD overlay.
//!
//! Donor provenance: `src/ui/hud/index.ts` in full (vital/status layout,
//! [`SeatHudMessages`], [`CommonHudData`], [`draw_common_hud`]). Gameplay
//! providers retain their source HUD/stat layouts; this module only draws the
//! shared per-seat overlay. Layout math uses `f32`; seat-owned message state
//! never leaves its seat. Like the donor index, this module re-exports the
//! wheel presentations.

pub(crate) mod token;

pub mod powerups;
pub mod q1_native;
pub mod q1_view_blend;
pub mod q1_wheel;
pub mod q2_native;
pub mod q2_rerelease_layout;
pub mod weapon;
pub mod wheel;

pub use q1_wheel::*;
pub use wheel::*;

use std::f32::consts::PI;
use std::rc::Rc;

use qa_core::identity::SeatId;
use qa_core::math::{dot3, vec2, vec3, Vec2, Vec3, Vec4};
use qa_core::time::SourceTime;

use self::powerups::{draw_powerup_timers, PowerupTimerView};
use self::weapon::{draw_weapon_hud, hud_status_rows, CommonWeaponHud, MeasureText};
// NOTE: `CarouselPresentation` and `WheelPresentation` resolve through the
// `pub use wheel::*` re-export above; a private import would shadow it.
use crate::error::ClientError;
use crate::render::scene::view::ViewProjector;
use crate::text::atlas::CapInk;
use crate::text::captions::ActiveCaption;
use crate::text::draw2d::Rect;
use crate::ui::common::accessibility::accessible_colors;
use crate::ui::common::captions::caption_commands;
use crate::ui::common::layout::{fit_ui, transform_ui, UiTransform};
use crate::ui::common::skin::UiSkin;
use crate::ui::types::{
    CenterPrintState, ResourceId, TextAlign, UiAppearance, UiDrawCommand, UiDrawContext, UiNotification,
    UiPreferenceValues,
};
use crate::view::SceneCamera;

/// Full-size image texture coordinates.
const FULL_UV: [Vec2; 2] = [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 1.0, y: 1.0 }];

/// One labeled numeric vital (health, armor, ammo) in the status strip.
#[derive(Debug, Clone, PartialEq)]
pub struct HudValue {
    /// Unlocalized label.
    pub label: String,
    /// Numeric value.
    pub value: f32,
    /// Status icon, if any.
    pub icon: Option<ResourceId>,
    /// Whether the vital renders in the accent color.
    pub warning: bool,
}

/// One inventory panel row.
#[derive(Debug, Clone, PartialEq)]
pub struct HudInventoryItem {
    /// Stable item identity.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Stack count.
    pub count: i32,
    /// Whether the row is selected.
    pub selected: bool,
    /// Key binding hint, if any.
    pub binding: Option<String>,
    /// Row icon, if any.
    pub icon: Option<ResourceId>,
}

/// One action prompt above the status strip.
#[derive(Debug, Clone, PartialEq)]
pub struct HudPrompt {
    /// Unlocalized action text.
    pub action: String,
    /// Binding hint; empty renders without brackets.
    pub binding: String,
    /// Prompt icon, if any.
    pub icon: Option<ResourceId>,
}

/// One boss or objective health bar.
#[derive(Debug, Clone, PartialEq)]
pub struct HudHealthBar {
    /// Stable bar identity.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Current value.
    pub value: f32,
    /// Maximum value; non-positive renders empty.
    pub maximum: f32,
    /// Bar fill color.
    pub color: Vec4,
}

/// One help overlay objective.
#[derive(Debug, Clone, PartialEq)]
pub struct HudObjective {
    /// Unlocalized objective text.
    pub text: String,
    /// Whether the objective is complete.
    pub complete: bool,
}

/// Full-screen help overlay content.
#[derive(Debug, Clone, PartialEq)]
pub struct HudHelp {
    /// Overlay title.
    pub title: String,
    /// Body lines.
    pub lines: Vec<String>,
    /// Objectives.
    pub objectives: Vec<HudObjective>,
}

/// One projected point of interest.
#[derive(Debug, Clone, PartialEq)]
pub struct HudPointOfInterest {
    /// Keyed identity; zero is unkeyed.
    pub id: i32,
    /// World origin.
    pub origin: Vec3,
    /// Marker image.
    pub image: ResourceId,
    /// Marker width in UI units.
    pub width: f32,
    /// Marker height in UI units.
    pub height: f32,
    /// Marker tint.
    pub color: Vec4,
    /// Whether the marker fades when aimed at.
    pub hide_on_aim: bool,
    /// Expiry time in milliseconds.
    pub expires_ms: i64,
}

/// Directional damage indicator picture.
#[derive(Debug, Clone, PartialEq)]
pub struct HudDamagePicture {
    /// Picture resource.
    pub image: ResourceId,
    /// Picture width in UI units.
    pub width: f32,
    /// Picture height in UI units.
    pub height: f32,
}

/// One damage direction indicator.
#[derive(Debug, Clone, PartialEq)]
pub enum HudDamageIndicator {
    /// World-origin indicator drawn as three approaching squares.
    Origin {
        /// World origin the damage came from.
        origin: Vec3,
        /// Damage amount.
        amount: f32,
        /// Expiry time in milliseconds.
        expires_ms: i64,
    },
    /// Camera-relative indicator, optionally with a yaw-rotated picture.
    Directional {
        /// Picture drawn at the damage yaw, if any.
        picture: Option<HudDamagePicture>,
        /// Damage direction in world space.
        direction: Vec3,
        /// Damage amount.
        amount: f32,
        /// Indicator color.
        color: Vec3,
        /// Whether health was damaged.
        health: bool,
        /// Whether armor was damaged.
        armor: bool,
        /// Whether shields were damaged.
        shield: bool,
        /// Expiry time in milliseconds.
        expires_ms: i64,
    },
}

impl HudDamageIndicator {
    /// Expiry time in milliseconds.
    fn expires_ms(&self) -> i64 {
        match *self {
            HudDamageIndicator::Origin { expires_ms, .. } | HudDamageIndicator::Directional { expires_ms, .. } => {
                expires_ms
            }
        }
    }
}

/// Crosshair presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct HudCrosshair {
    /// Whether the crosshair renders.
    pub visible: bool,
    /// Crosshair color.
    pub color: Vec4,
    /// Crosshair image; `None` draws the plus shape.
    pub image: Option<ResourceId>,
}

/// Pickup banner presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct HudPickup {
    /// Unlocalized pickup name.
    pub name: String,
    /// Pickup icon, if any.
    pub icon: Option<ResourceId>,
    /// Expiry time in milliseconds.
    pub expires_ms: i64,
}

/// Hit-marker presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct HudHitMarker {
    /// Damage dealt.
    pub damage: f32,
    /// Expiry time in milliseconds.
    pub expires_ms: i64,
}

/// World-space help-path ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudHelpPath {
    /// Ray origin.
    pub origin: Vec3,
    /// Ray direction.
    pub direction: Vec3,
}

/// Shared per-seat overlay inputs for one HUD frame.
#[derive(Debug, Clone)]
pub struct CommonHudData {
    /// Active powerup timers.
    pub powerups: Vec<PowerupTimerView>,
    /// Weapon status panel, if any.
    pub weapon: Option<CommonWeaponHud>,
    /// Owning seat.
    pub seat: SeatId,
    /// Whether the overlay renders at all.
    pub visible: bool,
    /// Status-strip vitals.
    pub vitals: Vec<HudValue>,
    /// Inventory panel rows; `None` hides the panel.
    pub inventory: Option<Vec<HudInventoryItem>>,
    /// Action prompts.
    pub prompts: Vec<HudPrompt>,
    /// Health bars.
    pub health_bars: Vec<HudHealthBar>,
    /// Help overlay; `Some` hides the crosshair.
    pub help: Option<HudHelp>,
    /// Active captions.
    pub captions: Vec<ActiveCaption>,
    /// Weapon wheel presentation; `Some` hides the crosshair.
    pub wheel: Option<WheelPresentation>,
    /// Weapon carousel presentation.
    pub carousel: Option<CarouselPresentation>,
    /// Crosshair presentation.
    pub crosshair: HudCrosshair,
    /// World-space help-path ray, if any.
    pub help_path: Option<HudHelpPath>,
    /// Damage indicators.
    pub damage_indicators: Vec<HudDamageIndicator>,
    /// Pickup banner, if any.
    pub pickup: Option<HudPickup>,
    /// Hit marker, if any.
    pub hit_marker: Option<HudHitMarker>,
}

/// Visible overlay defaults for one seat.
#[must_use]
pub fn empty_hud_data(seat: SeatId) -> CommonHudData {
    CommonHudData {
        powerups: Vec::new(),
        weapon: None,
        seat,
        visible: true,
        vitals: Vec::new(),
        inventory: None,
        prompts: Vec::new(),
        health_bars: Vec::new(),
        help: None,
        captions: Vec::new(),
        wheel: None,
        carousel: None,
        crosshair: HudCrosshair {
            visible: true,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            image: None,
        },
        help_path: None,
        damage_indicators: Vec::new(),
        pickup: None,
        hit_marker: None,
    }
}

/// Status-strip panel rects in 640x480 UI units.
#[must_use]
pub fn hud_vital_rects(count: i32, scale: f32, height: f32) -> Vec<Rect> {
    if count <= 0 {
        return Vec::new();
    }
    let width = (600.0 / scale / (count.max(1) as f32)).min(160.0);
    let start = 320.0 - width * (count as f32) / 2.0;
    (0..count)
        .map(|index| Rect {
            x: start + (index as f32) * width,
            y: 476.0 - height,
            width: width - 4.0,
            height,
        })
        .collect()
}

/// Status-strip layout: panel rects, their seat transform, and the tiny-text floor.
struct StatusLayout {
    rects: Vec<Rect>,
    transform: UiTransform,
    minimum_text_scale: f32,
}

/// Lay out `count` status panels, falling back to tiny pixel-space text rows
/// when the scaled cap height would render below eight pixels.
fn status_layout(
    context: &UiDrawContext,
    count: i32,
    hud_scale: f32,
    text_scale: f32,
    cap_height: f32,
) -> Result<StatusLayout, ClientError> {
    let area = &context.binding.safe_area;
    let fitted = fit_ui(area, 1.0)?;
    let group = hud_scale * context.binding.hud_scale;
    let requested = fitted.scale * group;
    if requested * text_scale * cap_height < 8.0 {
        let width = 180.0_f32.min((area.width - 8.0) / (count.max(1) as f32));
        let mut rects = Vec::new();
        for index in 0..count.max(0) {
            rects.push(Rect {
                x: area.x + (area.width - width * (count as f32)) / 2.0 + (index as f32) * width,
                y: area.y + area.height - 36.0,
                width: width - 4.0,
                height: 32.0,
            });
        }
        return Ok(StatusLayout {
            rects,
            transform: UiTransform {
                x: 0.0,
                y: 0.0,
                scale: 1.0,
            },
            minimum_text_scale: 8.0 / cap_height,
        });
    }
    let scale = requested;
    Ok(StatusLayout {
        rects: hud_vital_rects(count, group, hud_status_rows(text_scale, cap_height).height),
        minimum_text_scale: 8.0 / cap_height / scale,
        transform: UiTransform {
            scale,
            x: fitted.x + 320.0 * fitted.scale * (1.0 - group),
            y: fitted.y + 480.0 * fitted.scale * (1.0 - group),
        },
    })
}

/// Status-strip panel rects mapped into drawable pixels.
pub fn hud_vital_occupied_rects(
    context: &UiDrawContext,
    count: i32,
    hud_scale: f32,
    text_scale: f32,
    cap_height: f32,
) -> Result<Vec<Rect>, ClientError> {
    let layout = status_layout(context, count, hud_scale, text_scale, cap_height)?;
    Ok(layout
        .rects
        .iter()
        .map(|rect| Rect {
            x: layout.transform.x + rect.x * layout.transform.scale,
            y: layout.transform.y + rect.y * layout.transform.scale,
            width: rect.width * layout.transform.scale,
            height: rect.height * layout.transform.scale,
        })
        .collect())
}

/// Source time in milliseconds.
fn source_ms(time: &SourceTime) -> f64 {
    match *time {
        SourceTime::Seconds(value) => f64::from(value) * 1000.0,
        SourceTime::Milliseconds(value) => f64::from(value),
    }
}

/// Whether a start/duration window covers `now_ms`.
fn source_alive(starts: &SourceTime, duration: &SourceTime, now_ms: f64) -> bool {
    let start = source_ms(starts);
    now_ms >= start && now_ms < start + source_ms(duration)
}

/// Round donor fractional milliseconds onto the integer clock.
fn ms_i32(value: f64) -> i32 {
    value.round() as i32
}

/// Live notifications, center print, and points for one seat at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveHudMessages {
    /// Visible notifications.
    pub notifications: Vec<UiNotification>,
    /// Visible center print, if any.
    pub center_print: Option<CenterPrintState>,
    /// Unexpired local plus source points.
    pub points: Vec<HudPointOfInterest>,
}

/// Source private messages and POIs stored on the recipient seat, never a global HUD.
#[derive(Debug, Clone)]
pub struct SeatHudMessages {
    seat: SeatId,
    sequence: u64,
    notices: Vec<UiNotification>,
    center: Option<CenterPrintState>,
    queued_centers: Vec<CenterPrintState>,
    points: Vec<HudPointOfInterest>,
    source_points: Vec<HudPointOfInterest>,
}

impl SeatHudMessages {
    /// Empty message state for one seat.
    #[must_use]
    pub fn new(seat: SeatId) -> Self {
        Self {
            seat,
            sequence: 0,
            notices: Vec::new(),
            center: None,
            queued_centers: Vec::new(),
            points: Vec::new(),
            source_points: Vec::new(),
        }
    }

    /// Owning seat.
    #[must_use]
    pub fn seat(&self) -> SeatId {
        self.seat.clone()
    }

    /// Replace the provider-owned points drawn for `seat`.
    pub fn set_source_points(&mut self, seat: &SeatId, points: Vec<HudPointOfInterest>) -> Result<(), ClientError> {
        self.require_seat(seat)?;
        self.source_points = points;
        Ok(())
    }

    /// Queue one notification line on the seat clock.
    pub fn notify(
        &mut self,
        seat: &SeatId,
        text: &str,
        chat: bool,
        starts: SourceTime,
        duration: SourceTime,
    ) -> Result<(), ClientError> {
        self.require_seat(seat)?;
        self.notices.push(UiNotification {
            sequence: self.sequence,
            text: text.to_string(),
            chat,
            starts,
            duration,
        });
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }

    /// Show center print; instant prints replace the queue while typewriter
    /// prints chain after the last queued print.
    pub fn center_print(
        &mut self,
        seat: &SeatId,
        text: &str,
        starts: SourceTime,
        duration: SourceTime,
        instant: bool,
        character_ms: f64,
    ) -> Result<(), ClientError> {
        self.require_seat(seat)?;
        if instant {
            self.queued_centers.clear();
            self.center = Some(CenterPrintState {
                text: text.to_string(),
                starts,
                duration,
                instant: true,
                character_ms: None,
            });
            return Ok(());
        }
        let prior = self.queued_centers.last().or(self.center.as_ref());
        let start =
            source_ms(&starts).max(prior.map_or(0.0, |print| source_ms(&print.starts) + source_ms(&print.duration)));
        let print = CenterPrintState {
            text: text.to_string(),
            starts: SourceTime::Milliseconds(ms_i32(start)),
            instant: false,
            character_ms: Some(character_ms),
            duration: SourceTime::Milliseconds(ms_i32(
                source_ms(&duration) + text.chars().count() as f64 * character_ms,
            )),
        };
        if self.center.is_none() {
            self.center = Some(print);
        } else {
            self.queued_centers.push(print);
        }
        Ok(())
    }

    /// Drop all notifications.
    pub fn clear_notify(&mut self) {
        self.notices.clear();
    }

    /// Drop the center print and its queue.
    pub fn clear_center_print(&mut self) {
        self.center = None;
        self.queued_centers.clear();
    }

    /// Drop all seat messages and points.
    pub fn clear(&mut self) {
        self.clear_notify();
        self.clear_center_print();
        self.points.clear();
        self.source_points.clear();
    }

    /// Store one POI. Keyed POIs replace matching IDs; unkeyed POIs replace
    /// only expired or oldest unkeyed entries. Returns whether the point fit.
    pub fn add_point(
        &mut self,
        seat: &SeatId,
        point: HudPointOfInterest,
        now_ms: i64,
        capacity: usize,
    ) -> Result<bool, ClientError> {
        self.require_seat(seat)?;
        let mut index = if point.id == 0 {
            None
        } else {
            self.points.iter().position(|existing| existing.id == point.id)
        };
        if index.is_none() {
            index = self.points.iter().position(|existing| existing.expires_ms <= now_ms);
        }
        if index.is_none() && self.points.len() < capacity {
            self.points.push(point);
            return Ok(true);
        }
        if index.is_none() {
            let mut oldest = i64::MAX;
            for (candidate, existing) in self.points.iter().enumerate() {
                if existing.id == 0 && existing.expires_ms < oldest {
                    oldest = existing.expires_ms;
                    index = Some(candidate);
                }
            }
        }
        match index {
            None => Ok(false),
            Some(slot) => {
                self.points[slot] = point;
                Ok(true)
            }
        }
    }

    /// Remove one keyed POI; id zero never removes.
    pub fn remove_point(&mut self, id: i32) {
        if id == 0 {
            return;
        }
        if let Some(index) = self.points.iter().position(|point| point.id == id) {
            self.points.remove(index);
        }
    }

    /// Expire old state and return what is visible at `now_ms`.
    pub fn active(&mut self, now_ms: i64) -> ActiveHudMessages {
        let now = now_ms as f64;
        self.notices
            .retain(|notice| source_ms(&notice.starts) + source_ms(&notice.duration) > now);
        while let Some(center) = self.center.as_ref() {
            if source_ms(&center.starts) + source_ms(&center.duration) > now {
                break;
            }
            self.center = if self.queued_centers.is_empty() {
                None
            } else {
                Some(self.queued_centers.remove(0))
            };
        }
        ActiveHudMessages {
            notifications: self
                .notices
                .iter()
                .filter(|notice| source_alive(&notice.starts, &notice.duration, now))
                .cloned()
                .collect(),
            center_print: self
                .center
                .clone()
                .filter(|center| source_alive(&center.starts, &center.duration, now)),
            points: self
                .points
                .iter()
                .chain(self.source_points.iter())
                .filter(|point| point.expires_ms > now_ms)
                .cloned()
                .collect(),
        }
    }

    /// Reject messages addressed to another seat.
    fn require_seat(&self, seat: &SeatId) -> Result<(), ClientError> {
        if *seat != self.seat {
            return Err(ClientError::BadUi("HUD message belongs to another seat".to_string()));
        }
        Ok(())
    }
}

/// Services for one shared overlay draw.
#[derive(Clone)]
pub struct CommonHudDrawOptions {
    /// Menu skin.
    pub skin: UiSkin,
    /// Text width measurer; falls back to eight units per character.
    pub measure_text: Option<MeasureText>,
    /// Seat preferences.
    pub preferences: UiPreferenceValues,
    /// Scene camera for world-space markers; `None` skips them.
    pub camera: Option<SceneCamera>,
    /// Localizer applied to drawn text.
    pub localize: Rc<dyn Fn(&str) -> String>,
}

impl std::fmt::Debug for CommonHudDrawOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommonHudDrawOptions")
            .field("skin", &self.skin)
            .field("measure_text", &self.measure_text.is_some())
            .field("preferences", &self.preferences)
            .field("camera", &self.camera)
            .finish_non_exhaustive()
    }
}

/// One overlay command awaiting its seat transform.
#[derive(Debug, Clone, PartialEq)]
struct PendingHudCommand {
    command: UiDrawCommand,
    anchor: Vec2,
    scale: f32,
    transform: Option<UiTransform>,
}

/// Collects overlay commands in 640x480 UI units at the current anchor/scale.
struct HudEmitter<'a> {
    commands: Vec<PendingHudCommand>,
    anchor: Vec2,
    group_scale: f32,
    skin: &'a UiSkin,
    text_scale: f32,
    localize: &'a dyn Fn(&str) -> String,
}

impl HudEmitter<'_> {
    /// Localized shadowed text at the current anchor/scale.
    fn text(&mut self, value: &str, x: f32, y: f32, tint: Vec4, align: TextAlign) {
        self.commands.push(PendingHudCommand {
            command: UiDrawCommand::Text {
                origin: vec2(x, y),
                text: (self.localize)(value),
                font: self.skin.font.clone(),
                scale: self.text_scale,
                color: tint,
                align,
                shadow: true,
            },
            anchor: self.anchor,
            scale: self.group_scale,
            transform: None,
        });
    }

    /// Full-UV image at the current anchor/scale.
    fn image(&mut self, resource: &ResourceId, rect: Rect, tint: Vec4) {
        self.commands.push(PendingHudCommand {
            command: UiDrawCommand::Image {
                resource: resource.clone(),
                rect,
                tex_coords: FULL_UV,
                color: tint,
            },
            anchor: self.anchor,
            scale: self.group_scale,
            transform: None,
        });
    }

    /// Filled rect at the current anchor/scale.
    fn fill(&mut self, rect: Rect, tint: Vec4) {
        self.commands.push(PendingHudCommand {
            command: UiDrawCommand::Fill { rect, color: tint },
            anchor: self.anchor,
            scale: self.group_scale,
            transform: None,
        });
    }

    /// Status-strip command pinned to the status transform.
    fn status(&mut self, command: UiDrawCommand, transform: UiTransform) {
        self.commands.push(PendingHudCommand {
            command,
            anchor: self.anchor,
            scale: self.group_scale,
            transform: Some(transform),
        });
    }

    /// Raw command at the current anchor/scale.
    fn raw(&mut self, command: UiDrawCommand) {
        self.commands.push(PendingHudCommand {
            command,
            anchor: self.anchor,
            scale: self.group_scale,
            transform: None,
        });
    }
}

/// Draw the shared per-seat overlay inside a clip pair.
#[allow(clippy::too_many_lines)]
pub fn draw_common_hud(
    context: &UiDrawContext,
    data: &CommonHudData,
    options: &CommonHudDrawOptions,
    messages: &mut SeatHudMessages,
) -> Result<Vec<UiDrawCommand>, ClientError> {
    if data.seat != context.binding.seat || data.seat != messages.seat() {
        return Err(ClientError::BadUi("HUD frame belongs to another seat".to_string()));
    }
    if !data.visible {
        return Ok(Vec::new());
    }
    let preferences = &options.preferences;
    let skin = &options.skin;
    let palette = accessible_colors(
        &skin.colors,
        &UiAppearance {
            menu_scale: preferences.menu_scale,
            text_scale: preferences.text_scale,
            high_contrast: preferences.high_contrast,
            color_mode: preferences.color_mode,
        },
    );
    let color = palette.text;
    let accent = palette.accent;
    let background = palette.panel;
    let text_scale = skin.font_scale * preferences.text_scale;
    let line_height = skin.line_height * preferences.text_scale;
    let cap_height = skin.cap_ink.map_or(8.0, |ink| ink.height);
    let cap_top = skin.cap_ink.map_or(0.0, |ink| ink.top);
    let weapon_slot = match data.weapon.as_ref() {
        None => 0,
        Some(weapon) => i32::from(!weapon.native_status),
    };
    let status = status_layout(
        context,
        data.vitals.len() as i32 + weapon_slot,
        preferences.hud_scale,
        text_scale,
        cap_height,
    )?;
    let localize: &dyn Fn(&str) -> String = options.localize.as_ref();
    let mut emitter = HudEmitter {
        commands: Vec::new(),
        anchor: vec2(320.0, 240.0),
        group_scale: preferences.hud_scale * context.binding.hud_scale,
        skin,
        text_scale,
        localize,
    };
    let measure_at = |text: &str, scale: f32| -> f32 {
        match options.measure_text.as_ref() {
            Some(measure) => measure(text, scale),
            None => text.encode_utf16().count() as f32 * 8.0 * scale,
        }
    };
    let state = messages.active(context.time_ms);
    if preferences.crosshair
        && data.crosshair.visible
        && data.help.is_none()
        && data.inventory.is_none()
        && data.wheel.is_none()
    {
        let first = emitter.commands.len();
        let size = preferences.crosshair_size;
        match data.crosshair.image.as_ref() {
            Some(image) => emitter.image(
                image,
                Rect {
                    x: 320.0 - size / 2.0,
                    y: 240.0 - size / 2.0,
                    width: size,
                    height: size,
                },
                data.crosshair.color,
            ),
            None => {
                emitter.fill(
                    Rect {
                        x: 320.0 - size / 2.0,
                        y: 239.0,
                        width: size,
                        height: 2.0,
                    },
                    data.crosshair.color,
                );
                emitter.fill(
                    Rect {
                        x: 319.0,
                        y: 240.0 - size / 2.0,
                        width: 2.0,
                        height: size,
                    },
                    data.crosshair.color,
                );
            }
        }
        if let Some(marker) = data.hit_marker.as_ref() {
            if marker.expires_ms > context.time_ms && !preferences.reduced_flashes {
                let opacity = 0.0_f32.max(1.0_f32.min((marker.expires_ms - context.time_ms) as f32 / 150.0));
                for x in [-1.0_f32, 1.0] {
                    for y in [-1.0_f32, 1.0] {
                        emitter.fill(
                            Rect {
                                x: 320.0 + x * (size + 2.0) - 2.0,
                                y: 240.0 + y * (size + 2.0) - 2.0,
                                width: 4.0,
                                height: 4.0,
                            },
                            Vec4 { w: opacity, ..accent },
                        );
                    }
                }
            }
        }
        if let Some(camera) = options.camera.as_ref() {
            let viewport = &camera.viewport;
            let scale = fit_ui(&context.binding.safe_area, 1.0)?.scale * emitter.group_scale;
            let transform = UiTransform {
                x: viewport.x as f32 + viewport.width as f32 / 2.0 - 320.0 * scale,
                y: viewport.y as f32 + viewport.height as f32 / 2.0 - 240.0 * scale,
                scale,
            };
            for item in emitter.commands[first..].iter_mut() {
                item.transform = Some(transform);
            }
        }
    }
    if !data.vitals.is_empty() {
        emitter.anchor = vec2(320.0, 480.0);
        let rows = hud_status_rows(text_scale, cap_height);
        for (index, vital) in data.vitals.iter().enumerate() {
            let Some(rect) = status.rects.get(index).copied() else {
                continue;
            };
            let x = rect.x;
            let compact = rect.height == 32.0;
            emitter.status(
                UiDrawCommand::Fill {
                    rect,
                    color: background,
                },
                status.transform,
            );
            if let Some(icon) = vital.icon.as_ref() {
                emitter.status(
                    UiDrawCommand::Image {
                        resource: icon.clone(),
                        rect: Rect {
                            x: x + 6.0,
                            y: rect.y + 8.0,
                            width: 24.0,
                            height: 24.0,
                        },
                        tex_coords: FULL_UV,
                        color,
                    },
                    status.transform,
                );
            }
            let scale = if compact { status.minimum_text_scale } else { text_scale };
            let label = localize(&vital.label);
            let value = format!("{}", vital.value);
            let full = format!("{label} {value}");
            let left = x + if vital.icon.is_some() { 34.0 } else { 4.0 };
            let available = rect.x + rect.width - 4.0 - left;
            let lines: Vec<&str> = if compact {
                if measure_at(&full, scale) <= available {
                    vec![full.as_str()]
                } else {
                    vec![label.as_str(), value.as_str()]
                }
            } else {
                vec![value.as_str(), label.as_str()]
            };
            for (row, line) in lines.iter().enumerate() {
                let requested = if compact {
                    scale
                } else {
                    text_scale * if row == 0 { 1.5 } else { 0.8 }
                };
                let row_scale = if !compact && row == 0 {
                    requested.min(available / 1.0_f32.max(measure_at(line, 1.0)))
                } else {
                    requested
                };
                let mut chars: Vec<char> = line.chars().collect();
                while !chars.is_empty() && measure_at(&chars.iter().collect::<String>(), row_scale) > available {
                    chars.pop();
                }
                let y = rect.y
                    + if compact {
                        4.0 + row as f32 * 14.0 - cap_top * scale
                    } else if row == 0 {
                        4.0 - cap_top * row_scale
                    } else {
                        rows.label_top - cap_top * row_scale
                    };
                emitter.status(
                    UiDrawCommand::Text {
                        origin: vec2(left, y),
                        text: chars.iter().collect(),
                        font: skin.font.clone(),
                        scale: row_scale,
                        color: if vital.warning { accent } else { color },
                        align: TextAlign::Left,
                        shadow: true,
                    },
                    status.transform,
                );
            }
        }
    }
    if let Some(weapon) = data.weapon.as_ref() {
        let native_height = hud_status_rows(text_scale, cap_height).height;
        let rect = if weapon.native_status {
            Some(Rect {
                x: 8.0,
                y: 476.0 - native_height,
                width: 152.0,
                height: native_height,
            })
        } else {
            status.rects.get(data.vitals.len()).copied()
        };
        if let Some(rect) = rect {
            emitter.anchor = vec2(if weapon.native_status { 0.0 } else { 320.0 }, 480.0);
            let mut weapon_skin = skin.clone();
            weapon_skin.colors.text = color;
            weapon_skin.colors.accent = accent;
            weapon_skin.colors.panel = background;
            let minimum = if weapon.native_status {
                0.0
            } else {
                status.minimum_text_scale
            };
            for command in draw_weapon_hud(weapon, &rect, &weapon_skin, text_scale, minimum) {
                if weapon.native_status {
                    emitter.raw(command);
                } else {
                    emitter.status(command, status.transform);
                }
            }
        }
    }
    emitter.anchor = vec2(320.0, 0.0);
    for (index, bar) in data.health_bars.iter().enumerate() {
        let y = 24.0 + index as f32 * (line_height + 16.0);
        emitter.text(&bar.label, 320.0, y, color, TextAlign::Center);
        emitter.fill(
            Rect {
                x: 160.0,
                y: y + line_height + 2.0,
                width: 320.0,
                height: 8.0,
            },
            background,
        );
        let fraction = if bar.maximum <= 0.0 {
            0.0
        } else {
            0.0_f32.max(1.0_f32.min(bar.value / bar.maximum))
        };
        emitter.fill(
            Rect {
                x: 160.0,
                y: y + line_height + 2.0,
                width: 320.0 * fraction,
                height: 8.0,
            },
            bar.color,
        );
    }
    emitter.anchor = vec2(0.0, 0.0);
    let notices = &state.notifications;
    let first_notice = notices.len().saturating_sub(6);
    for (index, notice) in notices[first_notice..].iter().enumerate() {
        emitter.text(
            &notice.text,
            12.0,
            12.0 + index as f32 * line_height,
            if notice.chat { accent } else { color },
            TextAlign::Left,
        );
    }
    emitter.anchor = vec2(320.0, 240.0);
    if let Some(print) = state.center_print.as_ref() {
        let elapsed = context.time_ms as f64 - source_ms(&print.starts);
        let value = if print.instant {
            print.text.clone()
        } else {
            let shown = (elapsed / print.character_ms.unwrap_or(125.0)).floor().max(0.0) as usize;
            print.text.chars().take(shown).collect::<String>()
        };
        let lines: Vec<&str> = value.split('\n').collect();
        let start = 160.0 - lines.len() as f32 * line_height / 2.0;
        for (index, line) in lines.iter().enumerate() {
            emitter.text(
                line,
                320.0,
                start + index as f32 * line_height,
                color,
                TextAlign::Center,
            );
        }
    }
    if let Some(camera) = options.camera.as_ref() {
        if !preferences.reduced_flashes {
            for damage in data.damage_indicators.iter() {
                if damage.expires_ms() <= context.time_ms {
                    continue;
                }
                let delta = match damage {
                    HudDamageIndicator::Directional { direction, .. } => vec3(-direction.x, -direction.y, -direction.z),
                    HudDamageIndicator::Origin { origin, .. } => vec3(
                        origin.x - camera.origin.x,
                        origin.y - camera.origin.y,
                        origin.z - camera.origin.z,
                    ),
                };
                let horizontal = -dot3(delta, camera.axis[1]);
                let vertical = dot3(delta, camera.axis[0]);
                let length = horizontal.hypot(vertical);
                let length = if length == 0.0 { 1.0 } else { length };
                let directional = matches!(damage, HudDamageIndicator::Directional { .. });
                let base = match damage {
                    HudDamageIndicator::Directional { color, .. } => *color,
                    HudDamageIndicator::Origin { .. } => vec3(1.0, 0.15, 0.05),
                };
                let fade = if directional { 1000.0 } else { 400.0 };
                let tint = Vec4 {
                    x: base.x,
                    y: base.y,
                    z: base.z,
                    w: 1.0_f32.min((damage.expires_ms() - context.time_ms) as f32 / fade),
                };
                if let HudDamageIndicator::Directional {
                    picture: Some(picture),
                    direction,
                    amount,
                    ..
                } = damage
                {
                    let yaw = camera.axis[0].y.atan2(camera.axis[0].x) - direction.y.atan2(direction.x) - PI;
                    let width = picture.width.min(3.0 * amount);
                    let height = picture.height;
                    let radius = (if preferences.crosshair {
                        preferences.crosshair_size
                    } else {
                        0.0
                    }) + height / 2.0;
                    emitter.image(
                        &picture.image,
                        Rect {
                            x: 320.0 + radius * yaw.sin() - width / 2.0,
                            y: 240.0 - radius * yaw.cos() - height / 2.0,
                            width,
                            height,
                        },
                        tint,
                    );
                    continue;
                }
                for step in 0..3 {
                    let distance = 55.0 + (step as f32) * 7.0;
                    emitter.fill(
                        Rect {
                            x: 317.0 + horizontal / length * distance,
                            y: 237.0 - vertical / length * distance,
                            width: 6.0,
                            height: 6.0,
                        },
                        tint,
                    );
                }
            }
        }
    }
    if let Some(pickup) = data.pickup.as_ref() {
        if pickup.expires_ms > context.time_ms {
            emitter.anchor = vec2(320.0, 480.0);
            if let Some(icon) = pickup.icon.as_ref() {
                emitter.image(
                    icon,
                    Rect {
                        x: 184.0,
                        y: 346.0,
                        width: 28.0,
                        height: 28.0,
                    },
                    color,
                );
            }
            emitter.text(&pickup.name, 320.0, 350.0, accent, TextAlign::Center);
        }
    }
    if let Some(inventory) = data.inventory.as_ref() {
        emitter.fill(
            Rect {
                x: 128.0,
                y: 72.0,
                width: 384.0,
                height: 328.0,
            },
            background,
        );
        emitter.text("Inventory", 320.0, 88.0, accent, TextAlign::Center);
        let selected = inventory
            .iter()
            .position(|item| item.selected)
            .map_or(0, |index| index as i64);
        let rows = 1_i64.max((270.0 / line_height).floor() as i64);
        let start = 0_i64.max((inventory.len() as i64 - rows).min(selected - rows / 2));
        for (index, item) in inventory.iter().skip(start as usize).take(rows as usize).enumerate() {
            let y = 114.0 + index as f32 * line_height;
            if item.selected {
                emitter.fill(
                    Rect {
                        x: 136.0,
                        y,
                        width: 368.0,
                        height: line_height,
                    },
                    skin.colors.focused,
                );
            }
            emitter.text(item.binding.as_deref().unwrap_or(""), 144.0, y, accent, TextAlign::Left);
            if let Some(icon) = item.icon.as_ref() {
                emitter.image(
                    icon,
                    Rect {
                        x: 188.0,
                        y,
                        width: line_height,
                        height: line_height,
                    },
                    color,
                );
            }
            emitter.text(
                &item.label,
                212.0,
                y,
                if item.selected { accent } else { color },
                TextAlign::Left,
            );
            emitter.text(&item.count.to_string(), 494.0, y, color, TextAlign::Right);
        }
    }
    emitter.anchor = vec2(320.0, 480.0);
    for (index, prompt) in data.prompts.iter().enumerate() {
        let y = 394.0 - (data.prompts.len() - index - 1) as f32 * (line_height + 4.0);
        if let Some(icon) = prompt.icon.as_ref() {
            emitter.image(
                icon,
                Rect {
                    x: 176.0,
                    y: y - 2.0,
                    width: line_height + 4.0,
                    height: line_height + 4.0,
                },
                color,
            );
        }
        let inner = if prompt.binding.is_empty() {
            localize(&prompt.action)
        } else {
            format!("[{}] {}", prompt.binding, localize(&prompt.action))
        };
        emitter.text(&inner, 320.0, y, accent, TextAlign::Center);
    }
    if let Some(help) = data.help.as_ref() {
        emitter.anchor = vec2(320.0, 240.0);
        emitter.group_scale = 1.0;
        emitter.fill(
            Rect {
                x: 48.0,
                y: 48.0,
                width: 544.0,
                height: 360.0,
            },
            background,
        );
        emitter.text(&help.title, 320.0, 68.0, accent, TextAlign::Center);
        let mut y = 104.0;
        for line in help.lines.iter() {
            emitter.text(line, 68.0, y, color, TextAlign::Left);
            y += line_height;
        }
        y += line_height;
        for objective in help.objectives.iter() {
            let inner = format!(
                "{} {}",
                if objective.complete { "[x]" } else { "[ ]" },
                localize(&objective.text)
            );
            emitter.text(
                &inner,
                68.0,
                y,
                if objective.complete { color } else { accent },
                TextAlign::Left,
            );
            y += line_height;
        }
    }
    if let Some(wheel) = data.wheel.as_ref() {
        emitter.anchor = vec2(320.0, 240.0);
        emitter.group_scale = 1.0_f32.min(preferences.hud_scale * context.binding.hud_scale);
        let opacity = if preferences.reduced_flashes {
            1.0
        } else {
            wheel.opacity
        };
        emitter.fill(
            Rect {
                x: 128.0,
                y: 48.0,
                width: 384.0,
                height: 384.0,
            },
            Vec4 {
                w: background.w * opacity,
                ..background
            },
        );
        for (index, item) in wheel.items.iter().enumerate() {
            let angle = index as f32 * 2.0 * PI / wheel.items.len() as f32;
            let x = 320.0 + angle.sin() * 136.0;
            let y = 240.0 - angle.cos() * 136.0;
            let selected = Some(item.id.as_str()) == wheel.selected.as_deref();
            let base = if selected {
                accent
            } else if item.owned {
                color
            } else {
                skin.colors.disabled
            };
            let tint = Vec4 { w: opacity, ..base };
            let icon = if selected {
                item.selected_icon.as_ref().or(item.icon.as_ref())
            } else {
                item.icon.as_ref()
            };
            match icon {
                Some(icon) => emitter.image(
                    icon,
                    Rect {
                        x: x - 20.0,
                        y: y - 20.0,
                        width: 40.0,
                        height: 40.0,
                    },
                    tint,
                ),
                None => emitter.text(&item.label, x, y - 8.0, tint, TextAlign::Center),
            }
            if let Some(count) = item.count {
                let count_tint = Vec4 {
                    w: opacity,
                    ..if count <= item.warning_count as f32 {
                        accent
                    } else {
                        base
                    }
                };
                emitter.text(&count.to_string(), x, y + 24.0, count_tint, TextAlign::Center);
            }
            if selected {
                emitter.text(&item.label, 320.0, 220.0, tint, TextAlign::Center);
            }
        }
        emitter.fill(
            Rect {
                x: 318.0 + wheel.cursor.x * 150.0,
                y: 238.0 + wheel.cursor.y * 150.0,
                width: 4.0,
                height: 4.0,
            },
            accent,
        );
    }
    if let Some(carousel) = data.carousel.as_ref() {
        emitter.anchor = vec2(320.0, 480.0);
        emitter.group_scale = preferences.hud_scale * context.binding.hud_scale;
        let width = 48.0_f32.min(600.0 / carousel.items.len().max(1) as f32);
        let start = 320.0 - carousel.items.len() as f32 * width / 2.0;
        for (index, item) in carousel.items.iter().enumerate() {
            let x = start + index as f32 * width;
            let selected = Some(item.id.as_str()) == carousel.selected.as_deref();
            if selected {
                emitter.fill(
                    Rect {
                        x,
                        y: 324.0,
                        width: width - 2.0,
                        height: 50.0,
                    },
                    skin.colors.focused,
                );
            }
            let icon = if selected {
                item.selected_icon.as_ref().or(item.icon.as_ref())
            } else {
                item.icon.as_ref()
            };
            if let Some(icon) = icon {
                emitter.image(
                    icon,
                    Rect {
                        x: x + 4.0,
                        y: 328.0,
                        width: width - 10.0,
                        height: width - 10.0,
                    },
                    color,
                );
            }
            if let Some(count) = item.count {
                emitter.text(
                    &count.to_string(),
                    x + width / 2.0,
                    358.0,
                    if selected { accent } else { color },
                    TextAlign::Center,
                );
            }
        }
    }
    let transform = fit_ui(&context.binding.safe_area, 1.0)?;
    let mut result = vec![UiDrawCommand::Clip {
        rect: Some(context.binding.safe_area),
    }];
    let area = context.binding.safe_area;
    let mut timer_bottom = area.y + area.height - 42.0;
    for rect in status.rects.iter() {
        timer_bottom = timer_bottom.min(status.transform.y + rect.y * status.transform.scale - 4.0);
    }
    let mut powerup_skin = skin.clone();
    powerup_skin.colors.text = color;
    powerup_skin.colors.panel = background;
    result.extend(draw_powerup_timers(
        &data.powerups,
        &area,
        timer_bottom,
        &powerup_skin,
        text_scale * transform.scale * preferences.hud_scale * context.binding.hud_scale,
        &options.measure_text,
        options.localize.as_ref(),
    ));
    result.extend(emitter.commands.iter().map(|item| {
        let mapped = item.transform.unwrap_or(UiTransform {
            scale: transform.scale * item.scale,
            x: transform.x + item.anchor.x * transform.scale * (1.0 - item.scale),
            y: transform.y + item.anchor.y * transform.scale * (1.0 - item.scale),
        });
        transform_ui(&item.command, &mapped)
    }));
    if let Some(camera) = options.camera.as_ref() {
        let projector = ViewProjector::new(*camera, None);
        let viewport = &camera.viewport;
        let vx = viewport.x as f32;
        let vy = viewport.y as f32;
        let vw = viewport.width as f32;
        let vh = viewport.height as f32;
        if let Some(path) = data.help_path.as_ref() {
            for distance in [0.0_f32, 24.0, 48.0] {
                let point = projector
                    .project(vec3(
                        path.origin.x + path.direction.x * distance,
                        path.origin.y + path.direction.y * distance,
                        path.origin.z + path.direction.z * distance,
                    ))
                    .map_err(|error| ClientError::BadUi(error.to_string()))?;
                if point.w > 0.0 {
                    result.push(UiDrawCommand::Fill {
                        rect: Rect {
                            x: vx + (point.x / point.w * 0.5 + 0.5) * vw - 3.0,
                            y: vy + (-point.y / point.w * 0.5 + 0.5) * vh - 3.0,
                            width: 6.0,
                            height: 6.0,
                        },
                        color: accent,
                    });
                }
            }
        }
        for point in state.points.iter() {
            let clip = projector
                .project(point.origin)
                .map_err(|error| ClientError::BadUi(error.to_string()))?;
            let divisor = if clip.w == 0.0 { 1.0 } else { clip.w };
            let mut x = vx + (clip.x / divisor * 0.5 + 0.5) * vw;
            let mut y = vy + (-clip.y / divisor * 0.5 + 0.5) * vh;
            if clip.w < 0.0 {
                x = vx * 2.0 + vw - x;
                y = vy * 2.0 + vh - y;
                if y > vy {
                    x = if x < vx + vw / 2.0 { vx } else { vx + vw };
                    y = y.min(vy + vh * 0.75);
                }
            }
            let width = point.width * transform.scale;
            let height = point.height * transform.scale;
            let distance = (x - vx - vw / 2.0).hypot(y - vy - vh / 2.0);
            result.push(UiDrawCommand::Image {
                resource: point.image.clone(),
                rect: Rect {
                    x: vx.max((vx + vw - width).min(x - width / 2.0)),
                    y: vy.max((vy + vh - height).min(y - height / 2.0)),
                    width,
                    height,
                },
                tex_coords: FULL_UV,
                color: Vec4 {
                    x: point.color.x,
                    y: point.color.y,
                    z: point.color.z,
                    w: point.color.w
                        * if point.hide_on_aim {
                            0.25_f32.max(1.0_f32.min(distance / 1.0_f32.max(width * 3.0)))
                        } else {
                            1.0
                        },
                },
            });
        }
    }
    if preferences.captions {
        let fallback_ink = CapInk { top: 0.0, height: 8.0 };
        let ink = skin.cap_ink.as_ref().unwrap_or(&fallback_ink);
        let fallback_measure = |text: &str, scale: f32| text.chars().count() as f32 * 8.0 * scale;
        let measure: &dyn Fn(&str, f32) -> f32 = match options.measure_text.as_ref() {
            Some(measure) => measure.as_ref(),
            None => &fallback_measure,
        };
        result.extend(caption_commands(
            &data.captions,
            &Rect {
                x: area.x + 8.0,
                y: area.y + area.height * 0.6,
                width: area.width - 16.0,
                height: area.height * 0.22,
            },
            &skin.font,
            text_scale * transform.scale,
            measure,
            ink,
        ));
    }
    result.push(UiDrawCommand::Clip { rect: None });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use qa_core::identity::{ClientId, IdentityOwner};
    use qa_core::math::{vec2, vec3, vec4};

    use super::powerups::PowerupTimerView;
    use super::weapon::CommonWeaponHud;
    use super::wheel::{WheelItem, WheelMode};
    use super::*;
    use crate::text::captions::{CaptionCue, CaptionKind};
    use crate::ui::common::skin::default_ui_skin;
    use crate::ui::types::{
        ArsenalAmmoWarning, ContentId, DopplerSelection, EnvironmentSelection, ItemId, PresentationSelection,
        ProviderRef, SeatPresentationBinding, WeaponAmmo, WeaponHudStatus,
    };
    use crate::view::CameraClip;

    fn test_owner() -> IdentityOwner {
        IdentityOwner::create("hud-test").expect("owner")
    }

    fn test_font() -> ResourceId {
        ResourceId::new("resource:test:font").expect("font")
    }

    fn test_icon() -> ResourceId {
        ResourceId::new("resource:test:icon").expect("icon")
    }

    fn test_binding(seat: SeatId, client: ClientId, area: Rect) -> SeatPresentationBinding {
        SeatPresentationBinding {
            seat,
            client,
            viewport: area,
            safe_area: area,
            hud_scale: 1.0,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Disabled,
                environment: EnvironmentSelection::Disabled,
                assets: ContentId::new("assets"),
                hud: ProviderRef {
                    provider: "hud".to_string(),
                    content: ContentId::new("hud"),
                },
                effects: ProviderRef {
                    provider: "fx".to_string(),
                    content: ContentId::new("fx"),
                },
                audio: ProviderRef {
                    provider: "audio".to_string(),
                    content: ContentId::new("audio"),
                },
            },
        }
    }

    fn full_area() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        }
    }

    fn test_context(seat: SeatId, client: ClientId, time_ms: i64) -> UiDrawContext {
        UiDrawContext {
            binding: test_binding(seat, client, full_area()),
            time_ms,
        }
    }

    fn test_options(skin: UiSkin) -> CommonHudDrawOptions {
        CommonHudDrawOptions {
            skin,
            measure_text: None,
            preferences: UiPreferenceValues::default(),
            camera: None,
            localize: Rc::new(|text: &str| text.to_string()),
        }
    }

    fn test_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport: crate::view::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn test_point(id: i32, expires_ms: i64) -> HudPointOfInterest {
        HudPointOfInterest {
            id,
            origin: vec3(10.0, 0.0, 0.0),
            image: test_icon(),
            width: 20.0,
            height: 10.0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            hide_on_aim: false,
            expires_ms,
        }
    }

    fn assert_rect_approx(actual: &Rect, expected: &Rect) {
        for (found, wanted) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.width, expected.width),
            (actual.height, expected.height),
        ] {
            assert!(
                (found - wanted).abs() < 1e-3,
                "rect {actual:?} differs from {expected:?}"
            );
        }
    }

    fn ms(value: i32) -> SourceTime {
        SourceTime::Milliseconds(value)
    }

    #[test]
    fn empty_hud_data_defaults() {
        let owner = test_owner();
        let data = empty_hud_data(owner.seat(0));
        assert_eq!(data.seat, owner.seat(0));
        assert!(data.visible);
        assert!(data.crosshair.visible);
        assert_eq!(data.crosshair.color, vec4(1.0, 1.0, 1.0, 1.0));
        assert!(data.crosshair.image.is_none());
        assert!(data.powerups.is_empty());
        assert!(data.vitals.is_empty());
        assert!(data.prompts.is_empty());
        assert!(data.health_bars.is_empty());
        assert!(data.captions.is_empty());
        assert!(data.damage_indicators.is_empty());
        assert!(data.weapon.is_none());
        assert!(data.inventory.is_none());
        assert!(data.help.is_none());
        assert!(data.wheel.is_none());
        assert!(data.carousel.is_none());
        assert!(data.help_path.is_none());
        assert!(data.pickup.is_none());
        assert!(data.hit_marker.is_none());
    }

    #[test]
    fn notify_sequences_and_expiry() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        messages.notify(&seat, "n1", false, ms(1000), ms(500)).expect("notify");
        messages.notify(&seat, "c1", true, ms(1000), ms(500)).expect("notify");
        assert!(messages.active(999).notifications.is_empty());
        let live = messages.active(1000);
        assert_eq!(live.notifications.len(), 2);
        assert_eq!(live.notifications[0].sequence, 0);
        assert_eq!(live.notifications[1].sequence, 1);
        assert!(!live.notifications[0].chat);
        assert!(live.notifications[1].chat);
        assert!(messages.active(1500).notifications.is_empty());
        assert!(messages.active(1600).notifications.is_empty());
    }

    #[test]
    fn center_print_instant_replaces_queue() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        messages
            .center_print(&seat, "a", ms(0), ms(10_000), true, 125.0)
            .expect("instant");
        messages
            .center_print(&seat, "slow", ms(0), ms(100), false, 125.0)
            .expect("queued");
        messages
            .center_print(&seat, "b", ms(0), ms(10_000), true, 125.0)
            .expect("replace");
        let live = messages.active(100);
        assert_eq!(live.center_print.as_ref().expect("center").text, "b");
        assert!(messages.active(20_000).center_print.is_none());
    }

    #[test]
    fn center_print_typewriter_chains() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        messages
            .center_print(&seat, "abc", ms(1000), ms(100), false, 125.0)
            .expect("first");
        messages
            .center_print(&seat, "de", ms(1000), ms(50), false, 125.0)
            .expect("second");
        let first = messages.active(1000);
        let print = first.center_print.as_ref().expect("center");
        assert_eq!(print.text, "abc");
        assert_eq!(print.starts, ms(1000));
        assert_eq!(print.duration, ms(475));
        let second = messages.active(1475);
        let print = second.center_print.as_ref().expect("chained");
        assert_eq!(print.text, "de");
        assert_eq!(print.starts, ms(1475));
        assert_eq!(print.duration, ms(300));
        assert!(messages.active(1775).center_print.is_none());
    }

    #[test]
    fn clear_variants_drop_state() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        messages.notify(&seat, "n", false, ms(0), ms(1000)).expect("notify");
        messages
            .center_print(&seat, "c", ms(0), ms(1000), true, 125.0)
            .expect("center");
        messages.add_point(&seat, test_point(1, 5000), 100, 64).expect("point");
        messages
            .set_source_points(&seat, vec![test_point(2, 5000)])
            .expect("source");
        messages.clear_notify();
        let state = messages.active(100);
        assert!(state.notifications.is_empty());
        assert!(state.center_print.is_some());
        messages.clear_center_print();
        assert!(messages.active(100).center_print.is_none());
        messages.clear();
        let state = messages.active(100);
        assert!(state.points.is_empty());
    }

    #[test]
    fn source_points_merge_and_expire() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        messages.add_point(&seat, test_point(1, 5000), 100, 64).expect("local");
        messages
            .set_source_points(&seat, vec![test_point(2, 5000), test_point(3, 50)])
            .expect("source");
        let state = messages.active(100);
        let ids: Vec<i32> = state.points.iter().map(|point| point.id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn message_seat_mismatch_errors() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let other = owner.seat(1);
        let mut messages = SeatHudMessages::new(seat);
        for result in [
            messages.notify(&other, "n", false, ms(0), ms(1)),
            messages.center_print(&other, "c", ms(0), ms(1), true, 125.0),
            messages.set_source_points(&other, Vec::new()),
            messages.add_point(&other, test_point(1, 9), 0, 64).map(|_| ()),
        ] {
            let error = result.expect_err("wrong seat must fail");
            assert_eq!(error.to_string(), "HUD message belongs to another seat");
        }
    }

    #[test]
    fn add_point_capacity_and_replacement() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        assert!(messages.add_point(&seat, test_point(1, 2000), 1000, 2).expect("add"));
        assert!(messages.add_point(&seat, test_point(2, 2000), 1000, 2).expect("add"));
        assert!(!messages.add_point(&seat, test_point(3, 2000), 1000, 2).expect("full"));
        assert!(!messages.add_point(&seat, test_point(0, 2000), 1000, 2).expect("full"));
        assert!(messages.add_point(&seat, test_point(2, 2500), 1000, 2).expect("keyed"));
        assert_eq!(messages.active(1000).points.len(), 2);
        messages.remove_point(1);
        assert!(messages.add_point(&seat, test_point(0, 2000), 1000, 2).expect("room"));
        assert!(messages.add_point(&seat, test_point(0, 3000), 1000, 2).expect("oldest"));
        let mut expiries: Vec<i64> = messages
            .active(1000)
            .points
            .iter()
            .map(|point| point.expires_ms)
            .collect();
        expiries.sort_unstable();
        assert_eq!(expiries, vec![2500, 3000]);
    }

    #[test]
    fn add_point_reuses_expired_slots() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        assert!(messages.add_point(&seat, test_point(1, 500), 1000, 2).expect("add"));
        assert!(messages.add_point(&seat, test_point(2, 2000), 1000, 2).expect("reuse"));
        let state = messages.active(1000);
        assert_eq!(state.points.len(), 1);
        assert_eq!(state.points[0].id, 2);
    }

    #[test]
    fn remove_point_rules() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let mut messages = SeatHudMessages::new(seat.clone());
        messages.add_point(&seat, test_point(1, 2000), 1000, 64).expect("add");
        messages.add_point(&seat, test_point(0, 2000), 1000, 64).expect("add");
        messages.remove_point(0);
        messages.remove_point(9);
        assert_eq!(messages.active(1000).points.len(), 2);
        messages.remove_point(1);
        let state = messages.active(1000);
        assert_eq!(state.points.len(), 1);
        assert_eq!(state.points[0].id, 0);
    }

    #[test]
    fn hud_vital_rects_numbers() {
        assert!(hud_vital_rects(0, 1.0, 42.0).is_empty());
        assert!(hud_vital_rects(-2, 1.0, 42.0).is_empty());
        assert_eq!(
            hud_vital_rects(1, 1.0, 42.0),
            vec![Rect {
                x: 240.0,
                y: 434.0,
                width: 156.0,
                height: 42.0
            }]
        );
        assert_eq!(
            hud_vital_rects(2, 1.0, 42.0),
            vec![
                Rect {
                    x: 160.0,
                    y: 434.0,
                    width: 156.0,
                    height: 42.0
                },
                Rect {
                    x: 320.0,
                    y: 434.0,
                    width: 156.0,
                    height: 42.0
                },
            ]
        );
        assert_eq!(
            hud_vital_rects(3, 2.0, 42.0),
            vec![
                Rect {
                    x: 170.0,
                    y: 434.0,
                    width: 96.0,
                    height: 42.0
                },
                Rect {
                    x: 270.0,
                    y: 434.0,
                    width: 96.0,
                    height: 42.0
                },
                Rect {
                    x: 370.0,
                    y: 434.0,
                    width: 96.0,
                    height: 42.0
                },
            ]
        );
    }

    #[test]
    fn hud_vital_occupied_rects_normal_and_fallback() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat, owner.client(0, 0), 0);
        assert_eq!(
            hud_vital_occupied_rects(&context, 1, 1.0, 1.5, 8.0).expect("layout"),
            vec![Rect {
                x: 240.0,
                y: 434.0,
                width: 156.0,
                height: 42.0
            }]
        );
        let tiny = UiDrawContext {
            binding: test_binding(
                context.binding.seat.clone(),
                context.binding.client.clone(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 60.0,
                },
            ),
            time_ms: 0,
        };
        assert_eq!(
            hud_vital_occupied_rects(&tiny, 1, 1.0, 1.5, 8.0).expect("layout"),
            vec![Rect {
                x: 4.0,
                y: 24.0,
                width: 88.0,
                height: 32.0
            }]
        );
    }

    #[test]
    fn draw_seat_mismatch_and_hidden() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let options = test_options(default_ui_skin(&test_font()));
        let mut data = empty_hud_data(owner.seat(1));
        let mut messages = SeatHudMessages::new(seat.clone());
        let error = draw_common_hud(&context, &data, &options, &mut messages).expect_err("data seat must match");
        assert_eq!(error.to_string(), "HUD frame belongs to another seat");
        data = empty_hud_data(seat.clone());
        let mut foreign = SeatHudMessages::new(owner.seat(1));
        let error = draw_common_hud(&context, &data, &options, &mut foreign).expect_err("message seat must match");
        assert_eq!(error.to_string(), "HUD frame belongs to another seat");
        data.visible = false;
        let hidden = draw_common_hud(&context, &data, &options, &mut messages).expect("hidden");
        assert!(hidden.is_empty());
    }

    #[test]
    fn draw_crosshair_hitmarker_clip_wrap() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let options = test_options(default_ui_skin(&test_font()));
        let mut data = empty_hud_data(seat.clone());
        data.hit_marker = Some(HudHitMarker {
            damage: 10.0,
            expires_ms: 1150,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        let accent = vec4(1.0, 0.65, 0.22, 1.0);
        assert_eq!(result.len(), 8);
        assert_eq!(
            result[0],
            UiDrawCommand::Clip {
                rect: Some(full_area())
            }
        );
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 316.0,
                    y: 239.0,
                    width: 8.0,
                    height: 2.0
                },
                color: white,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 319.0,
                    y: 236.0,
                    width: 2.0,
                    height: 8.0
                },
                color: white,
            }
        );
        for (command, origin) in
            result[3..7]
                .iter()
                .zip([(308.0, 228.0), (308.0, 248.0), (328.0, 228.0), (328.0, 248.0)])
        {
            assert_eq!(
                *command,
                UiDrawCommand::Fill {
                    rect: Rect {
                        x: origin.0,
                        y: origin.1,
                        width: 4.0,
                        height: 4.0
                    },
                    color: accent,
                }
            );
        }
        assert_eq!(result[7], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_vitals_full_layout() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.vitals.push(HudValue {
            label: "Health".to_string(),
            value: 100.0,
            icon: None,
            warning: false,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 5);
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 240.0,
                    y: 434.0,
                    width: 156.0,
                    height: 42.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(244.0, 438.0),
                text: "100".to_string(),
                font: test_font(),
                scale: 2.25,
                color: skin.colors.text,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Text {
                origin: vec2(244.0, 460.0),
                text: "Health".to_string(),
                font: test_font(),
                scale: 1.2,
                color: skin.colors.text,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(result[4], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_vitals_compact_fallback() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let tiny = UiDrawContext {
            binding: test_binding(
                seat.clone(),
                owner.client(0, 0),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 60.0,
                },
            ),
            time_ms: 1000,
        };
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.vitals.push(HudValue {
            label: "HP".to_string(),
            value: 100.0,
            icon: None,
            warning: false,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&tiny, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 4.0,
                    y: 24.0,
                    width: 88.0,
                    height: 32.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(8.0, 28.0),
                text: "HP 100".to_string(),
                font: test_font(),
                scale: 1.0,
                color: skin.colors.text,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(result[3], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_weapon_status_panel() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.weapon = Some(CommonWeaponHud {
            status: WeaponHudStatus {
                source: ProviderRef {
                    provider: "q2:game".to_string(),
                    content: ContentId::new("game"),
                },
                item: ItemId::new("item:blaster"),
                label: "Blaster".to_string(),
                ammo: WeaponAmmo::Finite {
                    item: ItemId::new("item:cells"),
                    count: 10,
                    has_ammo_to_start: true,
                    low: false,
                },
            },
            warning: ArsenalAmmoWarning::None,
            weapon_icon: None,
            ammo_icon: None,
            icon_aspect: 1.0,
            ammo_aspect: 1.0,
            native_status: false,
            measure_text: None,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 240.0,
                    y: 434.0,
                    width: 156.0,
                    height: 42.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(result.last(), Some(&UiDrawCommand::Clip { rect: None }));
    }

    #[test]
    fn draw_health_bars() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.health_bars.push(HudHealthBar {
            id: "boss".to_string(),
            label: "Boss".to_string(),
            value: 50.0,
            maximum: 100.0,
            color: vec4(1.0, 0.0, 0.0, 1.0),
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 5);
        assert_eq!(
            result[1],
            UiDrawCommand::Text {
                origin: vec2(320.0, 24.0),
                text: "Boss".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 160.0,
                    y: 40.0,
                    width: 320.0,
                    height: 8.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 160.0,
                    y: 40.0,
                    width: 160.0,
                    height: 8.0
                },
                color: vec4(1.0, 0.0, 0.0, 1.0),
            }
        );
        assert_eq!(result[4], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_notices_and_centerprint() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        let mut messages = SeatHudMessages::new(seat.clone());
        messages
            .notify(&seat, "hello", false, ms(900), ms(500))
            .expect("notify");
        messages
            .center_print(&seat, "hi\nthere", ms(900), ms(500), true, 125.0)
            .expect("center");
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 5);
        assert_eq!(
            result[1],
            UiDrawCommand::Text {
                origin: vec2(12.0, 12.0),
                text: "hello".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(320.0, 146.0),
                text: "hi".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Text {
                origin: vec2(320.0, 160.0),
                text: "there".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(result[4], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_typewriter_reveal() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin);
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        let mut messages = SeatHudMessages::new(seat.clone());
        messages
            .center_print(&seat, "abc", ms(1000), ms(100), false, 125.0)
            .expect("typewriter");
        let start = test_context(seat.clone(), owner.client(0, 0), 1000);
        let result = draw_common_hud(&start, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 3);
        match &result[1] {
            UiDrawCommand::Text { text, origin, .. } => {
                assert_eq!(text, "");
                assert_eq!(*origin, vec2(320.0, 153.0));
            }
            other => panic!("expected text, got {other:?}"),
        }
        let later = test_context(seat.clone(), owner.client(0, 0), 1125);
        let result = draw_common_hud(&later, &data, &options, &mut messages).expect("draw");
        match &result[1] {
            UiDrawCommand::Text { text, .. } => assert_eq!(text, "a"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn draw_origin_damage_indicators() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let mut options = test_options(skin);
        options.camera = Some(test_camera());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.damage_indicators.push(HudDamageIndicator::Origin {
            origin: vec3(100.0, 0.0, 0.0),
            amount: 10.0,
            expires_ms: 1200,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 5);
        let tint = vec4(1.0, 0.15, 0.05, 0.5);
        for (command, y) in result[1..4].iter().zip([182.0, 175.0, 168.0]) {
            assert_eq!(
                *command,
                UiDrawCommand::Fill {
                    rect: Rect {
                        x: 317.0,
                        y,
                        width: 6.0,
                        height: 6.0
                    },
                    color: tint,
                }
            );
        }
        assert_eq!(result[4], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_picture_damage_yaw() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let mut options = test_options(skin);
        options.camera = Some(test_camera());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.damage_indicators.push(HudDamageIndicator::Directional {
            picture: Some(HudDamagePicture {
                image: test_icon(),
                width: 20.0,
                height: 10.0,
            }),
            direction: vec3(1.0, 0.0, 0.0),
            amount: 5.0,
            color: vec3(0.0, 1.0, 0.0),
            health: true,
            armor: false,
            shield: false,
            expires_ms: 1500,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 3);
        match &result[1] {
            UiDrawCommand::Image {
                rect, resource, color, ..
            } => {
                assert_rect_approx(
                    rect,
                    &Rect {
                        x: 312.5,
                        y: 248.0,
                        width: 15.0,
                        height: 10.0,
                    },
                );
                assert_eq!(*resource, test_icon());
                assert_eq!(*color, vec4(0.0, 1.0, 0.0, 0.5));
            }
            other => panic!("expected image, got {other:?}"),
        }
        assert_eq!(result[2], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_pickup_banner() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.pickup = Some(HudPickup {
            name: "Shells".to_string(),
            icon: Some(test_icon()),
            expires_ms: 1200,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[1],
            UiDrawCommand::Image {
                resource: test_icon(),
                rect: Rect {
                    x: 184.0,
                    y: 346.0,
                    width: 28.0,
                    height: 28.0
                },
                tex_coords: FULL_UV,
                color: skin.colors.text,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(320.0, 350.0),
                text: "Shells".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.accent,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(result[3], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_inventory_window() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.inventory = Some(vec![HudInventoryItem {
            id: "shells".to_string(),
            label: "Shells".to_string(),
            count: 5,
            selected: true,
            binding: Some("1".to_string()),
            icon: None,
        }]);
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 8);
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 128.0,
                    y: 72.0,
                    width: 384.0,
                    height: 328.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 136.0,
                    y: 114.0,
                    width: 368.0,
                    height: 14.0
                },
                color: skin.colors.focused,
            }
        );
        assert_eq!(
            result[5],
            UiDrawCommand::Text {
                origin: vec2(212.0, 114.0),
                text: "Shells".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.accent,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(
            result[6],
            UiDrawCommand::Text {
                origin: vec2(494.0, 114.0),
                text: "5".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Right,
                shadow: true,
            }
        );
        assert_eq!(result[7], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_prompts() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.prompts.push(HudPrompt {
            action: "Use".to_string(),
            binding: "E".to_string(),
            icon: None,
        });
        data.prompts.push(HudPrompt {
            action: "Jump".to_string(),
            binding: String::new(),
            icon: None,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[1],
            UiDrawCommand::Text {
                origin: vec2(320.0, 376.0),
                text: "[E] Use".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.accent,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(320.0, 394.0),
                text: "Jump".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.accent,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(result[3], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_help_overlay() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.help = Some(HudHelp {
            title: "Help".to_string(),
            lines: vec!["a".to_string()],
            objectives: vec![HudObjective {
                text: "done".to_string(),
                complete: true,
            }],
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 6);
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 48.0,
                    y: 48.0,
                    width: 544.0,
                    height: 360.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Text {
                origin: vec2(68.0, 104.0),
                text: "a".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(
            result[4],
            UiDrawCommand::Text {
                origin: vec2(68.0, 132.0),
                text: "[x] done".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Left,
                shadow: true,
            }
        );
        assert_eq!(result[5], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_wheel() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.wheel = Some(WheelPresentation {
            mode: WheelMode::Weapons,
            items: vec![WheelItem {
                id: "w1".to_string(),
                source_ordinal: 0,
                sort_order: 0,
                label: "Axe".to_string(),
                owned: true,
                has_ammo: true,
                count: Some(5.0),
                warning_count: 10,
                icon: None,
                selected_icon: None,
            }],
            selected: Some("w1".to_string()),
            opacity: 0.5,
            cursor: vec2(0.0, 0.0),
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 7);
        let panel = Vec4 {
            w: skin.colors.panel.w * 0.5,
            ..skin.colors.panel
        };
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 128.0,
                    y: 48.0,
                    width: 384.0,
                    height: 384.0
                },
                color: panel,
            }
        );
        let tint = Vec4 {
            w: 0.5,
            ..skin.colors.accent
        };
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(320.0, 96.0),
                text: "Axe".to_string(),
                font: test_font(),
                scale: 1.5,
                color: tint,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Text {
                origin: vec2(320.0, 128.0),
                text: "5".to_string(),
                font: test_font(),
                scale: 1.5,
                color: tint,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(
            result[5],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 318.0,
                    y: 238.0,
                    width: 4.0,
                    height: 4.0
                },
                color: skin.colors.accent,
            }
        );
        assert_eq!(result[6], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_carousel() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        let item = |id: &str, count: f32| WheelItem {
            id: id.to_string(),
            source_ordinal: 0,
            sort_order: 0,
            label: id.to_string(),
            owned: true,
            has_ammo: true,
            count: Some(count),
            warning_count: 0,
            icon: None,
            selected_icon: None,
        };
        data.carousel = Some(CarouselPresentation {
            items: vec![item("w1", 3.0), item("w2", 7.0)],
            selected: Some("w1".to_string()),
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 5);
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 272.0,
                    y: 324.0,
                    width: 46.0,
                    height: 50.0
                },
                color: skin.colors.focused,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(296.0, 358.0),
                text: "3".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.accent,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(
            result[3],
            UiDrawCommand::Text {
                origin: vec2(344.0, 358.0),
                text: "7".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Center,
                shadow: true,
            }
        );
        assert_eq!(result[4], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_powerup_timers_row() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin.clone());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.powerups.push(PowerupTimerView {
            item: ItemId::new("item:quad"),
            label: "Quad".to_string(),
            remaining_seconds: 30.0,
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[1],
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 436.0,
                    y: 422.0,
                    width: 200.0,
                    height: 16.0
                },
                color: skin.colors.panel,
            }
        );
        assert_eq!(
            result[2],
            UiDrawCommand::Text {
                origin: vec2(632.0, 424.0),
                text: "Quad 30s".to_string(),
                font: test_font(),
                scale: 1.5,
                color: skin.colors.text,
                align: TextAlign::Right,
                shadow: true,
            }
        );
        assert_eq!(result[3], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_help_path_markers() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let mut options = test_options(skin.clone());
        options.camera = Some(test_camera());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.help_path = Some(HudHelpPath {
            origin: vec3(10.0, 0.0, 0.0),
            direction: vec3(1.0, 0.0, 0.0),
        });
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 5);
        for command in result[1..4].iter() {
            assert_eq!(
                *command,
                UiDrawCommand::Fill {
                    rect: Rect {
                        x: 317.0,
                        y: 237.0,
                        width: 6.0,
                        height: 6.0
                    },
                    color: skin.colors.accent,
                }
            );
        }
        assert_eq!(result[4], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_poi_front_and_aim_fade() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let mut options = test_options(skin);
        options.camera = Some(test_camera());
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        let mut messages = SeatHudMessages::new(seat.clone());
        messages.add_point(&seat, test_point(1, 2000), 1000, 64).expect("point");
        let mut aimed = test_point(2, 2000);
        aimed.hide_on_aim = true;
        messages.add_point(&seat, aimed, 1000, 64).expect("point");
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[1],
            UiDrawCommand::Image {
                resource: test_icon(),
                rect: Rect {
                    x: 310.0,
                    y: 235.0,
                    width: 20.0,
                    height: 10.0
                },
                tex_coords: FULL_UV,
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }
        );
        match &result[2] {
            UiDrawCommand::Image { color, .. } => assert_eq!(*color, vec4(1.0, 1.0, 1.0, 0.25)),
            other => panic!("expected image, got {other:?}"),
        }
        assert_eq!(result[3], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_poi_behind_camera_flips() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let mut options = test_options(skin);
        let mut camera = test_camera();
        camera.projection[15] = -1.0;
        options.camera = Some(camera);
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        let mut messages = SeatHudMessages::new(seat.clone());
        messages.add_point(&seat, test_point(1, 2000), 1000, 64).expect("point");
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 3);
        assert_eq!(
            result[1],
            UiDrawCommand::Image {
                resource: test_icon(),
                rect: Rect {
                    x: 620.0,
                    y: 235.0,
                    width: 20.0,
                    height: 10.0
                },
                tex_coords: FULL_UV,
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }
        );
        assert_eq!(result[2], UiDrawCommand::Clip { rect: None });
    }

    #[test]
    fn draw_captions_region() {
        let owner = test_owner();
        let seat = owner.seat(0);
        let context = test_context(seat.clone(), owner.client(0, 0), 1000);
        let skin = default_ui_skin(&test_font());
        let options = test_options(skin);
        let mut data = empty_hud_data(seat.clone());
        data.crosshair.visible = false;
        data.captions.push(ActiveCaption {
            cue: CaptionCue {
                id: "cue".to_string(),
                kind: CaptionKind::Subtitle,
                start_ms: 0.0,
                duration_ms: 1000.0,
                text: "hi".to_string(),
                speaker: None,
                arguments: Vec::new(),
            },
            localized_text: "hi".to_string(),
            localized_speaker: None,
        });
        let mut messages = SeatHudMessages::new(seat.clone());
        let result = draw_common_hud(&context, &data, &options, &mut messages).expect("draw");
        assert_eq!(result.len(), 4);
        match &result[1] {
            UiDrawCommand::Fill { rect, .. } => assert_rect_approx(
                rect,
                &Rect {
                    x: 8.0,
                    y: 369.6,
                    width: 624.0,
                    height: 24.0,
                },
            ),
            other => panic!("expected fill, got {other:?}"),
        }
        match &result[2] {
            UiDrawCommand::Text { origin, text, .. } => {
                assert_eq!(text, "hi");
                assert!((origin.x - 320.0).abs() < 1e-3);
                assert!((origin.y - 373.6).abs() < 1e-3);
            }
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(result[3], UiDrawCommand::Clip { rect: None });
        let mut off = test_options(default_ui_skin(&test_font()));
        off.preferences.captions = false;
        let mut messages = SeatHudMessages::new(seat);
        let result = draw_common_hud(&context, &data, &off, &mut messages).expect("draw");
        assert_eq!(
            result,
            vec![
                UiDrawCommand::Clip {
                    rect: Some(full_area())
                },
                UiDrawCommand::Clip { rect: None },
            ]
        );
    }
}
