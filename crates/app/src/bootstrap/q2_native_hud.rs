//! Mounted-picture Quake II HUD with conchars console text.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q2-native-hud.ts`
//! (`NativeQ2HudRendering`, `ApplicationQ2NativeHud`). Layout operations, the scoreboard
//! table, and draw commands come from the ported [`q2_native_hud_operations`], table, and
//! [`UiDrawCommand`] types; content textures arrive through the [`Q2NativeHudAssets`] seam
//! and the donor's async loads are sync through the host. The operations call needs a
//! binding even where the donor passes `undefined`, so prepare uses a no-op binding, and
//! the float layout size folds to the merged integer contract by truncation. Currency
//! checks become a boolean closure that folds the donor's host-thrown staleness into
//! [`Q2NativeHudError::Retired`].

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::text::draw2d::{ImagePicture, PictureAsset, Rect};
use qa_client::ui::hud::q2_native::{
    q2_native_hud_operations, InventoryMode, NativeQ2HudArsenal, NativeQ2HudFrame, NativeQ2HudOperation,
};
use qa_client::ui::hud::q2_rerelease_layout::{
    NativeQ2HudEnvironment, NativeQ2HudTable, Q2HudLocalizeFn, Q2HudMeasureFn,
};
use qa_client::ui::types::{ResourceId, TextAlign, UiDrawCommand, UiDrawContext};
use qa_client::ClientError;
use qa_content::contract::ContentId;
use qa_core::math::{vec2, vec4};
use thiserror::Error;

/// Rendering services for the native HUD (donor `NativeQ2HudRendering`).
///
/// The scoreboard table lives on the HUD itself (the donor swaps the caller's table for
/// its own); everything else is stored as given.
#[derive(Clone)]
pub struct NativeQ2HudRendering {
    /// Whether scaled-font text renders instead of console text.
    pub use_font: bool,
    /// Scaled-font line height in HUD units.
    pub font_line_height: f32,
    /// Text measurer.
    pub measure: Q2HudMeasureFn,
    /// Localizer.
    pub localize: Q2HudLocalizeFn,
    /// Scaled font resource.
    pub font: ResourceId,
    /// Scaled font size.
    pub font_scale: f32,
}

/// One loaded texture (donor provider texture `image` view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2HudTexture {
    /// Image handle.
    pub handle: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Texture loading (donor `ApplicationAssets["provider"]` texture surface).
pub trait Q2NativeHudAssets {
    /// Load a Q2 texture path, or [`None`] when absent.
    fn texture(&mut self, content: &ContentId, path: &str) -> Option<Q2HudTexture>;
    /// Placeholder for missing textures.
    fn missing(&self) -> Q2HudTexture;
}

/// Prepare arguments (donor `prepare` parameters).
pub struct Q2NativeHudPrepare<'a> {
    /// Content to prepare.
    pub content: &'a ContentId,
    /// HUD frame.
    pub frame: &'a NativeQ2HudFrame,
    /// Draw context.
    pub context: &'a UiDrawContext,
    /// HUD scale (donor default `1`).
    pub scale: f32,
    /// Inventory mode (donor default `replace-status`).
    pub mode: InventoryMode,
    /// Currency probe; `&|| true` is the donor default.
    pub assert_current: &'a dyn Fn() -> bool,
    /// Inventory readout, when shown.
    pub arsenal: Option<&'a NativeQ2HudArsenal>,
    /// Rendering services, when scaled fonts apply.
    pub environment: Option<&'a NativeQ2HudRendering>,
}

/// Failure of a native-HUD operation, with donor messages.
#[derive(Debug, Error)]
pub enum Q2NativeHudError {
    /// Prepared media was retired.
    #[error("Native HUD media is retired")]
    Retired,
    /// A font-text op ran without prepared rendering services.
    #[error("Native HUD font was not prepared")]
    FontNotPrepared,
    /// Layout failure.
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Mounted-picture Q2 HUD (donor `ApplicationQ2NativeHud`).
#[derive(Default)]
pub struct ApplicationQ2NativeHud {
    pictures: HashMap<ResourceId, PictureAsset>,
    ids: HashMap<String, ResourceId>,
    content: Option<ContentId>,
    revision: u64,
    environment: Option<NativeQ2HudRendering>,
    table: NativeQ2HudTable,
    prepared_operations: Option<Vec<NativeQ2HudOperation>>,
}

/// Output accumulator shared by the picture and glyph painters.
struct HudDraw<'a> {
    /// Commands built so far.
    out: &'a mut Vec<UiDrawCommand>,
    /// Safe area.
    area: Rect,
    /// HUD scale.
    scale: f32,
}

