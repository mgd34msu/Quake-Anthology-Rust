//! Native Q2 HUD layout and inventory operations.
//!
//! Ported from the TypeScript donor's `src/ui/hud/q2-native.ts`: Q2
//! `client/cl_scrn.c` `SCR_ExecuteLayoutString`/`SCR_DrawField` and
//! `client/cl_inv.c` inventory panel. Classic protocols execute the layout
//! grammar here; rerelease protocols (`is_rerelease`) require a
//! [`NativeQ2HudEnvironment`] and delegate to the `q2_rerelease_layout`
//! sibling. Operation coordinates are `f32` HUD units: the classic grammar
//! computes integers and widens them, while the rerelease grammar keeps its
//! fractional layout math.

use std::collections::BTreeMap;

use qa_core::math::Vec4;

use super::q2_rerelease_layout::{q2_rerelease_inventory, q2_rerelease_layout};
use super::q2_rerelease_layout::{NativeQ2HudEnvironment, NativeQ2HudTable};
use super::token::HudTokenizer;
use crate::error::ClientError;
use crate::hud::{run_classic_layout, ClassicLayoutEnv};
use crate::ui::types::{Q2ProtocolFamily, ResourceId};

/// Native Q2 HUD frame: public playerstate plus received statusbar/configstrings.
///
/// `player_number` is zero based.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeQ2HudFrame {
    /// Protocol family selecting the layout grammar and configstring tables.
    pub protocol: Q2ProtocolFamily,
    /// Playerstate stats indexed by the layout grammar.
    pub stats: Vec<i32>,
    /// Received configstrings by index.
    pub configstrings: BTreeMap<i32, String>,
    /// Server-sent layout string drawn while the layout bit is set.
    pub layout: String,
    /// Classic inventory counts by item index.
    pub inventory: Vec<i32>,
    /// Zero-based player slot.
    pub player_number: i32,
    /// Server frame, driving field blink timing.
    pub server_frame: i32,
    /// Client time in milliseconds.
    pub time_ms: i64,
    /// Last frame duration in milliseconds, if measured.
    pub frame_time_ms: Option<i64>,
}

/// One native Q2 HUD draw operation.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeQ2HudOperation {
    /// Proportional-font text (rerelease `useFont` paths).
    FontText {
        /// Left edge in HUD units.
        x: f32,
        /// Top edge in HUD units.
        y: f32,
        /// Text to draw.
        text: String,
        /// Alternate (green) character set.
        alternate: bool,
    },
    /// Picture stretched to an explicit size.
    SizedPicture {
        /// Left edge in HUD units.
        x: f32,
        /// Top edge in HUD units.
        y: f32,
        /// Width in HUD units.
        width: f32,
        /// Height in HUD units.
        height: f32,
        /// Picture name.
        name: String,
    },
    /// Solid filled rectangle.
    Fill {
        /// Left edge in HUD units.
        x: f32,
        /// Top edge in HUD units.
        y: f32,
        /// Width in HUD units.
        width: f32,
        /// Height in HUD units.
        height: f32,
        /// Fill color.
        color: Vec4,
    },
    /// Picture drawn at its natural size.
    Picture {
        /// Left edge in HUD units.
        x: f32,
        /// Top edge in HUD units.
        y: f32,
        /// Picture name.
        name: String,
        /// Whether the picture anchors before the cursor.
        anchor_before: bool,
    },
    /// Provider-resolved arsenal icon (ammo or selected item).
    ArsenalPicture {
        /// Left edge in HUD units.
        x: f32,
        /// Top edge in HUD units.
        y: f32,
        /// Icon resource.
        resource: ResourceId,
        /// Width-over-height aspect.
        aspect: f32,
    },
    /// Monospace console text.
    Text {
        /// Left edge in HUD units.
        x: f32,
        /// Top edge in HUD units.
        y: f32,
        /// Text to draw.
        text: String,
        /// Alternate (green) character set.
        alternate: bool,
        /// Drop shadow.
        shadow: bool,
        /// Xor blend against the background.
        xor: bool,
    },
}

/// One provider-resolved inventory row.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInventoryItem {
    /// Display label.
    pub label: String,
    /// Stack count.
    pub count: i32,
    /// Item index comparable against [`NativeInventoryReadout::selected`].
    pub item: i32,
}

