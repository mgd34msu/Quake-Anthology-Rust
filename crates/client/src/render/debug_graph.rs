//! Quake II frame/network debug graphs (`cl_scrn.c`).
//!
//! Donor provenance: `src/render/debug-graph.ts` (`SourceDebugGraph`,
//! `SCR_DebugGraph`, `SCR_DrawDebugGraph`, `CL_AddNetgraph`). Samples live in
//! a 1024-entry ring; [`SourceDebugGraph::bars`] projects the newest samples
//! right-to-left onto one-pixel-wide bars over a dark background bar.

use qa_core::math::Vec4;

use super::types::Rect;

/// Ring capacity: the donor masks indices with `1023`.
pub const DEBUG_GRAPH_CAPACITY: usize = 1024;
/// Background bar palette index.
pub const DEBUG_GRAPH_BACKGROUND: i32 = 8;

/// Graph toggles and vertical mapping (`DebugGraphSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugGraphSettings {
    /// Nonzero enables the value graph.
    pub debuggraph: i32,
    /// Nonzero enables the frame-time graph.
    pub timegraph: i32,
    /// Nonzero enables the network graph.
    pub netgraph: i32,
    /// Graph height in pixels; non-positive heights draw nothing.
    pub height: f32,
    /// Vertical scale applied to each sample.
    pub scale: f32,
    /// Vertical shift applied after scaling.
    pub shift: f32,
}

/// One filled bar: background or a single sample column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugGraphBar {
    /// Bar rectangle in graph coordinates.
    pub rect: Rect,
    /// Palette index.
    pub color: i32,
}

/// One network sample for [`SourceDebugGraph::add_network`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetworkPacketSample {
    /// Dropped packets.
    pub dropped: u32,
    /// Suppressed packets.
    pub suppressed: u32,
    /// Ping in milliseconds.
    pub ping_milliseconds: f32,
}

/// Fill target for [`SourceDebugGraph::draw`]. The text-layer `Draw2D` works
/// on text rectangles plus picture assets, so graph drawing targets this
/// renderer-local surface instead.
pub trait DebugGraphSink {
    /// Fill `rect` with a solid `color`.
    fn fill_rect(&mut self, rect: &Rect, color: Vec4);
}

/// Resolve palette `index` to an opaque color. The index wraps mod 256 and
/// the palette holds 768 RGB bytes.
///
/// # Panics
///
/// Panics when `palette` is shorter than the resolved triple, matching the
/// donor's throw on an incomplete palette.
#[must_use]
pub fn debug_graph_color(palette: &[u8], index: i32) -> Vec4 {
    let offset = (index & 255) as usize * 3;
    let triple = palette.get(offset..offset + 3).expect("Incomplete debug graph palette");
    Vec4 {
        x: f32::from(triple[0]) / 255.0,
        y: f32::from(triple[1]) / 255.0,
        z: f32::from(triple[2]) / 255.0,
        w: 1.0,
    }
}

/// Ring-buffered debug graph samples.
#[derive(Debug, Clone)]
pub struct SourceDebugGraph {
    values: [f32; DEBUG_GRAPH_CAPACITY],
    colors: [i32; DEBUG_GRAPH_CAPACITY],
    current: usize,
}