impl ApplicationQ2NativeHud {
    /// Build an empty HUD.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every picture, id, table row, and prepared op (donor `clear`).
    pub fn clear(&mut self) {
        self.table.rows.clear();
        self.table.columns.clear();
        self.prepared_operations = None;
        self.environment = None;
        self.revision = self.revision.wrapping_add(1);
        self.content = None;
        self.ids.clear();
        self.pictures.clear();
    }

    /// Look up a prepared picture (donor `picture`).
    #[must_use]
    pub fn picture(&self, id: &ResourceId) -> Option<&PictureAsset> {
        self.pictures.get(id)
    }

    /// Fail when the caller retired this media or the HUD reset (donor `current`).
    fn check_current(
        current_revision: u64,
        revision: u64,
        assert_current: &dyn Fn() -> bool,
    ) -> Result<(), Q2NativeHudError> {
        if !assert_current() || revision != current_revision {
            return Err(Q2NativeHudError::Retired);
        }
        Ok(())
    }

    /// Run layout operations with the live scoreboard table.
    fn operations(
        &mut self,
        frame: &NativeQ2HudFrame,
        width: f32,
        height: f32,
        binding: &dyn Fn(&str) -> String,
        mode: InventoryMode,
        arsenal: Option<&NativeQ2HudArsenal>,
    ) -> Result<Vec<NativeQ2HudOperation>, ClientError> {
        let mut environment = self.environment.as_ref().map(|rendering| NativeQ2HudEnvironment {
            table: Some(std::mem::take(&mut self.table)),
            use_font: rendering.use_font,
            font_line_height: rendering.font_line_height,
            measure: Rc::clone(&rendering.measure),
            localize: Rc::clone(&rendering.localize),
        });
        let ops = q2_native_hud_operations(
            frame,
            width as i32,
            height as i32,
            binding,
            mode,
            arsenal,
            environment.as_mut(),
        )?;
        if let Some(environment) = environment.as_mut() {
            self.table = environment.table.take().unwrap_or_default();
        }
        Ok(ops)
    }

    /// Prepare pictures and layout ops (donor `prepare`).
    pub fn prepare(
        &mut self,
        assets: &mut impl Q2NativeHudAssets,
        options: &Q2NativeHudPrepare<'_>,
    ) -> Result<(), Q2NativeHudError> {
        if self.content.as_ref() != Some(options.content) {
            self.clear();
            self.content = Some(options.content.clone());
        }
        let revision = self.revision;
        self.environment = options.environment.cloned();
        Self::check_current(self.revision, revision, options.assert_current)?;
        let area = options.context.binding.safe_area;
        let no_binding = |_: &str| String::new();
        let ops = self.operations(
            options.frame,
            area.width / options.scale,
            area.height / options.scale,
            &no_binding,
            options.mode,
            options.arsenal,
        )?;
        self.prepared_operations = if self.environment.is_none() {
            None
        } else {
            Some(ops.clone())
        };
        Self::check_current(self.revision, revision, options.assert_current)?;
        let mut names = HashSet::from(["conchars".to_string(), "field_3".to_string()]);
        for prefix in ["num", "anum"] {
            for digit in 0..10 {
                names.insert(format!("{prefix}_{digit}"));
            }
            names.insert(format!("{prefix}_minus"));
        }
        for op in &ops {
            match op {
                NativeQ2HudOperation::Picture { name, .. } | NativeQ2HudOperation::SizedPicture { name, .. } => {
                    names.insert(name.clone());
                }
                _ => {}
            }
        }
        let mut names: Vec<String> = names.into_iter().collect();
        names.sort();
        for name in names {
            if self.ids.contains_key(&name) {
                continue;
            }
            let path = if name.starts_with('/') || name.starts_with('\\') {
                name[1..].to_string()
            } else {
                format!("pics/{name}.pcx")
            };
            let mut texture = assets.texture(options.content, &path);
            Self::check_current(self.revision, revision, options.assert_current)?;
            if texture.is_none() && path.starts_with("players/") {
                texture = assets.texture(options.content, "players/male/grunt_i.pcx");
            }
            Self::check_current(self.revision, revision, options.assert_current)?;
            let loaded = texture.unwrap_or_else(|| assets.missing());
            let id = ResourceId::new(&format!("resource:q2-native-hud:{}/{path}", options.content))
                .expect("native HUD resource id");
            self.ids.insert(name, id.clone());
            self.pictures.insert(
                id,
                PictureAsset::Image(ImagePicture {
                    image: loaded.handle,
                    width: loaded.width,
                    height: loaded.height,
                }),
            );
        }
        Ok(())
    }

