use super::*;
use crate::model_assets::ModelAssets;
use qa_compat::services::{ModelLookup, RendererRegistration, RendererRequest};
use qa_core::names::compare_folded;
use qa_gameplay::rules::appearance::PlayerModelRules;
use std::fmt::Write;

mod indexed;
pub(crate) use indexed::{PlayerSkinRefresh, PlayerSkinViews};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerModelOptions {
    pub no_skins: i32,
    pub visible_weapons: bool,
}
impl Default for PlayerModelOptions {
    fn default() -> Self {
        Self {
            no_skins: 0,
            visible_weapons: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerMetadata {
    pub user_id: Option<i32>,
    pub colors: Option<u8>,
    pub frags: i16,
    pub top_color: i32,
    pub bottom_color: i32,
}

#[derive(Clone, Copy, Default)]
struct PlayerBinding {
    body: Option<CapturedModel>,
    metadata: PeerMetadata,
}
#[derive(Clone, Copy)]
pub(super) enum IndexedPlayerSkin {
    External(crate::model_assets::SkinSelection),
    Model(crate::model_assets::SkinSelection),
}
impl IndexedPlayerSkin {
    pub(super) fn external(self) -> Option<crate::model_assets::SkinSelection> {
        match self {
            Self::External(selection) => Some(selection),
            Self::Model(_) => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct PlayerBindings {
    source: RuleSetId,
    rows: IndexSpan,
    models: IndexSpan,
    rules: Option<PlayerModelRules>,
    color_rules: Option<qa_gameplay::rules::appearance::PlayerColorRules>,
    options: PlayerModelOptions,
    indexed_options: indexed::IndexedPlayerOptions,
    indexed_model: Option<u32>,
    skin_revision: u64,
    indexed_models: Box<[Option<IndexedPlayerSkin>]>,
    pub(super) dirty: bool,
    pub(super) cold_dirty: bool,
    bodies: Box<[PlayerBinding]>,
    weapons: Box<[Option<CapturedModel>]>,
    weapon_rows: Box<[Option<u32>]>,
    weapon_count: usize,
}
impl PlayerBindings {
    pub(super) fn load(layout: ConfigLayout) -> Option<Self> {
        if layout.players.count == 0 {
            return None;
        }
        let policy = qa_gameplay::rules::appearance::policy(layout.rules);
        let rules = policy.player_models();
        let color_rules = policy.player_colors();
        let count = layout.players.count as usize + 1;
        let weapons = rules.map_or(0, |rules| rules.weapon_limit);
        Some(Self {
            source: layout.rules,
            rows: layout.players,
            models: layout.resources[0],
            rules,
            color_rules,
            options: Default::default(),
            indexed_options: Default::default(),
            indexed_model: None,
            skin_revision: 0,
            indexed_models: if color_rules.is_some() {
                vec![
                    None;
                    (layout.players.count as usize)
                        .checked_mul(layout.resources[0].count as usize)?
                ]
                .into_boxed_slice()
            } else {
                Box::default()
            },
            dirty: true,
            cold_dirty: true,
            bodies: vec![PlayerBinding::default(); count].into_boxed_slice(),
            weapons: vec![None; count.checked_mul(weapons)?].into_boxed_slice(),
            weapon_rows: vec![None; weapons].into_boxed_slice(),
            weapon_count: 1,
        })
    }
    pub(super) fn image_refresh_selections(
        &self,
        catalog: &ModelCatalog,
        profiles: &mut Vec<ModelProfile>,
        worlds: &mut Vec<(WorldId, WorldPresentationId)>,
    ) {
        for capture in self
            .bodies
            .iter()
            .filter_map(|row| row.body.as_ref())
            .chain(self.weapons.iter().flatten())
        {
            ClientModelBindings::capture_image_selections(*capture, catalog, profiles, worlds);
        }
    }

    pub(super) fn refresh_images(
        &mut self,
        refresh: &crate::model_assets::ImageRefresh,
        catalog: &ModelCatalog,
        profile: Option<ModelProfile>,
    ) -> Result<(), String> {
        for capture in self
            .bodies
            .iter_mut()
            .filter_map(|row| row.body.as_mut())
            .chain(self.weapons.iter_mut().flatten())
        {
            ClientModelBindings::refresh_capture_images(capture, refresh, catalog, profile)?;
        }
        for row in &mut self.indexed_models {
            if let Some(selection) = *row {
                *row = Some(match selection {
                    IndexedPlayerSkin::External(selection) => {
                        IndexedPlayerSkin::External(catalog.refresh_skin(selection, refresh)?)
                    }
                    IndexedPlayerSkin::Model(selection) => {
                        IndexedPlayerSkin::Model(catalog.refresh_skin(selection, refresh)?)
                    }
                });
            }
        }
        if self.color_rules.is_some()
            && profile.is_some_and(|profile| refresh.profile(profile) != profile)
        {
            self.dirty = true;
            self.cold_dirty = true;
        }
        Ok(())
    }
    pub(super) fn source(&self) -> RuleSetId {
        self.source
    }
    pub(super) fn contains(&self, index: usize) -> bool {
        self.rows.contains(index)
    }
    pub(super) fn clear_bindings(&mut self) {
        for row in &mut self.bodies {
            row.body = None;
        }
        self.indexed_models.fill(None);
        self.indexed_model = None;
        self.weapons.fill(None);
        self.dirty = true;
        self.cold_dirty = true;
    }
    pub(super) fn clear_received(&mut self) {
        self.skin_revision = self.skin_revision.wrapping_add(1);
        for row in &mut self.bodies {
            row.metadata = PeerMetadata::default();
        }
    }
    pub(super) fn metadata(&self, slot: usize) -> Option<PeerMetadata> {
        (slot < self.rows.count as usize).then(|| self.bodies[slot].metadata)
    }
    pub(super) fn receive(
        &mut self,
        update: qa_network::outputs::PeerUpdate<'_>,
        text: &[u8],
    ) -> bool {
        use qa_network::outputs::PeerUpdate;
        let slot = usize::from(update.slot());
        if slot >= self.rows.count as usize {
            return false;
        }
        let peer = &mut self.bodies[slot].metadata;
        let old_colors = (peer.colors, peer.top_color, peer.bottom_color);
        match update {
            PeerUpdate::UserInfo { user_id, .. } => peer.user_id = Some(user_id),
            PeerUpdate::Colors { colors, .. } => {
                peer.colors = Some(colors);
                peer.top_color = i32::from(colors >> 4);
                peer.bottom_color = i32::from(colors & 15);
            }
            PeerUpdate::Frags { frags, .. } => peer.frags = frags,
            PeerUpdate::Name { .. } | PeerUpdate::SetInfo { .. } => {}
        }
        if matches!(
            update,
            PeerUpdate::UserInfo { .. } | PeerUpdate::SetInfo { .. }
        ) {
            peer.top_color = qa_core::text::info_integer(text, b"topcolor");
            peer.bottom_color = qa_core::text::info_integer(text, b"bottomcolor");
        }
        if old_colors != (peer.colors, peer.top_color, peer.bottom_color) {
            self.dirty = true;
            self.cold_dirty = true;
        }
        true
    }
    fn weapon(&self, player: usize, weapon: usize) -> Option<CapturedModel> {
        let rules = self.rules?;
        *self.weapons.get(
            player
                .checked_mul(rules.weapon_limit)?
                .checked_add(weapon)?,
        )?
    }
    fn select(&self, skin: i32, cell: usize) -> Option<CapturedModel> {
        let rules = self.rules?;
        let base = self.rows.count as usize;
        let (player, mut weapon) = rules.player_selection(skin);
        let player = if player < base { player } else { base };
        if cell == 0 {
            return self.bodies[player].body.or(self.bodies[base].body);
        }
        if !self.options.visible_weapons || weapon >= self.weapon_count {
            weapon = 0;
        }
        self.weapon(player, weapon)
            .or_else(|| self.weapon(player, 0))
            .or_else(|| self.weapon(base, 0))
    }
    pub(super) fn refresh(
        &mut self,
        configs: &ConfigStrings,
        assets: &mut ClientAssets<'_>,
    ) -> u32 {
        if self.color_rules.is_some() {
            return self.refresh_indexed(configs, assets).unwrap_or(1);
        }
        for row in &mut self.bodies {
            row.body = None;
        }
        self.weapons.fill(None);
        self.weapon_rows.fill(None);
        self.weapon_count = 1;
        let Some(rules) = self.rules else {
            self.dirty = false;
            return 0;
        };
        if self.options.visible_weapons {
            for ordinal in rules.weapon_start..self.models.count as usize {
                let Some(row) = self
                    .models
                    .get(ordinal)
                    .and_then(|index| configs.config(index))
                else {
                    break;
                };
                if row.text.is_empty() {
                    if rules.skip_player_model_gap && ordinal == 255 {
                        continue;
                    }
                    break;
                }
                if row.text.starts_with(b"#") && self.weapon_count < rules.weapon_limit {
                    self.weapon_rows[self.weapon_count] = Some(ordinal as u32);
                    self.weapon_count += 1;
                }
            }
        }
        let base = self.rows.count as usize;
        self.load_player(base, b"unnamed\\male/grunt", configs, assets);
        let mut unresolved = 0;
        for player in 0..base {
            let Some(row) = self
                .rows
                .get(player)
                .and_then(|index| configs.config(index))
            else {
                continue;
            };
            if row.text.is_empty() {
                continue;
            }
            self.load_player(player, row.text, configs, assets);
            unresolved += u32::from(self.bodies[player].body.is_none());
        }
        self.dirty = false;
        unresolved
    }
    fn load_player(
        &mut self,
        player: usize,
        text: &[u8],
        configs: &ConfigStrings,
        assets: &mut ClientAssets<'_>,
    ) {
        let Some(rules) = self.rules else { return };
        let Some((mut model, mut skin, default_weapon_only)) =
            parse_player(text, self.options, rules)
        else {
            return;
        };
        let mut body = player_path(&["players/", model, "/tris.md2"])
            .and_then(|path| assets.model(path.as_bytes()));
        if body.is_none() {
            model = "male";
            body = assets.model(b"players/male/tris.md2");
        }
        let mut skin_handle = player_path(&["players/", model, "/", skin, ".pcx"])
            .and_then(|path| assets.skin(path.as_bytes()));
        if skin_handle.is_none()
            && rules.female_skin_fallback
            && compare_folded(model.as_bytes(), b"female").is_eq()
        {
            skin = "athena";
            skin_handle = assets.skin(b"players/female/athena.pcx");
        }
        if skin_handle.is_none() && !compare_folded(model.as_bytes(), b"male").is_eq() {
            model = "male";
            body = assets.model(b"players/male/tris.md2");
            skin_handle = player_path(&["players/male/", skin, ".pcx"])
                .and_then(|path| assets.skin(path.as_bytes()));
        }
        if skin_handle.is_none() {
            skin = "grunt";
            skin_handle = player_path(&["players/", model, "/grunt.pcx"])
                .and_then(|path| assets.skin(path.as_bytes()));
        }
        let cyborg = if rules.folded_weapon_fallback {
            compare_folded(model.as_bytes(), b"cyborg").is_eq()
        } else {
            model == "cyborg"
        };
        for weapon in 0..if default_weapon_only {
            1
        } else {
            self.weapon_count
        } {
            let name = self.weapon_rows[weapon]
                .and_then(|ordinal| self.models.get(ordinal as usize))
                .and_then(|index| configs.config(index))
                .and_then(|row| row.text.strip_prefix(b"#"))
                .unwrap_or(b"weapon.md2");
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            let mut binding = player_path(&["players/", model, "/", name])
                .and_then(|path| assets.model(path.as_bytes()));
            if binding.is_none() && cyborg {
                binding = player_path(&["players/male/", name])
                    .and_then(|path| assets.model(path.as_bytes()));
            }
            self.weapons[player * rules.weapon_limit + weapon] = binding;
        }
        let icon = !rules.require_icon
            || player_path(&["players/", model, "/", skin, "_i.pcx"])
                .and_then(|path| assets.skin(path.as_bytes()))
                .is_some();
        let partial_base = player == self.rows.count as usize && rules.partial_base;
        if !partial_base
            && (body.is_none()
                || skin_handle.is_none()
                || self.weapon(player, 0).is_none()
                || !icon)
        {
            self.weapons[player * rules.weapon_limit] = None;
            return;
        }
        if let Some(body) = &mut body {
            body.custom_skin = skin_handle.unwrap_or(0);
        }
        self.bodies[player].body = body;
    }
}

pub(super) enum ClientAssets<'a> {
    Loading {
        loader: &'a mut ModelAssets,
        vfs: &'a qa_content::vfs::Vfs,
        source: RuleSetId,
        ui_fullscreen: bool,
    },
    Frozen {
        catalog: &'a ModelCatalog,
        profile: Option<ModelProfile>,
    },
    Prepared {
        owner: &'a mut ModelAssets,
        vfs: &'a qa_content::vfs::Vfs,
        profile: ModelProfile,
        preparer: &'a mut dyn crate::renderer::AssetPreparer,
        ui_fullscreen: bool,
        published: &'a mut bool,
    },
}
impl ClientAssets<'_> {
    pub(super) fn catalog(&self) -> &ModelCatalog {
        match self {
            Self::Loading { loader, .. } => loader.catalog(),
            Self::Frozen { catalog, .. } => catalog,
            Self::Prepared { owner, .. } => owner.catalog(),
        }
    }

    fn prepare(&mut self, path: &[u8], kind: RendererRegistration) -> Option<i32> {
        let Self::Prepared {
            owner,
            vfs,
            profile,
            preparer,
            ui_fullscreen,
            published,
        } = self
        else {
            return None;
        };
        let request = RendererRequest {
            search: None,
            name: path,
            source: profile.source_rules,
            kind,
            world: None,
        };
        let (handle, _, _) = owner
            .prepare_registration(
                vfs,
                &request,
                *profile,
                crate::model_assets::RegistrationVariant::Guard(*ui_fullscreen),
                |admission| Ok(admission.choice_nodes()),
                |assets, choices| preparer.prepare_model_assets(assets, *choices),
            )
            .ok()?;
        if handle <= 0 {
            return None;
        }
        **published = true;
        Some(handle)
    }

    pub(super) fn model(&mut self, path: &[u8]) -> Option<CapturedModel> {
        let model = match self {
            Self::Loading {
                loader,
                vfs,
                source,
                ui_fullscreen,
            } => {
                let handle = loader
                    .preload_model(vfs, path, *source, *ui_fullscreen)
                    .ok()
                    .filter(|&handle| handle > 0)?;
                ModelCapture::Registered {
                    handle,
                    presentation: None,
                }
            }
            Self::Frozen { catalog, profile } => ModelCapture::Bound(ClientModel::Mesh(
                catalog.renderer_registration(catalog.file_handle(path)?, (*profile)?)?,
            )),
            Self::Prepared { .. } => {
                let handle = self.prepare(path, RendererRegistration::Model)?;
                let Self::Prepared { owner, profile, .. } = self else {
                    return None;
                };
                ModelCapture::Bound(ClientModel::Mesh(
                    owner.catalog().renderer_registration(handle, *profile)?,
                ))
            }
        };
        Some(CapturedModel {
            model,
            custom_skin: 0,
            features: Default::default(),
        })
    }
    fn skin(&mut self, path: &[u8]) -> Option<i32> {
        match self {
            Self::Loading {
                loader,
                vfs,
                source,
                ui_fullscreen,
            } => loader
                .preload_skin(vfs, path, *source, *ui_fullscreen, None)
                .ok()
                .filter(|&handle| handle > 0),
            Self::Frozen { catalog, profile } => {
                let handle = catalog
                    .skin_handle(path, (*profile)?.source_rules)
                    .filter(|&handle| handle > 0)?;
                catalog.renderer_skin(handle, (*profile)?).then_some(handle)
            }
            Self::Prepared { .. } => self.prepare(path, RendererRegistration::Skin),
        }
    }
}

fn player_path(parts: &[&str]) -> Option<qa_core::text::FixedText<64>> {
    let mut path = qa_core::text::FixedText::default();
    for part in parts {
        path.write_str(part).ok()?;
    }
    Some(path)
}
pub(super) fn download_path(
    text: &[u8],
    rules: PlayerModelRules,
    part: u8,
) -> Option<qa_core::text::FixedText<64>> {
    let (model, skin, _) = parse_player(text, PlayerModelOptions::default(), rules)?;
    match part {
        0 => player_path(&["players/", model, "/tris.md2"]),
        1 => player_path(&["players/", model, "/weapon.md2"]),
        2 => player_path(&["players/", model, "/weapon.pcx"]),
        3 => player_path(&["players/", model, "/", skin, ".pcx"]),
        4 => player_path(&["players/", model, "/", skin, "_i.pcx"]),
        _ => None,
    }
}
fn parse_player(
    text: &[u8],
    options: PlayerModelOptions,
    rules: PlayerModelRules,
) -> Option<(&str, &str, bool)> {
    let appearance = text
        .iter()
        .position(|&byte| byte == b'\\')
        .map_or(text, |at| &text[at + 1..]);
    if !rules.restricted_paths && (options.no_skins != 0 || appearance.is_empty()) {
        return Some(("male", "grunt", true));
    }
    let split = appearance
        .iter()
        .position(|&byte| byte == b'/')
        .or_else(|| {
            (!rules.restricted_paths)
                .then(|| appearance.iter().position(|&byte| byte == b'\\'))
                .flatten()
        });
    let (mut model, mut skin) = split.map_or_else(
        || {
            if rules.restricted_paths {
                (appearance, &[][..])
            } else {
                (&[][..], appearance.get(1..).unwrap_or_default())
            }
        },
        |at| (&appearance[..at], &appearance[at + 1..]),
    );
    if rules.restricted_paths {
        if let Some(at) = skin.iter().position(|&byte| byte == b'\\') {
            skin = &skin[..at];
        }
        if split.is_none()
            && let Some(at) = model.iter().position(|&byte| byte == b'\\')
        {
            model = &model[..at];
        }
        if model.is_empty() {
            model = b"male";
        }
        let valid = |path: &[u8]| {
            !path.is_empty()
                && path
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        };
        if options.no_skins == 2 || !valid(skin) {
            return Some(if compare_folded(model, b"female").is_eq() {
                ("female", "athena", false)
            } else {
                ("male", "grunt", false)
            });
        }
        if options.no_skins != 0 || !valid(model) {
            model = b"male";
            skin = b"grunt";
        }
    }
    Some((
        std::str::from_utf8(model).unwrap_or(""),
        std::str::from_utf8(skin).unwrap_or(""),
        false,
    ))
}

impl ClientModelBindings {
    pub(crate) fn player_download_path(
        &self,
        text: &[u8],
        part: u8,
    ) -> Option<qa_core::text::FixedText<64>> {
        download_path(text, self.players.as_ref()?.rules?, part)
    }
    pub(crate) fn config_received(&mut self, index: usize) {
        if self.range.contains(index) {
            self.cold.insert(index - self.range.first as usize);
        }
        if let Some(players) = &mut self.players
            && (players.rows.contains(index) || players.models.contains(index))
        {
            players.skin_revision = players.skin_revision.wrapping_add(1);
            players.dirty = true;
            players.cold_dirty = true;
        }
    }
    pub(crate) fn player_options(&mut self, options: PlayerModelOptions) {
        if let Some(players) = &mut self.players
            && players.options != options
        {
            players.options = options;
            players.dirty = true;
            players.cold_dirty = true;
        }
    }
    pub(crate) fn register_player_models(
        &mut self,
        configs: &ConfigStrings,
        loader: &mut ModelAssets,
        vfs: &qa_content::vfs::Vfs,
        source: RuleSetId,
        ui_fullscreen: bool,
    ) -> u32 {
        let Some(players) = &mut self.players else {
            return 0;
        };
        if !players.cold_dirty {
            return 0;
        }
        players.cold_dirty = false;
        players.refresh(
            configs,
            &mut ClientAssets::Loading {
                loader,
                vfs,
                source,
                ui_fullscreen,
            },
        )
    }
    pub(crate) fn refresh_player_models(
        &mut self,
        configs: &ConfigStrings,
        catalog: &ModelCatalog,
    ) -> u32 {
        let Some(players) = &mut self.players else {
            return 0;
        };
        if !players.dirty {
            return 0;
        }
        players.refresh(
            configs,
            &mut ClientAssets::Frozen {
                catalog,
                profile: self.profile,
            },
        )
    }
    pub(super) fn player_model(&self, skin: i32, cell: usize) -> Option<CapturedModel> {
        self.players.as_ref()?.select(skin, cell)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/peer_metadata.rs"]
mod peer_metadata_tests;
