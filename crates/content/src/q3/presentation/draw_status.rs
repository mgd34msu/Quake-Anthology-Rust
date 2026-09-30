//! Quake III presentation: draw status.
//!
//! Donor provenance: `src/content/q3/presentation/draw-status.ts`.

use qa_core::math::vec4;
use std::cell::{Cell, RefCell};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::mirrors_present_hud::*;

/// Lagometer sample counts.
pub(crate) const LAG_SAMPLES: usize = 128;

/// Maximum lagometer ping.
pub(crate) const MAX_LAGOMETER_PING: f32 = 900.0;

/// Maximum lagometer range.
pub(crate) const MAX_LAGOMETER_RANGE: f32 = 300.0;

/// Draw-status product variant (`ClientDrawStatusVariant`).
#[derive(Debug, Clone, PartialEq)]
pub enum ClientDrawStatusVariant {
    /// Base game.
    Baseq3,
    /// Mission pack with cgame fonts.
    Missionpack {
        /// Fonts.
        fonts: FontSet,
    },
}

impl ClientDrawStatusVariant {
    /// Product kind.
    #[must_use]
    pub fn kind(&self) -> Product {
        match self {
            Self::Baseq3 => Product::Baseq3,
            Self::Missionpack { .. } => Product::Missionpack,
        }
    }
}

/// Draw-status host services.
pub struct ClientDrawStatusHost {
    /// Command source.
    pub commands: Shared<dyn CommandSource>,
    /// Cvar reader.
    pub cvars: Shared<dyn HudCvarReader>,
}

/// Lagometer snapshot sample (`LagometerSnapshotSample`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagometerSnapshotSample {
    /// Ping.
    pub ping: i32,
    /// Flags.
    pub flags: i32,
}

/// Center-print source text (1023 cap).
pub(crate) fn center_source_text(input: &str) -> String {
    let cut = match input.find('\0') {
        Some(end) => &input[..end],
        None => input,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Center print requires source byte characters");
        }
    }
    cut.chars().take(1023).collect()
}

/// Status text and lagometer state (`ClientDrawStatus`).
pub struct ClientDrawStatus {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Draw tools.
    pub tools: ClientDrawTools,
    /// Variant.
    pub variant: ClientDrawStatusVariant,
    /// Host.
    pub host: ClientDrawStatusHost,
    /// Frame samples.
    frame_samples: RefCell<[i32; LAG_SAMPLES]>,
    /// Snapshot flags.
    snapshot_flags: RefCell<[i32; LAG_SAMPLES]>,
    /// Snapshot samples.
    snapshot_samples: RefCell<[i32; LAG_SAMPLES]>,
    /// Frame count.
    pub(crate) frame_count: Cell<i32>,
    /// Snapshot count.
    pub(crate) snapshot_count: Cell<i32>,
}

