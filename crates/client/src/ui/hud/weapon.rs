//! Common weapon status HUD.
//!
//! Donor provenance: `src/ui/hud/weapon.ts` in full (status-row metrics,
//! compact 32-unit path, full icon/count/label layout, ammo icon). Layout
//! math uses `f32`; text measurement falls back to eight units per character
//! when no measurer is supplied.

use std::rc::Rc;

use qa_core::math::{Vec2, Vec4};

use crate::text::draw2d::Rect;
use crate::ui::common::skin::UiSkin;
use crate::ui::types::{ArsenalAmmoWarning, ResourceId, TextAlign, UiDrawCommand, WeaponAmmo, WeaponHudStatus};

/// White image tint.
const WHITE: Vec4 = Vec4 {
    x: 1.0,
    y: 1.0,
    z: 1.0,
    w: 1.0,
};

/// Full-size image texture coordinates.
const FULL_UV: [Vec2; 2] = [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 1.0, y: 1.0 }];

/// Text width measurer: width of `text` rendered at `scale`.
pub type MeasureText = Rc<dyn Fn(&str, f32) -> f32>;

/// Weapon HUD inputs shared by every provider's status panel.
#[derive(Clone)]
pub struct CommonWeaponHud {
    /// Weapon status.
    pub status: WeaponHudStatus,
    /// Arsenal-level ammo warning.
    pub warning: ArsenalAmmoWarning,
    /// Weapon icon, preferred over the ammo icon.
    pub weapon_icon: Option<ResourceId>,
    /// Ammo icon, used alone or as the secondary icon.
    pub ammo_icon: Option<ResourceId>,
    /// Weapon icon width-over-height aspect.
    pub icon_aspect: f32,
    /// Ammo icon width-over-height aspect.
    pub ammo_aspect: f32,
    /// Whether the provider draws its own native status panel.
    pub native_status: bool,
    /// Text width measurer; falls back to eight units per character.
    pub measure_text: Option<MeasureText>,
}

impl std::fmt::Debug for CommonWeaponHud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommonWeaponHud")
            .field("status", &self.status)
            .field("warning", &self.warning)
            .field("weapon_icon", &self.weapon_icon)
            .field("ammo_icon", &self.ammo_icon)
            .field("icon_aspect", &self.icon_aspect)
            .field("ammo_aspect", &self.ammo_aspect)
            .field("native_status", &self.native_status)
            .field("measure_text", &self.measure_text.is_some())
            .finish()
    }
}

/// Status-row metrics for the full weapon panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusRows {
    /// Label block top offset in UI units.
    pub label_top: f32,
    /// Full panel height in UI units.
    pub height: f32,
}

/// Measure status-row metrics for one text scale and cap height.
#[must_use]
pub fn hud_status_rows(text_scale: f32, cap_height: f32) -> StatusRows {
    let label_top = 25.0_f32.max(4.0 + cap_height * text_scale * 1.5 + 4.0);
    let height = 42.0_f32.max((label_top + cap_height * text_scale * 0.9 + 4.0).ceil());
    StatusRows { label_top, height }
}

/// Count UTF-16 code units, matching donor `string.length`.
fn utf16_len(text: &str) -> f32 {
    text.encode_utf16().count() as f32
}

/// Measure text at unit scale with the fallback width.
fn measure_unit(measure_text: &Option<MeasureText>, text: &str) -> f32 {
    match measure_text {
        Some(measure) => measure(text, 1.0),
        None => text.chars().count() as f32 * 8.0,
    }
}

