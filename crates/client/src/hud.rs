//! HUD state: stat blocks, inventory display data, centerprint, layouts.
//!
//! Donor provenance: `src/contracts/ui.ts` (`CenterPrintState`),
//! `src/content/q3/base/shared/definitions.ts` (`statSchema`,
//! `ARMOR_PROTECTION`), `src/network/q2/state.ts` (`MAX_STATS`,
//! `MAX_STATS_STORAGE`), `src/app/bootstrap/network/q2-layout.ts`
//! (`q2ApplicationLayout`), `src/ui/hud/q2-native.ts`
//! (`q2LayoutOperations`, `nativeQ2HudStat`),
//! `src/content/q3/presentation/draw-tools.ts` (`getColorForHealth`,
//! `fadeColor`), `src/core/game-numeric.ts` (`gameAtoi`) and
//! `src/core/common-parse.ts` (`Tokenizer`).

use std::collections::BTreeMap;

use qa_core::math::{vec4, Vec4};

use crate::ClientError;

/// Q2 wire stat count (`MAX_STATS`).
pub const Q2_MAX_STATS: usize = 32;
/// Q2 stat storage slots (`MAX_STATS_STORAGE`).
pub const Q2_STATS_STORAGE: usize = 64;
/// Q3 player-state stat slots (`PlayerStateSlots(16)`).
pub const Q3_MAX_STATS: usize = 16;

/// Classic Q2 configstring layout (`q2ApplicationLayout`, non-rerelease).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ClassicLayout {
    /// First image configstring.
    pub images: i32,
    /// Image slots.
    pub max_images: i32,
    /// Configstring slots.
    pub max_configstrings: i32,
    /// First player-skin configstring.
    pub player_skins: i32,
    /// Max-clients configstring index.
    pub max_clients: i32,
}

/// Classic values: images 544/256, 2080 strings, skins 1312, clients 30.
pub const Q2_CLASSIC_LAYOUT: Q2ClassicLayout = Q2ClassicLayout {
    images: 544,
    max_images: 256,
    max_configstrings: 2080,
    player_skins: 1312,
    max_clients: 30,
};

/// Q2 stat block: 64 storage slots, 32 on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2StatBlock {
    stats: [i16; Q2_STATS_STORAGE],
}

impl Q2StatBlock {
    /// Zeroed block.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stats: [0; Q2_STATS_STORAGE],
        }
    }

    /// Bounds-checked read (`readElement`).
    pub fn get(&self, index: i32) -> Result<i16, ClientError> {
        if index < 0 || index >= Q2_STATS_STORAGE as i32 {
            return Err(ClientError::BadHudStat { index });
        }
        Ok(self.stats[index as usize])
    }

    /// Bounds-checked write.
    pub fn set(&mut self, index: i32, value: i16) -> Result<(), ClientError> {
        if index < 0 || index >= Q2_STATS_STORAGE as i32 {
            return Err(ClientError::BadHudStat { index });
        }
        self.stats[index as usize] = value;
        Ok(())
    }

    /// Wire-visible prefix.
    #[must_use]
    pub fn wire(&self) -> &[i16] {
        &self.stats[..Q2_MAX_STATS]
    }
}

impl Default for Q2StatBlock {
    fn default() -> Self {
        Self::new()
    }
}

/// Q3 product selecting the stat schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Product {
    /// Base Quake III.
    Base,
    /// Missionpack (inserts persistent powerup at slot 2).
    Missionpack,
}

/// Q3 stat schema indices (`statSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3StatSchema {
    /// Health slot.
    pub health: usize,
    /// Holdable-item slot.
    pub holdable_item: usize,
    /// Persistent-powerup slot (missionpack only).
    pub persistent_powerup: Option<usize>,
    /// Weapons bitmask slot.
    pub weapons: usize,
    /// Armor slot.
    pub armor: usize,
    /// Dead-yaw slot.
    pub dead_yaw: usize,
    /// Clients-ready slot.
    pub clients_ready: usize,
    /// Max-health slot.
    pub max_health: usize,
}