/// Provider-resolved inventory readout.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInventoryReadout {
    /// Rows in display order.
    pub items: Vec<NativeInventoryItem>,
    /// Selected item index.
    pub selected: i32,
}

/// One arsenal icon reference.
#[derive(Debug, Clone, PartialEq)]
pub struct ArsenalIcon {
    /// Icon resource.
    pub resource: ResourceId,
    /// Width-over-height aspect.
    pub aspect: f32,
}

/// Selected-item arsenal view.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedItemView {
    /// Unlocalized label.
    pub label: String,
    /// Localized label.
    pub localized_label: String,
    /// Item icon, if resolved.
    pub icon: Option<ArsenalIcon>,
}

/// Ammunition arsenal view.
#[derive(Debug, Clone, PartialEq)]
pub struct AmmunitionView {
    /// Ammo count; `None` hides the readout.
    pub count: Option<i32>,
    /// Ammo icon, if resolved.
    pub icon: Option<ArsenalIcon>,
}

/// Provider-resolved arsenal backing stats 2/3/6 and the inventory panel.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NativeQ2HudArsenal {
    /// Selected item, backing `pic 6`.
    pub selected_item: Option<SelectedItemView>,
    /// Inventory readout backing the inventory panel.
    pub inventory: Option<NativeInventoryReadout>,
    /// Ammunition backing stats 2/3 and `pic 2`.
    pub ammunition: Option<AmmunitionView>,
}

/// Inventory presentation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryMode {
    /// Draw the layout only, without statusbar or inventory.
    LayoutOverlay,
    /// Draw statusbar, layout, and the inventory panel.
    ReplaceStatus,
}

/// Execute a Q2 layout string into draw operations.
///
/// Rerelease protocols require `environment` and delegate to the rerelease
/// grammar; classic protocols run `SCR_ExecuteLayoutString` here.
pub fn q2_layout_operations(
    source: &str,
    frame: &NativeQ2HudFrame,
    width: i32,
    height: i32,
    arsenal: Option<&NativeQ2HudArsenal>,
    environment: Option<&mut NativeQ2HudEnvironment>,
) -> Result<Vec<NativeQ2HudOperation>, ClientError> {
    if frame.protocol.is_rerelease() {
        let environment = match environment {
            Some(environment) => environment,
            None => {
                return Err(ClientError::BadUi(
                    "Rerelease HUD requires source localization and font services".to_string(),
                ));
            }
        };
        return q2_rerelease_layout(source, frame, width, height, environment, arsenal);
    }
    let mut parser = HudTokenizer::new(source, "Q2 HUD layout");
    let mut tokens = Vec::new();
    while let Some(token) = parser.next(true)? {
        tokens.push(token.value);
    }
    let mut env = NativeLayoutEnv {
        frame,
        arsenal,
        out: Vec::new(),
    };
    run_classic_layout(&tokens, width, height, &mut env)?;
    Ok(env.out)
}

/// Native classic-layout environment over [`NativeQ2HudFrame`].
struct NativeLayoutEnv<'a> {
    frame: &'a NativeQ2HudFrame,
    arsenal: Option<&'a NativeQ2HudArsenal>,
    out: Vec<NativeQ2HudOperation>,
}