impl SourceDebugGraph {
    /// Empty graph.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: [0.0; DEBUG_GRAPH_CAPACITY],
            colors: [0; DEBUG_GRAPH_CAPACITY],
            current: 0,
        }
    }

    /// Record one sample.
    pub fn add(&mut self, value: f32, color: i32) {
        self.values[self.current] = value;
        self.colors[self.current] = color;
        self.current = (self.current + 1) & (DEBUG_GRAPH_CAPACITY - 1);
    }

    /// Record a frame time in seconds when the time graph is enabled.
    pub fn add_frame(&mut self, seconds: f32, settings: &DebugGraphSettings) {
        if settings.timegraph != 0 {
            self.add(seconds * 300.0, 0);
        }
    }

    /// Record network activity unless a value/time graph already owns the
    /// display.
    pub fn add_network(&mut self, packet: &NetworkPacketSample, settings: &DebugGraphSettings) {
        if settings.debuggraph != 0 || settings.timegraph != 0 {
            return;
        }
        for _ in 0..packet.dropped {
            self.add(30.0, 0x40);
        }
        for _ in 0..packet.suppressed {
            self.add(30.0, 0xdf);
        }
        self.add((packet.ping_milliseconds / 30.0).trunc().min(30.0), 0xd0);
    }

    /// Project the ring onto bars: a background bar plus one bar per pixel
    /// column, newest sample at the right edge.
    #[must_use]
    pub fn bars(&self, rect: &Rect, settings: &DebugGraphSettings) -> Vec<DebugGraphBar> {
        if settings.debuggraph == 0 && settings.timegraph == 0 && settings.netgraph == 0 {
            return Vec::new();
        }
        if !settings.height.is_finite() {
            return Vec::new();
        }
        let height = settings.height.trunc() as i32;
        if height <= 0 {
            return Vec::new();
        }
        let base = rect.y + rect.height;
        let mut bars = vec![DebugGraphBar {
            rect: Rect {
                x: rect.x,
                y: base - settings.height,
                width: rect.width,
                height: settings.height,
            },
            color: DEBUG_GRAPH_BACKGROUND,
        }];
        let columns = if rect.width > 0.0 {
            rect.width.ceil() as usize
        } else {
            0
        };
        for column in 0..columns {
            let slot =
                (self.current + DEBUG_GRAPH_CAPACITY - 1 - (column % DEBUG_GRAPH_CAPACITY)) % DEBUG_GRAPH_CAPACITY;
            let mut value = self.values[slot].mul_add(settings.scale, settings.shift);
            if value < 0.0 {
                value += settings.height * (1.0 + (-value / settings.height).trunc());
            }
            let bar_height = value.trunc() as i32 % height;
            bars.push(DebugGraphBar {
                rect: Rect {
                    x: rect.x + rect.width - 1.0 - column as f32,
                    y: base - bar_height as f32,
                    width: 1.0,
                    height: bar_height as f32,
                },
                color: self.colors[slot],
            });
        }
        bars
    }

    /// Fill every bar through `sink`, resolving colors with `palette_color`.
    pub fn draw(
        &self,
        sink: &mut impl DebugGraphSink,
        rect: &Rect,
        settings: &DebugGraphSettings,
        palette_color: impl Fn(i32) -> Vec4,
    ) {
        for bar in self.bars(rect, settings) {
            sink.fill_rect(&bar.rect, palette_color(bar.color));
        }
    }
}

