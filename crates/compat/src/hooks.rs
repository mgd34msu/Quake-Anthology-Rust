//! Optional VM instrumentation. The ordinary loop never calls it.
pub struct Hooks {
    pub instructions: u64,
    pub stores: u64,
    pub calls: u64,
    dirty_words: qa_core::bits::BitSet,
}
impl Hooks {
    pub(crate) fn load(bytes: usize) -> Self {
        Self {
            instructions: 0,
            stores: 0,
            calls: 0,
            dirty_words: qa_core::bits::BitSet::empty(bytes.div_ceil(4)),
        }
    }
    pub fn take_dirty_word(&mut self, word: usize) -> bool {
        self.dirty_words.test_and_clear(word)
    }
    pub(crate) fn write(&mut self, address: usize, length: usize) {
        if length == 0 {
            return;
        }
        for word in address / 4..=(address + length - 1) / 4 {
            self.dirty_words.insert(word);
        }
    }
}
