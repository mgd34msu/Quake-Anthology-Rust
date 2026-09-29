//! QuakeC save records ported from `src/persistence/q1-quakec.ts`.
//!
//! Capture and restore of QuakeC global/entity text pairs through a
//! [`Q1QuakeCMachine`] trait. Pair translation itself belongs to the
//! QuakeC compatibility layer (donor `src/compat/qc/save.ts`); this
//! module owns the header split, edict-count/capacity checks, host
//! callback ordering, and unknown-field reporting.

use super::super::PersistenceError;
use super::source_text::{Q1SaveData, QcTextPair};

/// Save header: everything except the global/entity record arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SaveHeader {
    /// Format header.
    pub format: super::source_text::Q1SaveFormat,
    /// Comment token.
    pub comment: String,
    /// Spawn parameters.
    pub spawn_parameters: Vec<f64>,
    /// Skill level.
    pub skill: i32,
    /// Map name.
    pub map: String,
    /// Server time.
    pub time: f64,
    /// Light styles.
    pub light_styles: Vec<String>,
    /// Trailing extension text.
    pub extension_text: String,
}

impl From<&Q1SaveData> for Q1SaveHeader {
    fn from(save: &Q1SaveData) -> Self {
        Self {
            format: save.format.clone(),
            comment: save.comment.clone(),
            spawn_parameters: save.spawn_parameters.clone(),
            skill: save.skill,
            map: save.map.clone(),
            time: save.time,
            light_styles: save.light_styles.clone(),
            extension_text: save.extension_text.clone(),
        }
    }
}

/// Fields the restoring machine did not recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1UnknownSaveFields {
    /// Unknown global pairs.
    pub globals: Vec<QcTextPair>,
    /// Unknown entity fields by slot.
    pub entities: Vec<Q1UnknownEntityFields>,
}

/// Unknown fields of one saved entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1UnknownEntityFields {
    /// Edict slot.
    pub slot: usize,
    /// Unknown pairs.
    pub fields: Vec<QcTextPair>,
}

/// Applied entity pairs: unknown fields plus the free-slot flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1AppliedEntity {
    /// Unknown pairs.
    pub unknown: Vec<QcTextPair>,
    /// Whether the record was empty (free slot).
    pub empty: bool,
}

/// QuakeC machine surface needed for save capture/restore.
pub trait Q1QuakeCMachine {
    /// Snapshot the VM (checks interpreted and host-builtin reentry).
    fn snapshot(&mut self);
    /// Current entity count.
    fn entity_count(&self) -> usize;
    /// Entity capacity.
    fn entity_capacity(&self) -> usize;
    /// Install the complete source count before references resolve.
    fn set_entity_count(&mut self, count: usize);
    /// Zero one entity record.
    fn clear_entity(&mut self, slot: usize);
    /// Save global pairs.
    fn save_global_pairs(&self) -> Vec<QcTextPair>;
    /// Save one entity's pairs.
    fn save_entity_pairs(&self, slot: usize, free: bool) -> Vec<QcTextPair>;
    /// Apply global pairs, returning unknown pairs.
    fn apply_global_pairs(&mut self, pairs: &[QcTextPair]) -> Vec<QcTextPair>;
    /// Apply one entity's pairs.
    fn apply_entity_pairs(&mut self, slot: usize, pairs: &[QcTextPair]) -> Q1AppliedEntity;
}

/// Host callbacks for QuakeC save restore.
pub trait Q1QuakeCSaveHost {
    /// Unlink old entities and prepare free flags before record loading.
    fn begin(&mut self, entity_count: usize);
    /// Update host metadata and link a non-free edict without spawning.
    fn entity(&mut self, slot: usize, free: bool);
    /// Restore server time, spawn parameters, light styles, and headers.
    fn finish(&mut self, header: Q1SaveHeader);
}

/// Capture a QuakeC save from a machine.
pub fn capture_q1_quakec_save(
    machine: &mut impl Q1QuakeCMachine,
    header: Q1SaveHeader,
    is_free: &dyn Fn(usize) -> bool,
) -> Q1SaveData {
    machine.snapshot();
    Q1SaveData {
        format: header.format,
        comment: header.comment,
        spawn_parameters: header.spawn_parameters,
        skill: header.skill,
        map: header.map,
        time: header.time,
        light_styles: header.light_styles,
        globals: machine.save_global_pairs(),
        entities: (0..machine.entity_count())
            .map(|slot| machine.save_entity_pairs(slot, is_free(slot)))
            .collect(),
        extension_text: header.extension_text,
    }
}

