//! Load-sized membership marks with one shared epoch rollover policy.

pub struct StampSet {
    marks: Box<[u32]>,
    epoch: u32,
}

impl StampSet {
    pub fn new(capacity: usize) -> Self {
        Self {
            marks: vec![0; capacity].into_boxed_slice(),
            epoch: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn begin(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.epoch = 1;
        }
    }

    pub fn contains(&self, index: usize) -> bool {
        self.marks[index] == self.epoch
    }

    pub fn mark(&mut self, index: usize) {
        self.marks[index] = self.epoch;
    }

    /// Marks an owned index and returns whether it was already in this epoch.
    pub fn test_and_set(&mut self, index: usize) -> bool {
        let previous = self.contains(index);
        self.mark(index);
        previous
    }
}

#[cfg(test)]
mod tests {
    use super::StampSet;

    #[test]
    fn rollover_clears_old_one_and_maximum_epoch_membership() {
        let mut set = StampSet::new(3);
        set.mark(0);
        set.epoch = u32::MAX;
        set.mark(1);
        set.begin();
        assert_eq!(set.epoch, 1);
        assert!((0..set.len()).all(|index| !set.contains(index)));
        assert!(!set.test_and_set(0));
        assert!(set.test_and_set(0));
        set.begin();
        assert!(!set.contains(0));
    }
}