    /// Draw the HUD (donor `commands`).
    pub fn commands(
        &mut self,
        frame: &NativeQ2HudFrame,
        context: &UiDrawContext,
        scale: f32,
        binding: &dyn Fn(&str) -> String,
        mode: InventoryMode,
        arsenal: Option<&NativeQ2HudArsenal>,
    ) -> Result<Vec<UiDrawCommand>, Q2NativeHudError> {
        let area = context.binding.safe_area;
        let mut out = vec![UiDrawCommand::Clip { rect: Some(area) }];
        let font = self.ids.get("conchars").cloned();
        let ops = match self.prepared_operations.clone() {
            Some(ops) => ops,
            None => self.operations(frame, area.width / scale, area.height / scale, binding, mode, arsenal)?,
        };
        for op in &ops {
            match op {
                NativeQ2HudOperation::Fill {
                    x,
                    y,
                    width,
                    height,
                    color,
                } => {
                    out.push(UiDrawCommand::Fill {
                        rect: Rect {
                            x: area.x + x * scale,
                            y: area.y + y * scale,
                            width: width * scale,
                            height: height * scale,
                        },
                        color: *color,
                    });
                }
                NativeQ2HudOperation::FontText { x, y, text, alternate } => {
                    let environment = self.environment.as_ref().ok_or(Q2NativeHudError::FontNotPrepared)?;
                    out.push(UiDrawCommand::Text {
                        origin: vec2(area.x + x * scale, area.y + y * scale),
                        text: text.clone(),
                        font: environment.font.clone(),
                        scale: environment.font_scale * scale,
                        color: if *alternate {
                            vec4(112.0 / 255.0, 1.0, 52.0 / 255.0, 1.0)
                        } else {
                            vec4(1.0, 1.0, 1.0, 1.0)
                        },
                        align: TextAlign::Left,
                        shadow: true,
                    });
                }
                NativeQ2HudOperation::ArsenalPicture { x, y, resource, aspect } => {
                    let width = 24.0 * aspect.min(1.0);
                    let height = 24.0 / aspect.max(1.0);
                    out.push(UiDrawCommand::Image {
                        rect: Rect {
                            x: area.x + (x + (24.0 - width) / 2.0) * scale,
                            y: area.y + (y + (24.0 - height) / 2.0) * scale,
                            width: width * scale,
                            height: height * scale,
                        },
                        resource: resource.clone(),
                        tex_coords: [vec2(0.0, 0.0), vec2(1.0, 1.0)],
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    });
                }
                NativeQ2HudOperation::Picture {
                    x,
                    y,
                    name,
                    anchor_before,
                } => {
                    let mut draw = HudDraw {
                        out: &mut out,
                        area,
                        scale,
                    };
                    self.push_picture(&mut draw, name, *x, *y, *anchor_before, None);
                }
                NativeQ2HudOperation::SizedPicture {
                    x,
                    y,
                    width,
                    height,
                    name,
                } => {
                    let mut draw = HudDraw {
                        out: &mut out,
                        area,
                        scale,
                    };
                    self.push_picture(&mut draw, name, *x, *y, false, Some((*width, *height)));
                }
                NativeQ2HudOperation::Text {
                    x,
                    y,
                    text,
                    alternate,
                    xor,
                    shadow,
                } => {
                    let Some(font) = &font else {
                        continue;
                    };
                    let alternate = if *alternate { 128 } else { 0 };
                    if *xor {
                        let bytes = text.as_bytes();
                        for (index, byte) in bytes.iter().enumerate() {
                            let mut draw = HudDraw {
                                out: &mut out,
                                area,
                                scale,
                            };
                            Self::push_glyph(
                                &mut draw,
                                font,
                                *x,
                                *y,
                                index,
                                (i32::from(*byte) ^ alternate) & 255,
                                *shadow,
                            );
                        }
                    } else {
                        for (index, unit) in text.encode_utf16().enumerate() {
                            let mut draw = HudDraw {
                                out: &mut out,
                                area,
                                scale,
                            };
                            Self::push_glyph(
                                &mut draw,
                                font,
                                *x,
                                *y,
                                index,
                                (i32::from(unit) | alternate) & 255,
                                *shadow,
                            );
                        }
                    }
                }
            }
        }
        out.push(UiDrawCommand::Clip { rect: None });
        Ok(out)
    }

