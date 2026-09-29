//! Original Quake source saves ported from `src/persistence/q1-source.ts`.
//!
//! Capture and restoration on a staged singleplayer NetQuake source. The
//! source surface (slots, bodies, events) is a pair of traits because the
//! application bootstrap owns the live types; this module owns the
//! singleplayer/skill/map gating, header assembly, and staging order.
//! [`Q1Source::split`] borrows the machine and the staging tables
//! disjointly so restore callbacks and VM records never alias.

use super::super::PersistenceError;
use super::quakec::{
    capture_q1_quakec_save, restore_q1_quakec_save, Q1QuakeCMachine, Q1QuakeCSaveHost, Q1SaveHeader,
    Q1UnknownSaveFields,
};
use super::source_text::{Q1SaveData, Q1SaveFormat};

/// Staging tables for original-save restore.
pub trait Q1SourceStaging {
    /// Whether a slot is free.
    fn is_free(&self, slot: usize) -> bool;
    /// Current staged entity count.
    fn staged_entity_count(&self) -> usize;
    /// Whether a slot has a bound actor.
    fn has_actor(&self, slot: usize) -> bool;
    /// Unlink a staged body.
    fn unlink_body(&mut self, slot: usize);
    /// Release a staged actor.
    fn release_actor(&mut self, slot: usize);
    /// Reserved client slots (never released).
    fn reserved_client_slots(&self) -> usize;
    /// Bind an existing staged slot.
    fn bind_existing(&mut self, slot: usize);
    /// Clear a freed staged slot.
    fn clear_freed(&mut self, slot: usize);
    /// Initialize a staged slot.
    fn initialize(&mut self, slot: usize);
    /// Link a restored slot.
    fn link(&mut self, slot: usize);
    /// Emit a light-style event.
    fn emit_lightstyle(&mut self, style: usize, pattern: &str);
}

/// Singleplayer NetQuake source surface for original saves.
pub trait Q1Source {
    /// Machine type.
    type Machine: Q1QuakeCMachine;
    /// Staging type.
    type Staging: Q1SourceStaging;

    /// Snapshot provider state.
    fn checkpoint(&mut self);
    /// Source kind (`netquake` required).
    fn kind(&self) -> String;
    /// Maximum clients (1 required).
    fn max_clients(&self) -> u32;
    /// Match mode (`singleplayer` required).
    fn mode(&self) -> String;
    /// Skill level.
    fn skill(&self) -> i32;
    /// Map geometry path (`maps/<name>.bsp`).
    fn map_geometry_path(&self) -> String;
    /// Server time in seconds.
    fn time_seconds(&self) -> f64;
    /// Light style pattern (empty when unset).
    fn light_style(&self, index: usize) -> String;
    /// Whether the source is still loading (restore requires staged).
    fn loading(&self) -> bool;
    /// Split the machine and staging borrows.
    fn split(&mut self) -> (&mut Self::Machine, &mut Self::Staging);
}

/// Strip `maps/` and `.bsp` from a geometry path.
fn map_name(geometry: &str) -> String {
    geometry
        .strip_prefix("maps/")
        .unwrap_or(geometry)
        .strip_suffix(".bsp")
        .unwrap_or(geometry)
        .to_string()
}

/// Capture an original save from a singleplayer NetQuake source.
pub fn capture_q1_source_save(
    source: &mut impl Q1Source,
    format: Q1SaveFormat,
    comment: &str,
    spawn_parameters: Vec<f64>,
    extension_text: &str,
) -> Result<Q1SaveData, PersistenceError> {
    source.checkpoint();
    if source.kind() != "netquake" || source.max_clients() != 1 || source.mode() != "singleplayer" {
        return Err(PersistenceError::BadSave(
            "Original Quake saves require a singleplayer NetQuake source".to_string(),
        ));
    }
    let header = Q1SaveHeader {
        format,
        comment: comment
            .chars()
            .map(|character| if character.is_whitespace() { '_' } else { character })
            .collect(),
        spawn_parameters,
        skill: source.skill(),
        map: map_name(&source.map_geometry_path()),
        time: source.time_seconds(),
        light_styles: (0..64)
            .map(|index| {
                let pattern = source.light_style(index);
                if pattern.is_empty() {
                    "m".to_string()
                } else {
                    pattern
                }
            })
            .collect(),
        extension_text: extension_text.to_string(),
    };
    let (machine, staging) = source.split();
    let free: Vec<bool> = (0..machine.entity_count()).map(|slot| staging.is_free(slot)).collect();
    Ok(capture_q1_quakec_save(machine, header, &|slot| free[slot]))
}

