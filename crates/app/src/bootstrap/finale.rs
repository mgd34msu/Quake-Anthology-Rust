//! Source finale overlay: staged Quake text reveal with banner art.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/finale.ts` (`SourceFinale`; Quake
//! `screen.c` center-string reveal and `sbar.c` finale overlay). Synchronous
//! port: asset loading and message localization arrive through
//! [`FinaleAssets`], drawing through [`FinaleDraw`]. The donor's async
//! revision race guard has no synchronous equivalent and is not kept.

use qa_content::contract::{same_presentation_owner, ContentId, PresentationOwner};
use std::collections::HashMap;
use thiserror::Error;

/// Failure of finale preparation.
#[derive(Debug, Error)]
pub enum FinaleError {
    /// Banner or message loading failed.
    #[error("{0}")]
    Load(String),
}

/// Presentation events consumed by the finale.
#[derive(Debug, Clone)]
pub enum FinalePresentationEvent {
    /// Presentation owner retired.
    OwnerRetired {
        /// Retired owner.
        owner: PresentationOwner,
    },
    /// Level-triggered finale text (always bannered).
    Q1LevelFinale {
        /// Presenting owner, when owned.
        owner: Option<PresentationOwner>,
        /// Source content.
        content: ContentId,
        /// Source text.
        text: String,
        /// Start time in seconds.
        seconds: f64,
    },
    /// Staged finale text (stages 0..=4; banner from stage 4).
    Q1Finale {
        /// Presenting owner, when owned.
        owner: Option<PresentationOwner>,
        /// Source content.
        content: ContentId,
        /// Source text.
        text: String,
        /// Start time in seconds.
        seconds: f64,
        /// Finale stage.
        stage: u32,
    },
}

/// Loaded finale banner dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinaleBanner {
    /// Banner width in source pixels.
    pub width: f64,
    /// Banner height in source pixels.
    pub height: f64,
}

/// Banner and message loading for the finale.
pub trait FinaleAssets {
    /// Load (or reuse) the banner for `content`.
    fn load_banner(&mut self, content: &ContentId) -> Result<FinaleBanner, FinaleError>;
    /// Localize `source` for `content`.
    fn resolve_message(&mut self, content: &ContentId, source: &str) -> Result<String, FinaleError>;
}

/// Finale drawing surface.
pub trait FinaleDraw {
    /// Surface width in pixels.
    fn width(&self) -> f64;
    /// Surface height in pixels.
    fn height(&self) -> f64;
    /// Draw the banner picture.
    fn draw_banner(&mut self, x: f64, y: f64, width: f64, height: f64);
    /// Measure one text line at `scale`.
    fn line_width(&self, text: &str, scale: f64) -> f64;
    /// Draw one centered line with a glyph budget.
    fn draw_line(&mut self, text: &str, scale: f64, max_glyphs: usize, x: f64, y: f64);
}

struct FinaleState {
    owner: Option<PresentationOwner>,
    content: ContentId,
    source_text: String,
    started: f64,
    banner: bool,
}

/// The source game controls stages and input gating; each seat reveals its own text.
pub struct SourceFinale<A> {
    assets: A,
    state: Option<FinaleState>,
    loaded: HashMap<String, FinaleBanner>,
    prepared: Option<FinaleBanner>,
    message: String,
}

impl<A: FinaleAssets> SourceFinale<A> {
    /// Create a finale bound to asset/message loading.
    pub fn new(assets: A) -> Self {
        Self {
            assets,
            state: None,
            loaded: HashMap::new(),
            prepared: None,
            message: String::new(),
        }
    }

    /// Whether a finale is staged.
    #[must_use]
    pub fn active(&self) -> bool {
        self.state.is_some()
    }

    /// Consume presentation events.
    pub fn receive(&mut self, events: &[FinalePresentationEvent]) {
        for event in events {
            match event {
                FinalePresentationEvent::OwnerRetired { owner } => {
                    if same_presentation_owner(self.state.as_ref().and_then(|state| state.owner.as_ref()), owner) {
                        self.state = None;
                        self.prepared = None;
                        self.message.clear();
                    }
                }
                FinalePresentationEvent::Q1LevelFinale {
                    owner,
                    content,
                    text,
                    seconds,
                } => {
                    self.state = Some(FinaleState {
                        owner: owner.clone(),
                        content: content.clone(),
                        source_text: text.clone(),
                        started: *seconds,
                        banner: true,
                    });
                }
                FinalePresentationEvent::Q1Finale {
                    owner,
                    content,
                    text,
                    seconds,
                    stage,
                } => {
                    if *stage <= 4 {
                        self.state = Some(FinaleState {
                            owner: owner.clone(),
                            content: content.clone(),
                            source_text: text.clone(),
                            started: *seconds,
                            banner: *stage >= 4,
                        });
                    }
                }
            }
        }
    }

    /// Load the banner and resolve the message for the staged finale.
    pub fn prepare(&mut self) -> Result<(), FinaleError> {
        let Some(state) = self.state.as_ref() else {
            return Ok(());
        };
        let key = state.content.as_str().to_owned();
        if !self.loaded.contains_key(&key) {
            let banner = self.assets.load_banner(&state.content)?;
            self.loaded.insert(key.clone(), banner);
        }
        let banner = self.loaded[&key];
        let message = self
            .assets
            .resolve_message(&state.content, &state.source_text.clone())?;
        self.prepared = Some(banner);
        self.message = message;
        Ok(())
    }

