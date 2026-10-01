//! Per-actor source HUD state folded from presentation events.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/seat-hud-state.ts`
//! (`SeatSourceHud`). State addressed to one local actor; source events
//! remain authoritative. Presentation events (unported `./simulation/types.ts`)
//! arrive as the absorbed [`SeatHudEvent`]; HUD rows reuse
//! `qa_client::ui::hud`; icon loads, pictures, and palettes (donor
//! `HudAssets` over `./weapon-hud.ts`) arrive through [`SeatHudIcons`].
//! Sync port: localization and icon loads are sync callbacks.

use std::collections::BTreeMap;

use qa_client::ui::hud::HudDamageIndicator;
use qa_client::ui::hud::HudDamagePicture;
use qa_client::ui::hud::HudHealthBar;
use qa_client::ui::hud::HudHelp;
use qa_client::ui::hud::HudHelpPath;
use qa_client::ui::hud::HudInventoryItem;
use qa_client::ui::hud::HudObjective;
use qa_client::ui::hud::HudPickup;
use qa_client::ui::hud::HudPointOfInterest;
use qa_client::ui::hud::HudPrompt;
use qa_client::ui::types::ContentId;
use qa_client::ui::types::ResourceId;
use qa_core::math::Vec3;
use qa_core::math::Vec4;

use super::remote_seat_source::RemoteActorId;

/// Maximum origin damage indicators retained.
const MAX_ORIGIN_DAMAGE: usize = 8;
/// Maximum points of interest retained.
const MAX_POIS: usize = 32;
/// Maximum directional damage indicators retained.
const MAX_DIRECTIONAL_DAMAGE: usize = 32;

/// Q1 capture team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatHudCaptureTeam {
    /// Red team.
    Red,
    /// Blue team.
    Blue,
}

/// One inventory entry (donor `entries` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatHudInventoryEntry {
    /// Item id.
    pub item: String,
    /// Stack count.
    pub count: i32,
}

/// One inventory label (donor `labels` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatHudInventoryLabel {
    /// Item id.
    pub item: String,
    /// Display name.
    pub name: String,
}

/// One scoreboard row (donor `rows` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatHudScoreRow {
    /// Score.
    pub score: i32,
    /// Player name.
    pub name: String,
    /// Ping.
    pub ping: i32,
    /// Minutes played.
    pub minutes: i32,
    /// Spectator flag.
    pub spectator: bool,
}

/// One end-of-unit level (donor `levels` row).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatHudUnitLevel {
    /// Level name.
    pub name: String,
    /// Map name.
    pub map: String,
    /// Visit order.
    pub visit_order: i32,
    /// Monsters killed.
    pub killed_monsters: i32,
    /// Total monsters.
    pub total_monsters: i32,
    /// Secrets found.
    pub found_secrets: i32,
    /// Total secrets.
    pub total_secrets: i32,
    /// Level time in seconds.
    pub time: f64,
}

/// Absorbed simulation presentation event (donor `SimulationPresentationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum SeatHudEvent {
    /// Q2 pickup banner.
    Q2Pickup {
        /// Receiving player.
        player: RemoteActorId,
        /// Pickup name.
        name: String,
        /// Icon path.
        icon: String,
    },
    /// Q2 origin damage indicator.
    Q2DamageIndicator {
        /// Damaged actor.
        actor: RemoteActorId,
        /// Damage origin.
        origin: Vec3,
        /// Damage amount.
        amount: f32,
    },
    /// Q2 help text slot.
    Q2Help {
        /// Text slot.
        slot: i32,
        /// Help text.
        text: String,
    },
    /// Q2 player inventory.
    Q2PlayerInventory {
        /// Owning actor.
        actor: RemoteActorId,
        /// Panel visibility.
        visible: bool,
        /// Entries.
        entries: Vec<SeatHudInventoryEntry>,
        /// Labels.
        labels: Vec<SeatHudInventoryLabel>,
        /// Selected item.
        selected: String,
    },
    /// Q2 player help visibility.
    Q2PlayerHelp {
        /// Owning actor.
        actor: RemoteActorId,
        /// Help visibility.
        visible: bool,
    },
    /// Q2 player layout.
    Q2PlayerView {
        /// Owning actor.
        actor: RemoteActorId,
        /// Layout bits.
        layouts: u32,
        /// Selected item.
        selected_item: String,
    },
    /// Q2 player scoreboard.
    Q2PlayerScoreboard {
        /// Owning actor.
        actor: RemoteActorId,
        /// Rows.
        rows: Vec<SeatHudScoreRow>,
    },
    /// Rerelease mission objective.
    MissionObjective {
        /// Owning actor.
        actor: RemoteActorId,
        /// Objective text.
        text: String,
        /// Format arguments.
        args: Vec<String>,
    },
    /// Rerelease mission status.
    MissionStatus {
        /// Owning actor.
        actor: RemoteActorId,
        /// Icon visibility.
        icon_visible: bool,
    },
    /// Rerelease point of interest.
    Poi {
        /// Owning actor.
        actor: RemoteActorId,
        /// Marker position.
        position: Vec3,
        /// Image path.
        image: String,
        /// Palette color.
        color: i32,
        /// Duration in milliseconds.
        duration: f64,
    },
    /// Rerelease keyed point of interest.
    KeyedPoi {
        /// Owning actor.
        actor: RemoteActorId,
        /// Marker key.
        key: i32,
        /// Marker flags.
        flags: i32,
        /// Marker position.
        position: Vec3,
        /// Image path.
        image: String,
        /// Palette color.
        color: i32,
        /// Duration in milliseconds.
        duration: f64,
    },
    /// Rerelease POI removal.
    RemovePoi {
        /// Owning actor.
        actor: RemoteActorId,
        /// Marker key.
        key: i32,
    },
    /// Rerelease directional damage.
    DirectionalDamage {
        /// Owning actor.
        actor: RemoteActorId,
        /// Damage direction.
        direction: Vec3,
        /// Damage amount.
        damage: f32,
        /// Health component.
        health: f32,
        /// Armor component.
        armor: f32,
        /// Shield component.
        shield: f32,
    },
    /// Rerelease help path.
    HelpPath {
        /// Owning actor.
        actor: RemoteActorId,
        /// Ray origin.
        position: Vec3,
        /// Ray direction.
        direction: Vec3,
    },
    /// Rerelease end of unit.
    EndOfUnit {
        /// Continue-button time in seconds.
        button_time: f64,
        /// Levels.
        levels: Vec<SeatHudUnitLevel>,
    },
    /// Rerelease boss health bar.
    Healthbar {
        /// Owning actor.
        actor: RemoteActorId,
        /// Bar slot.
        slot: i32,
        /// Bar visibility.
        visible: bool,
        /// Bar name.
        name: String,
        /// Bar fraction.
        fraction: f32,
    },
    /// Rerelease help computer.
    HelpComputer {
        /// Owning actor.
        actor: RemoteActorId,
        /// Help visibility.
        visible: bool,
        /// Primary text.
        primary: String,
        /// Secondary text.
        secondary: String,
    },
    /// Q1 composition client.
    Q1Client {
        /// Client slot.
        slot: i32,
        /// Client name.
        name: String,
        /// Frags.
        frags: i32,
        /// Team number.
        team: i32,
        /// Observer flag.
        observer: bool,
    },
    /// Q1 composition client departure.
    Q1ClientLeft {
        /// Client slot.
        slot: i32,
    },
    /// Q1 CTF status.
    Q1CtfStatus {
        /// Owning actor.
        actor: RemoteActorId,
        /// Red score.
        red: i32,
        /// Blue score.
        blue: i32,
        /// Flag bits.
        flags: i32,
        /// Rune item bits.
        rune_items: i32,
    },
    /// Q1 CTF capture.
    Q1CtfCapture {
        /// Capturing team.
        team: SeatHudCaptureTeam,
        /// New total.
        total: i32,
    },
}

