//! A row window consumes immutable prepared coverage. Geometry, shaders and
//! projection have already been evaluated by the common view preparation.
use super::{
    Assets, Buffers, Camera, DepthPolicy, PreparedDraw, RasterSelection, WorldBand, WorldPrepare,
    add_edge_stats, consume_span,
};

impl WorldBand {
    pub(super) fn load(
        width: u32,
        height: u32,
        catalog: std::sync::Arc<super::WorldCatalog>,
        cache_bytes: usize,
        max_spans: usize,
        selection: RasterSelection,
    ) -> Result<Self, &'static str> {
        if cache_bytes < catalog.mandatory_cache_bytes {
            return Err("CPU band cache cannot hold a mandatory surface");
        }
        Ok(Self {
            width,
            edges: super::Edges::load(
                width,
                height,
                catalog.edge_capacity,
                catalog.primitive_capacity,
                max_spans,
            )?,
            cache: super::SurfaceCache::load_shared(
                std::sync::Arc::clone(&catalog.surfaces_cache),
                cache_bytes,
            )?,
            catalog,
            stats: super::WorldStats::default(),
            selection,
        })
    }

    pub(super) fn render_opaque(
        &mut self,
        prepared: &WorldPrepare,
        camera: &Camera,
        assets: &Assets,
        mut buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        if buffers.rows(self.width, camera.refdef.viewport).is_none() {
            return;
        }
        let before = self.stats.rejected;
        if let Some(source) = prepared.background {
            super::super::sky::background(
                self.width,
                source,
                camera,
                assets,
                &mut buffers,
                &mut self.stats,
            );
        }
        stats.rejected = stats
            .rejected
            .saturating_add((self.stats.rejected - before).min(u32::MAX as u64) as u32);
        self.raster_range(
            prepared,
            camera,
            [0, prepared.opaque_count],
            true,
            prepared.policy,
            assets,
            &mut buffers,
            stats,
        );
    }

    pub(super) fn draw_range(
        &mut self,
        prepared: &WorldPrepare,
        camera: &Camera,
        ranks: [usize; 2],
        assets: &Assets,
        mut buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        // Preserve each original rank's edge scan and rover operations. Only
        // the platform barrier changes; rows remain exclusively owned here.
        for rank in ranks[0]..ranks[1] {
            self.draw_item(prepared, camera, rank, assets, buffers.reborrow(), stats);
        }
    }

    pub(super) fn draw_item(
        &mut self,
        prepared: &WorldPrepare,
        camera: &Camera,
        rank: usize,
        assets: &Assets,
        mut buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) -> bool {
        let Some(&draw) = prepared
            .draws
            .get(rank)
            .filter(|_| rank < prepared.draw_count)
        else {
            return false;
        };
        match draw {
            PreparedDraw::External => false,
            PreparedDraw::Skip => true,
            PreparedDraw::Surface(range) => {
                let indices = prepared.bins.range(self.selection, range);
                for &index in indices {
                    let primitive = &prepared.primitives[index as usize];
                    if primitive.overlay {
                        self.raster_indices(
                            prepared,
                            camera,
                            std::slice::from_ref(&index),
                            false,
                            DepthPolicy::PlaneDepth,
                            assets,
                            &mut buffers,
                            stats,
                        );
                    }
                }
                true
            }
            PreparedDraw::Sky { boxes, clouds } => {
                self.raster_range(
                    prepared,
                    camera,
                    boxes,
                    false,
                    DepthPolicy::PlaneDepth,
                    assets,
                    &mut buffers,
                    stats,
                );
                self.raster_range(
                    prepared,
                    camera,
                    clouds,
                    false,
                    DepthPolicy::PlaneDepth,
                    assets,
                    &mut buffers,
                    stats,
                );
                true
            }
        }
    }

    fn raster_range(
        &mut self,
        prepared: &WorldPrepare,
        camera: &Camera,
        range: [usize; 2],
        opaque_only: bool,
        policy: DepthPolicy,
        assets: &Assets,
        buffers: &mut Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        let indices = prepared.bins.range(self.selection, range);
        self.raster_indices(
            prepared,
            camera,
            indices,
            opaque_only,
            policy,
            assets,
            buffers,
            stats,
        );
    }

    fn raster_indices(
        &mut self,
        prepared: &WorldPrepare,
        camera: &Camera,
        indices: &[u32],
        opaque_only: bool,
        policy: DepthPolicy,
        assets: &Assets,
        buffers: &mut Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        if indices.is_empty() {
            return;
        }
        let Some(rows) = buffers.rows(self.width, camera.refdef.viewport) else {
            return;
        };
        let old_rejected = self.stats.rejected;
        let backend_rejected = stats.rejected;
        if !self.edges.begin_band(camera.refdef.viewport, rows, policy) {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
        } else {
            for &index in indices {
                let primitive = &prepared.primitives[index as usize];
                if opaque_only && primitive.overlay {
                    continue;
                }
                let vertices = &prepared.coverage
                    [primitive.first_coverage..primitive.first_coverage + primitive.coverage_count];
                if !self.edges.add_polygon(
                    index,
                    primitive.depth_key,
                    primitive.draw_rank,
                    vertices,
                ) {
                    self.stats.rejected = self.stats.rejected.saturating_add(1);
                }
            }
            let width = self.width;
            let cache = &mut self.cache;
            let counters = &mut self.stats;
            let catalog = &self.catalog;
            let edge_stats = self.edges.scan(|spans| {
                for &span in spans {
                    consume_span(
                        width,
                        span,
                        prepared.primitives[span.surface as usize],
                        &prepared.stages,
                        cache,
                        &catalog.rgba,
                        &prepared.rgba_prepared,
                        &catalog.factors,
                        assets,
                        camera,
                        buffers,
                        counters,
                    );
                }
            });
            add_edge_stats(&mut self.stats, edge_stats);
        }
        stats.rejected = backend_rejected
            .saturating_add((self.stats.rejected - old_rejected).min(u32::MAX as u64) as u32);
    }
}
