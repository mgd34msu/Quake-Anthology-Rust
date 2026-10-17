use super::*;
use crate::model_assets::{IndexedSkinError, SkinSelection};
use qa_core::text::{FixedText, info_value};
use qa_render::material::resources::{
    ImageRole, ImageUse, IndexedSkinLayout, IndexedSkinTranslation,
};

#[derive(Clone, Default)]
pub(super) struct IndexedPlayerOptions {
    base: FixedText<128>,
    all: FixedText<128>,
    player_mip: i32,
    no_colors: bool,
}
impl IndexedPlayerOptions {
    fn same(&self, other: &Self) -> bool {
        self.base.as_bytes() == other.base.as_bytes()
            && self.all.as_bytes() == other.all.as_bytes()
            && self.player_mip == other.player_mip
            && self.no_colors == other.no_colors
    }
}

pub(crate) struct PlayerSkinViews {
    base: qa_console::cvars::View,
    player_mip: qa_console::cvars::View,
    no_colors: qa_console::cvars::View,
    all: FixedText<128>,
}
impl PlayerSkinViews {
    pub(crate) fn bind(cvars: &qa_console::cvars::Cvars) -> Result<Self, String> {
        let context = qa_console::views::Context {
            source: RuleSetId::QuakeWorld,
            ..Default::default()
        };
        Ok(Self {
            base: cvars.bind("baseskin", context).ok_or("missing baseskin")?,
            player_mip: cvars
                .bind("gl_playermip", context)
                .ok_or("missing gl_playermip")?,
            no_colors: cvars
                .bind("gl_nocolors", context)
                .ok_or("missing gl_nocolors")?,
            all: Default::default(),
        })
    }
    pub(crate) fn set_all(&mut self, name: &[u8]) {
        let length = name.len().min(FixedText::<128>::CAPACITY - 1);
        let _ = self.all.set_bytes(&name[..length]);
    }
    pub(crate) fn update(
        &self,
        cvars: &qa_console::cvars::Cvars,
        models: &mut ClientModelBindings,
    ) {
        let Some(players) = &mut models.players else {
            return;
        };
        if players.source != RuleSetId::QuakeWorld {
            return;
        }
        let mut options = IndexedPlayerOptions {
            all: self.all.clone(),
            player_mip: cvars.integer_view(self.player_mip).unwrap_or(0),
            no_colors: cvars.integer_view(self.no_colors).unwrap_or(0) != 0,
            ..Default::default()
        };
        if let Ok(base) = cvars.read(self.base) {
            let bytes = base.as_str().as_bytes();
            let _ = options.base.set_bytes(&bytes[..bytes.len().min(127)]);
        }
        if !players.indexed_options.same(&options) {
            players.indexed_options = options;
            players.dirty = true;
            players.cold_dirty = true;
        }
    }
}

fn skin_name(bytes: &[u8]) -> FixedText<15> {
    let valid = !bytes.is_empty()
        && !bytes.starts_with(b".")
        && !bytes.windows(2).any(|pair| pair == b"..");
    let bytes = if valid { bytes } else { b"base" };
    // Skin_Find uses COM_StripExtension's first dot, then its 15-byte name.
    let end = bytes
        .iter()
        .position(|&byte| byte == b'.')
        .unwrap_or(bytes.len())
        .min(15);
    let mut name = FixedText::default();
    let _ = name.set_bytes(&bytes[..end]);
    name
}

