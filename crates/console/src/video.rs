//! Native video aliases project into the shared window settings.
use crate::{
    catalog::Operation,
    conversion::{Error, Input, Output, Text},
    cvars_generated::DEFINITIONS,
    numbers::integer,
};
use qa_core::{
    presentation::{
        DisplayMode, GlContextRequest, MAX_WINDOW_DIMENSION, VideoCapabilities, WindowMode,
        WindowRequest,
    },
    primitives::RuleSetId,
};
use std::sync::Arc;

pub const WINDOW_CVARS: &[&str] = &[
    "r_mode",
    "r_customwidth",
    "r_customheight",
    "r_fullscreen",
    "r_displayRefresh",
    "r_colorbits",
    "vid_modelist",
];
pub const GL_CVARS: &[&str] = &["r_depthbits", "r_stencilbits", "r_stereo", "r_swapInterval"];

// Q2's native mode table is the first ten entries of this same table.
const MODES: &[(u32, u32)] = &[
    (320, 240),
    (400, 300),
    (512, 384),
    (640, 480),
    (800, 600),
    (960, 720),
    (1024, 768),
    (1152, 864),
    (1280, 1024),
    (1600, 1200),
    (2048, 1536),
    (856, 480),
];

pub struct VideoPolicy {
    capabilities: Arc<VideoCapabilities>,
    windowed: DisplayMode,
    pub(crate) rows: [u16; 11],
}
impl VideoPolicy {
    pub(crate) fn load(
        capabilities: Arc<VideoCapabilities>,
        windowed: DisplayMode,
    ) -> Result<Self, Error> {
        let mut rows = [0; 11];
        for (row, name) in rows.iter_mut().zip(WINDOW_CVARS.iter().chain(GL_CVARS)) {
            *row = u16::try_from(
                DEFINITIONS
                    .iter()
                    .position(|definition| definition.name == *name)
                    .ok_or(Error::VideoPolicyRequired)?,
            )
            .map_err(|_| Error::VideoPolicyRequired)?;
        }
        if windowed.width == 0
            || windowed.height == 0
            || capabilities.desktop.width == 0
            || capabilities.desktop.height == 0
        {
            return Err(Error::InvalidVideoMode);
        }
        Ok(Self {
            capabilities,
            windowed,
            rows,
        })
    }
    pub(crate) fn windowed(&self) -> DisplayMode {
        self.windowed
    }
    pub(crate) fn dependencies(&self, operation: Operation) -> &[u16] {
        match operation {
            Operation::VideoMode | Operation::Fullscreen => &self.rows[..7],
            _ => &[],
        }
    }
    fn extent<'a>(&self, value: &dyn Fn(u16) -> &'a str) -> Result<(u32, u32), Error> {
        match integer(value(self.rows[0])) {
            -1 => {
                let width = u32::try_from(integer(value(self.rows[1])))
                    .map_err(|_| Error::InvalidVideoMode)?;
                let height = u32::try_from(integer(value(self.rows[2])))
                    .map_err(|_| Error::InvalidVideoMode)?;
                if width == 0 || height == 0 {
                    return Err(Error::InvalidVideoMode);
                }
                Ok((width, height))
            }
            index => Ok(usize::try_from(index)
                .ok()
                .and_then(|index| MODES.get(index))
                .copied()
                .unwrap_or(MODES[3])),
        }
    }
    fn mode(&self, source: RuleSetId, index: i32) -> Result<(DisplayMode, bool), Error> {
        match source {
            RuleSetId::Quake | RuleSetId::QuakeWorld => {
                if index == 0 {
                    return Ok((self.windowed, false));
                }
                let mode = usize::try_from(index.checked_sub(1).ok_or(Error::InvalidVideoMode)?)
                    .ok()
                    .and_then(|index| self.capabilities.fullscreen.get(index))
                    .copied()
                    .ok_or(Error::InvalidVideoMode)?;
                Ok((mode, true))
            }
            _ => {
                let limit = if source == RuleSetId::Quake3 {
                    MODES.len()
                } else {
                    10
                };
                let &(width, height) = usize::try_from(index)
                    .ok()
                    .filter(|&index| index < limit)
                    .and_then(|index| MODES.get(index))
                    .ok_or(Error::InvalidVideoMode)?;
                Ok((
                    DisplayMode {
                        width,
                        height,
                        frequency: 0,
                        color_bits: 0,
                    },
                    false,
                ))
            }
        }
    }
    fn store_extent<'a>(&self, out: &mut Output<'a>, mode: DisplayMode) -> Result<i32, Error> {
        let index = MODES
            .iter()
            .position(|&(width, height)| width == mode.width && height == mode.height)
            .map_or(-1, |index| index as i32);
        if index == -1 {
            out.change(self.rows[1], f64::from(mode.width))?;
            out.change(self.rows[2], f64::from(mode.height))?;
        }
        Ok(index)
    }
    fn fullscreen_entry(&self, list: &str, index: i32) -> Result<NativeMode, Error> {
        let index = usize::try_from(index.checked_sub(1).ok_or(Error::InvalidVideoMode)?)
            .map_err(|_| Error::InvalidVideoMode)?;
        for (position, mode) in Modelist::new(list).enumerate() {
            let mode = mode?;
            if position == index {
                return mode.checked();
            }
        }
        Err(Error::InvalidVideoMode)
    }
    fn requested_fullscreen(&self, list: &str, index: i32) -> NativeMode {
        self.fullscreen_entry(list, index)
            .unwrap_or(NativeMode::Desktop)
    }
    fn matches(&self, mode: NativeMode, extent: (u32, u32), frequency: i32, color: i32) -> bool {
        match mode {
            NativeMode::Desktop => {
                extent
                    == (
                        self.capabilities.desktop.width,
                        self.capabilities.desktop.height,
                    )
            }
            NativeMode::Exclusive(mode) => {
                extent == (mode.width, mode.height)
                    && frequency == mode.frequency
                    && color == mode.color_bits as i32
            }
        }
    }
    pub(crate) fn read<'a>(&self, input: &Input<'a, '_>) -> Result<Text<'a>, Error> {
        if input.conversion.operation == Operation::Fullscreen && integer(input.value) == 0 {
            return Text::numeric(0.0);
        }
        let extent = self.extent(&|row| {
            if row == self.rows[0] && input.conversion.operation == Operation::VideoMode {
                input.value
            } else {
                (input.operand)(row)
            }
        })?;
        let full = integer(if input.conversion.operation == Operation::Fullscreen {
            input.value
        } else {
            (input.operand)(self.rows[3])
        }) != 0;
        if input.conversion.operation == Operation::VideoMode
            && let Some(detail) = input.detail
            && let Ok((mode, selected_full)) = self.mode(input.context.source, integer(detail))
            && (mode.width, mode.height) == extent
            && (!matches!(
                input.context.source,
                RuleSetId::Quake | RuleSetId::QuakeWorld
            ) || selected_full == full)
        {
            return Ok(Text::Borrowed(detail));
        }
        let value = match input.conversion.operation {
            Operation::VideoMode => match input.context.source {
                RuleSetId::Quake | RuleSetId::QuakeWorld if !full => 0,
                RuleSetId::Quake | RuleSetId::QuakeWorld => self
                    .capabilities
                    .fullscreen
                    .iter()
                    .position(|mode| (mode.width, mode.height) == extent)
                    .map_or(-1, |index| index as i32 + 1),
                source => MODES[..if source == RuleSetId::Quake3 {
                    MODES.len()
                } else {
                    10
                }]
                    .iter()
                    .position(|&mode| mode == extent)
                    .map_or(-1, |index| index as i32),
            },
            Operation::Fullscreen => {
                if !full {
                    return Text::numeric(0.0);
                }
                let list = (input.operand)(self.rows[6]);
                let frequency = integer((input.operand)(self.rows[4]));
                let color = integer((input.operand)(self.rows[5]));
                if let Some(detail) = input.detail
                    && self.matches(
                        self.requested_fullscreen(list, integer(detail)),
                        extent,
                        frequency,
                        color,
                    )
                {
                    return Ok(Text::Borrowed(detail));
                }
                let mut found = None;
                for (index, mode) in Modelist::new(list).enumerate() {
                    if self.matches(
                        mode.and_then(NativeMode::checked)
                            .unwrap_or(NativeMode::Desktop),
                        extent,
                        frequency,
                        color,
                    ) {
                        found = Some(index as i32 + 1);
                        break;
                    }
                }
                found.ok_or(Error::InvalidVideoMode)?
            }
            _ => return Err(Error::VideoPolicyRequired),
        };
        Text::numeric(f64::from(value))
    }
    pub(crate) fn write<'a>(&self, input: &Input<'a, '_>) -> Result<Output<'a>, Error> {
        let index = integer(input.value);
        let mut out = Output::new(input.value);
        match input.conversion.operation {
            Operation::VideoMode => {
                let (mode, full) = match self.mode(input.context.source, index) {
                    Ok(mode) => mode,
                    Err(error)
                        if matches!(
                            input.context.source,
                            RuleSetId::Quake | RuleSetId::QuakeWorld
                        ) =>
                    {
                        return Err(error);
                    }
                    Err(_) => {
                        out.detail_value = Some("3");
                        out.change(self.rows[3], 0.0)?;
                        self.mode(RuleSetId::Quake3, 3)?
                    }
                };
                out.text = Text::numeric(f64::from(self.store_extent(&mut out, mode)?))?;
                if matches!(
                    input.context.source,
                    RuleSetId::Quake | RuleSetId::QuakeWorld
                ) {
                    out.change(self.rows[3], f64::from(u8::from(full)))?;
                    out.change(self.rows[4], f64::from(mode.frequency))?;
                    out.change(self.rows[5], f64::from(mode.color_bits))?;
                }
            }
            Operation::Fullscreen => {
                if index == 0 {
                    out.text = Text::numeric(0.0)?;
                } else {
                    let mode = self.requested_fullscreen((input.operand)(self.rows[6]), index);
                    let display = match mode {
                        NativeMode::Desktop => DisplayMode {
                            frequency: 0,
                            color_bits: 0,
                            ..self.capabilities.desktop
                        },
                        NativeMode::Exclusive(mode) => mode,
                    };
                    let canonical = self.store_extent(&mut out, display)?;
                    out.text = Text::numeric(1.0)?;
                    out.change(self.rows[0], f64::from(canonical))?;
                    out.change(self.rows[4], f64::from(display.frequency))?;
                    out.change(self.rows[5], f64::from(display.color_bits))?;
                }
            }
            _ => return Err(Error::VideoPolicyRequired),
        }
        out.detail = true;
        Ok(out)
    }
    pub(crate) fn request<'a>(
        &self,
        value: &dyn Fn(u16) -> &'a str,
        fullscreen_detail: Option<&str>,
        gl: bool,
    ) -> Result<WindowRequest, Error> {
        let (width, height) = self.extent(value)?;
        let fullscreen = integer(value(self.rows[3])) != 0;
        let frequency = integer(value(self.rows[4]));
        let color = integer(value(self.rows[5]));
        let mode = if !fullscreen {
            WindowMode::Windowed
        } else if fullscreen_detail
            .and_then(|detail| {
                Some(self.requested_fullscreen(value(self.rows[6]), integer(detail)))
            })
            .is_some_and(|mode| {
                matches!(mode, NativeMode::Desktop)
                    && self.matches(mode, (width, height), frequency, color)
            })
        {
            WindowMode::Desktop
        } else {
            WindowMode::Exclusive {
                frequency,
                color_bits: u32::try_from(color).map_err(|_| Error::InvalidVideoMode)?,
            }
        };
        let mut request = WindowRequest {
            width,
            height,
            mode,
            gl: GlContextRequest::default(),
        };
        if gl {
            let bits =
                |row| u32::try_from(integer(value(row))).map_err(|_| Error::InvalidVideoMode);
            request.gl = GlContextRequest {
                color_bits: bits(self.rows[5])?,
                depth_bits: bits(self.rows[7])?,
                stencil_bits: bits(self.rows[8])?,
                stereo: integer(value(self.rows[9])) != 0,
                swap_interval: integer(value(self.rows[10])),
            };
        }
        request.validate(gl).map_err(|_| Error::InvalidVideoMode)?;
        Ok(request)
    }
}
#[derive(Clone, Copy)]
enum NativeMode {
    Desktop,
    Exclusive(DisplayMode),
}
impl NativeMode {
    fn checked(self) -> Result<Self, Error> {
        if let Self::Exclusive(mode) = self
            && (!(320..=MAX_WINDOW_DIMENSION).contains(&mode.width)
                || !(240..=MAX_WINDOW_DIMENSION).contains(&mode.height)
                || !(0..=1000).contains(&mode.frequency)
                || mode.color_bits > 32)
        {
            return Err(Error::InvalidVideoMode);
        }
        Ok(self)
    }
}
struct Modelist<'a> {
    remaining: &'a str,
}
impl<'a> Modelist<'a> {
    fn new(text: &'a str) -> Self {
        Self { remaining: text }
    }
    fn number(&mut self) -> i64 {
        let (value, end) = qa_core::text::integer_prefix(self.remaining.as_bytes());
        self.remaining = &self.remaining[end..];
        value
    }
}
impl Iterator for Modelist<'_> {
    type Item = Result<NativeMode, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        self.remaining = self
            .remaining
            .trim_start_matches(|character: char| character.is_ascii_whitespace());
        if self.remaining.is_empty() {
            return None;
        }
        Some((|| {
            if let Some(rest) = self.remaining.strip_prefix("desktop") {
                if !rest.is_empty()
                    && !rest.starts_with(|character: char| character.is_ascii_whitespace())
                {
                    self.remaining = "";
                    return Err(Error::InvalidVideoMode);
                }
                self.remaining = rest;
                return Ok(NativeMode::Desktop);
            }
            let width = self.number();
            if !self.remaining.starts_with(['x', 'X']) {
                self.remaining = "";
                return Err(Error::InvalidVideoMode);
            }
            self.remaining = &self.remaining[1..];
            let height = self.number();
            let mut frequency = 0;
            let mut color_bits = 0;
            if self.remaining.starts_with('@') {
                self.remaining = &self.remaining[1..];
                frequency = self.number();
                if self.remaining.starts_with(':') {
                    self.remaining = &self.remaining[1..];
                    color_bits = self.number();
                }
            } else if self.remaining.starts_with(':') {
                self.remaining = &self.remaining[1..];
                color_bits = self.number();
                if self.remaining.starts_with('@') {
                    self.remaining = &self.remaining[1..];
                    frequency = self.number();
                }
            }
            // Bounds apply only to the selected native entry. Keep scanning
            // complete earlier entries even when their dimensions are unusable.
            Ok(NativeMode::Exclusive(DisplayMode {
                width: u32::try_from(width).unwrap_or(u32::MAX),
                height: u32::try_from(height).unwrap_or(u32::MAX),
                frequency: i32::try_from(frequency).unwrap_or(i32::MAX),
                color_bits: u32::try_from(color_bits).unwrap_or(u32::MAX),
            }))
        })())
    }
}