/// Draw the common weapon status panel.
#[must_use]
pub fn draw_weapon_hud(
    data: &CommonWeaponHud,
    rect: &Rect,
    skin: &UiSkin,
    text_scale: f32,
    minimum_text_scale: f32,
) -> Vec<UiDrawCommand> {
    let ammo = &data.status.ammo;
    let unavailable = matches!(
        ammo,
        WeaponAmmo::Finite {
            has_ammo_to_start: false,
            ..
        }
    );
    let low = matches!(ammo, WeaponAmmo::Finite { low: true, .. });
    let finite = matches!(ammo, WeaponAmmo::Finite { .. });
    let active_warning = if data.status.source.provider.starts_with("q3:") {
        None
    } else if unavailable {
        Some("NO AMMO")
    } else if finite && low {
        Some("LOW AMMO")
    } else {
        None
    };
    let warning = match data.warning {
        ArsenalAmmoWarning::Empty => Some("OUT OF AMMO"),
        ArsenalAmmoWarning::Low => Some("LOW AMMO WARNING"),
        ArsenalAmmoWarning::None => active_warning,
    };
    let color = if warning.is_none() && !unavailable {
        skin.colors.text
    } else {
        skin.colors.accent
    };
    if minimum_text_scale > 0.0 && rect.height == 32.0 {
        return draw_compact_weapon_hud(data, rect, skin, minimum_text_scale, warning, color);
    }
    draw_full_weapon_hud(data, rect, skin, text_scale, minimum_text_scale, warning, color)
}

/// Draw the compact 32-unit weapon strip.
fn draw_compact_weapon_hud(
    data: &CommonWeaponHud,
    rect: &Rect,
    skin: &UiSkin,
    scale: f32,
    warning: Option<&str>,
    color: Vec4,
) -> Vec<UiDrawCommand> {
    let top = skin.cap_ink.map_or(0.0, |ink| ink.top) * scale;
    let mut commands = vec![UiDrawCommand::Fill {
        rect: *rect,
        color: skin.colors.panel,
    }];
    let icon = data.weapon_icon.as_ref().or(data.ammo_icon.as_ref());
    if let Some(icon) = icon {
        commands.push(UiDrawCommand::Image {
            rect: Rect {
                x: rect.x + 4.0,
                y: rect.y + 4.0,
                width: 24.0,
                height: 24.0 / data.icon_aspect,
            },
            resource: icon.clone(),
            tex_coords: FULL_UV,
            color: WHITE,
        });
    }
    let left = rect.x + if icon.is_none() { 4.0 } else { 32.0 };
    let available = rect.x + rect.width - 4.0 - left;
    let count = match &data.status.ammo {
        WeaponAmmo::Finite { count, .. } => count.to_string(),
        WeaponAmmo::Unmetered => String::new(),
    };
    let first = if matches!(data.status.ammo, WeaponAmmo::Finite { .. }) {
        count
    } else if icon.is_none() {
        data.status.label.clone()
    } else {
        String::new()
    };
    let second = match warning {
        Some("LOW AMMO WARNING") => "LOW AMMO".to_string(),
        Some(text) => text.to_string(),
        None => String::new(),
    };
    for (row, label) in [first, second].iter().enumerate() {
        let mut chars: Vec<char> = label.chars().collect();
        loop {
            let candidate: String = chars.iter().collect();
            let width = match &data.measure_text {
                Some(measure) => measure(&candidate, scale),
                None => chars.len() as f32 * 8.0 * scale,
            };
            if chars.is_empty() || width <= available {
                break;
            }
            chars.pop();
        }
        if !chars.is_empty() {
            commands.push(UiDrawCommand::Text {
                origin: Vec2 {
                    x: left,
                    y: rect.y + 4.0 + row as f32 * 14.0 - top,
                },
                text: chars.iter().collect(),
                font: skin.font.clone(),
                scale,
                color,
                align: TextAlign::Left,
                shadow: true,
            });
        }
    }
    commands
}

