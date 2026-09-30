//! Quake III base/game: shader remaps.
//!
//! Donor provenance: `src/content/q3/base/game/shader-remaps.ts`.

use crate::value::arr;
use crate::value::num;
use crate::value::obj;
use crate::value::str;
use crate::value::SaveJson;
use crate::value::SaveReader;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;

// ---------------------------------------------------------------------------
// shader-remaps.ts: shader remap state (g_utils.c)
// ---------------------------------------------------------------------------

/// Maximum path length (`MAX_QPATH`).
pub const MAX_QPATH: usize = 64;

/// Maximum remaps (`MAX_SHADER_REMAPS`).
pub const MAX_SHADER_REMAPS: usize = 128;

/// Entry buffer bytes.
pub(crate) const ENTRY_BUFFER_BYTES: usize = MAX_QPATH * 2 + 5;

/// State buffer bytes.
pub(crate) const STATE_BUFFER_BYTES: usize = 1024 * 4;

/// Shader remap entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderRemap {
    /// Old name.
    pub old_name: String,
    /// New name.
    pub new_name: String,
    /// Time offset.
    pub time_offset: f32,
}

pub(crate) fn quake_path(value: &str) -> Result<String, Q3GameError> {
    let path = value.split('\0').next().unwrap_or("").to_string();
    for ch in path.chars() {
        if ch as u32 > 255 {
            return Err(range("shader remap paths must contain byte-valued code units"));
        }
    }
    if path.chars().count() >= MAX_QPATH {
        return Err(range("shader remap paths must fit MAX_QPATH including the terminator"));
    }
    Ok(path)
}

pub(crate) fn folded_ascii_byte(value: u8) -> u8 {
    if (97..=122).contains(&value) {
        value - 32
    } else {
        value
    }
}

pub(crate) fn quake_path_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .all(|(a, b)| folded_ascii_byte(a) == folded_ascii_byte(b))
}

pub(crate) fn stored_time_offset(value: f64) -> Result<f32, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    let stored = value as f32;
    if !stored.is_finite() || stored.abs() > 2_147_483_647.0 {
        return Err(range(
            "shader remap time is outside the source formatter's safe int-cast range",
        ));
    }
    Ok(stored)
}

/// Shader remap registry (`ShaderRemapRegistry`).
pub struct ShaderRemapRegistry {
    remaps: Vec<ShaderRemap>,
    print: Rc<dyn Fn(&str)>,
}

impl ShaderRemapRegistry {
    /// New registry with a print hook.
    #[must_use]
    pub fn new(print: Rc<dyn Fn(&str)>) -> Self {
        Self {
            remaps: Vec::new(),
            print,
        }
    }

    /// Remap entries.
    #[must_use]
    pub fn remaps(&self) -> &[ShaderRemap] {
        &self.remaps
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        arr(self
            .remaps
            .iter()
            .map(|remap| {
                obj(vec![
                    ("oldName", str(&remap.old_name)),
                    ("newName", str(&remap.new_name)),
                    ("timeOffset", num(f64::from(remap.time_offset))),
                ])
            })
            .collect())
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &SaveJson) -> Result<(), Q3GameError> {
        let reader = SaveReader::at(value, "q3.remaps");
        let remaps: Vec<ShaderRemap> = reader.list(|entry| -> Result<ShaderRemap, Q3GameError> {
            Ok(ShaderRemap {
                old_name: quake_path(&entry.field("oldName").string()?)?,
                new_name: quake_path(&entry.field("newName").string()?)?,
                time_offset: stored_time_offset(entry.field("timeOffset").number()?)?,
            })
        })?;
        let duplicate = remaps.iter().enumerate().any(|(index, entry)| {
            remaps[..index]
                .iter()
                .any(|previous| quake_path_equal(&previous.old_name, &entry.old_name))
        });
        if remaps.len() > MAX_SHADER_REMAPS || duplicate {
            return Err(reader.fail("invalid shader remap table").into());
        }
        self.remaps = remaps;
        Ok(())
    }

    /// Add or update a remap (`AddRemap`).
    pub fn add(&mut self, old_name: &str, new_name: &str, time_offset: f64) -> Result<(), Q3GameError> {
        let old_path = quake_path(old_name)?;
        let new_path = quake_path(new_name)?;
        let stored_time = stored_time_offset(time_offset)?;
        for remap in &mut self.remaps {
            if quake_path_equal(&old_path, &remap.old_name) {
                remap.new_name = new_path;
                remap.time_offset = stored_time;
                return Ok(());
            }
        }
        if self.remaps.len() < MAX_SHADER_REMAPS {
            self.remaps.push(ShaderRemap {
                old_name: old_path,
                new_name: new_path,
                time_offset: stored_time,
            });
        }
        Ok(())
    }

    /// Build the shader-state configstring (`BuildShaderStateConfig`).
    pub fn build_shader_state_config(&self) -> Result<String, Q3GameError> {
        let mut state = String::new();
        for remap in &self.remaps {
            let formatted = game_format(
                "%s=%s:%5.2f@",
                &[
                    GameFormatArg::Text(Some(remap.old_name.clone())),
                    GameFormatArg::Text(Some(remap.new_name.clone())),
                    GameFormatArg::Float(remap.time_offset),
                ],
            )?;
            if formatted.len() >= ENTRY_BUFFER_BYTES {
                (self.print)(&game_format(
                    "Com_sprintf: overflow of %i in %i\n",
                    &[
                        GameFormatArg::Int(formatted.len() as i32),
                        GameFormatArg::Int(ENTRY_BUFFER_BYTES as i32),
                    ],
                )?);
            }
            let entry = formatted[..formatted.len().min(ENTRY_BUFFER_BYTES - 1)].to_string();
            let writable = STATE_BUFFER_BYTES.saturating_sub(state.len()).saturating_sub(1);
            if writable > 0 {
                state.push_str(&entry[..entry.len().min(writable)]);
            }
        }
        Ok(state)
    }
}
