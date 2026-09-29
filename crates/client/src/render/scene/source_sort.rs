//! Draw-surface sorting from id Software renderer/tr_main.c.
//!
//! Donor provenance: `src/render/scene/source-sort.ts` (`qsortFast`,
//! `shortsort`, `packSourceDrawSort`). The caller supplies unsigned sort
//! words and swaps both words and the surface binding.

use crate::render::error::RenderError;

/// Sortable draw-surface range: sort words plus surface swaps.
pub trait SortRange {
    /// Number of sortable entries.
    fn len(&self) -> usize;
    /// Whether the range holds no entries.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Sort word at an index.
    fn get_sort(&self, index: usize) -> u32;
    /// Replace the sort word at an index.
    fn set_sort(&mut self, index: usize, sort: u32);
    /// Exchange two entries, sort words and bindings together.
    fn swap(&mut self, first: usize, second: usize);
}

/// Pack a source draw-sort word: `shader:15 | entity:10 | fog:5 | dlight:2`.
pub fn pack_source_draw_sort(shader: u32, entity: u32, fog: u32, dlight: u32) -> Result<u32, RenderError> {
    crate::render::pack_draw_sort(shader, entity, fog, dlight).map_err(|error| RenderError::Backend(error.to_string()))
}

/// Push a deferred quicksort interval, keeping the source 30-entry work stack.
fn defer_interval(stack: &mut Vec<(usize, usize)>, lo: usize, hi: usize) -> Result<(), RenderError> {
    if stack.len() >= 30 {
        return Err(RenderError::OutOfOrder(
            "qsortFast exceeds the source 30-entry work stack".to_string(),
        ));
    }
    stack.push((lo, hi));
    Ok(())
}

/// Selection sort for tiny intervals: largest remaining entry goes last.
fn shortsort<R: SortRange>(range: &mut R, lo: usize, mut hi: usize) {
    while hi > lo {
        let mut max = lo;
        let mut p = lo + 1;
        while p <= hi {
            if range.get_sort(p) > range.get_sort(max) {
                max = p;
            }
            p += 1;
        }
        range.swap(max, hi);
        hi -= 1;
    }
}

/// Sort draw surfaces ascending by sort word.
pub fn sort_draw_surfs<R: SortRange>(range: &mut R) -> Result<(), RenderError> {
    if range.len() < 2 {
        return Ok(());
    }

    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut lo = 0usize;
    let mut hi = range.len() - 1;

    loop {
        let size = hi - lo + 1;
        if size <= 8 {
            shortsort(range, lo, hi);
        } else {
            let mid = lo + size / 2;
            range.swap(mid, lo);
            let mut loguy = lo;
            let mut higuy = hi + 1;

            loop {
                loop {
                    loguy += 1;
                    if loguy > hi || range.get_sort(loguy) > range.get_sort(lo) {
                        break;
                    }
                }
                loop {
                    higuy -= 1;
                    if higuy <= lo || range.get_sort(higuy) < range.get_sort(lo) {
                        break;
                    }
                }
                if higuy < loguy {
                    break;
                }
                range.swap(loguy, higuy);
            }

            range.swap(lo, higuy);

            // The source subtracts one byte here, not one drawSurf_t. Keep its byte comparison.
            let left = (higuy as i64 - lo as i64) * 8 - 1;
            let right = (hi as i64 - loguy as i64) * 8;
            if left >= right {
                if lo + 1 < higuy {
                    defer_interval(&mut stack, lo, higuy - 1)?;
                }
                if loguy < hi {
                    lo = loguy;
                    continue;
                }
            } else {
                if loguy < hi {
                    defer_interval(&mut stack, loguy, hi)?;
                }
                if lo + 1 < higuy {
                    hi = higuy - 1;
                    continue;
                }
            }
        }

        let Some((next_lo, next_hi)) = stack.pop() else {
            return Ok(());
        };
        lo = next_lo;
        hi = next_hi;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecRange {
        sorts: Vec<u32>,
    }

    impl VecRange {
        fn new(sorts: Vec<u32>) -> Self {
            Self { sorts }
        }
    }

    impl SortRange for VecRange {
        fn len(&self) -> usize {
            self.sorts.len()
        }

        fn get_sort(&self, index: usize) -> u32 {
            self.sorts[index]
        }

        fn set_sort(&mut self, index: usize, sort: u32) {
            self.sorts[index] = sort;
        }

        fn swap(&mut self, first: usize, second: usize) {
            self.sorts.swap(first, second);
        }
    }

    fn sorted(range: &mut VecRange) -> Vec<u32> {
        sort_draw_surfs(range).unwrap();
        range.sorts.clone()
    }

    #[test]
    fn short_ranges_are_noops() {
        let mut empty = VecRange::new(vec![]);
        assert_eq!(sorted(&mut empty), Vec::<u32>::new());
        let mut single = VecRange::new(vec![42]);
        assert_eq!(sorted(&mut single), vec![42]);
    }

    #[test]
    fn sorted_input_stays_sorted() {
        let mut range = VecRange::new(vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(sorted(&mut range), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn reverse_input_sorts_on_shortsort_path() {
        let mut range = VecRange::new(vec![8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(sorted(&mut range), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn quicksort_path_sorts_duplicates_and_edges() {
        let mut range = VecRange::new(vec![
            30, 5, 17, 5, 99, 1, 42, 8, 73, 12, 55, 3, 28, 61, 19, 90, 7, 44, 23, 11,
        ]);
        let mut expected = range.sorts.clone();
        expected.sort_unstable();
        assert_eq!(sorted(&mut range), expected);
        let mut bounds = VecRange::new(vec![u32::MAX, 0, 7, 7, 7, 100, 50, 25, 75, 1, 2, 3, u32::MAX, 0, 9, 9]);
        let mut expected_bounds = bounds.sorts.clone();
        expected_bounds.sort_unstable();
        assert_eq!(sorted(&mut bounds), expected_bounds);
    }

    #[test]
    fn sort_words_round_trip_through_set() {
        let mut range = VecRange::new(vec![3, 1, 2]);
        range.set_sort(0, 0);
        assert_eq!(range.get_sort(0), 0);
        assert_eq!(sorted(&mut range), vec![0, 1, 2]);
    }

    #[test]
    fn pack_matches_source_bit_layout() {
        assert_eq!(
            pack_source_draw_sort(1, 2, 3, 1).unwrap(),
            (1 << 17) | (2 << 7) | (3 << 2) | 1
        );
        assert_eq!(pack_source_draw_sort(0, 0, 0, 0).unwrap(), 0);
    }

    #[test]
    fn pack_rejects_out_of_range_fields() {
        assert!(pack_source_draw_sort(16384, 0, 0, 0).is_err());
        assert!(pack_source_draw_sort(0, 1023, 0, 0).is_err());
        assert!(pack_source_draw_sort(0, 0, 32, 0).is_err());
        assert!(pack_source_draw_sort(0, 0, 0, 4).is_err());
        assert!(matches!(
            pack_source_draw_sort(0, 0, 0, 4),
            Err(RenderError::Backend(_))
        ));
    }
}
