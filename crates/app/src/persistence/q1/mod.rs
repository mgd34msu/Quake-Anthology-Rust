//! Quake I save providers.
//!
//! Donor provenance: `src/persistence/{q1,q1-source,q1-foundation,
//! q1-quakec,q1-selection}.ts`. [`source_text`] is the original
//! `Host_Savegame_f` version 5 text format (plus Ironwail/KEX version 6
//! header); [`foundation`] is the TypeScript foundation checkpoint;
//! [`quakec`] captures/restores QuakeC VM records through a machine
//! trait; [`source`] stages original saves on a singleplayer NetQuake
//! source; [`selection`] resolves the saved game product without
//! guessing between mods.

pub mod foundation;
pub mod quakec;
pub mod selection;
pub mod source;
pub mod source_text;