impl ClassicLayoutEnv for NativeLayoutEnv<'_> {
    type Error = ClientError;

    fn stat(&self, index: i32) -> Result<i32, Self::Error> {
        native_q2_hud_stat(self.frame, index, self.arsenal)
    }

    fn image_name(&self, image: i32) -> Result<String, Self::Error> {
        let config = self.frame.protocol.layout();
        if image < 0 || image >= config.max_images {
            return Err(ClientError::BadUi(format!(
                "Q2 HUD image {image} is outside configstrings"
            )));
        }
        Ok(self
            .frame
            .configstrings
            .get(&(config.images + image))
            .map_or("", String::as_str)
            .to_string())
    }

    fn stat_string(&self, index: i32) -> Result<String, Self::Error> {
        let config = self.frame.protocol.layout();
        if index < 0 || index >= config.max_config_strings {
            return Err(ClientError::BadUi(
                "Q2 HUD stat_string is outside configstrings".to_string(),
            ));
        }
        Ok(self
            .frame
            .configstrings
            .get(&index)
            .map_or("", String::as_str)
            .to_string())
    }

    fn client(&self, index: i32) -> Result<(String, String), Self::Error> {
        client_info(self.frame, index)
    }

    fn server_frame(&self) -> i32 {
        self.frame.server_frame
    }

    fn player_number(&self) -> i32 {
        self.frame.player_number
    }

    fn cstring_length(&self, line: &str) -> i32 {
        line.encode_utf16().count() as i32
    }

    fn try_arsenal_pic(&mut self, x: i32, y: i32, index: i32) -> Result<bool, Self::Error> {
        if index == 2 {
            if let Some(ammunition) = self.arsenal.and_then(|arsenal| arsenal.ammunition.as_ref()) {
                if let (Some(_), Some(icon)) = (ammunition.count, ammunition.icon.as_ref()) {
                    self.out.push(NativeQ2HudOperation::ArsenalPicture {
                        x: x as f32,
                        y: y as f32,
                        resource: icon.resource.clone(),
                        aspect: icon.aspect,
                    });
                }
                return Ok(true);
            }
            return Ok(false);
        }
        if index == 6 {
            if let Some(selected) = self.arsenal.and_then(|arsenal| arsenal.selected_item.as_ref()) {
                if native_q2_hud_stat(self.frame, index, self.arsenal)? != 0 {
                    if let Some(icon) = selected.icon.as_ref() {
                        self.out.push(NativeQ2HudOperation::ArsenalPicture {
                            x: x as f32,
                            y: y as f32,
                            resource: icon.resource.clone(),
                            aspect: icon.aspect,
                        });
                    }
                }
                return Ok(true);
            }
            return Ok(false);
        }
        Ok(false)
    }

    fn emit_text(&mut self, x: i32, y: i32, text: String, alternate: bool) {
        push_text(&mut self.out, x, y, &text, alternate);
    }

    fn emit_picture(&mut self, x: i32, y: i32, name: String) {
        push_picture(&mut self.out, x, y, &name);
    }
}

/// Draw the statusbar, layout, and classic inventory panel.
///
/// `binding` resolves `use <item>` commands to hotkey text. Rerelease
/// protocols require `environment`.
#[allow(clippy::too_many_arguments)]
pub fn q2_native_hud_operations(
    frame: &NativeQ2HudFrame,
    width: i32,
    height: i32,
    binding: &dyn Fn(&str) -> String,
    mode: InventoryMode,
    arsenal: Option<&NativeQ2HudArsenal>,
    mut environment: Option<&mut NativeQ2HudEnvironment>,
) -> Result<Vec<NativeQ2HudOperation>, ClientError> {
    let rerelease = frame.protocol.is_rerelease();
    if rerelease {
        if let Some(environment) = environment.as_deref_mut() {
            if environment.table.is_none() {
                environment.table = Some(NativeQ2HudTable::default());
            }
        }
    }
    let layouts = frame.stats.get(13).copied().unwrap_or(0);
    let mut out = if mode == InventoryMode::LayoutOverlay || rerelease && layouts & 4 != 0 {
        Vec::new()
    } else {
        q2_layout_operations(
            frame.configstrings.get(&5).map_or("", String::as_str),
            frame,
            width,
            height,
            arsenal,
            environment.as_deref_mut(),
        )?
    };
    if layouts & 1 != 0 {
        out.extend(q2_layout_operations(
            &frame.layout,
            frame,
            width,
            height,
            arsenal,
            environment.as_deref_mut(),
        )?);
    }
    if mode == InventoryMode::LayoutOverlay {
        return Ok(out);
    }
    if layouts & 2 == 0 {
        return Ok(out);
    }
    if rerelease {
        let environment = match environment {
            Some(environment) => environment,
            None => {
                return Err(ClientError::BadUi(
                    "Rerelease HUD requires source localization and font services".to_string(),
                ));
            }
        };
        let inventory = arsenal.and_then(|arsenal| arsenal.inventory.as_ref());
        out.extend(q2_rerelease_inventory(frame, width, height, environment, inventory)?);
        return Ok(out);
    }
    let config = frame.protocol.layout();
    let selected = frame.stats.get(12).copied().unwrap_or(0);
    let rows: Vec<InventoryRow> = match arsenal.and_then(|arsenal| arsenal.inventory.as_ref()) {
        Some(inventory) => inventory
            .items
            .iter()
            .map(|row| InventoryRow {
                name: row.label.clone(),
                count: row.count,
                selected: row.item == inventory.selected,
            })
            .collect(),
        None => frame
            .inventory
            .iter()
            .enumerate()
            .filter(|(_, count)| **count != 0)
            .map(|(index, count)| InventoryRow {
                name: frame
                    .configstrings
                    .get(&(config.items + index as i32))
                    .cloned()
                    .unwrap_or_default(),
                count: *count,
                selected: index as i32 == selected,
            })
            .collect(),
    };
    let selected_row = match arsenal.and_then(|arsenal| arsenal.inventory.as_ref()) {
        None => {
            if selected >= 0 && selected < frame.inventory.len() as i32 {
                frame
                    .inventory
                    .iter()
                    .take(selected as usize)
                    .filter(|count| **count != 0)
                    .count() as i32
            } else {
                0
            }
        }
        Some(_) => rows.iter().position(|row| row.selected).map_or(0, |index| index as i32),
    };
    let top = 0.max((rows.len() as i32 - 17).min(selected_row - 8));
    let x = (width - 256).div_euclid(2);
    let y = (height - 240).div_euclid(2);
    push_picture(&mut out, x, y + 8, "inventory");
    push_text(&mut out, x + 24, y + 24, "hotkey ### item", false);
    push_text(&mut out, x + 24, y + 32, "------ --- ----", false);
    for (row, item) in rows.iter().skip(top as usize).take(17).enumerate() {
        let line_y = y + 40 + row as i32 * 8;
        let hotkey = binding(&format!("use {}", item.name));
        push_text(
            &mut out,
            x + 24,
            line_y,
            &format!("{hotkey:>6} {:>3} {}", item.count, item.name),
            !item.selected,
        );
        if item.selected && (frame.time_ms / 100) & 1 != 0 {
            push_text(&mut out, x + 16, line_y, "\u{0f}", false);
        }
    }
    Ok(out)
}

