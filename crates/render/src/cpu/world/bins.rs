//! Ordered primitive indices for global native row coverage. Preparation owns
//! this fixed arena; workers borrow its completed per-band slices.
use super::{DepthPolicy, MAX_BANDS, Primitive, ProjectedVertex};
use crate::scene::Viewport;

#[derive(Clone, Copy)]
pub(super) enum RasterSelection {
    Band(usize),
    #[cfg(test)]
    Unbinned,
}

pub(super) struct CoverageBins {
    indices: Box<[u32]>,
    counts: [usize; MAX_BANDS],
    rows: [[u32; 2]; MAX_BANDS],
    count: usize,
    stride: usize,
    #[cfg(test)]
    unbinned: Box<[u32]>,
}

impl CoverageBins {
    pub(super) fn load(height: u32, count: usize, capacity: usize) -> Result<Self, &'static str> {
        if count == 0
            || count > MAX_BANDS
            || count > height as usize
            || height > 8192
            || capacity == 0
            || capacity > u32::MAX as usize
        {
            return Err("invalid CPU row index dimensions");
        }
        let length = capacity
            .checked_mul(count)
            .filter(|&n| n <= isize::MAX as usize / size_of::<u32>())
            .ok_or("CPU row index capacity overflow")?;
        let mut rows = [[0; 2]; MAX_BANDS];
        for (id, row) in rows.iter_mut().enumerate().take(count) {
            *row = [
                (id * height as usize / count) as u32,
                ((id + 1) * height as usize / count) as u32,
            ];
        }
        Ok(Self {
            indices: vec![0; length].into_boxed_slice(),
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

    pub(super) fn clear(&mut self) {
        self.counts.fill(0);
    }

    pub(super) fn rebuild(
        &mut self,
        viewport: Viewport,
        policy: DepthPolicy,
        opaque_count: usize,
        primitives: &[Primitive],
        coverage: &[ProjectedVertex],
    ) {
        self.clear();
        for (index, primitive) in primitives.iter().enumerate() {
            let vertices = &coverage
                [primitive.first_coverage..primitive.first_coverage + primitive.coverage_count];
            let depth_policy = if index < opaque_count && !primitive.overlay {
                policy
            } else {
                DepthPolicy::PlaneDepth
            };
            let certified = crate::edges::certified_rows(vertices, viewport, depth_policy);
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

    pub(super) fn range(&self, selection: RasterSelection, range: [usize; 2]) -> &[u32] {
        let indices = match selection {
            RasterSelection::Band(id) => {
                &self.indices[id * self.stride..id * self.stride + self.counts[id]]
            }
            #[cfg(test)]
            RasterSelection::Unbinned => return &self.unbinned[range[0]..range[1]],
        };
        let first = indices.partition_point(|&index| (index as usize) < range[0]);
        let end = indices.partition_point(|&index| (index as usize) < range[1]);
        &indices[first..end]
    }
}
