//! Optional VM instrumentation. The ordinary loop never calls it.
pub struct Hooks {
    pub instructions: u64,
    pub stores: u64,
    pub calls: u64,
    dirty_words: Box<[u64]>,
}
impl Hooks {
    pub(crate) fn load(bytes: usize) -> Self {
        Self {
            instructions: 0,
            stores: 0,
            calls: 0,
            dirty_words: vec![0; bytes.div_ceil(256)].into_boxed_slice(),
        }
    }
    pub fn take_dirty_word(&mut self, word: usize) -> bool {
        let Some(bits) = self.dirty_words.get_mut(word / 64) else {
            return false;
        };
        let mask = 1 << (word % 64);
        let dirty = *bits & mask != 0;
        *bits &= !mask;
        dirty
    }
    pub(crate) fn write(&mut self, address: usize, length: usize) {
        if length == 0 {
            return;
        }
        for word in address / 4..=(address + length - 1) / 4 {
            self.dirty_words[word / 64] |= 1 << (word % 64);
        }
    }
}
