//! Ordered primitive indices for global native row coverage. Preparation owns
//! this fixed arena; workers borrow its completed per-band slices.
use super::{DepthPolicy, MAX_BANDS, Primitive, ProjectedVertex};
use crate::scene::Viewport;
use qa_core::primitives::IndexSpan;

#[derive(Clone, Copy)]
pub(super) enum RasterSelection {
    Band(usize),
    #[cfg(test)]
    Unbinned,
}

pub(super) struct CoverageBins {
    indices: Box<[u32]>,
    edges: Box<[crate::edges::Edge]>,
    counts: [usize; MAX_BANDS],
    rows: [[u32; 2]; MAX_BANDS],
    count: usize,
    stride: usize,
    #[cfg(test)]
    unbinned: Box<[u32]>,
}

impl CoverageBins {
    pub(super) fn load(
        height: u32,
        count: usize,
        capacity: usize,
        edges: usize,
    ) -> Result<Self, &'static str> {
        if count == 0
            || count > MAX_BANDS
            || count > height as usize
            || height > 8192
            || capacity == 0
            || capacity > u32::MAX as usize
            || edges > u32::MAX as usize
            || edges > isize::MAX as usize / size_of::<crate::edges::Edge>()
        {
            return Err("invalid CPU row index dimensions");
        }
        let length = capacity
            .checked_mul(count)
            .filter(|&n| n <= isize::MAX as usize / size_of::<u32>())
            .ok_or("CPU row index capacity overflow")?;
        let mut rows = [[0; 2]; MAX_BANDS];
        for (id, row) in rows.iter_mut().enumerate().take(count) {
            let range = super::partition(id, count, height as usize);
            *row = [range.start as u32, range.end as u32];
        }
        Ok(Self {
            indices: vec![0; length].into_boxed_slice(),
            edges: vec![crate::edges::Edge::default(); edges].into_boxed_slice(),
            counts: [0; MAX_BANDS],
            rows,
            count,
            stride: capacity,
            #[cfg(test)]
            unbinned: (0..capacity as u32).collect::<Vec<_>>().into_boxed_slice(),
        })
    }

    pub(super) fn capacity_bytes(&self) -> usize {
        self.indices.len() * size_of::<u32>()
    }
    pub(super) fn edge_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.edges)
    }
    pub(super) fn edges(&self, span: IndexSpan) -> &[crate::edges::Edge] {
        &self.edges[span.range()]
    }

    pub(super) fn clear(&mut self) {
        self.counts.fill(0);
    }

    pub(super) fn rebuild(
        &mut self,
        viewport: Viewport,
        policy: DepthPolicy,
        opaque_count: usize,
        primitives: &mut [Primitive],
        coverage: &[ProjectedVertex],
    ) {
        self.clear();
        let mut edge_count = 0;
        for (index, primitive) in primitives.iter_mut().enumerate() {
            let vertices = &coverage[primitive.coverage.range()];
            primitive.depth_policy = if index < opaque_count && !primitive.overlay {
                policy
            } else {
                DepthPolicy::PlaneDepth
            };
            primitive.edges = IndexSpan {
                first: edge_count as u32,
                count: 0,
            };
            match crate::edges::prepare_polygon(
                vertices,
                viewport,
                primitive.depth_policy,
                primitive.winding,
                &mut self.edges[edge_count..],
            ) {
                Ok((count, winding)) => {
                    primitive.edges.count = count as u32;
                    primitive.winding = Some(winding);
                    edge_count += count;
                }
                Err(()) => primitive.winding = None,
            }
            let certified = crate::edges::certified_rows(
                self.edges(primitive.edges),
                viewport,
                primitive.depth_policy,
                primitive.winding,
            );
            for id in 0..self.count {
                let start = self.rows[id][0].max(viewport.y);
                let end = self.rows[id][1].min(viewport.y + viewport.height);
                if start >= end
                    || certified
                        .as_ref()
                        .is_ok_and(|rows| rows.end <= start || rows.start >= end)
                {
                    continue;
                }
                self.indices[id * self.stride + self.counts[id]] = index as u32;
                self.counts[id] += 1;
            }
        }
    }

    pub(super) fn range(&self, selection: RasterSelection, span: IndexSpan) -> &[u32] {
        let indices = match selection {
            RasterSelection::Band(id) => {
                &self.indices[id * self.stride..id * self.stride + self.counts[id]]
            }
            #[cfg(test)]
            RasterSelection::Unbinned => return &self.unbinned[span.range()],
        };
        let first = indices.partition_point(|&index| index < span.first);
        let end = indices.partition_point(|&index| index < span.first + span.count);
        &indices[first..end]
    }
}
