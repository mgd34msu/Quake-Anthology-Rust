//! CLIENT-owned display leases and configstrings feed the shared 2D scene.
mod pictures;
use super::{
    console::FontAssets,
    text::{BitmapFont, TextStats},
};
use qa_console::presentation::{Layout, Style};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    events::TextStore,
    primitives::{HudState, RuleSetId, ThinkTime},
    sys_events::EventTime,
    text::atoi_prefix,
};
use qa_formats::entities::{EntitySyntax, EntityTokens};
use qa_gameplay::rules::presentation::{center_y, lightstyle, text_rules};
use qa_render::{
    Assets, CpuPresentation, Frame, LightStyle, Refdef, Viewport, surface_cache::IndexedLighting,
};

const MAX_GLYPHS: usize = 2048;
#[derive(Clone, Copy)]
pub struct HudView<'a> {
    pub hud: &'a HudState,
    pub texts: &'a TextStore,
    pub configs: &'a ConfigStrings,
    pub layout: ConfigLayout,
    pub values: &'a qa_ui::values::ValueLayout,
    pub localization: &'a qa_content::localization::Catalog,
    pub rules: RuleSetId,
    pub time: EventTime,
    pub style_time: ThinkTime,
    pub server_frame: u32,
    pub show_text: bool,
}
impl HudView<'_> {
    /// Source data keeps its native lightstyle arithmetic. CPU receives these
    /// copied scales; updating GL atlas texels remains a separate consumer.
    pub fn apply_lightstyles(self, refdef: &mut Refdef) -> HudStats {
        let mut stats = HudStats::default();
        for ordinal in 0..refdef.lightstyles.len() {
            let Some(index) = self.layout.style_index(ordinal) else {
                break;
            };
            let Some(config) = self.configs.config(index) else {
                stats.rejected += 1;
                continue;
            };
            if let Some(sample) = lightstyle(self.layout.rules, config.text, self.style_time) {
                refdef.lightstyles[ordinal] = LightStyle {
                    rgb: sample.rgb,
                    indexed_scale: sample.indexed_scale,
                };
                stats.styles += 1;
                stats.unsupported += u32::from(sample.interpolation_unimplemented);
            }
        }
        stats
    }
    fn stat(self, index: usize) -> Option<i32> {
        let binding = self.values.native(self.layout.rules).stats.get(index)?;
        Some(self.hud.values.get(binding.id)?.as_integer())
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct HudStats {
    pub glyphs: u32,
    pub pictures: u32,
    pub missing_picture: u32,
    pub styles: u32,
    pub missing_font: u32,
    pub non_native_font: u32,
    pub expired: u32,
    pub unsupported: u32,
    pub rejected: u32,
}
impl std::ops::AddAssign for HudStats {
    fn add_assign(&mut self, rhs: Self) {
        self.glyphs += rhs.glyphs;
        self.pictures += rhs.pictures;
        self.missing_picture += rhs.missing_picture;
        self.styles += rhs.styles;
        self.missing_font += rhs.missing_font;
        self.non_native_font += rhs.non_native_font;
        self.expired += rhs.expired;
        self.unsupported += rhs.unsupported;
        self.rejected += rhs.rejected;
    }
}
impl HudStats {
    fn text(&mut self, stats: TextStats) {
        self.glyphs += stats.glyphs;
        self.rejected += stats.rejected;
    }
}
pub struct HudAssets {
    rules: RuleSetId,
    font: FontAssets,
    notify: Layout,
    pictures: pictures::Pictures,
}
impl HudAssets {
    pub fn from_font(rules: RuleSetId, font: FontAssets, width: u32) -> Result<Self, String> {
        let rules_data = text_rules(rules);
        let notify = Layout::load(
            (width / 8).saturating_sub(2).max(2) as usize,
            4,
            Style {
                colors: rules_data.colors,
                cursor_period_ms: 256,
                blink_cursor: false,
            },
        )
        .map_err(str::to_owned)?;
        Ok(Self {
            rules,
            font,
            notify,
            pictures: Default::default(),
        })
    }
    pub fn load(
        vfs: &qa_content::vfs::Vfs,
        assets: &mut Assets,
        rules: RuleSetId,
        font: FontAssets,
        settings: qa_render::material::resources::ImageSettings,
        width: u32,
    ) -> Result<Self, String> {
        let mut hud = Self::from_font(rules, font, width)?;
        hud.pictures = pictures::Pictures::load(vfs, assets, rules, font, settings)?;
        Ok(hud)
    }
    pub fn register_picture(
        &mut self,
        assets: &mut Assets,
        path: &str,
        material: qa_render::MaterialId,
    ) -> Result<(), String> {
        self.pictures.register(assets, path, material)
    }
    pub fn command_capacity(&self) -> usize {
        MAX_GLYPHS + 1
    }
    pub fn append(
        &mut self,
        frame: &mut Frame,
        assets: &Assets,
        view: HudView<'_>,
        refdef: Refdef,
    ) -> HudStats {
        let mut stats = HudStats::default();
        if self.rules != view.rules {
            stats.rejected += 1;
            return stats;
        }
        let font = self.font.font;
        stats.missing_font += u32::from(font.is_none());
        stats.non_native_font += u32::from(!self.font.native);
        frame.clear_scene();
        let ui = Refdef {
            viewport: refdef.viewport,
            time_ms: refdef.time_ms,
            cpu_presentation: self.font.palette.map_or(CpuPresentation::Rgb, |palette| {
                CpuPresentation::Indexed {
                    palette,
                    lighting: IndexedLighting::Gray,
                    ambient: 0,
                    fullbright: true,
                }
            }),
            ..Refdef::default()
        };
        if !frame.render_scene(ui, &[], assets) {
            stats.rejected += 1;
            return stats;
        }
        let viewport = refdef.viewport;
        let mut remaining = MAX_GLYPHS;
        let now = view.time.seconds();
        let chunks = view.hud.notify.iter().filter_map(|line| {
            let line = line.as_ref()?;
            (view.show_text && line.until > now)
                .then(|| view.texts.get(line.text.id()))
                .flatten()
        });
        stats.expired += view
            .hud
            .notify
            .iter()
            .filter(|line| line.as_ref().is_some_and(|line| line.until <= now))
            .count() as u32;
        if let Some(font) = font {
            if self
                .notify
                .prepare_text_chunks(
                    chunks,
                    (viewport.width / 8).saturating_sub(2).max(2) as usize,
                )
                .is_err()
            {
                stats.rejected += 1;
            } else {
                for (row, (_, cells)) in self.notify.lines().enumerate() {
                    let size = cells.len().min(remaining);
                    stats.text(font.cells(
                        frame,
                        &cells[..size],
                        i64::from(viewport.x) + 8,
                        i64::from(viewport.y) + row as i64 * i64::from(font.height),
                        viewport,
                    ));
                    stats.rejected += u32::from(size != cells.len());
                    remaining -= size;
                }
            }
            let policy = text_rules(view.rules);
            if let Some(line) = &view.hud.centerprint
                && view.show_text
            {
                if line.until <= now {
                    stats.expired += 1;
                } else if !policy.center {
                    stats.unsupported += 1;
                } else if let Some(text) = view.texts.get(line.text.id()) {
                    let lines = 1 + text.iter().filter(|&&byte| byte == b'\n').count();
                    let mut y = viewport.y as i32 + center_y(viewport.height, lines);
                    for line in text.split(|&byte| byte == b'\n') {
                        let line = &line[..line.len().min(40)];
                        let x =
                            viewport.x as i32 + (viewport.width as i32 - line.len() as i32 * 8) / 2;
                        append_bytes(
                            Some(font),
                            frame,
                            line,
                            0,
                            x,
                            y,
                            viewport,
                            &mut remaining,
                            &mut stats,
                        );
                        y += 8;
                    }
                } else {
                    stats.rejected += 1;
                }
            }
        }
        let policy = text_rules(view.rules);
        if policy.layouts
            && matches!(
                view.layout.rules,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
            )
        {
            if let Some(text) = view
                .layout
                .statusbar()
                .and_then(|index| view.configs.config(index))
            {
                append_layout(
                    font,
                    &self.pictures,
                    assets,
                    frame,
                    text.text,
                    view,
                    viewport,
                    &mut remaining,
                    &mut stats,
                );
            }
            let flags = policy
                .layout_stat
                .and_then(|index| view.stat(index))
                .unwrap_or(0);
            stats.unsupported += u32::from(flags & 2 != 0);
            if let Some(lease) = &view.hud.layout_text
                && flags & 1 != 0
            {
                if let Some(text) = view.texts.get(lease.id()) {
                    append_layout(
                        font,
                        &self.pictures,
                        assets,
                        frame,
                        text,
                        view,
                        viewport,
                        &mut remaining,
                        &mut stats,
                    );
                } else {
                    stats.rejected += 1;
                }
            }
        } else if view.hud.layout_text.is_some() {
            stats.unsupported += 1;
        }
        stats
    }
}
fn append_bytes(
    font: Option<BitmapFont>,
    frame: &mut Frame,
    text: &[u8],
    xor: u8,
    x: i32,
    y: i32,
    viewport: Viewport,
    remaining: &mut usize,
    stats: &mut HudStats,
) {
    let Some(font) = font else {
        return;
    };
    let count = text.len().min(*remaining);
    stats.text(font.bytes(
        frame,
        &text[..count],
        xor,
        i64::from(x),
        i64::from(y),
        viewport,
    ));
    stats.rejected += u32::from(count != text.len());
    *remaining -= count;
}
fn append_layout(
    font: Option<BitmapFont>,
    pictures: &pictures::Pictures,
    assets: &Assets,
    frame: &mut Frame,
    text: &[u8],
    view: HudView<'_>,
    viewport: Viewport,
    remaining: &mut usize,
    stats: &mut HudStats,
) {
    let mut tokens = EntityTokens::new(text, EntitySyntax::Quake2);
    let mut x = 0i32;
    let mut y = 0i32;
    loop {
        let command = match tokens.next_token() {
            Ok(Some(command)) => command,
            Ok(None) => break,
            Err(_) => {
                stats.rejected += 1;
                break;
            }
        };
        let result: Result<(), ()> = (|| {
            let mut next = || tokens.next_token().map_err(|_| ())?.ok_or(());
            match command {
                b"xl" => x = atoi_prefix(next()?),
                b"xr" => x = (viewport.width as i32).saturating_add(atoi_prefix(next()?)),
                b"xv" => x = (viewport.width as i32 / 2 - 160).saturating_add(atoi_prefix(next()?)),
                b"yt" => y = atoi_prefix(next()?),
                b"yb" => y = (viewport.height as i32).saturating_add(atoi_prefix(next()?)),
                b"yv" => {
                    y = (viewport.height as i32 / 2 - 120).saturating_add(atoi_prefix(next()?))
                }
                b"string" | b"string2" | b"cstring" | b"cstring2" => {
                    let text = next()?;
                    append_layout_text(
                        font,
                        frame,
                        text,
                        &[],
                        command,
                        view,
                        x,
                        y,
                        viewport,
                        remaining,
                        stats,
                    )?;
                }
                b"stat_string" | b"loc_stat_string" | b"loc_stat_rstring" | b"loc_stat_cstring"
                | b"loc_stat_cstring2" => {
                    let ordinal = usize::try_from(atoi_prefix(next()?)).map_err(|_| ())?;
                    let index = usize::try_from(view.stat(ordinal).ok_or(())?).map_err(|_| ())?;
                    let text = view.configs.config(index).ok_or(())?.text;
                    append_layout_text(
                        font,
                        frame,
                        text,
                        &[],
                        command,
                        view,
                        x,
                        y,
                        viewport,
                        remaining,
                        stats,
                    )?;
                }
                b"loc_string" | b"loc_string2" | b"loc_rstring" | b"loc_rstring2"
                | b"loc_cstring" | b"loc_cstring2" => {
                    let count = usize::try_from(atoi_prefix(next()?)).map_err(|_| ())?;
                    if count > 7 {
                        return Err(());
                    }
                    let base = next()?;
                    let mut arguments = [b"".as_slice(); 7];
                    for argument in &mut arguments[..count] {
                        *argument = next()?;
                    }
                    append_layout_text(
                        font,
                        frame,
                        base,
                        &arguments[..count],
                        command,
                        view,
                        x,
                        y,
                        viewport,
                        remaining,
                        stats,
                    )?;
                }
                b"if" => {
                    let ordinal = usize::try_from(atoi_prefix(next()?)).map_err(|_| ())?;
                    if view.stat(ordinal).ok_or(())? == 0 {
                        while next()? != b"endif" {}
                    }
                }
                b"endif" => (),
                b"pic" => {
                    let ordinal = usize::try_from(atoi_prefix(next()?)).map_err(|_| ())?;
                    let image = u32::try_from(view.stat(ordinal).ok_or(())?).map_err(|_| ())?;
                    let images = view.layout.resources[2];
                    if image >= images.count {
                        return Err(());
                    }
                    let name = view
                        .configs
                        .config((images.first + image) as usize)
                        .ok_or(())?
                        .text;
                    pictures.picture(
                        assets,
                        std::str::from_utf8(name).map_err(|_| ())?,
                        frame,
                        (viewport.x as i32).saturating_add(x),
                        (viewport.y as i32).saturating_add(y),
                        viewport,
                        remaining,
                        stats,
                    );
                }
                b"picn" => {
                    pictures.picture(
                        assets,
                        std::str::from_utf8(next()?).map_err(|_| ())?,
                        frame,
                        (viewport.x as i32).saturating_add(x),
                        (viewport.y as i32).saturating_add(y),
                        viewport,
                        remaining,
                        stats,
                    );
                }
                b"num" => {
                    let width = atoi_prefix(next()?);
                    let ordinal = usize::try_from(atoi_prefix(next()?)).map_err(|_| ())?;
                    pictures.number(
                        frame,
                        (viewport.x as i32).saturating_add(x),
                        (viewport.y as i32).saturating_add(y),
                        width,
                        view.stat(ordinal).ok_or(())?,
                        false,
                        viewport,
                        remaining,
                        stats,
                    );
                }
                b"hnum" | b"anum" | b"rnum" => {
                    let (index, flag) = match command {
                        b"hnum" => (1, 1),
                        b"anum" => (3, 4),
                        _ => (5, 2),
                    };
                    let value = view.stat(index).ok_or(())?;
                    let policy =
                        qa_gameplay::rules::presentation::hud_number_rules(view.layout.rules);
                    if (index == 3 && value < 0) || (index == 5 && value < policy.minimum_armor) {
                        return Ok(());
                    }
                    let flash = policy.flash(view.server_frame, view.time.milliseconds());
                    let warning = if index == 1 { 25 } else { 5 };
                    let alternate =
                        (index == 1 && value <= 0) || (index != 5 && value <= warning && flash);
                    if view.stat(15).ok_or(())? & flag != 0 {
                        pictures.picture(
                            assets,
                            "field_3",
                            frame,
                            (viewport.x as i32).saturating_add(x),
                            (viewport.y as i32).saturating_add(y),
                            viewport,
                            remaining,
                            stats,
                        );
                    }
                    pictures.number(
                        frame,
                        (viewport.x as i32).saturating_add(x),
                        (viewport.y as i32).saturating_add(y),
                        3,
                        value,
                        alternate,
                        viewport,
                        remaining,
                        stats,
                    );
                }
                b"client" => {
                    for _ in 0..6 {
                        next()?;
                    }
                    stats.unsupported += 1;
                }
                b"ctf" => {
                    for _ in 0..5 {
                        next()?;
                    }
                    stats.unsupported += 1;
                }
                _ => {
                    stats.unsupported += 1;
                    return Err(());
                }
            }
            Ok(())
        })();
        if result.is_err() {
            stats.rejected += 1;
            break;
        }
    }
}

fn append_layout_text(
    font: Option<BitmapFont>,
    frame: &mut Frame,
    text: &[u8],
    arguments: &[&[u8]],
    command: &[u8],
    view: HudView<'_>,
    x: i32,
    y: i32,
    viewport: Viewport,
    remaining: &mut usize,
    stats: &mut HudStats,
) -> Result<(), ()> {
    let mut localized = qa_formats::localization::LocText::default();
    let text = if command.starts_with(b"loc_") {
        view.localization.format(
            text,
            arguments.len(),
            |index| Ok::<_, ()>(arguments.get(index).copied()),
            &mut localized,
        )?;
        localized.as_bytes()
    } else {
        text
    };
    let centered = matches!(
        command,
        b"cstring"
            | b"cstring2"
            | b"loc_cstring"
            | b"loc_cstring2"
            | b"loc_stat_cstring"
            | b"loc_stat_cstring2"
    );
    let right = matches!(
        command,
        b"loc_rstring" | b"loc_rstring2" | b"loc_stat_rstring"
    );
    let alternate = command.ends_with(b"2");
    if !centered {
        let x = if right {
            x.saturating_sub((text.len() as i32).saturating_mul(8))
        } else {
            x
        };
        append_bytes(
            font,
            frame,
            text,
            if alternate { 128 } else { 0 },
            (viewport.x as i32).saturating_add(x),
            (viewport.y as i32).saturating_add(y),
            viewport,
            remaining,
            stats,
        );
        return Ok(());
    }
    for (row, line) in text.split(|&b| b == b'\n').enumerate() {
        let width = (line.len() as i32).saturating_mul(8);
        let x = x.saturating_add(160 - width / 2);
        append_bytes(
            font,
            frame,
            line,
            if alternate { 128 } else { 0 },
            (viewport.x as i32).saturating_add(x),
            (viewport.y as i32)
                .saturating_add(y)
                .saturating_add(row as i32 * 8),
            viewport,
            remaining,
            stats,
        );
    }
    Ok(())
}