/// Restore a QuakeC save after loading the matching progs and map baseline.
pub fn restore_q1_quakec_save(
    machine: &mut impl Q1QuakeCMachine,
    save: &Q1SaveData,
    host: &mut impl Q1QuakeCSaveHost,
) -> Result<Q1UnknownSaveFields, PersistenceError> {
    machine.snapshot();
    if save.entities.is_empty() || save.entities.len() > machine.entity_capacity() {
        return Err(PersistenceError::BadSave(
            "q1.entities: saved edict count exceeds the selected guest capacity".to_string(),
        ));
    }
    host.begin(save.entities.len());
    machine.set_entity_count(save.entities.len());
    let globals = machine.apply_global_pairs(&save.globals);
    let mut entities = Vec::new();
    for (slot, pairs) in save.entities.iter().enumerate() {
        machine.clear_entity(slot);
        let parsed = machine.apply_entity_pairs(slot, pairs);
        if !parsed.unknown.is_empty() {
            entities.push(Q1UnknownEntityFields {
                slot,
                fields: parsed.unknown,
            });
        }
        host.entity(slot, parsed.empty);
    }
    host.finish(Q1SaveHeader::from(save));
    Ok(Q1UnknownSaveFields { globals, entities })
}

#[cfg(test)]
mod tests {
    use super::super::source_text::Q1SaveFormat;
    use super::*;

    struct FakeMachine {
        count: usize,
        capacity: usize,
        applied: Vec<Vec<QcTextPair>>,
    }

    impl Q1QuakeCMachine for FakeMachine {
        fn snapshot(&mut self) {}
        fn entity_count(&self) -> usize {
            self.count
        }
        fn entity_capacity(&self) -> usize {
            self.capacity
        }
        fn set_entity_count(&mut self, count: usize) {
            self.count = count;
        }
        fn clear_entity(&mut self, _slot: usize) {}
        fn save_global_pairs(&self) -> Vec<QcTextPair> {
            vec![QcTextPair {
                key: "time".to_string(),
                value: "1".to_string(),
            }]
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
        fn apply_global_pairs(&mut self, pairs: &[QcTextPair]) -> Vec<QcTextPair> {
            pairs.iter().filter(|pair| pair.key == "mystery").cloned().collect()
        }
        fn apply_entity_pairs(&mut self, slot: usize, pairs: &[QcTextPair]) -> Q1AppliedEntity {
            self.applied.push(pairs.to_vec());
            Q1AppliedEntity {
                unknown: pairs.iter().filter(|pair| pair.key == "mystery").cloned().collect(),
                empty: pairs.is_empty() && slot == 2,
            }
        }
    }

    struct FakeHost {
        began: usize,
        entities: Vec<(usize, bool)>,
        finished: bool,
    }

    impl Q1QuakeCSaveHost for FakeHost {
        fn begin(&mut self, entity_count: usize) {
            self.began = entity_count;
        }
        fn entity(&mut self, slot: usize, free: bool) {
            self.entities.push((slot, free));
        }
        fn finish(&mut self, _header: Q1SaveHeader) {
            self.finished = true;
        }
    }

    fn header() -> Q1SaveHeader {
        Q1SaveHeader {
            format: Q1SaveFormat::V5,
            comment: "test".to_string(),
            spawn_parameters: vec![0.0; 16],
            skill: 1,
            map: "e1m1".to_string(),
            time: 5.0,
            light_styles: vec!["m".to_string(); 64],
            extension_text: String::new(),
        }
    }

    #[test]
    fn capture_restore_round_trip_reports_unknowns() {
        let mut machine = FakeMachine {
            count: 3,
            capacity: 8,
            applied: Vec::new(),
        };
        let save = capture_q1_quakec_save(&mut machine, header(), &|slot| slot == 2);
        assert_eq!(save.entities.len(), 3);
        assert!(save.entities[2].is_empty());
        let mut host = FakeHost {
            began: 0,
            entities: Vec::new(),
            finished: false,
        };
        let unknowns = restore_q1_quakec_save(&mut machine, &save, &mut host).unwrap();
        assert_eq!(host.began, 3);
        assert_eq!(host.entities, vec![(0, false), (1, false), (2, true)]);
        assert!(host.finished);
        assert!(unknowns.globals.is_empty());
        assert!(unknowns.entities.is_empty());
    }

    #[test]
    fn capacity_is_enforced() {
        let mut machine = FakeMachine {
            count: 3,
            capacity: 2,
            applied: Vec::new(),
        };
        let save = capture_q1_quakec_save(&mut machine, header(), &|_| false);
        let mut host = FakeHost {
            began: 0,
            entities: Vec::new(),
            finished: false,
        };
        assert!(restore_q1_quakec_save(&mut machine, &save, &mut host).is_err());
    }
}