impl SeatHudEvent {
    /// Whether the event localizes at prepare time (donor `receive` gate).
    fn localizes(&self) -> bool {
        matches!(
            self,
            Self::Healthbar { .. } | Self::HelpComputer { .. } | Self::MissionObjective { .. }
        )
    }
}

/// One sourced presentation event (donor `{ kind, content, event, seconds }`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatHudSourceEvent {
    /// Source content.
    pub content: ContentId,
    /// Source time in seconds.
    pub seconds: f64,
    /// Event payload.
    pub event: SeatHudEvent,
}

/// Icon loads, pictures, and palettes (donor `HudAssets`).
pub trait SeatHudIcons {
    /// Load an image resource.
    fn load_image(&mut self, content: &ContentId, path: &str) -> ResourceId;
    /// Picture size in UI units.
    fn picture_size(&self, image: &ResourceId) -> Option<(f32, f32)>;
    /// RGB palette triplets, if any.
    fn palette(&mut self, content: &ContentId) -> Option<Vec<u8>>;
}

/// One objective print (donor `{ text, seconds }` row).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatHudObjectivePrint {
    /// Print text.
    pub text: String,
    /// Source time in seconds.
    pub seconds: f64,
}

/// One HUD frame (donor `presentation` pick).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatHudFrame {
    /// Boss health bars.
    pub health_bars: Vec<HudHealthBar>,
    /// Help overlay.
    pub help: Option<HudHelp>,
    /// Action prompts.
    pub prompts: Vec<HudPrompt>,
    /// Inventory rows.
    pub inventory: Option<Vec<HudInventoryItem>>,
    /// Pickup banner.
    pub pickup: Option<HudPickup>,
    /// Damage indicators.
    pub damage_indicators: Vec<HudDamageIndicator>,
    /// Help-path ray.
    pub help_path: Option<HudHelpPath>,
}

#[derive(Debug, Clone)]
struct HudPoi {
    key: i32,
    flags: i32,
    width: f32,
    height: f32,
    content: ContentId,
    path: String,
    image: Option<ResourceId>,
    origin: Vec3,
    expires_ms: f64,
    color: u8,
    tint: Vec4,
}

#[derive(Debug, Clone)]
struct HudPickupState {
    name: String,
    path: String,
    content: ContentId,
    icon: Option<ResourceId>,
    expires_ms: f64,
}

#[derive(Debug, Clone)]
struct HudDamagePictureState {
    content: ContentId,
    image: Option<ResourceId>,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
struct HudHelpPathState {
    origin: Vec3,
    direction: Vec3,
    expires_ms: f64,
}

#[derive(Debug, Clone)]
struct Q1ClientRow {
    name: String,
    frags: i32,
    team: i32,
    observer: bool,
}

#[derive(Debug, Clone)]
struct UnitReport {
    lines: Vec<String>,
    ready_ms: f64,
}

fn normalize3(value: Vec3) -> Vec3 {
    let length = (value.x * value.x + value.y * value.y + value.z * value.z).sqrt();
    let length = if length == 0.0 { 1.0 } else { length };
    Vec3 {
        x: value.x / length,
        y: value.y / length,
        z: value.z / length,
    }
}

fn dot3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn flag_status(bits: i32) -> &'static str {
    if bits & 4 != 0 {
        "dropped"
    } else if bits & 2 != 0 {
        "carried"
    } else {
        "home"
    }
}