struct RestoreHost<'a, T: Q1SourceStaging> {
    staging: &'a mut T,
    save: &'a Q1SaveData,
    reserved: usize,
}

impl<T: Q1SourceStaging> Q1QuakeCSaveHost for RestoreHost<'_, T> {
    fn begin(&mut self, count: usize) {
        for slot in 0..self.staging.staged_entity_count() {
            if !self.staging.has_actor(slot) {
                continue;
            }
            self.staging.unlink_body(slot);
            if slot > self.reserved {
                self.staging.release_actor(slot);
            }
        }
        for slot in 0..count {
            if self.save.entities.get(slot).is_some_and(Vec::is_empty) {
                self.staging.clear_freed(slot);
            } else {
                self.staging.bind_existing(slot);
                self.staging.initialize(slot);
            }
        }
    }

    fn entity(&mut self, slot: usize, free: bool) {
        if !free {
            self.staging.link(slot);
        }
    }

    fn finish(&mut self, header: Q1SaveHeader) {
        for (style, pattern) in header.light_styles.iter().enumerate() {
            self.staging.emit_lightstyle(style, pattern);
        }
    }
}

/// Restore an original save onto a staged, precached singleplayer source.
pub fn restore_q1_source_save(
    source: &mut impl Q1Source,
    save: &Q1SaveData,
    finish: impl FnOnce(Q1SaveHeader),
) -> Result<Q1UnknownSaveFields, PersistenceError> {
    if source.kind() != "netquake" || source.max_clients() != 1 || source.mode() != "singleplayer" || source.loading() {
        return Err(PersistenceError::BadSave(
            "Original Quake restoration requires a precached singleplayer NetQuake candidate".to_string(),
        ));
    }
    let map = map_name(&source.map_geometry_path());
    if save.map != map
        || save.skill != source.skill()
        || save.entities.len() < 2
        || save.entities.first().is_some_and(Vec::is_empty)
        || save.entities.get(1).is_some_and(Vec::is_empty)
    {
        return Err(PersistenceError::BadSave(
            "Original Quake save does not match the candidate map, skill or player".to_string(),
        ));
    }
    source.checkpoint();
    let unknowns = {
        let (machine, staging) = source.split();
        let reserved = staging.reserved_client_slots();
        let mut host = RestoreHost {
            staging,
            save,
            reserved,
        };
        restore_q1_quakec_save(machine, save, &mut host)?
    };
    finish(Q1SaveHeader::from(save));
    Ok(unknowns)
}

#[cfg(test)]
mod tests {
    use super::super::quakec::Q1AppliedEntity;
    use super::super::source_text::QcTextPair;
    use super::*;

    struct FakeMachine {
        count: usize,
    }

    impl Q1QuakeCMachine for FakeMachine {
        fn snapshot(&mut self) {}
        fn entity_count(&self) -> usize {
            self.count
        }
        fn entity_capacity(&self) -> usize {
            16
        }
        fn set_entity_count(&mut self, count: usize) {
            self.count = count;
        }
        fn clear_entity(&mut self, _slot: usize) {}
        fn save_global_pairs(&self) -> Vec<QcTextPair> {
            Vec::new()
        }
        fn save_entity_pairs(&self, slot: usize, free: bool) -> Vec<QcTextPair> {
            if free {
                Vec::new()
            } else {
                vec![QcTextPair {
                    key: "slot".to_string(),
                    value: format!("{slot}"),
                }]
            }
        }
        fn apply_global_pairs(&mut self, _pairs: &[QcTextPair]) -> Vec<QcTextPair> {
            Vec::new()
        }
        fn apply_entity_pairs(&mut self, _slot: usize, pairs: &[QcTextPair]) -> Q1AppliedEntity {
            Q1AppliedEntity {
                unknown: Vec::new(),
                empty: pairs.is_empty(),
            }
        }
    }