/// Schema for a product.
#[must_use]
pub const fn q3_stat_schema(product: Q3Product) -> Q3StatSchema {
    match product {
        Q3Product::Base => Q3StatSchema {
            health: 0,
            holdable_item: 1,
            persistent_powerup: None,
            weapons: 2,
            armor: 3,
            dead_yaw: 4,
            clients_ready: 5,
            max_health: 6,
        },
        Q3Product::Missionpack => Q3StatSchema {
            health: 0,
            holdable_item: 1,
            persistent_powerup: Some(2),
            weapons: 3,
            armor: 4,
            dead_yaw: 5,
            clients_ready: 6,
            max_health: 7,
        },
    }
}

/// Q3 16-slot stat block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3StatBlock {
    stats: [i32; Q3_MAX_STATS],
    /// Schema product.
    pub product: Q3Product,
}

impl Q3StatBlock {
    /// Zeroed block for a product.
    #[must_use]
    pub const fn new(product: Q3Product) -> Self {
        Self {
            stats: [0; Q3_MAX_STATS],
            product,
        }
    }

    /// Bounds-checked read.
    pub fn get(&self, index: i32) -> Result<i32, ClientError> {
        if index < 0 || index >= Q3_MAX_STATS as i32 {
            return Err(ClientError::BadHudStat { index });
        }
        Ok(self.stats[index as usize])
    }

    /// Bounds-checked write.
    pub fn set(&mut self, index: i32, value: i32) -> Result<(), ClientError> {
        if index < 0 || index >= Q3_MAX_STATS as i32 {
            return Err(ClientError::BadHudStat { index });
        }
        self.stats[index as usize] = value;
        Ok(())
    }

    /// Schema-aware health.
    #[must_use]
    pub fn health(&self) -> i32 {
        self.stats[q3_stat_schema(self.product).health]
    }

    /// Schema-aware armor.
    #[must_use]
    pub fn armor(&self) -> i32 {
        self.stats[q3_stat_schema(self.product).armor]
    }

    /// Schema-aware weapons bitmask.
    #[must_use]
    pub fn weapons(&self) -> i32 {
        self.stats[q3_stat_schema(self.product).weapons]
    }
}

/// Q1 stat table (`svc_updatestat`).
///
/// The donor types the stat index as a byte and the value as a
/// long (NetQuake, QuakeWorld `updatestatlong`) or byte (QuakeWorld
/// `updatestat`), with no fixed count; this stores decoded values
/// sparsely by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Q1StatTable {
    stats: BTreeMap<u8, i32>,
}

impl Q1StatTable {
    /// Empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self { stats: BTreeMap::new() }
    }

    /// Apply a stat update; unknown stats read as zero.
    pub fn update(&mut self, index: u8, value: i32) {
        self.stats.insert(index, value);
    }

    /// Read a stat (zero when never set).
    #[must_use]
    pub fn get(&self, index: u8) -> i32 {
        self.stats.get(&index).copied().unwrap_or(0)
    }
}

/// Centerprint state (`CenterPrintState` contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CenterPrint {
    /// Full text.
    pub text: String,
    /// Start time in milliseconds.
    pub start_ms: i64,
    /// Display duration in milliseconds.
    pub duration_ms: i64,
    /// Instant prints ignore character crawl.
    pub instant: bool,
    /// Milliseconds per revealed character (teletype crawl).
    pub char_ms: Option<u32>,
}

impl CenterPrint {
    /// Visible text at `now_ms`, or [`None`] when expired.
    #[must_use]
    pub fn visible_text(&self, now_ms: i64) -> Option<&str> {
        let elapsed = now_ms - self.start_ms;
        if elapsed < 0 || elapsed >= self.duration_ms {
            return None;
        }
        let Some(char_ms) = self.char_ms.filter(|_| !self.instant) else {
            return Some(&self.text);
        };
        if char_ms == 0 {
            return Some(&self.text);
        }
        let chars = (elapsed as u64 / u64::from(char_ms)) as usize;
        let end = self
            .text
            .char_indices()
            .map(|(index, _)| index)
            .nth(chars)
            .unwrap_or(self.text.len());
        Some(&self.text[..end])
    }
}