/// Read one HUD stat, applying the ammunition arsenal mapping.
///
/// With an ammunition view, stat 2 reports presence (1) or absence (0) and
/// stat 3 reports the count (`-1` when hidden).
pub fn native_q2_hud_stat(
    frame: &NativeQ2HudFrame,
    index: i32,
    arsenal: Option<&NativeQ2HudArsenal>,
) -> Result<i32, ClientError> {
    if index < 0 || index >= frame.stats.len() as i32 {
        return Err(ClientError::BadUi(format!(
            "Q2 HUD stat {index} is outside playerstate"
        )));
    }
    if let Some(ammunition) = arsenal.and_then(|arsenal| arsenal.ammunition.as_ref()) {
        if index == 2 {
            return Ok(i32::from(ammunition.count.is_some()));
        }
        if index == 3 {
            return Ok(ammunition.count.unwrap_or(-1));
        }
    }
    Ok(frame.stats[index as usize])
}

/// One classic inventory panel row.
struct InventoryRow {
    name: String,
    count: i32,
    selected: bool,
}

/// Push a picture unless its name is empty.
fn push_picture(out: &mut Vec<NativeQ2HudOperation>, x: i32, y: i32, name: &str) {
    if name.is_empty() {
        return;
    }
    out.push(NativeQ2HudOperation::Picture {
        x: x as f32,
        y: y as f32,
        name: name.to_string(),
        anchor_before: false,
    });
}

/// Push monospace console text.
fn push_text(out: &mut Vec<NativeQ2HudOperation>, x: i32, y: i32, text: &str, alternate: bool) {
    out.push(NativeQ2HudOperation::Text {
        x: x as f32,
        y: y as f32,
        text: text.to_string(),
        alternate,
        shadow: false,
        xor: false,
    });
}

