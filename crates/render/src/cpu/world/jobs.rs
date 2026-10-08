//! Scoped raster jobs borrow a single frozen view and exclusively own row
//! windows. The callback completes all published jobs before these loans end.
use super::{
    Assets, Buffers, Camera, MAX_BANDS, PreparedDraw, WorldBand, WorldPrepare, WorldRaster,
};

#[derive(Clone, Copy)]
enum BandPass {
    Opaque,
    Draw(usize),
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
pub struct BandJob<'a> {
    band_id: usize,
    rows: [u32; 2],
    work: Option<BandWork<'a>>,
    stats: crate::BackendStats,
}
impl BandJob<'_> {
    fn empty() -> Self {
        Self {
            band_id: 0,
            rows: [0; 2],
            work: None,
            stats: crate::BackendStats::default(),
        }
    }
    pub fn band_id(&self) -> usize {
        self.band_id
    }
    pub fn rows(&self) -> std::ops::Range<u32> {
        self.rows[0]..self.rows[1]
    }
}

/// Runs one owned band. Calling this again on the same completed job does no
/// work; its rover and output loans were consumed by the first invocation.
pub fn render_band(job: &mut BandJob<'_>) {
    let Some(work) = job.work.take() else {
        return;
    };
    match work.pass {
        BandPass::Opaque => work.band.render_opaque(
            work.prepared,
            work.camera,
            work.assets,
            work.buffers,
            &mut job.stats,
        ),
        BandPass::Draw(rank) => {
            work.band.draw_item(
                work.prepared,
                work.camera,
                rank,
                work.assets,
                work.buffers,
                &mut job.stats,
            );
        }
    }
}

impl WorldRaster {
    fn dispatch<E>(
        &mut self,
        camera: &Camera,
        assets: &Assets,
        mut remainder: Buffers<'_>,
        pass: BandPass,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [BandJob<'job>]) -> Result<(), E>,
    ) -> Result<(), E> {
        let count = self.bands.len();
        let height = remainder.frame_height;
        let mut jobs: [BandJob<'_>; MAX_BANDS] = std::array::from_fn(|_| BandJob::empty());
        for (id, (slot, band)) in jobs.iter_mut().zip(self.bands.iter_mut()).enumerate() {
            let first = (id * height as usize / count) as u32;
            let end = ((id + 1) * height as usize / count) as u32;
            let (buffers, tail) = remainder.split_rows(band.width, end - first);
            remainder = tail;
            *slot = BandJob {
                band_id: id,
                rows: [first, end],
                work: Some(BandWork {
                    band,
                    prepared: &self.prepare,
                    camera,
                    assets,
                    buffers,
                    pass,
                }),
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
        list: &super::CommandList,
        scene: super::SceneRanges,
        assets: &Assets,
        evaluator: &super::StageEvaluator,
        buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [BandJob<'job>]) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.prepare_view(camera, list, scene, assets, evaluator, stats) {
            self.dispatch(camera, assets, buffers, BandPass::Opaque, stats, dispatch)
        } else {
            Ok(())
        }
    }

    pub(in crate::cpu) fn draw_item<E>(
        &mut self,
        camera: &Camera,
        rank: usize,
        assets: &Assets,
        buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [BandJob<'job>]) -> Result<(), E>,
    ) -> Result<bool, E> {
        match self.draw(rank) {
            PreparedDraw::External => Ok(false),
            PreparedDraw::Skip => Ok(true),
            PreparedDraw::Surface(_) | PreparedDraw::Sky { .. } => {
                self.dispatch(
                    camera,
                    assets,
                    buffers,
                    BandPass::Draw(rank),
                    stats,
                    dispatch,
                )?;
                Ok(true)
            }
        }
    }
}