/// Inventory display data (selected item, counts, ammunition).
///
/// Headless labels/icons resolve through content services; this owns
/// selection and counts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudInventory {
    /// Per-item counts by item index.
    pub counts: Vec<i32>,
    /// Selected item index.
    pub selected: Option<usize>,
    /// Ammunition count for the selected weapon (Q2 `stat(3)` override).
    pub ammo_count: Option<i32>,
}

impl HudInventory {
    /// Empty inventory.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counts: Vec::new(),
            selected: None,
            ammo_count: None,
        }
    }

    /// Count for an item (zero when outside the table).
    #[must_use]
    pub fn count(&self, item: usize) -> i32 {
        self.counts.get(item).copied().unwrap_or(0)
    }

    /// Select an item; out-of-range selections clear.
    pub fn select(&mut self, item: usize) {
        self.selected = if item < self.counts.len() { Some(item) } else { None };
    }

    /// Step selection to the next nonzero item, wrapping around.
    pub fn select_next(&mut self) {
        self.step(1);
    }

    /// Step selection to the previous nonzero item, wrapping around.
    pub fn select_previous(&mut self) {
        self.step(-1);
    }

    fn step(&mut self, direction: i32) {
        if self.counts.is_empty() {
            self.selected = None;
            return;
        }
        let len = self.counts.len() as i32;
        let mut cursor = self.selected.map_or(0, |selected| selected as i32);
        for _ in 0..len {
            cursor = (cursor + direction).rem_euclid(len);
            if self.counts[cursor as usize] != 0 {
                self.selected = Some(cursor as usize);
                return;
            }
        }
    }
}

impl Default for HudInventory {
    fn default() -> Self {
        Self::new()
    }
}

/// Armor protection fraction (`ARMOR_PROTECTION` in `bg_public`).
pub const ARMOR_PROTECTION: f64 = 0.66;

/// Q3 health color (`CG_ColorForHealth` / `getColorForHealth`).
///
/// Dead players are black; effective health folds armor in up to the
/// protection cap, then ramps red (0) through yellow to white (100+).
#[must_use]
pub fn health_color(health: i32, armor: i32) -> Vec4 {
    if health <= 0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let maximum = (f64::from(health) * ARMOR_PROTECTION / (1.0 - ARMOR_PROTECTION)) as i32;
    let health = health + armor.min(maximum);
    vec4(
        1.0,
        if health > 60 {
            1.0
        } else if health < 30 {
            0.0
        } else {
            (health - 30) as f32 / 30.0
        },
        if health >= 100 {
            1.0
        } else if health < 66 {
            0.0
        } else {
            (health - 66) as f32 / 33.0
        },
        1.0,
    )
}

/// `CG_FadeColor` alpha: full until the last 200 ms, then linear.
///
/// Zero starts and elapsed windows return [`None`].
#[must_use]
pub fn fade_alpha(now_ms: i32, start_ms: i32, total_ms: i32) -> Option<f32> {
    let elapsed = now_ms.wrapping_sub(start_ms);
    if start_ms == 0 || elapsed >= total_ms {
        return None;
    }
    let remaining = total_ms - elapsed;
    Some(if remaining < 200 { remaining as f32 / 200.0 } else { 1.0 })
}

/// Quake `atoi` (`bg_lib.c`): skips `<= 32` bytes, optional sign,
/// wrapping decimal accumulation, stops at the first non-digit.
#[must_use]
pub fn game_atoi(text: &str) -> i32 {
    fn byte(bytes: &[u8], offset: usize) -> i32 {
        if offset >= bytes.len() {
            return 0;
        }
        let byte = bytes[offset];
        if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        }
    }
    let bytes = text.as_bytes();
    let mut offset = 0;
    while byte(bytes, offset) <= 32 && byte(bytes, offset) != 0 {
        offset += 1;
    }
    if byte(bytes, offset) == 0 {
        return 0;
    }
    let mut sign = 1;
    let head = byte(bytes, offset);
    if head == 43 || head == 45 {
        offset += 1;
        if head == 45 {
            sign = -1;
        }
    }
    let mut value = 0i32;
    loop {
        let byte = byte(bytes, offset);
        offset += 1;
        if !(48..=57).contains(&byte) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(byte - 48);
    }
    value.wrapping_mul(sign)
}