    /// Draw the banner and revealed text.
    pub fn draw<D: FinaleDraw>(&self, draw: &mut D, seconds: f64) {
        let (Some(state), Some(banner)) = (self.state.as_ref(), self.prepared.as_ref()) else {
            return;
        };
        let scale = (draw.width() / 320.0).min(draw.height() / 200.0);
        let height = draw.height() / scale;
        if state.banner {
            draw.draw_banner(
                (draw.width() - banner.width * scale) / 2.0,
                16.0 * scale,
                banner.width * scale,
                banner.height * scale,
            );
        }
        let lines: Vec<&str> = self.message.split('\n').collect();
        let mut y = (if lines.len() <= 4 {
            (height * 0.35).trunc()
        } else {
            48.0
        }) * scale;
        let mut remaining = (8.0 * (seconds - state.started)).trunc().max(0.0) as usize + 1;
        for source_line in &lines {
            let line: String = source_line.chars().take(40).collect();
            let width = draw.line_width(&line, scale);
            draw.draw_line(&line, scale, remaining, ((draw.width() - width) / 2.0).trunc(), y);
            remaining = remaining.saturating_sub(line.chars().count());
            if remaining == 0 {
                break;
            }
            y += 8.0 * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeAssets {
        banners: HashMap<String, FinaleBanner>,
        loads: Vec<String>,
    }

    impl FinaleAssets for FakeAssets {
        fn load_banner(&mut self, content: &ContentId) -> Result<FinaleBanner, FinaleError> {
            self.loads.push(content.as_str().to_owned());
            self.banners
                .get(content.as_str())
                .copied()
                .ok_or_else(|| FinaleError::Load("Quake finale picture is absent from selected content".to_owned()))
        }

        fn resolve_message(&mut self, _content: &ContentId, source: &str) -> Result<String, FinaleError> {
            Ok(source.to_owned())
        }
    }

    struct FakeDraw {
        width: f64,
        height: f64,
        banners: Vec<(f64, f64, f64, f64)>,
        lines: Vec<(String, usize, f64, f64)>,
    }

    impl FinaleDraw for FakeDraw {
        fn width(&self) -> f64 {
            self.width
        }

        fn height(&self) -> f64 {
            self.height
        }

        fn draw_banner(&mut self, x: f64, y: f64, width: f64, height: f64) {
            self.banners.push((x, y, width, height));
        }

        fn line_width(&self, text: &str, scale: f64) -> f64 {
            text.chars().count() as f64 * 8.0 * scale
        }

        fn draw_line(&mut self, text: &str, _scale: f64, max_glyphs: usize, x: f64, y: f64) {
            self.lines.push((text.to_owned(), max_glyphs, x, y));
        }
    }

    fn owner() -> PresentationOwner {
        PresentationOwner {
            provider: qa_core::identity::ProviderId::new("q1", "game"),
            generation: 7,
        }
    }

    fn finale() -> SourceFinale<FakeAssets> {
        let mut banners = HashMap::new();
        banners.insert(
            "q1".to_owned(),
            FinaleBanner {
                width: 320.0,
                height: 200.0,
            },
        );
        SourceFinale::new(FakeAssets {
            banners,
            loads: Vec::new(),
        })
    }

    #[test]
    fn stages_finale_and_clears_on_retire() {
        let mut finale = finale();
        assert!(!finale.active());
        finale.receive(&[FinalePresentationEvent::Q1Finale {
            owner: Some(owner()),
            content: ContentId("q1".to_owned()),
            text: "done".to_owned(),
            seconds: 10.0,
            stage: 5,
        }]);
        assert!(!finale.active());
        finale.receive(&[FinalePresentationEvent::Q1Finale {
            owner: Some(owner()),
            content: ContentId("q1".to_owned()),
            text: "done".to_owned(),
            seconds: 10.0,
            stage: 2,
        }]);
        assert!(finale.active());
        finale.prepare().expect("prepare");
        let mut draw = FakeDraw {
            width: 640.0,
            height: 400.0,
            banners: Vec::new(),
            lines: Vec::new(),
        };
        finale.draw(&mut draw, 11.0);
        assert!(draw.banners.is_empty());
        assert_eq!(draw.lines.len(), 1);
        finale.receive(&[FinalePresentationEvent::OwnerRetired { owner: owner() }]);
        assert!(!finale.active());
    }

    #[test]
    fn level_finale_always_bannered_and_reveals_over_time() {
        let mut finale = finale();
        finale.receive(&[FinalePresentationEvent::Q1LevelFinale {
            owner: None,
            content: ContentId("q1".to_owned()),
            text: "ab\ncd".to_owned(),
            seconds: 0.0,
        }]);
        finale.prepare().expect("prepare");
        let mut draw = FakeDraw {
            width: 320.0,
            height: 200.0,
            banners: Vec::new(),
            lines: Vec::new(),
        };
        finale.draw(&mut draw, 0.0);
        assert_eq!(draw.banners.len(), 1);
        assert_eq!(draw.banners[0].1, 16.0);
        assert_eq!(draw.lines[0].1, 1);
        let mut late = FakeDraw {
            width: 320.0,
            height: 200.0,
            banners: Vec::new(),
            lines: Vec::new(),
        };
        finale.draw(&mut late, 1.0);
        assert_eq!(late.lines[0].1, 9);
        assert_eq!(late.lines.len(), 2);
    }
}
