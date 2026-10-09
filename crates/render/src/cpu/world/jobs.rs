//! Scoped CPU jobs borrow a frozen view and own preparation chunks or raster
//! rows. The callback completes all published jobs before these loans end.
use super::{
    Assets, Buffers, Camera, MAX_BANDS, PreparedDraw, WorldBand, WorldPrepare, WorldRaster,
};

#[derive(Clone, Copy)]
enum BandPass {
    Opaque,
    DrawRange([usize; 2]),
}

struct BandWork<'a> {
    band: &'a mut WorldBand,
    prepared: &'a WorldPrepare,
    camera: &'a Camera,
    assets: &'a Assets,
    buffers: Buffers<'a>,
    pass: BandPass,
}

/// A job is valid only during its render_with_dispatch callback. The callback
/// must execute every supplied job once and return after all jobs complete.
pub struct CpuJob<'a> {
    band_id: usize,
    rows: [u32; 2],
    work: Option<CpuWork<'a>>,
    kind: JobKind,
    complete: bool,
    stats: crate::BackendStats,
}
impl CpuJob<'_> {
    fn empty() -> Self {
        Self {
            band_id: 0,
            rows: [0; 2],
            work: None,
            kind: JobKind::Raster,
            complete: false,
            stats: crate::BackendStats::default(),
        }
    }
    pub fn kind(&self) -> JobKind {
        self.kind
    }
    pub fn band_id(&self) -> usize {
        self.band_id
    }
    pub fn rows(&self) -> Option<std::ops::Range<u32>> {
        (self.kind == JobKind::Raster).then_some(self.rows[0]..self.rows[1])
    }
}