/// Maximum layout token length (`TOKEN_MAX`).
pub const TOKEN_MAX: usize = 1024;

/// Split a layout string into tokens (`Tokenizer`).
///
/// Whitespace (code `<= 32`) separates; `//` and `/* */` comments are
/// skipped; double quotes group without escapes.
#[must_use]
pub fn layout_tokens(source: &str) -> Vec<String> {
    fn is_space(byte: u8) -> bool {
        byte <= 32
    }
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        while offset < bytes.len() && is_space(bytes[offset]) {
            offset += 1;
        }
        if bytes[offset..].starts_with(b"//") {
            while offset < bytes.len() && bytes[offset] != b'\n' && bytes[offset] != b'\r' {
                offset += 1;
            }
            continue;
        }
        if bytes[offset..].starts_with(b"/*") {
            offset += 2;
            while offset < bytes.len() && !bytes[offset..].starts_with(b"*/") {
                offset += 1;
            }
            offset = (offset + 2).min(bytes.len());
            continue;
        }
        if offset >= bytes.len() {
            break;
        }
        let mut token = Vec::new();
        if bytes[offset] == b'"' {
            offset += 1;
            while offset < bytes.len() && bytes[offset] != b'"' {
                token.push(bytes[offset]);
                offset += 1;
            }
            offset = (offset + 1).min(bytes.len());
        } else {
            while offset < bytes.len() && !is_space(bytes[offset]) {
                token.push(bytes[offset]);
                offset += 1;
            }
        }
        if token.len() < TOKEN_MAX {
            tokens.push(String::from_utf8_lossy(&token).into_owned());
        }
    }
    tokens
}

/// Classic Q2 HUD frame: public playerstate and received data only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2HudFrame {
    /// Player stats.
    pub stats: Vec<i16>,
    /// Received configstrings by index.
    pub configstrings: BTreeMap<i32, String>,
    /// Zero-based local player number.
    pub player_number: i32,
    /// Current server frame (drives warning flash).
    pub server_frame: i32,
    /// Ammunition readout override (`arsenal.ammunition`).
    pub ammo: Option<AmmoReadout>,
}

/// Ammunition count readout; a missing count reads as empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmmoReadout {
    /// Shells for the selected weapon, if known.
    pub count: Option<i32>,
}

/// Classic Q2 HUD draw operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2HudOp {
    /// Text line.
    Text {
        /// Origin.
        x: i32,
        /// Origin.
        y: i32,
        /// Text.
        text: String,
        /// Alternate (green) font.
        alternate: bool,
    },
    /// Named picture.
    Picture {
        /// Origin.
        x: i32,
        /// Origin.
        y: i32,
        /// Picture name.
        name: String,
    },
}

fn hud_stat(frame: &Q2HudFrame, index: i32) -> Result<i32, ClientError> {
    if index < 0 || index >= frame.stats.len() as i32 {
        return Err(ClientError::BadHudStat { index });
    }
    if let Some(ammo) = frame.ammo {
        if index == 2 {
            return Ok(i32::from(ammo.count.is_some()));
        }
        if index == 3 {
            return Ok(ammo.count.unwrap_or(-1));
        }
    }
    Ok(i32::from(frame.stats[index as usize]))
}

