//! Per-seat component drawing events.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/component-drawings.ts`
//! (`ComponentDrawings`).
//! Each viewing seat consumes source drawing events with their activation
//! and server clock. Source events arrive as the absorbed
//! [`ComponentDrawingEvent`] pick (donor `SimulationPresentationEvent`,
//! owned by the simulation lane); graph drawing targets the Rust
//! [`DebugGraphSink`] surface instead of the donor's `Draw2D` plus white
//! picture. World-text content tags hash to the `u32` the Rust store
//! carries opaquely.

use qa_client::render::debug_graph::{debug_graph_color, DebugGraphSettings, DebugGraphSink, SourceDebugGraph};
use qa_client::render::types::{Palette, Rect};
use qa_client::text::ui_world::{WorldText, WorldTextInput, WorldTextStore};
use qa_client::ClientError;
use qa_content::contract::{ContentId, PresentationOwner};
use qa_core::identity::ProviderId;
use thiserror::Error;

use crate::debug::{DebugError, DebugLine, WorldDebugLineStore};

/// Component drawings failure.
#[derive(Debug, Error)]
pub enum ComponentDrawingsError {
    /// A component debug graph changed source content.
    #[error("Component debug graph changed source content")]
    GraphContentChanged,
    /// Debug line submission failure.
    #[error(transparent)]
    Debug(#[from] DebugError),
    /// World text submission failure.
    #[error(transparent)]
    Text(#[from] ClientError),
}

/// Absorbed presentation event pick consumed by component drawings.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDrawingEvent {
    /// Presenting owner, when the source sets one.
    pub owner: Option<PresentationOwner>,
    /// Presentation sequence.
    pub sequence: i64,
    /// Source content.
    pub content: ContentId,
    /// Event time in seconds.
    pub seconds: f64,
    /// Event payload.
    pub kind: ComponentDrawingEventKind,
}

/// Consumed drawing sources.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentDrawingEventKind {
    /// Owner lifecycle (`presentation-owner`); `retired` marks retirement.
    PresentationOwner {
        /// Owner retired.
        retired: bool,
    },
    /// Debug graph sample (`debug-graph`).
    DebugGraph {
        /// Sample value.
        value: f32,
        /// Sample color.
        color: i32,
    },
    /// World text (`q2-rerelease` world-text).
    WorldText {
        /// Text input.
        text: WorldTextInput,
        /// Lifetime in seconds.
        lifetime: f64,
    },
    /// Debug shapes (`q2-rerelease` debug-shapes).
    DebugShapes {
        /// Lines.
        lines: Vec<DebugLine>,
        /// Lifetime in milliseconds.
        lifetime_ms: u32,
    },
    /// Any other source; skipped like the donor's filter.
    Other,
}

#[derive(Debug)]
struct DrawingGraph {
    content: ContentId,
    samples: SourceDebugGraph,
    palette: Option<Palette>,
}

#[derive(Debug)]
struct DrawingOwner {
    owner: PresentationOwner,
    lines: WorldDebugLineStore,
    text: WorldTextStore,
    graph: Option<DrawingGraph>,
    sequence: i64,
    retired: bool,
}

