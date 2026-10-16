use super::*;
use crate::model_assets::ModelAssets;
use qa_compat::services::{ModelLoader, ModelLookup};
use qa_core::names::compare_folded;
use qa_gameplay::rules::appearance::PlayerModelRules;
use std::fmt::Write;

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

pub(super) struct PlayerBindings {
    source: RuleSetId,
    rows: IndexSpan,
    models: IndexSpan,
    rules: PlayerModelRules,
    options: PlayerModelOptions,
    dirty: bool,
    bodies: Box<[Option<CapturedModel>]>,
    weapons: Box<[Option<CapturedModel>]>,
    weapon_rows: Box<[Option<u32>]>,
    weapon_count: usize,
}
impl PlayerBindings {
    pub(super) fn load(layout: ConfigLayout) -> Option<Self> {
        let rules = qa_gameplay::rules::appearance::policy(layout.rules).player_models()?;
        let count = layout.players.count as usize + 1;
        Some(Self {
            source: layout.rules,
            rows: layout.players,
            models: layout.resources[0],
            rules,
            options: Default::default(),
            dirty: true,
            bodies: vec![None; count].into_boxed_slice(),
            weapons: vec![None; count.checked_mul(rules.weapon_limit)?].into_boxed_slice(),
            weapon_rows: vec![None; rules.weapon_limit].into_boxed_slice(),
            weapon_count: 1,
        })
    }
    pub(super) fn source(&self) -> RuleSetId {
        self.source
    }
    pub(super) fn contains(&self, index: usize) -> bool {
        self.rows.contains(index)
    }
    pub(super) fn clear(&mut self) {
        self.bodies.fill(None);
        self.weapons.fill(None);
        self.dirty = true;
    }
    fn weapon(&self, player: usize, weapon: usize) -> Option<CapturedModel> {
        *self.weapons.get(
            player
                .checked_mul(self.rules.weapon_limit)?
                .checked_add(weapon)?,
        )?
    }
    fn select(&self, skin: i32, cell: usize) -> Option<CapturedModel> {
        let base = self.rows.count as usize;
        let (player, mut weapon) = self.rules.player_selection(skin);
        let player = if player < base { player } else { base };
        if cell == 0 {
            return self.bodies[player].or(self.bodies[base]);
        }
        if !self.options.visible_weapons || weapon >= self.weapon_count {
            weapon = 0;
        }
        self.weapon(player, weapon)
            .or_else(|| self.weapon(player, 0))
            .or_else(|| self.weapon(base, 0))
    }
    fn refresh(&mut self, configs: &ConfigStrings, assets: &mut PlayerAssets<'_>) -> u32 {
        self.bodies.fill(None);
        self.weapons.fill(None);
        self.weapon_rows.fill(None);
        self.weapon_count = 1;
        if self.options.visible_weapons {
            for ordinal in self.rules.weapon_start..self.models.count as usize {
                let Some(row) = self
                    .models
                    .get(ordinal)
                    .and_then(|index| configs.config(index))
                else {
                    break;
                };
                if row.text.is_empty() {
                    if self.rules.skip_player_model_gap && ordinal == 255 {
                        continue;
                    }
                    break;
                }
                if row.text.starts_with(b"#") && self.weapon_count < self.rules.weapon_limit {
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
            unresolved += u32::from(self.bodies[player].is_none());
        }
        self.dirty = false;
        unresolved
    }
    fn load_player(
        &mut self,
        player: usize,
        text: &[u8],
        configs: &ConfigStrings,
        assets: &mut PlayerAssets<'_>,
    ) {
        let Some((mut model, mut skin, default_weapon_only)) =
            parse_player(text, self.options, self.rules)
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
            && self.rules.female_skin_fallback
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
        let cyborg = if self.rules.folded_weapon_fallback {
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
            self.weapons[player * self.rules.weapon_limit + weapon] = binding;
        }
        let icon = !self.rules.require_icon
            || player_path(&["players/", model, "/", skin, "_i.pcx"])
                .and_then(|path| assets.skin(path.as_bytes()))
                .is_some();
        let partial_base = player == self.rows.count as usize && self.rules.partial_base;
        if !partial_base
            && (body.is_none()
                || skin_handle.is_none()
                || self.weapon(player, 0).is_none()
                || !icon)
        {
            self.weapons[player * self.rules.weapon_limit] = None;
            return;
        }
        if let Some(body) = &mut body {
            body.custom_skin = skin_handle.unwrap_or(0);
        }
        self.bodies[player] = body;
    }
}

enum PlayerAssets<'a> {
    Loading {
        loader: &'a mut ModelAssets,
        vfs: &'a qa_content::vfs::Vfs,
        source: RuleSetId,
    },
    Frozen {
        catalog: &'a ModelCatalog,
        profile: Option<ModelProfile>,
    },
}
impl PlayerAssets<'_> {
    fn model(&mut self, path: &[u8]) -> Option<CapturedModel> {
        let model = match self {
            Self::Loading {
                loader,
                vfs,
                source,
            } => {
                let handle = loader
                    .register_model(vfs, path, *source)
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
            } => loader
                .register_skin(vfs, path, *source)
                .ok()
                .filter(|&handle| handle > 0),
            Self::Frozen { catalog, profile } => {
                let handle = catalog
                    .skin_handle(path, (*profile)?.source_rules)
                    .filter(|&handle| handle > 0)?;
                catalog.renderer_skin(handle, (*profile)?).then_some(handle)
            }
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
    pub(crate) fn config_received(&mut self, index: usize) {
        if let Some(players) = &mut self.players
            && (players.rows.contains(index) || players.models.contains(index))
        {
            players.dirty = true;
        }
    }
    pub(crate) fn player_options(&mut self, options: PlayerModelOptions) {
        if let Some(players) = &mut self.players
            && players.options != options
        {
            players.options = options;
            players.dirty = true;
        }
    }
    pub(crate) fn register_player_models(
        &mut self,
        configs: &ConfigStrings,
        loader: &mut ModelAssets,
        vfs: &qa_content::vfs::Vfs,
        source: RuleSetId,
    ) -> u32 {
        let Some(players) = &mut self.players else {
            return 0;
        };
        if !players.dirty {
            return 0;
        }
        players.refresh(
            configs,
            &mut PlayerAssets::Loading {
                loader,
                vfs,
                source,
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
            &mut PlayerAssets::Frozen {
                catalog,
                profile: self.profile,
            },
        )
    }
    pub(super) fn player_model(&self, skin: i32, cell: usize) -> Option<CapturedModel> {
        self.players.as_ref()?.select(skin, cell)
    }
}