impl PlayerBindings {
    pub(super) fn refresh_indexed(
        &mut self,
        configs: &ConfigStrings,
        assets: &mut ClientAssets<'_>,
    ) -> u32 {
        self.indexed_model = self.models.range().find_map(|index| {
            let name = configs.config(index)?.text;
            name.iter()
                .copied()
                .map(qa_core::names::path_byte)
                .eq(b"progs/player.mdl".iter().copied())
                .then_some((index - self.models.first as usize) as u32)
        });
        if self.options.no_skins == 1 || self.indexed_model.is_none() {
            for row in &mut self.bodies {
                row.indexed = None;
            }
            self.dirty = false;
            return 0;
        }
        let ClientAssets::Prepared {
            owner,
            profile,
            vfs,
            preparer,
            ui_fullscreen,
            published,
        } = assets
        else {
            self.cold_dirty |= matches!(assets, ClientAssets::Loading { .. });
            self.dirty = false;
            return 0;
        };
        let Some(registration) = owner
            .catalog()
            .file_handle(b"progs/player.mdl")
            .and_then(|handle| owner.catalog().renderer_registration(handle, *profile))
        else {
            self.dirty = false;
            return 0;
        };
        let Some((coordinate_extent, palette)) = owner.catalog().native_skin(registration) else {
            self.dirty = false;
            return 0;
        };
        let base = skin_name(self.indexed_options.base.as_bytes());
        let upload_extent = [512, 256].map(|size: u32| {
            (size.min(profile.image_settings.max_dimension)
                >> self.indexed_options.player_mip.clamp(0, 31))
            .max(1)
        });
        let mut rejected = 0;
        for (slot, binding) in self.bodies[..self.rows.count as usize]
            .iter_mut()
            .enumerate()
        {
            binding.indexed = None;
            let Some(info) = self.rows.get(slot).and_then(|index| configs.config(index)) else {
                continue;
            };
            if info.text.is_empty() {
                continue;
            }
            let requested =
                info_value(info.text, b"skin", false).map_or(&b""[..], |range| &info.text[range]);
            let selected = skin_name(if !self.indexed_options.all.as_bytes().is_empty() {
                self.indexed_options.all.as_bytes()
            } else if !requested.is_empty() {
                requested
            } else {
                self.indexed_options.base.as_bytes()
            });
            let native = binding.metadata;
            let colors = [native.top_color, native.bottom_color];
            let translation = IndexedSkinTranslation {
                palette,
                target: profile.palette,
                indexed_colors: Some(colors.map(|color| {
                    if (0..=13).contains(&color) {
                        color as u8
                    } else {
                        13
                    }
                })),
                rgba_colors: Some(colors.map(|color| color.clamp(0, 13) as u8)),
                crop: Some([0, 0, 296, 194]),
                coordinate_extent,
                upload_extent: Some(upload_extent),
                usage: ImageUse {
                    role: ImageRole::Skin,
                    sampler: qa_render::assets::Sampler {
                        mipmaps: false,
                        ..Default::default()
                    },
                    allow_picmip: false,
                },
            };
            let layout = IndexedSkinLayout {
                storage: Some([320, 200]),
                padding: 0,
                omit_last_row: true,
            };
            let mut prepare = |name: &str| {
                let path = player_path(&["skins/", name, ".pcx"]).ok_or_else(|| {
                    IndexedSkinError::Rejected("player skin path capacity".into())
                })?;
                owner.prepare_indexed_skin(
                    vfs,
                    path.as_str(),
                    *profile,
                    *ui_fullscreen,
                    layout,
                    translation,
                    |assets, choices| preparer.prepare_model_assets(assets, choices),
                )
            };
            let result = match prepare(selected.as_str()) {
                Err(IndexedSkinError::Missing) if selected.as_bytes() != base.as_bytes() => {
                    prepare(base.as_str())
                }
                result => result,
            };
            match result {
                Ok(selection) => {
                    binding.indexed = Some(selection);
                    **published = true;
                }
                Err(IndexedSkinError::Missing) => {}
                Err(IndexedSkinError::Rejected(_)) => rejected += 1,
            }
        }
        self.dirty = false;
        rejected
    }

    fn indexed_skin(&self, ordinal: u32, colormap: u32, software: bool) -> Option<SkinSelection> {
        if self.indexed_model != Some(ordinal) || (!software && self.indexed_options.no_colors) {
            return None;
        }
        let peer = usize::try_from(colormap.checked_sub(1)?).ok()?;
        if peer >= self.rows.count as usize {
            return None;
        }
        self.bodies.get(peer)?.indexed
    }
}

impl ClientModelBindings {
    pub(in crate::client_scene) fn indexed_skin(
        &self,
        reference: ModelRef,
        colormap: u32,
        software: bool,
    ) -> Option<SkinSelection> {
        if reference.namespace != ResourceNamespace::Client(self.client) {
            return None;
        }
        self.players
            .as_ref()?
            .indexed_skin(reference.ordinal, colormap, software)
    }
}