    /// Draw one HUD picture unless its texture is missing (donor picture branch).
    fn push_picture(
        &self,
        draw: &mut HudDraw<'_>,
        name: &str,
        x: f32,
        y: f32,
        anchor_before: bool,
        size: Option<(f32, f32)>,
    ) {
        let picture = self
            .ids
            .get(name)
            .and_then(|id| self.pictures.get(id).map(|picture| (id, picture)));
        let Some((id, PictureAsset::Image(image))) = picture else {
            return;
        };
        let (width, height) = size.unwrap_or((image.width as f32, image.height as f32));
        draw.out.push(UiDrawCommand::Image {
            rect: Rect {
                x: draw.area.x + (x - (if anchor_before { image.width as f32 + 2.0 } else { 0.0 })) * draw.scale,
                y: draw.area.y + y * draw.scale,
                width: width * draw.scale,
                height: height * draw.scale,
            },
            resource: id.clone(),
            tex_coords: [vec2(0.0, 0.0), vec2(1.0, 1.0)],
            color: vec4(1.0, 1.0, 1.0, 1.0),
        });
    }

    /// Draw one conchars glyph unless it is a space (donor glyph loop).
    fn push_glyph(draw: &mut HudDraw<'_>, font: &ResourceId, x: f32, y: f32, index: usize, code: i32, shadow: bool) {
        if code & 127 == 32 {
            return;
        }
        let column = code & 15;
        let row = code >> 4;
        let glyph = UiDrawCommand::Image {
            rect: Rect {
                x: draw.area.x + (x + index as f32 * 8.0) * draw.scale,
                y: draw.area.y + y * draw.scale,
                width: 8.0 * draw.scale,
                height: 8.0 * draw.scale,
            },
            resource: font.clone(),
            tex_coords: [
                vec2(column as f32 / 16.0, row as f32 / 16.0),
                vec2((column + 1) as f32 / 16.0, (row + 1) as f32 / 16.0),
            ],
            color: vec4(1.0, 1.0, 1.0, 1.0),
        };
        if shadow {
            if let UiDrawCommand::Image {
                mut rect,
                resource,
                tex_coords,
                ..
            } = glyph.clone()
            {
                rect.x += draw.scale;
                rect.y += draw.scale;
                draw.out.push(UiDrawCommand::Image {
                    rect,
                    resource,
                    tex_coords,
                    color: vec4(0.0, 0.0, 0.0, 1.0),
                });
            }
        }
        draw.out.push(glyph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::ui::hud::q2_native::NativeQ2HudFrame;
    use qa_client::ui::types::Q2ProtocolFamily;
    use qa_client::ui::types::{
        ContentId as UiContentId, DopplerSelection, EnvironmentSelection, PresentationSelection, ProviderRef,
        SeatPresentationBinding,
    };
    use qa_core::identity::IdentityOwner;
    use std::collections::BTreeMap;

    struct Stub {
        textures: HashMap<String, Q2HudTexture>,
    }

    impl Q2NativeHudAssets for Stub {
        fn texture(&mut self, _content: &ContentId, path: &str) -> Option<Q2HudTexture> {
            self.textures.get(path).copied()
        }
        fn missing(&self) -> Q2HudTexture {
            Q2HudTexture {
                handle: 0,
                width: 8,
                height: 8,
            }
        }
    }

    fn context() -> (UiDrawContext, IdentityOwner) {
        let owner = IdentityOwner::create("q2-hud").unwrap();
        let provider = |name: &str| ProviderRef {
            provider: name.to_string(),
            content: UiContentId::new("assets"),
        };
        let binding = SeatPresentationBinding {
            seat: owner.seat(0),
            client: owner.client(0, 1),
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
            safe_area: Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
            hud_scale: 1.0,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Disabled,
                environment: EnvironmentSelection::Disabled,
                assets: UiContentId::new("assets"),
                hud: provider("hud"),
                effects: provider("effects"),
                audio: provider("audio"),
            },
        };
        (UiDrawContext { binding, time_ms: 0 }, owner)
    }

    fn frame() -> NativeQ2HudFrame {
        NativeQ2HudFrame {
            protocol: Q2ProtocolFamily::Classic,
            stats: vec![0; 32],
            configstrings: BTreeMap::new(),
            layout: String::new(),
            inventory: Vec::new(),
            player_number: 0,
            server_frame: 0,
            time_ms: 0,
            frame_time_ms: None,
        }
    }

    #[test]
    fn prepare_loads_conchars_and_numbers() {
        let mut hud = ApplicationQ2NativeHud::new();
        let (context, _) = context();
        let mut assets = Stub {
            textures: HashMap::new(),
        };
        assets.textures.insert(
            "pics/conchars.pcx".to_string(),
            Q2HudTexture {
                handle: 7,
                width: 128,
                height: 128,
            },
        );
        let content = ContentId("q2:classic:baseq2:1".to_string());
        let frame = frame();
        let options = Q2NativeHudPrepare {
            content: &content,
            frame: &frame,
            context: &context,
            scale: 1.0,
            mode: InventoryMode::ReplaceStatus,
            assert_current: &|| true,
            arsenal: None,
            environment: None,
        };
        hud.prepare(&mut assets, &options).unwrap();
        let id = ResourceId::new("resource:q2-native-hud:q2:classic:baseq2:1/pics/conchars.pcx").unwrap();
        let PictureAsset::Image(image) = hud.picture(&id).unwrap() else {
            panic!("expected image picture");
        };
        assert_eq!(image.image, 7);
    }

    #[test]
    fn commands_clip_and_draw_text() {
        let mut hud = ApplicationQ2NativeHud::new();
        let (context, _) = context();
        let mut assets = Stub {
            textures: HashMap::new(),
        };
        let content = ContentId("q2:classic:baseq2:1".to_string());
        let frame = frame();
        let options = Q2NativeHudPrepare {
            content: &content,
            frame: &frame,
            context: &context,
            scale: 1.0,
            mode: InventoryMode::ReplaceStatus,
            assert_current: &|| true,
            arsenal: None,
            environment: None,
        };
        hud.prepare(&mut assets, &options).unwrap();
        let binding = |_: &str| String::new();
        let commands = hud
            .commands(&frame, &context, 1.0, &binding, InventoryMode::ReplaceStatus, None)
            .unwrap();
        assert!(matches!(commands.first(), Some(UiDrawCommand::Clip { rect: Some(_) })));
        assert!(matches!(commands.last(), Some(UiDrawCommand::Clip { rect: None })));
    }

    #[test]
    fn retired_media_fails() {
        let mut hud = ApplicationQ2NativeHud::new();
        let (context, _) = context();
        let mut assets = Stub {
            textures: HashMap::new(),
        };
        let content = ContentId("q2:classic:baseq2:1".to_string());
        let frame = frame();
        let options = Q2NativeHudPrepare {
            content: &content,
            frame: &frame,
            context: &context,
            scale: 1.0,
            mode: InventoryMode::ReplaceStatus,
            assert_current: &|| false,
            arsenal: None,
            environment: None,
        };
        let error = hud.prepare(&mut assets, &options).unwrap_err();
        assert!(matches!(error, Q2NativeHudError::Retired));
    }
}