impl Default for SourceDebugGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> DebugGraphSettings {
        DebugGraphSettings {
            debuggraph: 1,
            timegraph: 0,
            netgraph: 0,
            height: 64.0,
            scale: 1.0,
            shift: 0.0,
        }
    }

    fn rect() -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            width: 8.0,
            height: 64.0,
        }
    }

    #[test]
    fn disabled_graphs_draw_nothing() {
        let graph = SourceDebugGraph::new();
        let off = DebugGraphSettings {
            debuggraph: 0,
            timegraph: 0,
            netgraph: 0,
            ..settings()
        };
        assert!(graph.bars(&rect(), &off).is_empty());
    }

    #[test]
    fn invalid_heights_draw_nothing() {
        let graph = SourceDebugGraph::new();
        for height in [0.0, -4.0, f32::NAN, f32::INFINITY, 0.9] {
            let settings = DebugGraphSettings { height, ..settings() };
            assert!(graph.bars(&rect(), &settings).is_empty(), "height {height}");
        }
    }

    #[test]
    fn first_bar_is_the_background() {
        let graph = SourceDebugGraph::new();
        let bars = graph.bars(&rect(), &settings());
        assert_eq!(bars.len(), 9, "background plus one bar per column");
        assert_eq!(
            bars[0],
            DebugGraphBar {
                rect: Rect {
                    x: 10.0,
                    y: 20.0,
                    width: 8.0,
                    height: 64.0,
                },
                color: DEBUG_GRAPH_BACKGROUND,
            }
        );
    }

    #[test]
    fn newest_sample_draws_at_the_right_edge() {
        let mut graph = SourceDebugGraph::new();
        graph.add(10.0, 3);
        graph.add(20.0, 5);
        let bars = graph.bars(&rect(), &settings());
        assert_eq!(bars[1].color, 5);
        assert_eq!(bars[1].rect.height, 20.0);
        assert_eq!(bars[1].rect.x, 17.0);
        assert_eq!(bars[2].color, 3);
        assert_eq!(bars[2].rect.height, 10.0);
        assert_eq!(bars[2].rect.x, 16.0);
    }

    #[test]
    fn ring_wraps_after_1024_samples() {
        let mut graph = SourceDebugGraph::new();
        for sample in 0..DEBUG_GRAPH_CAPACITY as i32 + 2 {
            graph.add(sample as f32, sample);
        }
        let bars = graph.bars(&rect(), &settings());
        assert_eq!(bars[1].color, DEBUG_GRAPH_CAPACITY as i32 + 1);
        assert_eq!(bars[2].color, DEBUG_GRAPH_CAPACITY as i32);
        assert_eq!(bars[1].rect.height, 1025.0 % 64.0);
    }

    #[test]
    fn negative_values_wrap_into_range() {
        let mut graph = SourceDebugGraph::new();
        graph.add(-5.0, 1);
        let bars = graph.bars(&rect(), &settings());
        assert_eq!(bars[1].rect.height, 59.0);
        assert!(bars[1].rect.y >= rect().y);
    }

    #[test]
    fn tall_values_wrap_modulo_height() {
        let mut graph = SourceDebugGraph::new();
        graph.add(130.0, 1);
        let bars = graph.bars(&rect(), &settings());
        assert_eq!(bars[1].rect.height, 130.0 % 64.0);
    }

    #[test]
    fn frame_samples_scale_by_300_and_need_timegraph() {
        let mut graph = SourceDebugGraph::new();
        graph.add_frame(0.05, &settings());
        assert_eq!(graph.bars(&rect(), &settings())[1].rect.height, 0.0);
        let time = DebugGraphSettings {
            timegraph: 1,
            ..settings()
        };
        graph.add_frame(0.05, &time);
        assert_eq!(graph.bars(&rect(), &time)[1].rect.height, 15.0);
    }

    #[test]
    fn network_samples_cover_drops_suppression_and_ping() {
        let net = DebugGraphSettings {
            debuggraph: 0,
            netgraph: 1,
            ..settings()
        };
        let mut graph = SourceDebugGraph::new();
        graph.add_network(
            &NetworkPacketSample {
                dropped: 2,
                suppressed: 1,
                ping_milliseconds: 90.0,
            },
            &net,
        );
        let bars = graph.bars(&rect(), &net);
        assert_eq!(bars[1].color, 0xd0);
        assert_eq!(bars[1].rect.height, 3.0);
        assert_eq!(bars[2].color, 0xdf);
        assert_eq!(bars[3].color, 0x40);
        assert_eq!(bars[4].color, 0x40);
    }

    #[test]
    fn network_samples_are_suppressed_by_value_graphs() {
        let mut graph = SourceDebugGraph::new();
        graph.add_network(
            &NetworkPacketSample {
                dropped: 3,
                suppressed: 3,
                ping_milliseconds: 90.0,
            },
            &settings(),
        );
        let bars = graph.bars(&rect(), &settings());
        assert!(bars[1..].iter().all(|bar| bar.rect.height == 0.0));
    }

    #[test]
    fn palette_index_wraps_mod_256() {
        let mut palette = vec![0u8; 768];
        palette[0..3].copy_from_slice(&[255, 0, 0]);
        assert_eq!(
            debug_graph_color(&palette, 0),
            Vec4 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                w: 1.0
            }
        );
        assert_eq!(debug_graph_color(&palette, 256), debug_graph_color(&palette, 0));
        assert_eq!(debug_graph_color(&palette, -1), debug_graph_color(&palette, 255));
    }

    #[test]
    #[should_panic(expected = "Incomplete debug graph palette")]
    fn short_palette_panics() {
        debug_graph_color(&[1, 2, 3], 200);
    }

    struct RecordingSink {
        fills: Vec<(Rect, Vec4)>,
    }

    impl DebugGraphSink for RecordingSink {
        fn fill_rect(&mut self, rect: &Rect, color: Vec4) {
            self.fills.push((*rect, color));
        }
    }

    #[test]
    fn draw_fills_every_bar() {
        let mut graph = SourceDebugGraph::new();
        graph.add(4.0, 7);
        let mut sink = RecordingSink { fills: Vec::new() };
        let palette = vec![9u8; 768];
        graph.draw(&mut sink, &rect(), &settings(), |index| {
            debug_graph_color(&palette, index)
        });
        assert_eq!(sink.fills.len(), 9);
        assert_eq!(sink.fills[0].0, graph.bars(&rect(), &settings())[0].rect);
    }
}
