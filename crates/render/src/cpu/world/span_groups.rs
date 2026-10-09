//! Surface chains borrow a single bounded scanner flush. A cached texel loan
//! ends before the next surface can reuse the rover; no frame pointer survives.
use crate::edges::Span;
use qa_core::stamps::StampSet;

const NONE: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Group {
    head: usize,
    tail: usize,
    len: usize,
}

pub(super) struct SpanGroups {
    groups: Box<[Group]>,
    next: Box<[usize]>,
    order: Box<[usize]>,
    mips: Box<[[u8; 2]]>,
    marks: StampSet,
}

#[derive(Clone, Copy)]
pub(super) struct SpanChain<'a> {
    spans: &'a [Span],
    links: &'a [usize],
    next: usize,
    remaining: usize,
}

impl Iterator for SpanChain<'_> {
    type Item = (usize, Span);

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NONE {
            return None;
        }
        let index = self.next;
        self.next = self.links[index];
        self.remaining -= 1;
        Some((index, self.spans[index]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }

    fn count(self) -> usize {
        self.remaining
    }
}

impl ExactSizeIterator for SpanChain<'_> {}

impl SpanGroups {
    pub(super) fn load(primitives: usize, spans: usize) -> Self {
        Self {
            groups: vec![
                Group {
                    head: NONE,
                    tail: NONE,
                    len: 0
                };
                primitives
            ]
            .into_boxed_slice(),
            next: vec![NONE; spans].into_boxed_slice(),
            order: vec![0; primitives.min(spans)].into_boxed_slice(),
            mips: vec![[0; 2]; spans].into_boxed_slice(),
            marks: StampSet::new(primitives),
        }
    }

    pub(super) fn capacity_bytes(&self) -> usize {
        self.groups.len() * size_of::<Group>()
            + (self.next.len() + self.order.len()) * size_of::<usize>()
            + self.mips.len() * size_of::<[u8; 2]>()
            + self.marks.len() * size_of::<u32>()
    }

    pub(super) fn consume(
        &mut self,
        spans: &[Span],
        mut consume: impl FnMut(usize, SpanChain<'_>, &mut [[u8; 2]]),
    ) {
        self.marks.begin();
        let mut groups = 0;
        for (index, span) in spans.iter().enumerate() {
            let surface = span.surface as usize;
            self.next[index] = NONE;
            if !self.marks.test_and_set(surface) {
                self.groups[surface] = Group {
                    head: index,
                    tail: index,
                    len: 1,
                };
                self.order[groups] = surface;
                groups += 1;
            } else {
                self.next[self.groups[surface].tail] = index;
                self.groups[surface].tail = index;
                self.groups[surface].len += 1;
            }
        }
        // Preserve first occurrence order and each surface's original spans.
        // The scanner has already selected disjoint visible intervals; overlay
        // scans remain separate draw/primitive barriers in WorldBand.
        for &surface in &self.order[..groups] {
            consume(
                surface,
                SpanChain {
                    spans,
                    links: &self.next,
                    next: self.groups[surface].head,
                    remaining: self.groups[surface].len,
                },
                &mut self.mips,
            );
        }
    }
}