/// Classic Q2 layout-grammar environment: stat/config/client resolution plus
/// thin op emitters.
///
/// The headless [`q2_layout_ops`] interpreter and the native HUD classic path
/// share the grammar core ([`run_classic_layout`]); each keeps its own stat
/// sources, config tables, client-limit parsing, cstring width, error
/// taxonomy, and op types behind this trait.
pub(crate) trait ClassicLayoutEnv {
    /// Caller error type.
    type Error;
    /// Read one HUD stat.
    fn stat(&self, index: i32) -> Result<i32, Self::Error>;
    /// Resolve a `pic` image index to its picture name (range-checked).
    fn image_name(&self, image: i32) -> Result<String, Self::Error>;
    /// Resolve a `stat_string` index to its configstring (range-checked).
    fn stat_string(&self, index: i32) -> Result<String, Self::Error>;
    /// Resolve a client slot to its scoreboard name and icon picture.
    fn client(&self, index: i32) -> Result<(String, String), Self::Error>;
    /// Server frame driving warning flash.
    fn server_frame(&self) -> i32;
    /// Zero-based local player number for `ctf` highlight.
    fn player_number(&self) -> i32;
    /// Centered-string length unit: bytes here, UTF-16 units natively.
    fn cstring_length(&self, line: &str) -> i32;
    /// Emit an arsenal-backed `pic` (native `pic 2/6`); `Ok(false)` runs the
    /// normal image path.
    fn try_arsenal_pic(&mut self, x: i32, y: i32, index: i32) -> Result<bool, Self::Error>;
    /// Emit one text op.
    fn emit_text(&mut self, x: i32, y: i32, text: String, alternate: bool);
    /// Emit one named-picture op.
    fn emit_picture(&mut self, x: i32, y: i32, name: String);
}

/// Right-aligned digit pictures (`SCR_DrawField`) through an environment.
fn draw_layout_field<E: ClassicLayoutEnv>(env: &mut E, value: i32, digits: i32, alternate: bool, x: i32, y: i32) {
    let count = digits.min(5);
    if count < 1 {
        return;
    }
    let digits_text = value.to_string();
    let length = digits_text.len().min(count as usize);
    let mut draw_x = x + 2 + 16 * (count - length as i32);
    for digit in digits_text.chars().take(length) {
        let name = format!(
            "{}_{}",
            if alternate { "anum" } else { "num" },
            if digit == '-' {
                "minus".to_string()
            } else {
                digit.to_string()
            }
        );
        env.emit_picture(draw_x, y, name);
        draw_x += 16;
    }
}

