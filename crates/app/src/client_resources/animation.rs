//! Accepted model transitions own their phase; rendering only borrows it.
use qa_core::{
    bits::BitSet,
    primitives::{ModelFileId, ModelRef},
    random::{Algorithm, Random},
};

#[derive(Clone, Copy, Default)]
struct ModelPhase {
    reference: Option<ModelRef>,
    source: Option<(ModelFileId, bool)>,
    random_sync: bool,
    base: f32,
}

pub struct ModelSynchronization {
    entities: Box<[[ModelPhase; 4]]>,
    occupied: BitSet,
    pending: BitSet,
    random: Random,
}
impl ModelSynchronization {
    pub fn load(entities: usize) -> Result<Self, &'static str> {
        let cells = entities.checked_mul(4).ok_or("model phase capacity")?;
        Ok(Self {
            entities: vec![[ModelPhase::default(); 4]; entities].into_boxed_slice(),
            occupied: BitSet::empty(entities),
            pending: BitSet::empty(cells),
            random: Random::with_algorithm(1, Algorithm::GlibcType3),
        })
    }

    pub fn base(&self, number: u32, cell: usize) -> f32 {
        self.entities
            .get(number as usize)
            .and_then(|row| row.get(cell))
            .map_or(0.0, |row| row.base)
    }

    /// A server header clears entities, while the owned CLIENT random stream
    /// continues. Neither a new view nor an image-policy reload calls this.
    pub fn clear(&mut self) {
        self.entities.fill([ModelPhase::default(); 4]);
        self.occupied.clear();
        self.pending.clear();
    }

    pub(crate) fn observe(
        &mut self,
        number: u32,
        cell: usize,
        reference: Option<ModelRef>,
        random_sync: bool,
        source: Result<Option<(ModelFileId, i32)>, ()>,
    ) -> bool {
        let Some(row) = self
            .entities
            .get_mut(number as usize)
            .and_then(|row| row.get_mut(cell))
        else {
            return false;
        };
        row.reference = reference;
        row.random_sync = random_sync;
        self.resolve_at(number as usize * 4 + cell, source);
        if self.entities[number as usize]
            .iter()
            .any(|row| row.reference.is_some())
        {
            self.occupied.insert(number as usize);
        } else {
            self.occupied.remove(number as usize);
        }
        true
    }

    /// NetQuake's CLIENT relink clears models absent from the last message.
    /// Run after intake, not between successive datagrams in the same frame.
    pub(crate) fn retire_missing(&mut self, mut present: impl FnMut(u32) -> bool) {
        let mut next = self.occupied.next_set(0);
        while let Some(number) = next {
            next = self.occupied.next_set(number + 1);
            if !present(number as u32) {
                self.entities[number].fill(ModelPhase::default());
                self.occupied.remove(number);
                for cell in 0..4 {
                    self.pending.remove(number * 4 + cell);
                }
            }
        }
    }

    pub(crate) fn resolve_pending(
        &mut self,
        mut lookup: impl FnMut(ModelRef) -> Result<Option<(ModelFileId, i32)>, ()>,
    ) {
        let mut next = self.pending.next_set(0);
        while let Some(index) = next {
            let reference = self.entities[index / 4][index % 4].reference;
            let source = reference.map_or(Ok(None), &mut lookup);
            self.resolve_at(index, source);
            next = self.pending.next_set(index + 1);
        }
    }

    fn resolve_at(&mut self, index: usize, source: Result<Option<(ModelFileId, i32)>, ()>) {
        let Ok(source) = source else {
            let row = &mut self.entities[index / 4][index % 4];
            row.source = None;
            row.base = 0.0;
            self.pending.insert(index);
            return;
        };
        self.pending.remove(index);
        let row = &mut self.entities[index / 4][index % 4];
        let source = source.map(|(name, sync)| (name, row.random_sync && sync == 1));
        if row.source != source {
            row.source = source;
            row.base = if source.is_some_and(|(_, random)| random) {
                (self.random.next_u32() & 0x7fff) as f32 / 32767.0
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/model_synchronization.rs"]
mod tests;