/// Resolve a client slot to its scoreboard name and player-icon picture.
fn client_info(frame: &NativeQ2HudFrame, index: i32) -> Result<(String, String), ClientError> {
    let config = frame.protocol.layout();
    let max = match frame.configstrings.get(&config.max_clients) {
        None => 256.0,
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                0.0
            } else {
                trimmed.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
    };
    // The donor's `Math.max(1, max)` stays NaN for unparseable counts, which
    // disables the upper-bound check (`index >= NaN` is false).
    let limit = if max.is_nan() { f64::NAN } else { 1.0f64.max(max) };
    if index < 0 || f64::from(index) >= limit {
        return Err(ClientError::BadUi(format!(
            "Q2 HUD client {index} is outside clientinfo"
        )));
    }
    let info = frame
        .configstrings
        .get(&(config.player_skins + index))
        .cloned()
        .unwrap_or_default();
    let (name, skin) = match info.find('\\') {
        None => (info, "male/grunt".to_string()),
        Some(slash) => {
            let skin = info[slash + 1..].to_string();
            (
                info[..slash].to_string(),
                if skin.is_empty() {
                    "male/grunt".to_string()
                } else {
                    skin
                },
            )
        }
    };
    Ok((name, format!("/players/{skin}_i.pcx")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> NativeQ2HudFrame {
        let mut stats = vec![0i32; 32];
        stats[1] = 100;
        stats[3] = 25;
        stats[5] = 8;
        NativeQ2HudFrame {
            protocol: Q2ProtocolFamily::Classic,
            stats,
            configstrings: BTreeMap::new(),
            layout: String::new(),
            inventory: Vec::new(),
            player_number: 0,
            server_frame: 0,
            time_ms: 0,
            frame_time_ms: None,
        }
    }

    fn text(x: i32, y: i32, text: &str, alternate: bool) -> NativeQ2HudOperation {
        NativeQ2HudOperation::Text {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            alternate,
            shadow: false,
            xor: false,
        }
    }

    fn picture(x: i32, y: i32, name: &str) -> NativeQ2HudOperation {
        NativeQ2HudOperation::Picture {
            x: x as f32,
            y: y as f32,
            name: name.to_string(),
            anchor_before: false,
        }
    }

    fn icon() -> ArsenalIcon {
        ArsenalIcon {
            resource: ResourceId::new("resource:ammo").expect("resource"),
            aspect: 1.5,
        }
    }

    #[test]
    fn positions_and_pictures() {
        let frame = frame();
        let ops = q2_layout_operations(
            "xv 10 yv 20 picn shell string hi string2 yo",
            &frame,
            640,
            480,
            None,
            None,
        )
        .expect("layout");
        assert_eq!(
            ops,
            vec![
                picture(170, 140, "shell"),
                text(170, 140, "hi", false),
                text(170, 140, "yo", true),
            ]
        );
    }

    #[test]
    fn right_edges_anchor_to_viewport() {
        let frame = frame();
        let ops = q2_layout_operations("xr -50 yb -30 string hi", &frame, 640, 480, None, None).expect("layout");
        assert_eq!(ops, vec![text(590, 450, "hi", false)]);
    }

    #[test]
    fn digit_fields_right_align() {
        let frame = frame();
        let ops = q2_layout_operations("xl 0 yt 0 hnum", &frame, 640, 480, None, None).expect("layout");
        assert_eq!(
            ops,
            vec![picture(2, 0, "num_1"), picture(18, 0, "num_0"), picture(34, 0, "num_0"),]
        );
    }

    #[test]
    fn low_health_flashes_alternate_digits() {
        let mut frame = frame();
        frame.stats[1] = 20;
        frame.server_frame = 1 << 2;
        let ops = q2_layout_operations("xl 0 yt 0 hnum", &frame, 640, 480, None, None).expect("layout");
        assert!(ops.iter().all(|op| matches!(
            op,
            NativeQ2HudOperation::Picture { name, .. } if name.starts_with("anum_")
        )));
    }

    #[test]
    fn negative_ammo_skips_anum() {
        let mut frame = frame();
        frame.stats[3] = -1;
        let ops = q2_layout_operations("xl 0 yt 0 anum", &frame, 640, 480, None, None).expect("layout");
        assert!(ops.is_empty());
    }

    #[test]
    fn pic_uses_arsenal_views() {
        let frame = frame();
        let arsenal = NativeQ2HudArsenal {
            selected_item: Some(SelectedItemView {
                label: "shotgun".to_string(),
                localized_label: "Shotgun".to_string(),
                icon: Some(icon()),
            }),
            inventory: None,
            ammunition: Some(AmmunitionView {
                count: Some(10),
                icon: Some(icon()),
            }),
        };
        let ops = q2_layout_operations("xl 4 yt 6 pic 2", &frame, 640, 480, Some(&arsenal), None).expect("layout");
        assert_eq!(
            ops,
            vec![NativeQ2HudOperation::ArsenalPicture {
                x: 4.0,
                y: 6.0,
                resource: ResourceId::new("resource:ammo").expect("resource"),
                aspect: 1.5,
            }]
        );
    }

    #[test]
    fn pic_image_range_is_checked() {
        let mut frame = frame();
        frame.stats[7] = 999;
        let error = q2_layout_operations("pic 7", &frame, 640, 480, None, None).expect_err("range");
        assert_eq!(error.to_string(), "Q2 HUD image 999 is outside configstrings");
    }

    #[test]
    fn stat_string_and_cstring() {
        let mut frame = frame();
        frame.stats[4] = 42;
        frame.configstrings.insert(42, "objective".to_string());
        let ops = q2_layout_operations(
            "xl 0 yt 0 stat_string 4 xl 0 yv 0 cstring ab",
            &frame,
            640,
            480,
            None,
            None,
        )
        .expect("layout");
        assert_eq!(ops, vec![text(0, 0, "objective", false), text(152, 120, "ab", false),]);
    }

    #[test]
    fn client_row_reports_scoreboard() {
        let mut frame = frame();
        frame.configstrings.insert(1312, "mike\\male/grunt".to_string());
        let ops = q2_layout_operations("client 0 0 0 12 34 56", &frame, 640, 480, None, None).expect("layout");
        assert_eq!(
            ops,
            vec![
                text(192, 120, "mike", true),
                text(192, 128, "Score: ", false),
                text(248, 128, "12", true),
                text(192, 136, "Ping:  34", false),
                text(192, 144, "Time:  56", false),
                picture(160, 120, "/players/male/grunt_i.pcx"),
            ]
        );
    }

    #[test]
    fn ctf_row_highlights_local_player() {
        let mut frame = frame();
        frame.configstrings.insert(1312, "mike\\male/grunt".to_string());
        let ops = q2_layout_operations("ctf 0 0 0 7 1200", &frame, 640, 480, None, None).expect("layout");
        assert_eq!(ops, vec![text(160, 120, "  7 999 mike        ", true)]);
    }

    #[test]
    fn client_slot_range_is_checked() {
        let frame = frame();
        let error = q2_layout_operations("client 0 0 999 1 2 3", &frame, 640, 480, None, None).expect_err("range");
        assert_eq!(error.to_string(), "Q2 HUD client 999 is outside clientinfo");
    }

    #[test]
    fn if_skips_to_endif() {
        let frame = frame();
        let ops = q2_layout_operations("if 9 string hidden endif string shown", &frame, 640, 480, None, None)
            .expect("layout");
        assert_eq!(ops, vec![text(0, 0, "shown", false)]);
    }

    #[test]
    fn stat_read_applies_ammunition_mapping() {
        let frame = frame();
        let arsenal = NativeQ2HudArsenal {
            ammunition: Some(AmmunitionView {
                count: Some(7),
                icon: None,
            }),
            ..NativeQ2HudArsenal::default()
        };
        assert_eq!(native_q2_hud_stat(&frame, 2, Some(&arsenal)).expect("stat"), 1);
        assert_eq!(native_q2_hud_stat(&frame, 3, Some(&arsenal)).expect("stat"), 7);
        assert_eq!(native_q2_hud_stat(&frame, 1, Some(&arsenal)).expect("stat"), 100);
        let hidden = NativeQ2HudArsenal {
            ammunition: Some(AmmunitionView {
                count: None,
                icon: None,
            }),
            ..NativeQ2HudArsenal::default()
        };
        assert_eq!(native_q2_hud_stat(&frame, 2, Some(&hidden)).expect("stat"), 0);
        assert_eq!(native_q2_hud_stat(&frame, 3, Some(&hidden)).expect("stat"), -1);
        native_q2_hud_stat(&frame, 99, None).expect_err("range");
    }

    #[test]
    fn rerelease_requires_environment() {
        let mut frame = frame();
        frame.protocol = Q2ProtocolFamily::Rerelease;
        let error = q2_layout_operations("", &frame, 640, 480, None, None).expect_err("env");
        assert_eq!(
            error.to_string(),
            "Rerelease HUD requires source localization and font services"
        );
    }

    #[test]
    fn native_hud_draws_statusbar_layout_and_inventory() {
        let mut frame = frame();
        frame.configstrings.insert(5, "string status".to_string());
        frame.configstrings.insert(1056, "Shells".to_string());
        frame.layout = "string active".to_string();
        frame.stats[13] = 1 | 2;
        frame.stats[12] = 0;
        frame.inventory = vec![10];
        let ops = q2_native_hud_operations(
            &frame,
            640,
            480,
            &|command| format!("<{command}>"),
            InventoryMode::ReplaceStatus,
            None,
            None,
        )
        .expect("hud");
        assert_eq!(ops[0], text(0, 0, "status", false));
        assert_eq!(ops[1], text(0, 0, "active", false));
        assert_eq!(ops[2], picture(192, 128, "inventory"));
        assert!(ops.iter().any(|op| matches!(
            op,
            NativeQ2HudOperation::Text { text, alternate: false, .. }
            if text.contains("Shells")
        )));
    }

    #[test]
    fn shared_grammar_matches_headless_emitter() {
        use crate::hud::{q2_layout_ops, Q2HudFrame, Q2HudOp};

        let source = "xv 0 yv 0 hnum anum rnum num 4 1 pic 7 picn shell string hi string2 yo \
            cstring ab stat_string 4 if 1 picn shown endif client 0 0 0 12 34 56 ctf 0 0 0 7 1200";
        let mut stats16 = vec![0i16; 32];
        stats16[1] = 100;
        stats16[3] = 25;
        stats16[5] = 8;
        stats16[4] = 42;
        stats16[7] = 0;
        let mut stats32 = vec![0i32; 32];
        stats32[1] = 100;
        stats32[3] = 25;
        stats32[5] = 8;
        stats32[4] = 42;
        stats32[7] = 0;
        let mut configstrings = BTreeMap::new();
        configstrings.insert(30, "8".to_string());
        configstrings.insert(42, "objective".to_string());
        configstrings.insert(544, "pic/health".to_string());
        configstrings.insert(1312, "mike\\male/grunt".to_string());
        let headless = q2_layout_ops(
            source,
            &Q2HudFrame {
                stats: stats16,
                configstrings: configstrings.clone(),
                player_number: 0,
                server_frame: 0,
                ammo: None,
            },
            640,
            480,
        )
        .expect("headless layout");
        let native = q2_layout_operations(
            source,
            &NativeQ2HudFrame {
                protocol: Q2ProtocolFamily::Classic,
                stats: stats32,
                configstrings,
                layout: String::new(),
                inventory: Vec::new(),
                player_number: 0,
                server_frame: 0,
                time_ms: 0,
                frame_time_ms: None,
            },
            640,
            480,
            None,
            None,
        )
        .expect("native layout");
        assert_eq!(native.len(), headless.len());
        for (native, headless) in native.iter().zip(headless.iter()) {
            match (native, headless) {
                (
                    NativeQ2HudOperation::Text {
                        x,
                        y,
                        text,
                        alternate,
                        shadow,
                        xor,
                    },
                    Q2HudOp::Text {
                        x: hx,
                        y: hy,
                        text: htext,
                        alternate: halt,
                    },
                ) => {
                    assert_eq!((*x, *y), (*hx as f32, *hy as f32));
                    assert_eq!((text, *alternate), (htext, *halt));
                    assert!(!shadow && !xor);
                }
                (
                    NativeQ2HudOperation::Picture {
                        x,
                        y,
                        name,
                        anchor_before,
                    },
                    Q2HudOp::Picture {
                        x: hx,
                        y: hy,
                        name: hname,
                    },
                ) => {
                    assert_eq!((*x, *y), (*hx as f32, *hy as f32));
                    assert_eq!(name, hname);
                    assert!(!anchor_before);
                }
                other => panic!("op mismatch: {other:?}"),
            }
        }
    }

    #[test]
    fn layout_overlay_skips_statusbar_and_inventory() {
        let mut frame = frame();
        frame.configstrings.insert(5, "string status".to_string());
        frame.layout = "string active".to_string();
        frame.stats[13] = 1 | 2;
        let ops = q2_native_hud_operations(
            &frame,
            640,
            480,
            &|_| String::new(),
            InventoryMode::LayoutOverlay,
            None,
            None,
        )
        .expect("hud");
        assert_eq!(ops, vec![text(0, 0, "active", false)]);
    }
}