/// State addressed to one local actor.
pub struct SeatSourceHud<Localize = fn(&ContentId, &str, &[String]) -> String> {
    actor: RemoteActorId,
    localize: Option<Localize>,
    pending: Vec<SeatHudSourceEvent>,
    objective_prints: Vec<SeatHudObjectivePrint>,
    objective: String,
    mission_visible: bool,
    bars: BTreeMap<i32, HudHealthBar>,
    help_text: BTreeMap<i32, String>,
    help_visible: bool,
    inventory: Option<Vec<HudInventoryItem>>,
    scores_held: bool,
    score_visible: bool,
    score_rows: Vec<String>,
    clients: BTreeMap<i32, Q1ClientRow>,
    report: Option<UnitReport>,
    pois: Vec<HudPoi>,
    path: Option<HudHelpPathState>,
    flags: i32,
    rune_items: i32,
    help_computer: Option<HudHelp>,
    red: i32,
    blue: i32,
    ctf: bool,
    pickup: Option<HudPickupState>,
    damage: Vec<HudDamageIndicator>,
    damage_picture: Option<HudDamagePictureState>,
    capture: String,
    capture_until_ms: f64,
}

impl SeatSourceHud {
    /// Create state without localization.
    #[must_use]
    pub fn new(actor: RemoteActorId) -> Self {
        Self::with_localize(actor, None::<fn(&ContentId, &str, &[String]) -> String>)
    }
}