    #[derive(Default)]
    struct FakeStaging {
        bound: Vec<bool>,
        linked: Vec<usize>,
        styles: Vec<(usize, String)>,
    }

    impl Q1SourceStaging for FakeStaging {
        fn is_free(&self, slot: usize) -> bool {
            !self.bound.get(slot).copied().unwrap_or(false)
        }
        fn staged_entity_count(&self) -> usize {
            self.bound.len()
        }
        fn has_actor(&self, slot: usize) -> bool {
            self.bound.get(slot).copied().unwrap_or(false)
        }
        fn unlink_body(&mut self, _slot: usize) {}
        fn release_actor(&mut self, slot: usize) {
            self.bound[slot] = false;
        }
        fn reserved_client_slots(&self) -> usize {
            1
        }
        fn bind_existing(&mut self, slot: usize) {
            if self.bound.len() <= slot {
                self.bound.resize(slot + 1, false);
            }
            self.bound[slot] = true;
        }
        fn clear_freed(&mut self, slot: usize) {
            if self.bound.len() <= slot {
                self.bound.resize(slot + 1, false);
            }
        }
        fn initialize(&mut self, _slot: usize) {}
        fn link(&mut self, slot: usize) {
            self.linked.push(slot);
        }
        fn emit_lightstyle(&mut self, style: usize, pattern: &str) {
            self.styles.push((style, pattern.to_string()));
        }
    }

    struct FakeSource {
        machine: FakeMachine,
        staging: FakeStaging,
    }

    impl Q1Source for FakeSource {
        type Machine = FakeMachine;
        type Staging = FakeStaging;
        fn checkpoint(&mut self) {}
        fn kind(&self) -> String {
            "netquake".to_string()
        }
        fn max_clients(&self) -> u32 {
            1
        }
        fn mode(&self) -> String {
            "singleplayer".to_string()
        }
        fn skill(&self) -> i32 {
            2
        }
        fn map_geometry_path(&self) -> String {
            "maps/e1m1.bsp".to_string()
        }
        fn time_seconds(&self) -> f64 {
            9.0
        }
        fn light_style(&self, _index: usize) -> String {
            String::new()
        }
        fn loading(&self) -> bool {
            false
        }
        fn split(&mut self) -> (&mut FakeMachine, &mut FakeStaging) {
            (&mut self.machine, &mut self.staging)
        }
    }

    #[test]
    fn source_capture_restore_round_trip() {
        let mut source = FakeSource {
            machine: FakeMachine { count: 3 },
            staging: FakeStaging {
                bound: vec![true, true, false],
                linked: Vec::new(),
                styles: Vec::new(),
            },
        };
        let save = capture_q1_source_save(&mut source, Q1SaveFormat::V5, "my save", vec![0.0; 16], "").unwrap();
        assert_eq!(save.comment, "my_save");
        assert_eq!(save.map, "e1m1");
        assert_eq!(save.entities.len(), 3);
        let mut finished = false;
        restore_q1_source_save(&mut source, &save, |_| finished = true).unwrap();
        assert!(finished);
        assert_eq!(source.staging.linked, vec![0, 1]);
        assert_eq!(source.staging.styles.len(), 64);
    }

    #[test]
    fn source_gating_rejects_mismatches() {
        let mut source = FakeSource {
            machine: FakeMachine { count: 3 },
            staging: FakeStaging {
                bound: vec![true, true, true],
                linked: Vec::new(),
                styles: Vec::new(),
            },
        };
        let mut save = capture_q1_source_save(&mut source, Q1SaveFormat::V5, "x", vec![0.0; 16], "").unwrap();
        save.map = "e1m2".to_string();
        assert!(restore_q1_source_save(&mut source, &save, |_| {}).is_err());
    }
}