impl ClientDrawStatus {
    /// Assemble draw status.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        tools: ClientDrawTools,
        variant: ClientDrawStatusVariant,
        host: ClientDrawStatusHost,
    ) -> Self {
        if state.borrow().product != static_state.borrow().product
            || !same(&tools.media.borrow().static_state, &static_state)
        {
            panic!("Draw status services must share canonical cgame state");
        }
        if variant.kind() != state.borrow().product {
            panic!("Draw status product variant differs from cgame state");
        }
        if let ClientDrawStatusVariant::Missionpack { fonts } = &variant {
            if fonts.profile != FontProfile::Cgame {
                panic!("Missionpack center print requires cgame fonts");
            }
        }
        Self {
            state,
            static_state,
            tools,
            variant,
            host,
            frame_samples: RefCell::new([0; LAG_SAMPLES]),
            snapshot_flags: RefCell::new([0; LAG_SAMPLES]),
            snapshot_samples: RefCell::new([0; LAG_SAMPLES]),
            frame_count: Cell::new(0),
            snapshot_count: Cell::new(0),
        }
    }

    /// Center print (`centerPrint`).
    pub fn center_print(&self, text: &str, y: i32, char_width: i32) {
        let mut state = self.state.borrow_mut();
        state.center_print = center_source_text(text);
        state.center_print_time = state.time;
        state.center_print_y = y;
        state.center_print_char_width = char_width;
        state.center_print_lines = 1;
        let source = state.center_print.clone();
        for unit in source.chars() {
            if unit == '\n' {
                state.center_print_lines += 1;
            }
        }
    }

    /// Draw the center string (`drawCenterString`).
    pub fn draw_center_string(&self) {
        let (center_time, center_print, lines, char_width, center_y, time) = {
            let state = self.state.borrow();
            (
                state.center_print_time,
                state.center_print.clone(),
                state.center_print_lines,
                state.center_print_char_width,
                state.center_print_y,
                state.time,
            )
        };
        if center_time == 0 {
            return;
        }
        let duration = 1000.0 * self.host.cvars.borrow().read_vm_cvar("cg_centertime").numeric_value;
        let color = fade_color(time, center_time, duration);
        let Some(color) = color else {
            return;
        };
        let mut y = center_y - lines.wrapping_mul(16) / 2;
        let units: Vec<char> = center_print.chars().collect();
        let mut start = 0usize;
        loop {
            let mut end = start;
            while end < units.len() && units[end] != '\n' {
                end += 1;
            }
            let line: String = units[start..end.min(start + 50)].iter().collect();
            if let ClientDrawStatusVariant::Missionpack { fonts } = &self.variant {
                let width = text_width(fonts, &line, 0.5, 0);
                let height = text_height(fonts, &line, 0.5, 0);
                let x = (640 - width) / 2;
                text_paint(
                    &self.tools.draw,
                    fonts,
                    &TextPaintOptions {
                        x: x as f32,
                        y: (y + height) as f32,
                        scale: 0.5,
                        color,
                        text: line,
                        adjust: 0.0,
                        limit: 0,
                        style: 6,
                    },
                );
                y += height + 6;
            } else {
                let width = char_width.wrapping_mul(draw_strlen(&line));
                let x = (640 - width) / 2;
                let height = (char_width as f32 * 1.5).trunc() as i32;
                self.tools.draw_string_ext(&FixedTextOptions {
                    x: x as f32,
                    y: y as f32,
                    text: line,
                    color,
                    force_color: false,
                    shadow: true,
                    char_width,
                    char_height: height,
                    max_chars: 0,
                });
                y = (y as f32 + char_width as f32 * 1.5).trunc() as i32;
            }
            if end == units.len() {
                break;
            }
            start = end + 1;
        }
        self.tools.draw.set_color(None);
    }

    /// Add lagometer frame info (`addLagometerFrameInfo`).
    pub fn add_lagometer_frame_info(&self) {
        let (time, latest) = {
            let state = self.state.borrow();
            (state.time, state.latest_snapshot_time)
        };
        let count = self.frame_count.get();
        self.frame_samples.borrow_mut()[(count & 127) as usize] = time.wrapping_sub(latest);
        self.frame_count.set(count.wrapping_add(1));
    }

    /// Add lagometer snapshot info (`addLagometerSnapshotInfo`).
    pub fn add_lagometer_snapshot_info(&self, sample: Option<LagometerSnapshotSample>) {
        let count = self.snapshot_count.get();
        let index = (count & 127) as usize;
        match sample {
            None => self.snapshot_samples.borrow_mut()[index] = -1,
            Some(sample) => {
                self.snapshot_samples.borrow_mut()[index] = sample.ping;
                self.snapshot_flags.borrow_mut()[index] = sample.flags;
            }
        }
        self.snapshot_count.set(count.wrapping_add(1));
    }

    /// Draw the disconnect notice (`drawDisconnect`).
    pub fn draw_disconnect(&self) {
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            panic!("CG_DrawDisconnect requires a current snapshot");
        };
        let command_number = self
            .host
            .commands
            .borrow()
            .current_number()
            .wrapping_sub(64)
            .wrapping_add(1);
        let command = self.host.commands.borrow().read(command_number);
        let Some(command) = command else {
            panic!("CG_DrawDisconnect command fell outside CMD_BACKUP");
        };
        let time = self.state.borrow().time;
        if command.server_time <= snapshot.player_state.command_time || command.server_time > time {
            return;
        }
        let message = "Connection Interrupted";
        let width = draw_strlen(message).wrapping_mul(16);
        self.tools.draw_big_string(320 - width / 2, 100, message, 1.0);
        if (time >> 9) & 1 != 0 {
            return;
        }
        let shader = self
            .tools
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader("gfx/2d/net.tga");
        self.tools
            .draw_pic(rect2d(640.0 - 48.0, 480.0 - 48.0, 48.0, 48.0), &shader);
    }

    /// Draw the lagometer (`drawLagometer`).
    pub fn draw_lagometer(&self) {
        if self.host.cvars.borrow().read_vm_cvar("cg_lagometer").integer_value == 0
            || self.static_state.borrow().local_server != 0
        {
            self.draw_disconnect();
            return;
        }
        let product = self.state.borrow().product;
        let (x, y) = (
            640.0 - 48.0,
            if product == Product::Missionpack {
                480.0 - 144.0
            } else {
                480.0 - 48.0
            },
        );
        self.tools.draw.set_color(None);
        let lagometer = self.tools.media.borrow().graphics.lagometer_shader.clone();
        self.tools.draw_pic(rect2d(x, y, 48.0, 48.0), &lagometer);
        let adjusted = self.tools.adjust_from_640(rect2d(x, y, 48.0, 48.0));
        let picture = {
            let media = self.tools.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        let yellow = vec4(1.0, 1.0, 0.0, 1.0);
        let blue = vec4(0.0, 0.0, 1.0, 1.0);
        let green = vec4(0.0, 1.0, 0.0, 1.0);
        let red = vec4(1.0, 0.0, 0.0, 1.0);
        let mut color = -1;
        let frame_count = self.frame_count.get();
        let mut range = adjusted.height / 3.0;
        let mid = adjusted.y + range;
        let scale = range / MAX_LAGOMETER_RANGE;
        let steps = adjusted.width.trunc() as i32;
        for a in 0..steps {
            let index = (frame_count.wrapping_sub(1).wrapping_sub(a) & 127) as usize;
            let mut value = self.frame_samples.borrow()[index] as f32 * scale;
            if value > 0.0 {
                if color != 1 {
                    color = 1;
                    self.tools.draw.set_color(Some(yellow));
                }
                if value > range {
                    value = range;
                }
                self.tools.draw.stretch_pixels(
                    rect2d(adjusted.x + adjusted.width - a as f32, mid - value, 1.0, value),
                    ZERO_UV,
                    picture,
                );
            } else if value < 0.0 {
                if color != 2 {
                    color = 2;
                    self.tools.draw.set_color(Some(blue));
                }
                value = -value;
                if value > range {
                    value = range;
                }
                self.tools.draw.stretch_pixels(
                    rect2d(adjusted.x + adjusted.width - a as f32, mid, 1.0, value),
                    ZERO_UV,
                    picture,
                );
            }
        }
        range = adjusted.height / 2.0;
        let scale = range / MAX_LAGOMETER_PING;
        let snapshot_count = self.snapshot_count.get();
        for a in 0..steps {
            let index = (snapshot_count.wrapping_sub(1).wrapping_sub(a) & 127) as usize;
            let mut value = self.snapshot_samples.borrow()[index] as f32;
            if value > 0.0 {
                if self.snapshot_flags.borrow()[index] & 1 != 0 {
                    if color != 5 {
                        color = 5;
                        self.tools.draw.set_color(Some(yellow));
                    }
                } else if color != 3 {
                    color = 3;
                    self.tools.draw.set_color(Some(green));
                }
                value *= scale;
                if value > range {
                    value = range;
                }
                self.tools.draw.stretch_pixels(
                    rect2d(
                        adjusted.x + adjusted.width - a as f32,
                        adjusted.y + adjusted.height - value,
                        1.0,
                        value,
                    ),
                    ZERO_UV,
                    picture,
                );
            } else if value < 0.0 {
                if color != 4 {
                    color = 4;
                    self.tools.draw.set_color(Some(red));
                }
                self.tools.draw.stretch_pixels(
                    rect2d(
                        adjusted.x + adjusted.width - a as f32,
                        adjusted.y + adjusted.height - range,
                        1.0,
                        range,
                    ),
                    ZERO_UV,
                    picture,
                );
            }
        }
        self.tools.draw.set_color(None);
        if self.host.cvars.borrow().read_vm_cvar("cg_nopredict").integer_value != 0
            || self
                .host
                .cvars
                .borrow()
                .read_vm_cvar("g_synchronousClients")
                .integer_value
                != 0
        {
            self.tools
                .draw_big_string(adjusted.x as i32, adjusted.y as i32, "snc", 1.0);
        }
        self.draw_disconnect();
    }
}