/// Runs one owned CPU job. A second invocation does no work; the first consumes
/// its preparation or raster output loans.
pub fn run_cpu_job(job: &mut CpuJob<'_>) {
    let Some(work) = job.work.take() else {
        return;
    };
    job.complete = true;
    let work = match work {
        CpuWork::Prepare(work) => {
            work.chunk.stats = super::WorldStats::default();
            job.complete = work.chunk.prepare_references(
                work.camera,
                work.references,
                work.first_world,
                work.assets,
                work.evaluator,
                super::RgbaInput::Ready(work.rgba),
                &mut job.stats,
            );
            return;
        }
        CpuWork::Raster(work) => work,
    };
    match work.pass {
        BandPass::Opaque => work.band.render_opaque(
            work.prepared,
            work.camera,
            work.assets,
            work.buffers,
            &mut job.stats,
        ),
        BandPass::DrawRange(ranks) => {
            work.band.draw_range(
                work.prepared,
                work.camera,
                ranks,
                work.assets,
                work.buffers,
                &mut job.stats,
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Prepare,
    Raster,
}

struct PrepareWork<'a> {
    chunk: &'a mut super::SurfacePrepare,
    references: &'a [super::SurfaceRef],
    first_world: Option<super::WorldId>,
    rgba: &'a [Option<super::super::rgba::Prepared>],
    camera: &'a Camera,
    assets: &'a Assets,
    evaluator: &'a super::StageEvaluator,
}
enum CpuWork<'a> {
    Prepare(PrepareWork<'a>),
    Raster(BandWork<'a>),
}
impl WorldRaster {
    #[expect(
        clippy::too_many_arguments,
        reason = "One scoped dispatcher receives frozen view inputs and owned outputs"
    )]
    pub(in crate::cpu) fn prepare_view_dispatched<E>(
        &mut self,
        camera: &Camera,
        list: &super::CommandList,
        scene: super::SceneRanges,
        assets: &Assets,
        evaluator: &super::StageEvaluator,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
    ) -> Result<bool, E> {
        let references = list.surfaces(scene.surfaces);
        if self.lanes.is_empty()
            || references.len() < self.lanes.len()
            || self
                .prepare
                .geometry
                .requirements(references)
                .is_none_or(|total| {
                    !self.prepare.geometry.fits(total)
                        || total.primitives
                            < self.lanes.len() * self.config.prepare_minimum_primitives_per_job
                })
            || self.lanes.iter().enumerate().any(|(id, lane)| {
                let first = id * references.len() / self.lanes.len();
                let end = (id + 1) * references.len() / self.lanes.len();
                !lane.admits(&references[first..end])
            })
        {
            return Ok(self
                .prepare
                .prepare_view(camera, list, scene, assets, evaluator, stats));
        }
        let previous_stats = *stats;
        let previous_world_stats = self.prepare.geometry.stats;
        if !self.prepare.begin_view(camera, list, scene, assets, stats) {
            return Ok(false);
        }
        self.prepare
            .prepare_colors(references, camera, assets, evaluator);
        let count = self.lanes.len();
        let first_world = references.first().map(|r| r.world);
        {
            let mut jobs: [CpuJob<'_>; MAX_BANDS] = std::array::from_fn(|_| CpuJob::empty());
            for (id, (job, lane)) in jobs.iter_mut().zip(self.lanes.iter_mut()).enumerate() {
                let first = id * references.len() / count;
                let end = (id + 1) * references.len() / count;
                *job = CpuJob {
                    band_id: id,
                    rows: [0; 2],
                    kind: JobKind::Prepare,
                    complete: false,
                    work: Some(CpuWork::Prepare(PrepareWork {
                        chunk: lane,
                        references: &references[first..end],
                        first_world,
                        rgba: &self.prepare.rgba_prepared,
                        camera,
                        assets,
                        evaluator,
                    })),
                    stats: crate::BackendStats::default(),
                };
            }
            let result = dispatch(&mut jobs[..count]);
            for job in &jobs[..count] {
                stats.surfaces = stats.surfaces.saturating_add(job.stats.surfaces);
                stats.stages = stats.stages.saturating_add(job.stats.stages);
                stats.rejected = stats.rejected.saturating_add(job.stats.rejected);
            }
            result?;
            if jobs[..count].iter().any(|job| !job.complete) {
                return Ok(false);
            }
        }
        // Clipping can expand a nonconvex/deformed boundary beyond its static
        // load estimate. Check actual totals before copying; the same serial
        // preparer remains the bounded fallback for that view.
        let total = self
            .lanes
            .iter()
            .try_fold(super::PrepareCapacity::default(), |total, lane| {
                Some(super::PrepareCapacity {
                    primitives: total.primitives.checked_add(lane.primitive_count)?,
                    stages: total.stages.checked_add(lane.stage_count)?,
                    coverage: total.coverage.checked_add(lane.coverage_count)?,
                })
            });
        if self.lanes.iter().any(|lane| lane.capacity_exhausted)
            || !total.is_some_and(|total| self.prepare.geometry.fits(total))
        {
            *stats = previous_stats;
            self.prepare.geometry.stats = previous_world_stats;
            return Ok(self
                .prepare
                .prepare_view(camera, list, scene, assets, evaluator, stats));
        }
        self.prepare.geometry.certified = true;
        for (id, lane) in self.lanes.iter().enumerate() {
            let first = id * references.len() / count;
            let end = (id + 1) * references.len() / count;
            self.prepare.geometry.append(lane, first, end - first);
        }
        Ok(self
            .prepare
            .finish_view(camera, list, scene, assets, evaluator, stats))
    }

    fn dispatch<E>(
        &mut self,
        camera: &Camera,
        assets: &Assets,
        mut remainder: Buffers<'_>,
        pass: BandPass,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
    ) -> Result<(), E> {
        let count = self.bands.len();
        let height = remainder.frame_height;
        let mut jobs: [CpuJob<'_>; MAX_BANDS] = std::array::from_fn(|_| CpuJob::empty());
        for (id, (slot, band)) in jobs.iter_mut().zip(self.bands.iter_mut()).enumerate() {
            let first = (id * height as usize / count) as u32;
            let end = ((id + 1) * height as usize / count) as u32;
            let (buffers, tail) = remainder.split_rows(band.width, end - first);
            remainder = tail;
            *slot = CpuJob {
                band_id: id,
                rows: [first, end],
                work: Some(CpuWork::Raster(BandWork {
                    band,
                    prepared: &self.prepare,
                    camera,
                    assets,
                    buffers,
                    pass,
                })),
                kind: JobKind::Raster,
                complete: false,
                stats: crate::BackendStats::default(),
            };
        }
        let result = dispatch(&mut jobs[..count]);
        // Completion order cannot change rejection accounting. Counts already
        // attempted remain observable through the band state even on error.
        for job in &jobs[..count] {
            stats.rejected = stats.rejected.saturating_add(job.stats.rejected);
        }
        result
    }

    pub(in crate::cpu) fn render_opaque<E>(
        &mut self,
        camera: &Camera,
        assets: &Assets,
        buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
    ) -> Result<(), E> {
        self.dispatch(camera, assets, buffers, BandPass::Opaque, stats, dispatch)
    }

    pub(in crate::cpu) fn draw_run<E>(
        &mut self,
        camera: &Camera,
        rank: usize,
        assets: &Assets,
        buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
    ) -> Result<Option<usize>, E> {
        let mut end = rank;
        let mut raster = false;
        while end < self.prepare.draw_count {
            match self.draw(end) {
                PreparedDraw::External => break,
                PreparedDraw::Skip => {}
                PreparedDraw::Surface(_) | PreparedDraw::Sky { .. } => raster = true,
            }
            end += 1;
        }
        if end == rank {
            return Ok(None);
        }
        // Converted sky entities/polys belong to the prepared world stream.
        // External draws remain barriers; skips alone need no worker wake-up.
        if raster {
            self.dispatch(
                camera,
                assets,
                buffers,
                BandPass::DrawRange([rank, end]),
                stats,
                dispatch,
            )?;
        }
        Ok(Some(end))
    }
}