/// Shared classic Q2 layout grammar (`SCR_ExecuteLayoutString`) over tokens.
///
/// Supports cursor (`xl/xr/xv/yt/yb/yv`), `pic/picn`, `num`,
/// `hnum/anum/rnum`, `stat_string`, `string/string2`,
/// `cstring/cstring2`, `client/ctf`, and flat `if/endif` skip.
/// Unknown words are ignored, matching the source switch. Each caller
/// tokenizes its own way (long-token drop versus error included) and feeds
/// the token values here; missing arguments read as empty/`0`.
pub(crate) fn run_classic_layout<E: ClassicLayoutEnv>(
    tokens: &[String],
    width: i32,
    height: i32,
    env: &mut E,
) -> Result<(), E::Error> {
    struct Cursor<'a> {
        tokens: &'a [String],
        index: usize,
    }
    impl Cursor<'_> {
        fn next(&mut self) -> String {
            if self.index >= self.tokens.len() {
                return String::new();
            }
            let token = self.tokens[self.index].clone();
            self.index += 1;
            token
        }
        fn integer(&mut self) -> i32 {
            game_atoi(&self.next())
        }
    }
    let mut cursor = Cursor { tokens, index: 0 };
    let mut x = 0;
    let mut y = 0;
    while cursor.index < cursor.tokens.len() {
        let word = cursor.next();
        if word == "if" && env.stat(cursor.integer())? == 0 {
            while cursor.index < cursor.tokens.len() && cursor.next() != "endif" {}
            continue;
        }
        match word.as_str() {
            "xl" => x = cursor.integer(),
            "xr" => x = width + cursor.integer(),
            "xv" => x = width / 2 - 160 + cursor.integer(),
            "yt" => y = cursor.integer(),
            "yb" => y = height + cursor.integer(),
            "yv" => y = height / 2 - 120 + cursor.integer(),
            "pic" => {
                let index = cursor.integer();
                if env.try_arsenal_pic(x, y, index)? {
                    continue;
                }
                let image = env.stat(index)?;
                let name = env.image_name(image)?;
                if !name.is_empty() {
                    env.emit_picture(x, y, name);
                }
            }
            "picn" => {
                let name = cursor.next();
                if !name.is_empty() {
                    env.emit_picture(x, y, name);
                }
            }
            "num" => {
                let digits = cursor.integer();
                let value = env.stat(cursor.integer())?;
                draw_layout_field(env, value, digits, false, x, y);
            }
            "hnum" => {
                let value = env.stat(1)?;
                if env.stat(15)? & 1 != 0 {
                    env.emit_picture(x, y, "field_3".to_string());
                }
                let flash = (env.server_frame() >> 2) & 1 != 0;
                draw_layout_field(env, value, 3, value <= 0 || value <= 25 && flash, x, y);
            }
            "anum" => {
                let value = env.stat(3)?;
                if value >= 0 {
                    if env.stat(15)? & 4 != 0 {
                        env.emit_picture(x, y, "field_3".to_string());
                    }
                    let flash = (env.server_frame() >> 2) & 1 != 0;
                    draw_layout_field(env, value, 3, value <= 5 && flash, x, y);
                }
            }
            "rnum" => {
                let value = env.stat(5)?;
                if value >= 1 {
                    if env.stat(15)? & 2 != 0 {
                        env.emit_picture(x, y, "field_3".to_string());
                    }
                    draw_layout_field(env, value, 3, false, x, y);
                }
            }
            "stat_string" => {
                let index = env.stat(cursor.integer())?;
                let value = env.stat_string(index)?;
                env.emit_text(x, y, value, false);
            }
            "string" | "string2" => {
                let value = cursor.next();
                env.emit_text(x, y, value, word == "string2");
            }
            "cstring" | "cstring2" => {
                let alternate = word == "cstring2";
                let mut line_y = y;
                for line in cursor.next().split('\n') {
                    let line_x = x + (320 - env.cstring_length(line) * 8) / 2;
                    env.emit_text(line_x, line_y, line.to_string(), alternate);
                    line_y += 8;
                }
            }
            "client" => {
                x = width / 2 - 160 + cursor.integer();
                y = height / 2 - 120 + cursor.integer();
                let (name, icon) = env.client(cursor.integer())?;
                let (score, ping, time) = (cursor.integer(), cursor.integer(), cursor.integer());
                env.emit_text(x + 32, y, name, true);
                env.emit_text(x + 32, y + 8, "Score: ".to_string(), false);
                env.emit_text(x + 88, y + 8, score.to_string(), true);
                env.emit_text(x + 32, y + 16, format!("Ping:  {ping}"), false);
                env.emit_text(x + 32, y + 24, format!("Time:  {time}"), false);
                if !icon.is_empty() {
                    env.emit_picture(x, y, icon);
                }
            }
            "ctf" => {
                x = width / 2 - 160 + cursor.integer();
                y = height / 2 - 120 + cursor.integer();
                let index = cursor.integer();
                let (name, _) = env.client(index)?;
                let (score, ping) = (cursor.integer(), cursor.integer().min(999));
                let short: String = name.chars().take(12).collect();
                env.emit_text(
                    x,
                    y,
                    format!("{score:>3} {ping:>3} {short:<12}"),
                    index == env.player_number(),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn client_name(frame: &Q2HudFrame, index: i32) -> Result<(String, String), ClientError> {
    let max: i32 = frame
        .configstrings
        .get(&Q2_CLASSIC_LAYOUT.max_clients)
        .map_or(256, |text| game_atoi(text));
    if index < 0 || index >= max.max(1) {
        return Err(ClientError::BadHudClient { index });
    }
    let info = frame
        .configstrings
        .get(&(Q2_CLASSIC_LAYOUT.player_skins + index))
        .cloned()
        .unwrap_or_default();
    let (name, skin) = match info.find('\\') {
        Some(slash) => (info[..slash].to_string(), info[slash + 1..].to_string()),
        None => (info, "male/grunt".to_string()),
    };
    let skin = if skin.is_empty() {
        "male/grunt".to_string()
    } else {
        skin
    };
    Ok((name, format!("/players/{skin}_i.pcx")))
}

/// Headless layout environment over [`Q2HudFrame`].
struct HudLayoutEnv<'a> {
    frame: &'a Q2HudFrame,
    ops: Vec<Q2HudOp>,
}

impl ClassicLayoutEnv for HudLayoutEnv<'_> {
    type Error = ClientError;

    fn stat(&self, index: i32) -> Result<i32, Self::Error> {
        hud_stat(self.frame, index)
    }

    fn image_name(&self, image: i32) -> Result<String, Self::Error> {
        if !(0..Q2_CLASSIC_LAYOUT.max_images).contains(&image) {
            return Err(ClientError::BadHudImage { index: image });
        }
        Ok(self
            .frame
            .configstrings
            .get(&(Q2_CLASSIC_LAYOUT.images + image))
            .cloned()
            .unwrap_or_default())
    }

    fn stat_string(&self, index: i32) -> Result<String, Self::Error> {
        if !(0..Q2_CLASSIC_LAYOUT.max_configstrings).contains(&index) {
            return Err(ClientError::BadHudConfigstring { index });
        }
        Ok(self.frame.configstrings.get(&index).cloned().unwrap_or_default())
    }

    fn client(&self, index: i32) -> Result<(String, String), Self::Error> {
        client_name(self.frame, index)
    }

    fn server_frame(&self) -> i32 {
        self.frame.server_frame
    }

    fn player_number(&self) -> i32 {
        self.frame.player_number
    }

    #[allow(clippy::cast_possible_wrap)]
    fn cstring_length(&self, line: &str) -> i32 {
        line.len() as i32
    }

    fn try_arsenal_pic(&mut self, _x: i32, _y: i32, _index: i32) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn emit_text(&mut self, x: i32, y: i32, text: String, alternate: bool) {
        self.ops.push(Q2HudOp::Text { x, y, text, alternate });
    }

    fn emit_picture(&mut self, x: i32, y: i32, name: String) {
        self.ops.push(Q2HudOp::Picture { x, y, name });
    }
}

/// Execute a classic Q2 layout string (`SCR_ExecuteLayoutString`) through the
/// shared grammar core.
pub fn q2_layout_ops(source: &str, frame: &Q2HudFrame, width: i32, height: i32) -> Result<Vec<Q2HudOp>, ClientError> {
    let tokens = layout_tokens(source);
    let mut env = HudLayoutEnv { frame, ops: Vec::new() };
    run_classic_layout(&tokens, width, height, &mut env)?;
    Ok(env.ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q2_stats_bounds_check_and_slice_wire() {
        let mut block = Q2StatBlock::new();
        block.set(1, 100).unwrap();
        assert_eq!(block.get(1).unwrap(), 100);
        assert_eq!(block.wire().len(), 32);
        assert!(block.get(64).is_err());
        assert!(block.set(-1, 0).is_err());
    }

    #[test]
    fn q3_schema_shifts_missionpack_slots() {
        let base = q3_stat_schema(Q3Product::Base);
        let pack = q3_stat_schema(Q3Product::Missionpack);
        assert_eq!((base.weapons, base.armor, base.max_health), (2, 3, 6));
        assert_eq!((pack.weapons, pack.armor, pack.max_health), (3, 4, 7));
        assert_eq!(pack.persistent_powerup, Some(2));
        let mut block = Q3StatBlock::new(Q3Product::Base);
        block.set(0, 75).unwrap();
        block.set(3, 50).unwrap();
        block.set(2, 0b101).unwrap();
        assert_eq!((block.health(), block.armor(), block.weapons()), (75, 50, 0b101));
        assert!(block.get(16).is_err());
    }

    #[test]
    fn q1_stats_default_zero_and_update() {
        let mut table = Q1StatTable::new();
        assert_eq!(table.get(0), 0);
        table.update(0, 100);
        table.update(255, -5);
        assert_eq!(table.get(0), 100);
        assert_eq!(table.get(255), -5);
    }

    #[test]
    fn centerprint_reveals_and_expires() {
        let print = CenterPrint {
            text: "hello".to_string(),
            start_ms: 1000,
            duration_ms: 500,
            instant: false,
            char_ms: Some(100),
        };
        assert_eq!(print.visible_text(999), None);
        assert_eq!(print.visible_text(1000), Some(""));
        assert_eq!(print.visible_text(1250), Some("he"));
        assert_eq!(print.visible_text(1499), Some("hell"));
        assert_eq!(print.visible_text(1500), None);
        let instant = CenterPrint {
            instant: true,
            ..print.clone()
        };
        assert_eq!(instant.visible_text(1000), Some("hello"));
    }

    #[test]
    fn inventory_selection_wraps_nonzero() {
        let mut inventory = HudInventory {
            counts: vec![0, 5, 0, 7],
            selected: Some(1),
            ammo_count: Some(12),
        };
        inventory.select_next();
        assert_eq!(inventory.selected, Some(3));
        inventory.select_next();
        assert_eq!(inventory.selected, Some(1));
        inventory.select_previous();
        assert_eq!(inventory.selected, Some(3));
        assert_eq!(inventory.count(9), 0);
        inventory.select(9);
        assert_eq!(inventory.selected, None);
    }

    #[test]
    fn health_color_ramps_and_fade_expires() {
        assert_eq!(health_color(0, 100), vec4(0.0, 0.0, 0.0, 1.0));
        assert_eq!(health_color(100, 0), vec4(1.0, 1.0, 1.0, 1.0));
        let low = health_color(20, 0);
        assert_eq!((low.x, low.y, low.z), (1.0, 0.0, 0.0));
        let mid = health_color(50, 0);
        assert!((mid.y - 20.0 / 30.0).abs() < 1e-6);
        assert_eq!(fade_alpha(100, 0, 1000), None);
        assert_eq!(fade_alpha(2000, 100, 1000), None);
        assert_eq!(fade_alpha(500, 100, 1000), Some(1.0));
        assert_eq!(fade_alpha(950, 100, 1000), Some(0.75));
    }

    #[test]
    fn atoi_and_tokens_match_donor() {
        assert_eq!(game_atoi("  -42x"), -42);
        assert_eq!(game_atoi(""), 0);
        assert_eq!(game_atoi("+7"), 7);
        assert_eq!(game_atoi("abc"), 0);
        assert_eq!(game_atoi("2147483647x"), 2147483647);
        assert_eq!(game_atoi("2147483648"), -2147483648);
        let tokens = layout_tokens("xl 10 // comment\nstring \"a b\" /* c */ yv -20");
        assert_eq!(tokens, vec!["xl", "10", "string", "a b", "yv", "-20"]);
    }

    fn frame() -> Q2HudFrame {
        let mut stats = vec![0i16; 32];
        stats[1] = 100;
        stats[3] = 25;
        stats[5] = 3;
        let mut configstrings = BTreeMap::new();
        configstrings.insert(30, "8".to_string());
        configstrings.insert(544, "pic/health".to_string());
        configstrings.insert(1312, "player\\male/grunt".to_string());
        Q2HudFrame {
            stats,
            configstrings,
            player_number: 0,
            server_frame: 8,
            ammo: None,
        }
    }

    #[test]
    fn q2_layout_draws_fields_and_strings() {
        let ops = q2_layout_ops("xv 0 yv 0 hnum xv 20 stat_string 1 string hi", &frame(), 640, 480).unwrap();
        assert!(ops.iter().any(|op| matches!(
            op,
            Q2HudOp::Picture { name, .. } if name == "num_1"
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            Q2HudOp::Text { text, .. } if text == "hi"
        )));
        let skipped = q2_layout_ops("if 7 picn shown endif picn always", &frame(), 640, 480).unwrap();
        assert_eq!(skipped.len(), 1);
        let shown = q2_layout_ops("if 1 picn shown endif", &frame(), 640, 480).unwrap();
        assert_eq!(shown.len(), 1);
        let score = q2_layout_ops("ctf 0 0 0 12 34", &frame(), 640, 480).unwrap();
        assert!(matches!(score[0], Q2HudOp::Text { alternate: true, .. }));
        assert!(q2_layout_ops("pic 99", &frame(), 640, 480).is_err());
    }
}