/// Draw the full weapon status panel.
#[allow(clippy::too_many_lines)]
fn draw_full_weapon_hud(
    data: &CommonWeaponHud,
    rect: &Rect,
    skin: &UiSkin,
    text_scale: f32,
    minimum_text_scale: f32,
    warning: Option<&str>,
    color: Vec4,
) -> Vec<UiDrawCommand> {
    let cap_height = skin.cap_ink.map_or(8.0, |ink| ink.height);
    let rows = hud_status_rows(text_scale, cap_height);
    let top = skin.cap_ink.map_or(0.0, |ink| ink.top);
    let mut commands = vec![UiDrawCommand::Fill {
        rect: *rect,
        color: skin.colors.panel,
    }];
    let icon = data.weapon_icon.as_ref().or(data.ammo_icon.as_ref());
    let finite_count = match &data.status.ammo {
        WeaponAmmo::Finite { count, .. } => Some(*count),
        WeaponAmmo::Unmetered => None,
    };
    let number_width = match finite_count {
        Some(count) => {
            let value = count.to_string();
            match &data.measure_text {
                Some(measure) => measure(&value, text_scale * 1.5),
                None => utf16_len(&value) * 8.0 * text_scale * 1.5,
            }
        }
        None => 0.0,
    };
    let secondary_icon = data.ammo_icon.is_some() && data.weapon_icon.is_some() && finite_count.is_some();
    let stacked_icon = icon.is_some() && number_width > rect.width - if secondary_icon { 84.0 } else { 66.0 };
    let aspect = data.icon_aspect;
    let max_width: f32 = if warning.is_none() { 48.0 } else { 30.0 };
    let max_height: f32 = if warning.is_none() { 32.0 } else { 20.0 };
    let width = (if stacked_icon { 24.0 } else { max_width }).min(
        (if stacked_icon {
            24.0_f32.min(rect.height - rows.label_top - 4.0)
        } else {
            max_height
        }) * aspect,
    );
    let height = width / aspect;
    if let Some(icon) = icon {
        commands.push(UiDrawCommand::Image {
            rect: Rect {
                x: rect.x + 4.0 + ((if stacked_icon { 24.0 } else { 48.0 }) - width) / 2.0,
                y: rect.y + if stacked_icon { rows.label_top } else { 5.0 },
                width,
                height,
            },
            resource: icon.clone(),
            tex_coords: FULL_UV,
            color: WHITE,
        });
    }
    let x = rect.x + if icon.is_none() || stacked_icon { 8.0 } else { 58.0 };
    if let Some(count) = finite_count {
        let value = count.to_string();
        let available = rect.x + rect.width - if secondary_icon && !stacked_icon { 26.0 } else { 8.0 } - x;
        let unit = match &data.measure_text {
            Some(measure) => measure(&value, 1.0),
            None => utf16_len(&value) * 8.0,
        };
        let scale = (text_scale * 1.5).min(available / unit.max(1.0));
        commands.push(UiDrawCommand::Text {
            origin: Vec2 {
                x,
                y: rect.y + 4.0 - top * scale,
            },
            text: value,
            font: skin.font.clone(),
            scale,
            color,
            align: TextAlign::Left,
            shadow: true,
        });
    }
    if warning.is_some() || icon.is_none() {
        let original = warning.map_or_else(|| data.status.label.clone(), str::to_string);
        let left = if stacked_icon {
            rect.x + 32.0
        } else if warning.is_none() {
            x
        } else {
            rect.x + 4.0
        };
        let available = rect.x + rect.width - if stacked_icon && secondary_icon { 26.0 } else { 4.0 } - left;
        let unit_original = match &data.measure_text {
            Some(measure) => measure(&original, 1.0),
            None => utf16_len(&original) * 8.0,
        };
        let text = if warning == Some("LOW AMMO WARNING") && unit_original * minimum_text_scale > available {
            "LOW AMMO".to_string()
        } else {
            original
        };
        let unit_width = measure_unit(&data.measure_text, &text);
        let scale = minimum_text_scale.max((text_scale * 0.9).min(available / unit_width.max(1.0)));
        let measure_scaled = |value: &str| -> f32 {
            (match &data.measure_text {
                Some(measure) => measure(value, 1.0),
                None => value.chars().count() as f32 * 8.0,
            }) * scale
        };
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in text.split(' ') {
            let next = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && measure_scaled(&next) > available {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = next;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        let maximum_lines = 1_usize.max(((rect.height - rows.label_top - 4.0) / (cap_height * scale)).floor() as usize);
        for (index, value) in lines.iter().take(maximum_lines).enumerate() {
            let mut visible = value.clone();
            let truncated =
                (index == maximum_lines - 1 && lines.len() > maximum_lines) || measure_scaled(&visible) > available;
            if truncated {
                let mut characters: Vec<char> = visible.chars().collect();
                loop {
                    let candidate: String = characters.iter().collect::<String>() + "…";
                    if characters.is_empty() || measure_scaled(&candidate) <= available {
                        visible = candidate;
                        break;
                    }
                    characters.pop();
                }
            }
            commands.push(UiDrawCommand::Text {
                origin: Vec2 {
                    x: left,
                    y: rect.y + rows.label_top + index as f32 * 8.0 * scale - top * scale,
                },
                text: visible,
                font: skin.font.clone(),
                scale,
                color,
                align: TextAlign::Left,
                shadow: true,
            });
        }
    }
    let ammo_aspect = data.ammo_aspect;
    let ammo_width: f32 = 18.0_f32.min(18.0 * ammo_aspect);
    if data.ammo_icon.is_some() && data.weapon_icon.is_some() && finite_count.is_some() {
        if let Some(ammo_icon) = data.ammo_icon.as_ref() {
            commands.push(UiDrawCommand::Image {
                rect: Rect {
                    x: rect.x + rect.width - 22.0,
                    y: rect.y + if stacked_icon { rows.label_top } else { 4.0 },
                    width: ammo_width,
                    height: ammo_width / ammo_aspect,
                },
                resource: ammo_icon.clone(),
                tex_coords: FULL_UV,
                color: WHITE,
            });
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::common::skin::default_ui_skin;
    use crate::ui::types::{ContentId, ItemId, ProviderRef};

    fn font() -> ResourceId {
        ResourceId::new("resource:test:font").unwrap()
    }

    fn icon(name: &str) -> ResourceId {
        ResourceId::new(&format!("resource:test:{name}")).unwrap()
    }

    fn status(provider: &str, label: &str, ammo: WeaponAmmo) -> WeaponHudStatus {
        WeaponHudStatus {
            source: ProviderRef {
                provider: provider.to_string(),
                content: ContentId::new("content:test:game"),
            },
            item: ItemId::new("item:test:weapon"),
            label: label.to_string(),
            ammo,
        }
    }

    fn finite(count: i32, has_ammo_to_start: bool, low: bool) -> WeaponAmmo {
        WeaponAmmo::Finite {
            item: ItemId::new("item:test:shells"),
            count,
            has_ammo_to_start,
            low,
        }
    }

    fn hud(status: WeaponHudStatus) -> CommonWeaponHud {
        CommonWeaponHud {
            status,
            warning: ArsenalAmmoWarning::None,
            weapon_icon: Some(icon("weapon")),
            ammo_icon: Some(icon("ammo")),
            icon_aspect: 1.0,
            ammo_aspect: 1.0,
            native_status: false,
            measure_text: None,
        }
    }

    fn text_of(command: &UiDrawCommand) -> (Vec2, String, f32, Vec4) {
        match command {
            UiDrawCommand::Text {
                origin,
                text,
                scale,
                color,
                align,
                shadow,
                ..
            } => {
                assert_eq!(*align, TextAlign::Left);
                assert!(*shadow);
                (*origin, text.clone(), *scale, *color)
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    fn image_of(command: &UiDrawCommand) -> (Rect, ResourceId) {
        match command {
            UiDrawCommand::Image {
                rect,
                resource,
                tex_coords,
                color,
            } => {
                assert_eq!(*tex_coords, FULL_UV);
                assert_eq!(*color, WHITE);
                (*rect, resource.clone())
            }
            other => panic!("expected image, got {other:?}"),
        }
    }

    #[test]
    fn status_rows_match_donor_numbers() {
        assert_eq!(
            hud_status_rows(1.0, 8.0),
            StatusRows {
                label_top: 25.0,
                height: 42.0
            }
        );
        assert_eq!(
            hud_status_rows(2.0, 8.0),
            StatusRows {
                label_top: 32.0,
                height: 51.0
            }
        );
        assert_eq!(
            hud_status_rows(1.0, 10.0),
            StatusRows {
                label_top: 25.0,
                height: 42.0
            }
        );
    }

    #[test]
    fn compact_strip_draws_count_only_when_healthy() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 32.0,
        };
        let data = hud(status("q1:quake", "Shotgun", finite(12, true, false)));
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 1.0);
        assert_eq!(commands.len(), 3);
        assert_eq!(
            commands[0],
            UiDrawCommand::Fill {
                rect,
                color: skin.colors.panel
            }
        );
        let (image_rect, resource) = image_of(&commands[1]);
        assert_eq!(resource, icon("weapon"));
        assert_eq!(
            image_rect,
            Rect {
                x: 4.0,
                y: 4.0,
                width: 24.0,
                height: 24.0
            }
        );
        let (origin, text, scale, color) = text_of(&commands[2]);
        assert_eq!(text, "12");
        assert_eq!(scale, 1.0);
        assert_eq!(color, skin.colors.text);
        assert_eq!(origin, Vec2 { x: 32.0, y: 4.0 });
    }

    #[test]
    fn compact_strip_shortens_low_warning_and_uses_accent() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 32.0,
        };
        let mut data = hud(status("q2:quake2", "Blaster", finite(3, true, true)));
        data.warning = ArsenalAmmoWarning::Low;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 1.0);
        assert_eq!(commands.len(), 4);
        let (_, count, _, color) = text_of(&commands[2]);
        assert_eq!((count, color), ("3".to_string(), skin.colors.accent));
        let (origin, text, scale, _) = text_of(&commands[3]);
        assert_eq!(text, "LOW AMMO");
        assert_eq!(scale, 1.0);
        assert_eq!(origin, Vec2 { x: 32.0, y: 18.0 });
    }

    #[test]
    fn compact_strip_trims_out_of_ammo_to_fit() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 32.0,
        };
        let mut data = hud(status("q1:quake", "Shotgun", finite(0, false, false)));
        data.warning = ArsenalAmmoWarning::Empty;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 1.0);
        assert_eq!(commands.len(), 4);
        let (_, count, _, color) = text_of(&commands[2]);
        assert_eq!((count, color), ("0".to_string(), skin.colors.accent));
        let (_, text, _, _) = text_of(&commands[3]);
        assert_eq!(text, "OUT");
    }

    #[test]
    fn compact_strip_suppresses_q3_provider_warning_but_keeps_accent() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 32.0,
        };
        let data = hud(status("q3:arena", "Gauntlet", finite(0, false, false)));
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 1.0);
        assert_eq!(commands.len(), 3);
        let (_, text, _, color) = text_of(&commands[2]);
        assert_eq!(text, "0");
        assert_eq!(color, skin.colors.accent);
    }

    #[test]
    fn compact_strip_shows_label_for_unmetered_without_icons() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 32.0,
        };
        let mut data = hud(status("q1:quake", "Axe", WeaponAmmo::Unmetered));
        data.weapon_icon = None;
        data.ammo_icon = None;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 1.0);
        assert_eq!(commands.len(), 2);
        let (origin, text, _, color) = text_of(&commands[1]);
        assert_eq!(text, "Axe");
        assert_eq!(color, skin.colors.text);
        assert_eq!(origin, Vec2 { x: 14.0, y: 24.0 });
    }

    #[test]
    fn full_panel_draws_icon_count_and_ammo_icon() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 64.0,
        };
        let data = hud(status("q1:quake", "Shotgun", finite(30, true, false)));
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 4);
        assert_eq!(
            commands[0],
            UiDrawCommand::Fill {
                rect,
                color: skin.colors.panel
            }
        );
        let (image_rect, resource) = image_of(&commands[1]);
        assert_eq!(resource, icon("weapon"));
        assert_eq!(
            image_rect,
            Rect {
                x: 12.0,
                y: 5.0,
                width: 32.0,
                height: 32.0
            }
        );
        let (origin, text, scale, color) = text_of(&commands[2]);
        assert_eq!((text, scale, color), ("30".to_string(), 1.5, skin.colors.text));
        assert_eq!(origin, Vec2 { x: 58.0, y: 4.0 });
        let (ammo_rect, resource) = image_of(&commands[3]);
        assert_eq!(resource, icon("ammo"));
        assert_eq!(
            ammo_rect,
            Rect {
                x: 178.0,
                y: 4.0,
                width: 18.0,
                height: 18.0
            }
        );
    }

    #[test]
    fn full_panel_draws_out_of_ammo_label_in_accent() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 64.0,
        };
        let mut data = hud(status("q1:quake", "Shotgun", finite(0, false, false)));
        data.warning = ArsenalAmmoWarning::Empty;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 5);
        let (image_rect, _) = image_of(&commands[1]);
        assert_eq!(
            image_rect,
            Rect {
                x: 18.0,
                y: 5.0,
                width: 20.0,
                height: 20.0
            }
        );
        let (_, count, _, _) = text_of(&commands[2]);
        assert_eq!(count, "0");
        let (origin, text, scale, color) = text_of(&commands[3]);
        assert_eq!(text, "OUT OF AMMO");
        assert_eq!(scale, 0.9);
        assert_eq!(color, skin.colors.accent);
        assert_eq!(origin, Vec2 { x: 4.0, y: 25.0 });
    }

    #[test]
    fn full_panel_stacks_icon_when_count_is_wide() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 64.0,
        };
        let data = hud(status("q1:quake", "Nails", finite(888, true, false)));
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 4);
        let (image_rect, _) = image_of(&commands[1]);
        assert_eq!(
            image_rect,
            Rect {
                x: 4.0,
                y: 25.0,
                width: 24.0,
                height: 24.0
            }
        );
        let (origin, text, scale, _) = text_of(&commands[2]);
        assert_eq!(text, "888");
        assert_eq!(scale, 1.5);
        assert_eq!(origin, Vec2 { x: 8.0, y: 4.0 });
        let (ammo_rect, _) = image_of(&commands[3]);
        assert_eq!(
            ammo_rect,
            Rect {
                x: 78.0,
                y: 25.0,
                width: 18.0,
                height: 18.0
            }
        );
    }

    #[test]
    fn full_panel_draws_label_for_unmetered_without_icons() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 64.0,
        };
        let mut data = hud(status("q1:quake", "Lightning Gun", WeaponAmmo::Unmetered));
        data.weapon_icon = None;
        data.ammo_icon = None;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 2);
        let (origin, text, scale, color) = text_of(&commands[1]);
        assert_eq!(text, "Lightning Gun");
        assert_eq!(scale, 0.9);
        assert_eq!(color, skin.colors.text);
        assert_eq!(origin, Vec2 { x: 8.0, y: 25.0 });
    }

    #[test]
    fn full_panel_wraps_long_labels_word_by_word() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 64.0,
        };
        let mut data = hud(status(
            "q1:quake",
            "Super Shotgun Of Ultimate Destruction",
            WeaponAmmo::Unmetered,
        ));
        data.weapon_icon = None;
        data.ammo_icon = None;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 3);
        let (first_origin, first, first_scale, _) = text_of(&commands[1]);
        assert_eq!(first, "Super Shotgun Of Ultimate");
        assert_eq!(first_scale, 0.5);
        assert_eq!(first_origin, Vec2 { x: 8.0, y: 25.0 });
        let (second_origin, second, second_scale, _) = text_of(&commands[2]);
        assert_eq!(second, "Destruction");
        assert_eq!(second_scale, 0.5);
        assert_eq!(second_origin, Vec2 { x: 8.0, y: 29.0 });
    }

    #[test]
    fn full_panel_ellipsizes_when_lines_exceed_maximum() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 33.0,
        };
        let mut data = hud(status(
            "q1:quake",
            "Alpha Bravo Charlie Delta Echo Foxtrot",
            WeaponAmmo::Unmetered,
        ));
        data.weapon_icon = None;
        data.ammo_icon = None;
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 2);
        let (origin, text, scale, _) = text_of(&commands[1]);
        assert_eq!(text, "Alpha Bravo Charlie Delta…");
        assert_eq!(scale, 0.5);
        assert_eq!(origin, Vec2 { x: 8.0, y: 25.0 });
    }

    #[test]
    fn full_panel_uses_custom_measurer() {
        let skin = default_ui_skin(&font());
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 64.0,
        };
        let mut data = hud(status("q1:quake", "Shotgun", finite(30, true, false)));
        data.measure_text = Some(Rc::new(|text: &str, scale: f32| {
            text.chars().count() as f32 * 4.0 * scale
        }));
        let commands = draw_weapon_hud(&data, &rect, &skin, 1.0, 0.5);
        assert_eq!(commands.len(), 4);
        let (_, text, scale, _) = text_of(&commands[2]);
        assert_eq!(text, "30");
        assert_eq!(scale, 1.5);
    }
}
