//! A row window consumes immutable prepared coverage. Geometry, shaders and
//! projection have already been evaluated by the common view preparation.
use super::{
    Assets, Buffers, Camera, DepthPolicy, PreparedDraw, RasterSelection, WorldBand, WorldPrepare,
    add_edge_stats, consume_spans,
};
use qa_core::primitives::IndexSpan;

fn fold_rejected(stats: &mut crate::BackendStats, delta: u64) {
    stats.rejected = stats
        .rejected
        .saturating_add(delta.min(u32::MAX as u64) as u32);
}

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
            span_groups: super::span_groups::SpanGroups::load(
                catalog.primitive_capacity,
                max_spans,
            ),
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
        if buffers
            .rows(buffers.frame_width, camera.refdef.viewport)
            .is_none()
        {
            return;
        }
        let before = self.stats.rejected;
        if let Some(source) = prepared.background {
            super::super::sky::background(
                buffers.frame_width,
                source,
                camera,
                assets,
                &mut buffers,
                &mut self.stats,
            );
        }
        fold_rejected(stats, self.stats.rejected - before);
        self.raster_range(
            prepared,
            camera,
            IndexSpan {
                first: 0,
                count: prepared.opaque_count as u32,
            },
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
        ranks: IndexSpan,
        assets: &Assets,
        mut buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        // Preserve each original rank's edge scan and rover operations. Only
        // the platform barrier changes; rows remain exclusively owned here.
        for rank in ranks.range() {
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
            PreparedDraw::External | PreparedDraw::SkyPending => false,
            PreparedDraw::Skip => true,
            PreparedDraw::Surface(range) => {
                let indices = prepared.bins.range(self.selection, range);
                for &index in indices {
                    let primitive = &prepared.geometry.primitives[index as usize];
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

    #[expect(
        clippy::too_many_arguments,
        reason = "Keep independent validated draw and span inputs explicit at the raster dispatch boundary"
    )]
    fn raster_range(
        &mut self,
        prepared: &WorldPrepare,
        camera: &Camera,
        range: IndexSpan,
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

    #[expect(
        clippy::too_many_arguments,
        reason = "Keep independent validated draw and span inputs explicit at the raster dispatch boundary"
    )]
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
        let Some(rows) = buffers.rows(buffers.frame_width, camera.refdef.viewport) else {
            return;
        };
        let old_rejected = self.stats.rejected;
        if !self.edges.begin_band(camera.refdef.viewport, rows, policy) {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
        } else {
            for &index in indices {
                let primitive = &prepared.geometry.primitives[index as usize];
                if opaque_only && primitive.overlay {
                    continue;
                }
                debug_assert_eq!(primitive.depth_policy, policy);
                self.edges.add_polygon(
                    index,
                    primitive.depth_key,
                    primitive.draw_rank,
                    prepared.bins.edges(primitive.edges),
                    primitive.winding,
                );
            }
            let width = buffers.frame_width;
            let cache = &mut self.cache;
            let counters = &mut self.stats;
            let catalog = &self.catalog;
            let groups = &mut self.span_groups;
            let edge_stats = self.edges.scan(|spans| {
                groups.consume(spans, |surface, chain, mips| {
                    consume_spans(
                        width,
                        chain,
                        mips,
                        prepared.geometry.primitives[surface],
                        &prepared.geometry.stages,
                        cache,
                        &catalog.rgba,
                        &prepared.rgba_prepared,
                        &catalog.factors,
                        &catalog.tiled_table,
                        assets,
                        camera,
                        buffers,
                        counters,
                    );
                });
            });
            add_edge_stats(&mut self.stats, edge_stats);
        }
        fold_rejected(stats, self.stats.rejected - old_rejected);
    }
}