impl<Localize> SeatSourceHud<Localize>
where
    Localize: FnMut(&ContentId, &str, &[String]) -> String,
{
    /// Create state with an optional localizer.
    pub fn with_localize(actor: RemoteActorId, localize: Option<Localize>) -> Self {
        Self {
            actor,
            localize,
            pending: Vec::new(),
            objective_prints: Vec::new(),
            objective: String::new(),
            mission_visible: false,
            bars: BTreeMap::new(),
            help_text: BTreeMap::new(),
            help_visible: false,
            inventory: None,
            scores_held: false,
            score_visible: false,
            score_rows: Vec::new(),
            clients: BTreeMap::new(),
            report: None,
            pois: Vec::new(),
            path: None,
            flags: 0,
            rune_items: 0,
            help_computer: None,
            red: 0,
            blue: 0,
            ctf: false,
            pickup: None,
            damage: Vec::new(),
            damage_picture: None,
            capture: String::new(),
            capture_until_ms: 0.0,
        }
    }

    /// Drain objective prints.
    pub fn drain_objective_prints(&mut self) -> Vec<SeatHudObjectivePrint> {
        std::mem::take(&mut self.objective_prints)
    }

    /// Current inventory rows.
    #[must_use]
    pub fn inventory_items(&self) -> Option<&[HudInventoryItem]> {
        self.inventory.as_deref()
    }

    /// Hold or release the scoreboard.
    pub fn scores(&mut self, down: bool) {
        self.scores_held = down;
    }

    /// Receive one source event, localizing at prepare time when needed.
    pub fn receive(&mut self, source: SeatHudSourceEvent) {
        if self.localize.is_some() && source.event.localizes() {
            self.pending.push(source);
        } else {
            self.apply(&source.event, &source.content, source.seconds);
        }
    }

    /// Localize pending events and load marker icons.
    pub fn prepare<Icons: SeatHudIcons>(&mut self, icons: &mut Icons) {
        for mut source in std::mem::take(&mut self.pending) {
            let localize = self.localize.as_mut().expect("pending implies a localizer");
            match &mut source.event {
                SeatHudEvent::Healthbar { name, .. } => {
                    *name = localize(&source.content, name, &[]);
                }
                SeatHudEvent::HelpComputer { primary, secondary, .. } => {
                    *primary = localize(&source.content, primary, &[]);
                    *secondary = localize(&source.content, secondary, &[]);
                }
                SeatHudEvent::MissionObjective { text, args, .. } => {
                    *text = localize(&source.content, text, args);
                    *args = Vec::new();
                }
                _ => {}
            }
            self.apply(&source.event, &source.content, source.seconds);
        }
        for poi in &mut self.pois {
            if poi.image.is_none() {
                let image = icons.load_image(&poi.content, &format!("pics/{}.pcx", poi.path));
                if let Some((width, height)) = icons.picture_size(&image) {
                    poi.width = width;
                    poi.height = height;
                }
                poi.image = Some(image);
                if let Some(colors) = icons.palette(&poi.content) {
                    let at = |index: usize| f32::from(colors.get(index).copied().unwrap_or(255)) / 255.0;
                    let base = usize::from(poi.color) * 3;
                    poi.tint = Vec4 {
                        x: at(base),
                        y: at(base + 1),
                        z: at(base + 2),
                        w: 1.0,
                    };
                }
            }
        }
        if let Some(picture) = self.damage_picture.as_mut() {
            if picture.image.is_none() {
                let image = icons.load_image(&picture.content, "pics/damage_indicator.pcx");
                if let Some((width, height)) = icons.picture_size(&image) {
                    picture.width = width;
                    picture.height = height;
                }
                picture.image = Some(image);
            }
        }
        if let Some(pickup) = self.pickup.as_mut() {
            if pickup.icon.is_none() && !pickup.path.is_empty() {
                pickup.icon = Some(icons.load_image(&pickup.content, &format!("pics/{}.pcx", pickup.path)));
            }
        }
    }

    /// Projected points of interest with loaded markers.
    #[must_use]
    pub fn points(&self) -> Vec<HudPointOfInterest> {
        self.pois
            .iter()
            .filter_map(|poi| {
                poi.image.clone().map(|image| HudPointOfInterest {
                    id: poi.key,
                    origin: poi.origin,
                    image,
                    width: poi.width,
                    height: poi.height,
                    color: poi.tint,
                    hide_on_aim: poi.flags & 1 != 0,
                    #[allow(clippy::cast_possible_truncation)]
                    expires_ms: poi.expires_ms as i64,
                })
            })
            .collect()
    }

    fn apply(&mut self, event: &SeatHudEvent, content: &ContentId, seconds: f64) {
        let now_ms = seconds * 1000.0;
        match event {
            SeatHudEvent::Q2Pickup { player, name, icon } if *player == self.actor => {
                self.pickup = Some(HudPickupState {
                    name: name.clone(),
                    path: icon.clone(),
                    content: content.clone(),
                    icon: None,
                    expires_ms: now_ms + 3000.0,
                });
            }
            SeatHudEvent::Q2DamageIndicator { actor, origin, amount } if *actor == self.actor => {
                self.damage.push(HudDamageIndicator::Origin {
                    origin: *origin,
                    amount: *amount,
                    #[allow(clippy::cast_possible_truncation)]
                    expires_ms: (now_ms + 800.0) as i64,
                });
                if self.damage.len() > MAX_ORIGIN_DAMAGE {
                    self.damage.remove(0);
                }
            }
            SeatHudEvent::Q2Help { slot, text } => {
                self.help_text.insert(*slot, text.clone());
            }
            SeatHudEvent::Q2PlayerInventory {
                actor,
                visible,
                entries,
                labels,
                selected,
            } if *actor == self.actor => {
                self.inventory = if !visible {
                    None
                } else {
                    Some(
                        entries
                            .iter()
                            .filter(|entry| entry.count > 0)
                            .map(|entry| {
                                let label = labels
                                    .iter()
                                    .find(|label| label.item == entry.item)
                                    .map(|label| label.name.clone())
                                    .unwrap_or_else(|| {
                                        entry.item.strip_prefix("q2:").unwrap_or(&entry.item).replace('_', " ")
                                    });
                                HudInventoryItem {
                                    id: entry.item.clone(),
                                    label,
                                    count: entry.count,
                                    selected: entry.item == *selected,
                                    binding: None,
                                    icon: None,
                                }
                            })
                            .collect(),
                    )
                };
                if self.inventory.is_some() {
                    self.help_visible = false;
                }
            }
            SeatHudEvent::Q2PlayerHelp { actor, visible } if *actor == self.actor => {
                self.help_visible = *visible;
                if self.help_visible {
                    self.inventory = None;
                    self.score_visible = false;
                }
            }
            SeatHudEvent::Q2PlayerView {
                actor,
                layouts,
                selected_item,
            } if *actor == self.actor => {
                if layouts & 2 == 0 {
                    self.inventory = None;
                } else if let Some(inventory) = self.inventory.as_mut() {
                    for item in inventory.iter_mut() {
                        item.selected = item.id == *selected_item;
                    }
                }
                if layouts & 1 == 0 {
                    self.help_visible = false;
                    self.score_visible = false;
                }
            }
            SeatHudEvent::Q2PlayerScoreboard { actor, rows } if *actor == self.actor => {
                self.score_rows = rows
                    .iter()
                    .map(|row| {
                        format!(
                            "{}  {}  {}ms  {}m{}",
                            row.score,
                            row.name,
                            row.ping,
                            row.minutes,
                            if row.spectator { "  Spectator" } else { "" }
                        )
                    })
                    .collect();
                self.score_visible = true;
                self.help_visible = false;
                self.inventory = None;
            }
            SeatHudEvent::MissionObjective { actor, text, .. } if *actor == self.actor => {
                self.objective = text.clone();
                self.objective_prints.push(SeatHudObjectivePrint {
                    text: text.clone(),
                    seconds,
                });
            }
            SeatHudEvent::MissionStatus { actor, icon_visible } if *actor == self.actor => {
                self.mission_visible = *icon_visible;
            }
            SeatHudEvent::Poi {
                actor,
                position,
                image,
                color,
                duration,
            } if *actor == self.actor => {
                self.store_poi(1, 1, *position, image, *color, *duration, content, now_ms);
            }
            SeatHudEvent::KeyedPoi {
                actor,
                key,
                flags,
                position,
                image,
                color,
                duration,
            } if *actor == self.actor => {
                self.store_poi(*key, *flags, *position, image, *color, *duration, content, now_ms);
            }
            SeatHudEvent::RemovePoi { actor, key } if *actor == self.actor && *key != 0 => {
                if let Some(index) = self.pois.iter().position(|poi| poi.key == *key) {
                    self.pois.remove(index);
                }
            }
            SeatHudEvent::DirectionalDamage {
                actor,
                direction,
                damage,
                health,
                armor,
                shield,
            } if *actor == self.actor => {
                let stale = self
                    .damage_picture
                    .as_ref()
                    .is_none_or(|picture| picture.content.as_str() != content.as_str());
                if stale {
                    self.damage_picture = Some(HudDamagePictureState {
                        content: content.clone(),
                        image: None,
                        width: 0.0,
                        height: 0.0,
                    });
                }
                let mut index = self.damage.iter().position(|value| {
                    damage_expires_ms(value) as f64 <= now_ms
                        || matches!(value, HudDamageIndicator::Directional { direction: prior, .. }
                            if dot3(*prior, *direction) >= 0.95)
                });
                if index.is_none() {
                    index = Some(if self.damage.len() < MAX_DIRECTIONAL_DAMAGE {
                        self.damage.len()
                    } else {
                        0
                    });
                }
                let index = index.expect("slot selected above");
                let retain = self.damage.get(index).filter(|previous| {
                    damage_expires_ms(previous) as f64 > now_ms
                        && matches!(previous, HudDamageIndicator::Directional { direction: prior, .. }
                            if dot3(*prior, *direction) >= 0.95)
                });
                let (retain_amount, retain_color, retain_health, retain_armor, retain_shield) = match retain {
                    Some(HudDamageIndicator::Directional {
                        amount,
                        color,
                        health,
                        armor,
                        shield,
                        ..
                    }) => (*amount, *color, *health, *armor, *shield),
                    _ => (0.0, Vec3 { x: 0.0, y: 0.0, z: 0.0 }, false, false, false),
                };
                let color = normalize3(Vec3 {
                    x: health + armor,
                    y: shield + armor,
                    z: *armor,
                });
                let entry = HudDamageIndicator::Directional {
                    picture: None,
                    direction: *direction,
                    amount: damage + retain_amount,
                    color: normalize3(Vec3 {
                        x: color.x + retain_color.x,
                        y: color.y + retain_color.y,
                        z: color.z + retain_color.z,
                    }),
                    health: *health != 0.0 || retain_health,
                    armor: *armor != 0.0 || retain_armor,
                    shield: *shield != 0.0 || retain_shield,
                    #[allow(clippy::cast_possible_truncation)]
                    expires_ms: (now_ms + 1000.0) as i64,
                };
                if index == self.damage.len() {
                    self.damage.push(entry);
                } else {
                    self.damage[index] = entry;
                }
            }
            SeatHudEvent::HelpPath {
                actor,
                position,
                direction,
            } if *actor == self.actor => {
                self.path = Some(HudHelpPathState {
                    origin: *position,
                    direction: *direction,
                    expires_ms: now_ms + 10000.0,
                });
            }
            SeatHudEvent::EndOfUnit { button_time, levels } => {
                let mut levels = levels.clone();
                levels.sort_by_key(|level| level.visit_order);
                self.report = Some(UnitReport {
                    ready_ms: button_time * 1000.0,
                    lines: levels
                        .iter()
                        .map(|level| {
                            let name = if level.name.is_empty() { &level.map } else { &level.name };
                            format!(
                                "{name}: {}/{} kills  {}/{} secrets  {}:{:02}",
                                level.killed_monsters,
                                level.total_monsters,
                                level.found_secrets,
                                level.total_secrets,
                                (level.time / 60.0).floor() as i64,
                                (level.time % 60.0).floor() as i64,
                            )
                        })
                        .collect(),
                });
            }
            SeatHudEvent::Healthbar {
                actor,
                slot,
                visible,
                name,
                fraction,
            } if *actor == self.actor => {
                if !visible {
                    self.bars.remove(slot);
                } else {
                    self.bars.insert(
                        *slot,
                        HudHealthBar {
                            id: format!("boss:{slot}"),
                            label: name.clone(),
                            value: *fraction,
                            maximum: 1.0,
                            color: Vec4 {
                                x: 0.8,
                                y: 0.12,
                                z: 0.08,
                                w: 1.0,
                            },
                        },
                    );
                }
            }
            SeatHudEvent::HelpComputer {
                actor,
                visible,
                primary,
                secondary,
            } if *actor == self.actor => {
                self.help_visible = *visible;
                if self.help_visible {
                    self.inventory = None;
                    self.score_visible = false;
                }
                self.help_computer = Some(HudHelp {
                    title: "Help computer".to_string(),
                    lines: [primary, secondary]
                        .iter()
                        .filter(|text| !text.is_empty())
                        .map(|text| (*text).clone())
                        .collect(),
                    objectives: Vec::new(),
                });
            }
            SeatHudEvent::Q1Client {
                slot,
                name,
                frags,
                team,
                observer,
            } => {
                self.clients.insert(
                    *slot,
                    Q1ClientRow {
                        name: name.clone(),
                        frags: *frags,
                        team: *team,
                        observer: *observer,
                    },
                );
            }
            SeatHudEvent::Q1ClientLeft { slot } => {
                self.clients.remove(slot);
            }
            SeatHudEvent::Q1CtfStatus {
                actor,
                red,
                blue,
                flags,
                rune_items,
            } if *actor == self.actor => {
                self.ctf = true;
                self.red = *red;
                self.blue = *blue;
                self.flags = *flags;
                self.rune_items = *rune_items;
            }
            SeatHudEvent::Q1CtfCapture { team, total } => {
                self.ctf = true;
                match team {
                    SeatHudCaptureTeam::Red => self.red = *total,
                    SeatHudCaptureTeam::Blue => self.blue = *total,
                }
                self.capture = format!(
                    "{} captured the flag",
                    match team {
                        SeatHudCaptureTeam::Red => "Red",
                        SeatHudCaptureTeam::Blue => "Blue",
                    }
                );
                self.capture_until_ms = now_ms + 3000.0;
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn store_poi(
        &mut self,
        key: i32,
        flags: i32,
        position: Vec3,
        image: &str,
        color: i32,
        duration: f64,
        content: &ContentId,
        now_ms: f64,
    ) {
        let mut index = if key == 0 {
            None
        } else {
            self.pois.iter().position(|poi| poi.key == key)
        };
        if index.is_none() {
            index = self.pois.iter().position(|poi| poi.expires_ms <= now_ms);
        }
        if index.is_none() && self.pois.len() < MAX_POIS {
            index = Some(self.pois.len());
        }
        if index.is_none() {
            let mut oldest = f64::INFINITY;
            for (candidate, poi) in self.pois.iter().enumerate() {
                if poi.key == 0 && poi.expires_ms < oldest {
                    oldest = poi.expires_ms;
                    index = Some(candidate);
                }
            }
        }
        if let Some(index) = index {
            let poi = HudPoi {
                key,
                width: 32.0,
                height: 32.0,
                flags,
                content: content.clone(),
                path: image.to_string(),
                image: None,
                origin: position,
                color: (color & 255) as u8,
                tint: Vec4 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    w: 1.0,
                },
                expires_ms: now_ms + duration,
            };
            if index == self.pois.len() {
                self.pois.push(poi);
            } else {
                self.pois[index] = poi;
            }
        }
    }

    /// Build one HUD frame, expiring transient rows.
    #[must_use]
    pub fn presentation(&self, now_ms: f64) -> SeatHudFrame {
        let mut prompts = Vec::new();
        if self.ctf {
            prompts.push(HudPrompt {
                action: format!("Red {} - Blue {}", self.red, self.blue),
                binding: String::new(),
                icon: None,
            });
        }
        if self.mission_visible && !self.objective.is_empty() {
            prompts.push(HudPrompt {
                action: "New objective".to_string(),
                binding: String::new(),
                icon: None,
            });
        }
        if self.ctf {
            prompts.push(HudPrompt {
                action: format!(
                    "Red flag {} - Blue flag {}",
                    flag_status(self.flags & 7),
                    flag_status((self.flags >> 3) & 7)
                ),
                binding: String::new(),
                icon: None,
            });
            for (bit, label) in [
                (32, "Resistance"),
                (64, "Strength"),
                (128, "Haste"),
                (256, "Regeneration"),
            ] {
                if self.rune_items & bit != 0 {
                    prompts.push(HudPrompt {
                        action: label.to_string(),
                        binding: String::new(),
                        icon: None,
                    });
                }
            }
        }
        if self.capture_until_ms > now_ms {
            prompts.push(HudPrompt {
                action: self.capture.clone(),
                binding: String::new(),
                icon: None,
            });
        }
        let scores = if self.score_visible {
            Some(self.score_rows.clone())
        } else if self.scores_held && !self.clients.is_empty() {
            let mut clients: Vec<&Q1ClientRow> = self.clients.values().collect();
            clients.sort_by_key(|client| std::cmp::Reverse(client.frags));
            Some(
                clients
                    .iter()
                    .map(|client| {
                        format!(
                            "{}  {}{}{}",
                            client.frags,
                            client.name,
                            if client.team == 0 {
                                String::new()
                            } else {
                                format!("  Team {}", client.team)
                            },
                            if client.observer { "  Spectator" } else { "" }
                        )
                    })
                    .collect(),
            )
        } else {
            None
        };
        let help = if let Some(report) = &self.report {
            Some(HudHelp {
                title: "Unit complete".to_string(),
                lines: report.lines.clone(),
                objectives: vec![HudObjective {
                    text: if now_ms >= report.ready_ms {
                        "Press attack to continue".to_string()
                    } else {
                        String::new()
                    },
                    complete: false,
                }],
            })
        } else if let Some(scores) = scores {
            Some(HudHelp {
                title: "Scores".to_string(),
                lines: scores,
                objectives: Vec::new(),
            })
        } else if self.help_visible {
            Some(self.help_computer.clone().unwrap_or(HudHelp {
                title: "Help computer".to_string(),
                lines: self.help_text.values().cloned().collect(),
                objectives: Vec::new(),
            }))
        } else {
            None
        };
        SeatHudFrame {
            health_bars: self.bars.values().cloned().collect(),
            help,
            prompts,
            inventory: self.inventory.clone(),
            pickup: self.pickup.as_ref().map(|pickup| HudPickup {
                name: pickup.name.clone(),
                icon: pickup.icon.clone(),
                #[allow(clippy::cast_possible_truncation)]
                expires_ms: pickup.expires_ms as i64,
            }),
            damage_indicators: self
                .damage
                .iter()
                .filter(|damage| damage_expires_ms(damage) as f64 > now_ms)
                .map(|damage| match damage {
                    HudDamageIndicator::Directional {
                        direction,
                        amount,
                        color,
                        health,
                        armor,
                        shield,
                        expires_ms,
                        ..
                    } => {
                        let picture = match self.damage_picture.as_ref() {
                            Some(loaded) if loaded.image.is_some() && loaded.width > 0.0 => {
                                loaded.image.clone().map(|image| HudDamagePicture {
                                    image,
                                    width: loaded.width,
                                    height: loaded.height,
                                })
                            }
                            _ => None,
                        };
                        HudDamageIndicator::Directional {
                            picture,
                            direction: *direction,
                            amount: *amount,
                            color: *color,
                            health: *health,
                            armor: *armor,
                            shield: *shield,
                            expires_ms: *expires_ms,
                        }
                    }
                    other => other.clone(),
                })
                .collect(),
            help_path: self
                .path
                .as_ref()
                .filter(|path| path.expires_ms > now_ms)
                .map(|path| HudHelpPath {
                    origin: path.origin,
                    direction: path.direction,
                }),
        }
    }
}

fn damage_expires_ms(damage: &HudDamageIndicator) -> i64 {
    match damage {
        HudDamageIndicator::Origin { expires_ms, .. } | HudDamageIndicator::Directional { expires_ms, .. } => {
            *expires_ms
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn actor() -> RemoteActorId {
        RemoteActorId {
            session: 1,
            slot: 2,
            generation: 3,
        }
    }

    fn other() -> RemoteActorId {
        RemoteActorId {
            session: 9,
            slot: 9,
            generation: 9,
        }
    }

    fn content() -> ContentId {
        ContentId::new("q2")
    }

    fn sourced(event: SeatHudEvent, seconds: f64) -> SeatHudSourceEvent {
        SeatHudSourceEvent {
            content: content(),
            seconds,
            event,
        }
    }

    fn origin() -> Vec3 {
        Vec3 { x: 1.0, y: 2.0, z: 3.0 }
    }

    struct FakeIcons {
        loaded: Vec<String>,
        palettes: HashMap<String, Vec<u8>>,
    }

    impl SeatHudIcons for FakeIcons {
        fn load_image(&mut self, content: &ContentId, path: &str) -> ResourceId {
            self.loaded.push(format!("{}:{path}", content.as_str()));
            ResourceId::new(&format!("resource:{}", self.loaded.len())).unwrap()
        }

        fn picture_size(&self, image: &ResourceId) -> Option<(f32, f32)> {
            let _ = image;
            Some((64.0, 32.0))
        }

        fn palette(&mut self, content: &ContentId) -> Option<Vec<u8>> {
            self.palettes.get(content.as_str()).cloned()
        }
    }

    #[test]
    fn pickup_damage_and_help_fold() {
        let mut hud = SeatSourceHud::new(actor());
        hud.receive(sourced(
            SeatHudEvent::Q2Pickup {
                player: actor(),
                name: "Shells".to_string(),
                icon: "shells".to_string(),
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Q2DamageIndicator {
                actor: actor(),
                origin: origin(),
                amount: 10.0,
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Q2Help {
                slot: 2,
                text: "Duck".to_string(),
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Q2Help {
                slot: 1,
                text: "Run".to_string(),
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Q2Pickup {
                player: other(),
                name: "Ignored".to_string(),
                icon: "x".to_string(),
            },
            1.0,
        ));
        let frame = hud.presentation(1500.0);
        let pickup = frame.pickup.unwrap();
        assert_eq!(pickup.name, "Shells");
        assert_eq!(frame.damage_indicators.len(), 1);
        // Help text sorts by slot when the help panel shows.
        hud.receive(sourced(
            SeatHudEvent::Q2PlayerHelp {
                actor: actor(),
                visible: true,
            },
            1.0,
        ));
        let frame = hud.presentation(1500.0);
        let help = frame.help.unwrap();
        assert_eq!(help.lines, vec!["Run".to_string(), "Duck".to_string()]);
        assert!(hud.presentation(5000.0).damage_indicators.is_empty());
    }

    #[test]
    fn inventory_scoreboard_and_view_interact() {
        let mut hud = SeatSourceHud::new(actor());
        hud.receive(sourced(
            SeatHudEvent::Q2PlayerInventory {
                actor: actor(),
                visible: true,
                entries: vec![
                    SeatHudInventoryEntry {
                        item: "q2:shotgun".to_string(),
                        count: 1,
                    },
                    SeatHudInventoryEntry {
                        item: "q2:super_shotgun".to_string(),
                        count: 0,
                    },
                ],
                labels: vec![SeatHudInventoryLabel {
                    item: "q2:shotgun".to_string(),
                    name: "Shotgun".to_string(),
                }],
                selected: "q2:shotgun".to_string(),
            },
            1.0,
        ));
        let items = hud.inventory_items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Shotgun");
        assert!(items[0].selected);
        hud.receive(sourced(
            SeatHudEvent::Q2PlayerView {
                actor: actor(),
                layouts: 3,
                selected_item: "q2:shotgun".to_string(),
            },
            1.0,
        ));
        assert!(hud.inventory_items().is_some());
        hud.receive(sourced(
            SeatHudEvent::Q2PlayerView {
                actor: actor(),
                layouts: 0,
                selected_item: String::new(),
            },
            1.0,
        ));
        assert!(hud.inventory_items().is_none());
        hud.receive(sourced(
            SeatHudEvent::Q2PlayerScoreboard {
                actor: actor(),
                rows: vec![SeatHudScoreRow {
                    score: 10,
                    name: "Player".to_string(),
                    ping: 42,
                    minutes: 3,
                    spectator: true,
                }],
            },
            1.0,
        ));
        let frame = hud.presentation(1500.0);
        let help = frame.help.unwrap();
        assert_eq!(help.title, "Scores");
        assert_eq!(help.lines, vec!["10  Player  42ms  3m  Spectator".to_string()]);
    }

    #[test]
    fn objectives_pois_and_paths_fold() {
        let mut hud = SeatSourceHud::new(actor());
        hud.receive(sourced(
            SeatHudEvent::MissionObjective {
                actor: actor(),
                text: "Find exit".to_string(),
                args: Vec::new(),
            },
            2.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::MissionStatus {
                actor: actor(),
                icon_visible: true,
            },
            2.0,
        ));
        assert_eq!(hud.drain_objective_prints().len(), 1);
        assert!(hud.drain_objective_prints().is_empty());
        hud.receive(sourced(
            SeatHudEvent::KeyedPoi {
                actor: actor(),
                key: 5,
                flags: 0,
                position: origin(),
                image: "exit".to_string(),
                color: 300,
                duration: 5000.0,
            },
            2.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::HelpPath {
                actor: actor(),
                position: origin(),
                direction: origin(),
            },
            2.0,
        ));
        let mut icons = FakeIcons {
            loaded: Vec::new(),
            palettes: HashMap::new(),
        };
        hud.prepare(&mut icons);
        assert_eq!(icons.loaded, vec!["q2:pics/exit.pcx".to_string()]);
        let points = hud.points();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].id, 5);
        assert_eq!((points[0].width, points[0].height), (64.0, 32.0));
        let frame = hud.presentation(2500.0);
        assert!(frame.help_path.is_some());
        assert_eq!(frame.prompts.len(), 1);
        assert_eq!(frame.prompts[0].action, "New objective");
        hud.receive(sourced(SeatHudEvent::RemovePoi { actor: actor(), key: 5 }, 3.0));
        assert!(hud.points().is_empty());
    }

    #[test]
    fn directional_damage_merges_and_pictures() {
        let mut hud = SeatSourceHud::new(actor());
        let direction = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
        hud.receive(sourced(
            SeatHudEvent::DirectionalDamage {
                actor: actor(),
                direction,
                damage: 10.0,
                health: 5.0,
                armor: 0.0,
                shield: 0.0,
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::DirectionalDamage {
                actor: actor(),
                direction,
                damage: 4.0,
                health: 0.0,
                armor: 2.0,
                shield: 0.0,
            },
            1.1,
        ));
        let mut icons = FakeIcons {
            loaded: Vec::new(),
            palettes: HashMap::new(),
        };
        hud.prepare(&mut icons);
        assert_eq!(icons.loaded, vec!["q2:pics/damage_indicator.pcx".to_string()]);
        let frame = hud.presentation(1500.0);
        assert_eq!(frame.damage_indicators.len(), 1);
        match &frame.damage_indicators[0] {
            HudDamageIndicator::Directional {
                amount,
                health,
                armor,
                picture,
                ..
            } => {
                assert_eq!(*amount, 14.0);
                assert!(*health);
                assert!(*armor);
                assert!(picture.is_some());
            }
            other => panic!("expected directional, got {other:?}"),
        }
    }

    #[test]
    fn unit_report_healthbar_and_help_computer_fold() {
        let mut hud = SeatSourceHud::new(actor());
        hud.receive(sourced(
            SeatHudEvent::EndOfUnit {
                button_time: 2.0,
                levels: vec![SeatHudUnitLevel {
                    name: String::new(),
                    map: "base1".to_string(),
                    visit_order: 0,
                    killed_monsters: 5,
                    total_monsters: 8,
                    found_secrets: 1,
                    total_secrets: 2,
                    time: 125.0,
                }],
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Healthbar {
                actor: actor(),
                slot: 1,
                visible: true,
                name: "Boss".to_string(),
                fraction: 0.5,
            },
            1.0,
        ));
        let frame = hud.presentation(1500.0);
        let help = frame.help.unwrap();
        assert_eq!(help.title, "Unit complete");
        assert_eq!(help.lines, vec!["base1: 5/8 kills  1/2 secrets  2:05".to_string()]);
        assert_eq!(frame.health_bars.len(), 1);
        assert_eq!(frame.health_bars[0].id, "boss:1");
        hud.receive(sourced(
            SeatHudEvent::HelpComputer {
                actor: actor(),
                visible: true,
                primary: "Primary".to_string(),
                secondary: String::new(),
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Healthbar {
                actor: actor(),
                slot: 1,
                visible: false,
                name: String::new(),
                fraction: 0.0,
            },
            1.0,
        ));
        let frame = hud.presentation(2500.0);
        assert!(frame.health_bars.is_empty());
        assert_eq!(frame.help.unwrap().objectives[0].text, "Press attack to continue");
    }

    #[test]
    fn q1_clients_and_ctf_fold() {
        let mut hud = SeatSourceHud::new(actor());
        hud.receive(sourced(
            SeatHudEvent::Q1Client {
                slot: 0,
                name: "Alice".to_string(),
                frags: 3,
                team: 1,
                observer: false,
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Q1Client {
                slot: 1,
                name: "Bob".to_string(),
                frags: 7,
                team: 0,
                observer: true,
            },
            1.0,
        ));
        hud.scores(true);
        let frame = hud.presentation(1500.0);
        let help = frame.help.unwrap();
        assert_eq!(
            help.lines,
            vec!["7  Bob  Spectator".to_string(), "3  Alice  Team 1".to_string()]
        );
        hud.receive(sourced(SeatHudEvent::Q1ClientLeft { slot: 1 }, 1.0));
        hud.receive(sourced(
            SeatHudEvent::Q1CtfStatus {
                actor: actor(),
                red: 2,
                blue: 3,
                flags: 2 | (4 << 3),
                rune_items: 64,
            },
            1.0,
        ));
        hud.receive(sourced(
            SeatHudEvent::Q1CtfCapture {
                team: SeatHudCaptureTeam::Red,
                total: 3,
            },
            1.0,
        ));
        let frame = hud.presentation(1500.0);
        let actions: Vec<&str> = frame.prompts.iter().map(|prompt| prompt.action.as_str()).collect();
        assert_eq!(
            actions,
            vec![
                "Red 3 - Blue 3",
                "Red flag carried - Blue flag dropped",
                "Strength",
                "Red captured the flag"
            ]
        );
    }

    #[test]
    fn localization_defers_until_prepare() {
        let mut hud = SeatSourceHud::with_localize(
            actor(),
            Some(|_: &ContentId, text: &str, _: &[String]| format!("L:{text}")),
        );
        hud.receive(sourced(
            SeatHudEvent::Healthbar {
                actor: actor(),
                slot: 1,
                visible: true,
                name: "Boss".to_string(),
                fraction: 1.0,
            },
            1.0,
        ));
        assert!(hud.presentation(1500.0).health_bars.is_empty());
        let mut icons = FakeIcons {
            loaded: Vec::new(),
            palettes: HashMap::new(),
        };
        hud.prepare(&mut icons);
        assert_eq!(hud.presentation(1500.0).health_bars[0].label, "L:Boss");
    }
}