fn content_tag(content: &ContentId) -> u32 {
    // FNV-1a over the content text; the store carries the tag opaquely.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in content.as_str().as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Per-seat component drawings (`ComponentDrawings`).
#[derive(Debug, Default)]
pub struct ComponentDrawings {
    owners: Vec<(ProviderId, DrawingOwner)>,
}

impl ComponentDrawings {
    /// Empty drawings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume source drawing events (`receive`).
    pub fn receive(&mut self, events: &[ComponentDrawingEvent]) -> Result<(), ComponentDrawingsError> {
        for source in events {
            let lifecycle = matches!(source.kind, ComponentDrawingEventKind::PresentationOwner { .. });
            if !lifecycle
                && !matches!(
                    source.kind,
                    ComponentDrawingEventKind::DebugGraph { .. }
                        | ComponentDrawingEventKind::WorldText { .. }
                        | ComponentDrawingEventKind::DebugShapes { .. }
                )
            {
                continue;
            }
            let Some(owner) = source.owner.clone() else {
                continue;
            };
            let position = self.owners.iter().position(|(provider, _)| provider == &owner.provider);
            if let Some(index) = position {
                if self.owners[index].1.owner.generation > owner.generation {
                    continue;
                }
                if self.owners[index].1.owner.generation < owner.generation {
                    self.owners[index].1 = DrawingOwner {
                        owner,
                        lines: WorldDebugLineStore::new(),
                        text: WorldTextStore::new(),
                        graph: None,
                        sequence: -1,
                        retired: false,
                    };
                }
            } else {
                self.owners.push((
                    owner.provider.clone(),
                    DrawingOwner {
                        owner,
                        lines: WorldDebugLineStore::new(),
                        text: WorldTextStore::new(),
                        graph: None,
                        sequence: -1,
                        retired: false,
                    },
                ));
            }
            let index = position.unwrap_or_else(|| self.owners.len() - 1);
            let state = &mut self.owners[index].1;
            if source.sequence <= state.sequence {
                continue;
            }
            state.sequence = source.sequence;
            if let ComponentDrawingEventKind::PresentationOwner { retired } = &source.kind {
                state.lines.clear();
                state.text.clear();
                state.graph = None;
                if *retired {
                    state.retired = true;
                }
                continue;
            }
            if !state.retired && matches!(source.kind, ComponentDrawingEventKind::DebugGraph { .. }) {
                let ComponentDrawingEventKind::DebugGraph { value, color } = &source.kind else {
                    unreachable!("debug graph kind checked");
                };
                if state.graph.is_none() {
                    state.graph = Some(DrawingGraph {
                        content: source.content.clone(),
                        samples: SourceDebugGraph::new(),
                        palette: None,
                    });
                }
                let graph = state.graph.as_mut().expect("drawing graph installed");
                if graph.content != source.content {
                    return Err(ComponentDrawingsError::GraphContentChanged);
                }
                graph.samples.add(*value, *color);
                continue;
            }
            if state.retired {
                continue;
            }
            match &source.kind {
                ComponentDrawingEventKind::WorldText { text, lifetime } => {
                    state.text.submit(
                        WorldText {
                            input: text.clone(),
                            content: content_tag(&source.content),
                        },
                        source.seconds,
                        *lifetime,
                    )?;
                }
                ComponentDrawingEventKind::DebugShapes { lines, lifetime_ms } => {
                    state.lines.submit(lines, source.seconds * 1000.0, *lifetime_ms)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Load pending graph palettes (`prepareGraphs`).
    pub fn prepare_graphs<E>(
        &mut self,
        load_palette: &mut dyn FnMut(&ContentId) -> Result<Palette, E>,
    ) -> Result<(), E> {
        for (_, state) in &mut self.owners {
            let ready = state.graph.as_ref().is_some_and(|graph| graph.palette.is_some());
            if state.retired || state.graph.is_none() || ready {
                continue;
            }
            let content = state.graph.as_ref().expect("drawing graph checked").content.clone();
            let palette = load_palette(&content)?;
            if state.retired {
                continue;
            }
            if let Some(graph) = state.graph.as_mut() {
                if graph.content == content {
                    graph.palette = Some(palette);
                }
            }
        }
        Ok(())
    }

    /// Draw stacked graphs, returning the drawn height (`drawGraphs`).
    pub fn draw_graphs(&self, sink: &mut impl DebugGraphSink, view: &Rect, settings: &DebugGraphSettings) -> f32 {
        if (settings.debuggraph == 0 && settings.timegraph == 0 && settings.netgraph == 0)
            || !settings.height.is_finite()
            || settings.height.trunc() <= 0.0
        {
            return 0.0;
        }
        let mut bottom = view.y + view.height;
        for (_, state) in &self.owners {
            let palette = state.graph.as_ref().and_then(|graph| graph.palette.as_ref());
            let Some(palette) = palette else {
                continue;
            };
            if state.retired {
                continue;
            }
            let graph = state.graph.as_ref().expect("drawing palette checked");
            graph.samples.draw(
                sink,
                &Rect {
                    height: bottom - view.y,
                    ..*view
                },
                settings,
                |index| debug_graph_color(&palette.colors, index),
            );
            bottom -= settings.height;
            if bottom <= view.y {
                break;
            }
        }
        view.height.min(view.y + view.height - bottom)
    }

    /// Snapshot visible text and lines (`snapshot`).
    pub fn snapshot(&mut self, seconds: f64, frame: u64) -> Result<ComponentDrawingSnapshot, ComponentDrawingsError> {
        let mut text = Vec::new();
        let mut lines = Vec::new();
        for (_, state) in &mut self.owners {
            if state.retired {
                continue;
            }
            text.extend(state.text.snapshot(seconds, frame));
            lines.extend(state.lines.snapshot(seconds * 1000.0, frame)?);
        }
        Ok(ComponentDrawingSnapshot { text, lines })
    }

    /// Drop every owner (`clear`).
    pub fn clear(&mut self) {
        self.owners.clear();
    }
}

/// Visible drawing snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDrawingSnapshot {
    /// Visible world text.
    pub text: Vec<WorldText>,
    /// Visible debug lines.
    pub lines: Vec<DebugLine>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec3, vec4};

    fn owner(generation: u64) -> PresentationOwner {
        PresentationOwner {
            provider: ProviderId::new("test", "drawings"),
            generation,
        }
    }

    fn content() -> ContentId {
        ContentId("q2:rerelease:baseq2:1".to_string())
    }

    fn text_input() -> WorldTextInput {
        WorldTextInput {
            text: "hello".to_string(),
            origin: vec3(0.0, 0.0, 0.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            cell_size: 8.0,
            distance_cull_factor: None,
            orientation: qa_client::text::ui_world::WorldTextOrientation::Billboard,
            depth_test: true,
            font: qa_client::text::ui_world::WorldTextFont::Classic,
        }
    }

    fn line() -> DebugLine {
        DebugLine {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(1.0, 0.0, 0.0),
            color: vec4(1.0, 0.0, 0.0, 1.0),
            depth_test: true,
        }
    }

    #[test]
    fn generations_replacement_and_sequences_gate_events() {
        let mut drawings = ComponentDrawings::new();
        drawings
            .receive(&[ComponentDrawingEvent {
                owner: Some(owner(1)),
                sequence: 1,
                content: content(),
                seconds: 1.0,
                kind: ComponentDrawingEventKind::WorldText {
                    text: text_input(),
                    lifetime: 10.0,
                },
            }])
            .unwrap();
        // Stale sequences are ignored.
        drawings
            .receive(&[ComponentDrawingEvent {
                owner: Some(owner(1)),
                sequence: 1,
                content: content(),
                seconds: 2.0,
                kind: ComponentDrawingEventKind::DebugShapes {
                    lines: vec![line()],
                    lifetime_ms: 1000,
                },
            }])
            .unwrap();
        let snapshot = drawings.snapshot(2.0, 7).unwrap();
        assert_eq!(snapshot.text.len(), 1);
        assert!(snapshot.lines.is_empty());
        // A newer generation replaces the owner and clears submissions.
        drawings
            .receive(&[ComponentDrawingEvent {
                owner: Some(owner(2)),
                sequence: 2,
                content: content(),
                seconds: 3.0,
                kind: ComponentDrawingEventKind::PresentationOwner { retired: false },
            }])
            .unwrap();
        let snapshot = drawings.snapshot(3.0, 8).unwrap();
        assert!(snapshot.text.is_empty());
        // An older generation is ignored.
        drawings
            .receive(&[ComponentDrawingEvent {
                owner: Some(owner(1)),
                sequence: 3,
                content: content(),
                seconds: 4.0,
                kind: ComponentDrawingEventKind::WorldText {
                    text: text_input(),
                    lifetime: 10.0,
                },
            }])
            .unwrap();
        assert!(drawings.snapshot(4.0, 9).unwrap().text.is_empty());
    }

    #[test]
    fn graphs_reject_content_changes_and_stack_draws() {
        use qa_client::render::debug_graph::DebugGraphSink;
        struct Sink {
            fills: usize,
        }
        impl DebugGraphSink for Sink {
            fn fill_rect(&mut self, _rect: &Rect, _color: qa_core::math::Vec4) {
                self.fills += 1;
            }
        }
        let mut drawings = ComponentDrawings::new();
        drawings
            .receive(&[ComponentDrawingEvent {
                owner: Some(owner(1)),
                sequence: 1,
                content: content(),
                seconds: 1.0,
                kind: ComponentDrawingEventKind::DebugGraph { value: 4.0, color: 3 },
            }])
            .unwrap();
        assert!(matches!(
            drawings
                .receive(&[ComponentDrawingEvent {
                    owner: Some(owner(1)),
                    sequence: 2,
                    content: ContentId("q3:classic:baseq3:1".to_string()),
                    seconds: 2.0,
                    kind: ComponentDrawingEventKind::DebugGraph { value: 1.0, color: 1 },
                }])
                .unwrap_err(),
            ComponentDrawingsError::GraphContentChanged
        ));
        drawings
            .prepare_graphs(&mut |_| {
                Ok::<Palette, String>(Palette {
                    colors: vec![128; 768],
                    source: "pics/colormap.pcx".to_string(),
                })
            })
            .unwrap();
        let settings = DebugGraphSettings {
            debuggraph: 1,
            timegraph: 0,
            netgraph: 0,
            height: 32.0,
            scale: 1.0,
            shift: 0.0,
        };
        let view = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 64.0,
        };
        let mut sink = Sink { fills: 0 };
        assert_eq!(drawings.draw_graphs(&mut sink, &view, &settings), 32.0);
        assert!(sink.fills > 0);
        let off = DebugGraphSettings {
            debuggraph: 0,
            ..settings
        };
        assert_eq!(drawings.draw_graphs(&mut sink, &view, &off), 0.0);
    }

    #[test]
    fn retirement_freezes_submissions_and_snapshots() {
        let mut drawings = ComponentDrawings::new();
        drawings
            .receive(&[
                ComponentDrawingEvent {
                    owner: Some(owner(1)),
                    sequence: 1,
                    content: content(),
                    seconds: 1.0,
                    kind: ComponentDrawingEventKind::WorldText {
                        text: text_input(),
                        lifetime: 10.0,
                    },
                },
                ComponentDrawingEvent {
                    owner: Some(owner(1)),
                    sequence: 2,
                    content: content(),
                    seconds: 1.0,
                    kind: ComponentDrawingEventKind::PresentationOwner { retired: true },
                },
                ComponentDrawingEvent {
                    owner: Some(owner(1)),
                    sequence: 3,
                    content: content(),
                    seconds: 2.0,
                    kind: ComponentDrawingEventKind::WorldText {
                        text: text_input(),
                        lifetime: 10.0,
                    },
                },
            ])
            .unwrap();
        let snapshot = drawings.snapshot(2.0, 3).unwrap();
        assert!(snapshot.text.is_empty());
        drawings.clear();
        drawings
            .receive(&[ComponentDrawingEvent {
                owner: None,
                sequence: 4,
                content: content(),
                seconds: 3.0,
                kind: ComponentDrawingEventKind::Other,
            }])
            .unwrap();
    }
}
