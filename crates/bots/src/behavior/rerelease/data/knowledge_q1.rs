//! Q1 knowledge loader from `src/bots/behavior/rerelease/data/knowledge-q1.ts`.
//!
//! Source mount precedence is already resolved by the owner. Weapons
//! are required; settings resolve PC, then Consoles, then Nintendo.

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::rerelease::data::botdata::BotDataFormat;
use crate::behavior::rerelease::data::knowledge::{BotDataFilesT, BotKnowledge};
use crate::behavior::rerelease::data::source_files::read_bot_source_text;
use crate::error::BotsError;

/// Load Q1 knowledge, or `None` when weapons are unmounted.
pub fn load_quake1_knowledge(files: &dyn BotSourceFiles) -> Result<Option<BotKnowledge>, BotsError> {
    let weapons = match read_bot_source_text(files, "bots/weapons.txt") {
        Some(text) => text,
        None => return Ok(None),
    };
    let settings = read_bot_source_text(files, "bots/settings_PC.txt")
        .or_else(|| read_bot_source_text(files, "bots/settings_Consoles.txt"))
        .or_else(|| read_bot_source_text(files, "bots/settings_Nintendo.txt"))
        .ok_or_else(|| {
            BotsError::BotScript("quake rerelease bot weapons were mounted without source skill settings".to_owned())
        })?;
    Ok(Some(BotKnowledge::new(
        &BotDataFilesT {
            weapons,
            settings,
            characters: read_bot_source_text(files, "bots/characters.txt").unwrap_or_default(),
            items: read_bot_source_text(files, "bots/items.txt").unwrap_or_default(),
            monsters: read_bot_source_text(files, "bots/monsters.txt").unwrap_or_default(),
            interactables: read_bot_source_text(files, "bots/interactables.txt").unwrap_or_default(),
            game_rules: read_bot_source_text(files, "bots/game_rules.txt").unwrap_or_default(),
            teams: read_bot_source_text(files, "bots/teams.txt").unwrap_or_default(),
            chats: read_bot_source_text(files, "bots/chats.txt").unwrap_or_default(),
            dangers: read_bot_source_text(files, "bots/dangers.txt"),
        },
        BotDataFormat::Q1,
        None,
    )))
}
