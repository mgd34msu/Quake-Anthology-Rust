//! Fixed load-sized membership and ascending word scans.
#[derive(Clone, Debug, Default)]
pub struct BitSet {
    words: Box<[u64]>,
    len: usize,
}
impl BitSet {
    pub fn empty(len: usize) -> Self {
        Self::load(len, false)
    }
    pub fn filled(len: usize) -> Self {
        Self::load(len, true)
    }
    fn load(len: usize, filled: bool) -> Self {
        Self {
            words: vec![if filled { u64::MAX } else { 0 }; len.div_ceil(64)].into_boxed_slice(),
            len,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn words(&self) -> &[u64] {
        &self.words
    }
    pub fn clear(&mut self) {
        self.words.fill(0);
    }
    pub fn contains(&self, bit: usize) -> bool {
        bit < self.len && self.words[bit / 64] & (1 << (bit % 64)) != 0
    }
    /// Return whether membership changed; out-of-capacity indices stay clear.
    pub fn insert(&mut self, bit: usize) -> bool {
        self.update(bit, true)
    }
    pub fn remove(&mut self, bit: usize) -> bool {
        self.update(bit, false)
    }
    pub fn test_and_clear(&mut self, bit: usize) -> bool {
        self.remove(bit)
    }
    fn update(&mut self, bit: usize, value: bool) -> bool {
        if bit >= self.len {
            return false;
        }
        let mask = 1 << (bit % 64);
        let word = &mut self.words[bit / 64];
        let old = *word & mask != 0;
        *word = (*word & !mask) | if value { mask } else { 0 };
        old != value
    }
    pub fn next_set(&self, start: usize) -> Option<usize> {
        self.next(start, false)
    }
    pub fn next_clear(&self, start: usize) -> Option<usize> {
        self.next(start, true)
    }
    fn next(&self, start: usize, inverse: bool) -> Option<usize> {
        if start >= self.len {
            return None;
        }
        let mut word = start / 64;
        let mut mask = u64::MAX << (start % 64);
        while word < self.words.len() {
            let bits = if inverse {
                !self.words[word]
            } else {
                self.words[word]
            } & mask;
            if bits != 0 {
                let bit = word * 64 + bits.trailing_zeros() as usize;
                return (bit < self.len).then_some(bit);
            }
            word += 1;
            mask = u64::MAX;
        }
        None
    }
}
