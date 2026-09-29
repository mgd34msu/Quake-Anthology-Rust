//! Source spline cameras: `.camera` files, playback, and view override.
//!
//! Donor provenance: `src/camera/spline.ts` (camera files and cubic
//! B-splines adapted from Q3 `code/splines/splines.cpp`, id Software,
//! GPL-2.0-or-later) and `src/camera/application.ts`
//! (`ApplicationSplineCamera`). Same file grammar, spline tessellation,
//! velocity/wait/event timing, and projection override (near/far/aspect
//! recovered from the live projection, Q3 `fovX`/`fovY` split). Console
//! commands (`loadcamera`, `startcamera`, `stopcamera`, `savecamera`) live
//! in `qa-app`; loading here takes file text so hosts keep their own IO.

use std::collections::HashSet;

use qa_core::math::{add3, angles_to_axis, length3, normalize3, scale3, sub3, vec3, vector_to_angles, Vec3};

use crate::view::{perspective_projection, CameraClip, SceneCamera};
use crate::ClientError;

/// Velocity segment on a camera path (donor `CameraVelocity`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraVelocity {
    /// Start offset in milliseconds.
    pub start: f64,
    /// Duration in milliseconds.
    pub duration: f64,
    /// Speed in units per second.
    pub speed: f64,
}

/// Shared path fields (donor `PositionFields`).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraPath {
    /// Path name.
    pub name: String,
    /// Path duration in milliseconds.
    pub time: f64,
    /// Base velocity.
    pub base_velocity: f64,
    /// Velocity segments.
    pub velocities: Vec<CameraVelocity>,
}

/// Camera or target position (donor `CameraPosition`).
#[derive(Debug, Clone, PartialEq)]
pub enum CameraPosition {
    /// Fixed point.
    Fixed {
        /// Shared path fields.
        path: CameraPath,
        /// Fixed point.
        point: Vec3,
    },
    /// Interpolated segment.
    Interpolated {
        /// Shared path fields.
        path: CameraPath,
        /// Segment start.
        start: Vec3,
        /// Segment end.
        end: Vec3,
    },
    /// Cubic B-spline.
    Spline {
        /// Shared path fields.
        path: CameraPath,
        /// Tessellation granularity.
        granularity: f32,
        /// Control points.
        points: Vec<Vec3>,
    },
}

impl CameraPosition {
    #[must_use]
    fn path(&self) -> &CameraPath {
        match self {
            Self::Fixed { path, .. } | Self::Interpolated { path, .. } | Self::Spline { path, .. } => path,
        }
    }
}

/// Camera event (donor `CameraEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraEvent {
    /// Event type (`0..=9`).
    pub event_type: u8,
    /// Event parameter.
    pub param: String,
    /// Event time in milliseconds.
    pub time: f64,
}

/// Camera field of view program (donor `CameraDefinition["fov"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFov {
    /// Fixed FOV.
    pub value: f32,
    /// FOV ramp start.
    pub start: f32,
    /// FOV ramp end.
    pub end: f32,
    /// Ramp duration in milliseconds.
    pub time: f64,
}

/// Parsed camera definition (donor `CameraDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraDefinition {
    /// Playback length in seconds (excluding waits).
    pub seconds: f64,
    /// Camera path.
    pub position: CameraPosition,
    /// Target paths.
    pub targets: Vec<CameraPosition>,
    /// Timed events.
    pub events: Vec<CameraEvent>,
    /// FOV program.
    pub fov: CameraFov,
}

fn camera_error(message: String) -> ClientError {
    ClientError::BadCamera(message)
}

fn tokenize(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if byte == b'"' {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end] != b'"' && bytes[end] != b'\r' && bytes[end] != b'\n' {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b'"' {
                tokens.push(text[index..=end].to_string());
                index = end + 1;
            } else {
                tokens.push("\"".to_string());
                index += 1;
            }
            continue;
        }
        if matches!(byte, b'{' | b'}' | b'(' | b')') {
            tokens.push(text[index..index + 1].to_string());
            index += 1;
            continue;
        }
        let mut end = index;
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() && !matches!(bytes[end], b'{' | b'}' | b'(' | b')')
        {
            end += 1;
        }
        tokens.push(text[index..end].to_string());
        index = end;
    }
    tokens
}

struct Tokens {
    tokens: Vec<String>,
    index: usize,
}

impl Tokens {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.index).map(String::as_str)
    }

    fn next(&mut self) -> Result<String, ClientError> {
        let Some(token) = self.tokens.get(self.index).cloned() else {
            return Err(camera_error("Unexpected end of camera file".to_string()));
        };
        self.index += 1;
        if token.starts_with('"') && token.len() >= 2 {
            Ok(token[1..token.len() - 1].to_string())
        } else {
            Ok(token)
        }
    }

    fn expect(&mut self, value: &str) -> Result<(), ClientError> {
        if self.next()?.to_lowercase() != value {
            return Err(camera_error(format!("Expected camera token {value}")));
        }
        Ok(())
    }

    fn number(&mut self) -> Result<f64, ClientError> {
        let token = self.next()?;
        let value: f64 = token.parse().unwrap_or(f64::NAN);
        if token.trim().is_empty() || !value.is_finite() {
            return Err(camera_error(format!("Invalid camera number {token}")));
        }
        Ok(value)
    }

    fn vector(&mut self) -> Result<Vec3, ClientError> {
        self.expect("(")?;
        #[allow(clippy::cast_possible_truncation)]
        let point = vec3(self.number()? as f32, self.number()? as f32, self.number()? as f32);
        self.expect(")")?;
        Ok(point)
    }
}

fn parse_position(tokens: &mut Tokens, kind: &str) -> Result<CameraPosition, ClientError> {
    tokens.expect("{")?;
    let mut name = "position".to_string();
    let mut time = 0.0;
    let mut base_velocity = 0.0;
    let mut point = vec3(0.0, 0.0, 0.0);
    let mut start = vec3(0.0, 0.0, 0.0);
    let mut end = vec3(0.0, 0.0, 0.0);
    let mut granularity = 0.025f32;
    let mut velocities = Vec::new();
    let mut points = Vec::new();
    while tokens.peek() != Some("}") {
        if tokens.peek().is_none() {
            return Err(camera_error("Unexpected end of camera file".to_string()));
        }
        match tokens.next()?.to_lowercase().as_str() {
            "name" => name = tokens.next()?,
            "type" => {
                tokens.number()?;
            }
            "time" => time = tokens.number()?,
            "basevelocity" => base_velocity = tokens.number()?,
            "velocity" => velocities.push(CameraVelocity {
                start: tokens.number()?,
                duration: tokens.number()?,
                speed: tokens.number()?,
            }),
            "pos" => point = tokens.vector()?,
            "startpos" => start = tokens.vector()?,
            "endpos" => end = tokens.vector()?,
            "target" => {
                tokens.expect("{")?;
                while tokens.peek() != Some("}") {
                    if tokens.peek().is_none() {
                        return Err(camera_error("Unexpected end of camera file".to_string()));
                    }
                    if tokens.peek() == Some("(") {
                        points.push(tokens.vector()?);
                    } else {
                        match tokens.next()?.to_lowercase().as_str() {
                            "granularity" => {
                                #[allow(clippy::cast_possible_truncation)]
                                {
                                    granularity = tokens.number()? as f32;
                                }
                            }
                            "name" => {
                                tokens.next()?;
                            }
                            property => {
                                return Err(camera_error(format!("Unknown spline property {property}")));
                            }
                        }
                    }
                }
                tokens.expect("}")?;
            }
            key => return Err(camera_error(format!("Unknown camera position property {key}"))),
        }
    }
    tokens.expect("}")?;
    if time < 0.0
        || velocities
            .iter()
            .any(|velocity| velocity.start < 0.0 || velocity.duration < 0.0 || velocity.speed < 0.0)
    {
        return Err(camera_error("Negative camera timing or speed".to_string()));
    }
    let path = CameraPath {
        name,
        time,
        base_velocity,
        velocities,
    };
    match kind {
        "fixed" => Ok(CameraPosition::Fixed { path, point }),
        "interpolated" => Ok(CameraPosition::Interpolated { path, start, end }),
        _ => {
            if !(0.0001..=1.0).contains(&granularity) || points.len() < 4 {
                return Err(camera_error(
                    "Spline requires four points and granularity between 0.0001 and 1".to_string(),
                ));
            }
            Ok(CameraPosition::Spline {
                path,
                granularity,
                points,
            })
        }
    }
}

/// Parse a `.camera` file (donor `parseCamera`).
pub fn parse_camera(text: &str) -> Result<CameraDefinition, ClientError> {
    let mut tokens = Tokens {
        tokens: tokenize(text),
        index: 0,
    };
    if matches!(
        tokens.peek().map(str::to_lowercase).as_deref(),
        Some("camera" | "camerapathdef")
    ) {
        tokens.next()?;
    }
    tokens.expect("{")?;
    let mut seconds = 30.0;
    let mut camera: Option<CameraPosition> = None;
    let mut value = 90.0f32;
    let mut start = 90.0f32;
    let mut end = 90.0f32;
    let mut time = 0.0;
    let mut targets = Vec::new();
    let mut events = Vec::new();
    while tokens.peek() != Some("}") {
        if tokens.peek().is_none() {
            return Err(camera_error("Unexpected end of camera file".to_string()));
        }
        match tokens.next()?.to_lowercase().as_str() {
            "time" => seconds = tokens.number()?,
            "camera_fixed" => camera = Some(parse_position(&mut tokens, "fixed")?),
            "camera_interpolated" => camera = Some(parse_position(&mut tokens, "interpolated")?),
            "camera_spline" => camera = Some(parse_position(&mut tokens, "spline")?),
            "target_fixed" => targets.push(parse_position(&mut tokens, "fixed")?),
            "target_interpolated" => targets.push(parse_position(&mut tokens, "interpolated")?),
            "target_spline" => targets.push(parse_position(&mut tokens, "spline")?),
            "event" => {
                let mut event_type = 0u8;
                let mut param = String::new();
                let mut event_time = 0.0;
                tokens.expect("{")?;
                while tokens.peek() != Some("}") {
                    if tokens.peek().is_none() {
                        return Err(camera_error("Unexpected end of camera file".to_string()));
                    }
                    match tokens.next()?.to_lowercase().as_str() {
                        "type" => {
                            let number = tokens.number()?;
                            if number.trunc() != number || number < 0.0 || number > 9.0 {
                                return Err(camera_error("Invalid camera event".to_string()));
                            }
                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            {
                                event_type = number as u8;
                            }
                        }
                        "param" => param = tokens.next()?,
                        "time" => event_time = tokens.number()?,
                        field => return Err(camera_error(format!("Unknown camera event property {field}"))),
                    }
                }
                tokens.expect("}")?;
                if event_time < 0.0 {
                    return Err(camera_error("Invalid camera event".to_string()));
                }
                if event_type == 1 {
                    let wait: f64 = param.parse().unwrap_or(f64::NAN);
                    if !wait.is_finite() || wait < 0.0 {
                        return Err(camera_error("Invalid camera wait duration".to_string()));
                    }
                }
                events.push(CameraEvent {
                    event_type,
                    param,
                    time: event_time,
                });
            }
            "fov" => {
                tokens.expect("{")?;
                while tokens.peek() != Some("}") {
                    if tokens.peek().is_none() {
                        return Err(camera_error("Unexpected end of camera file".to_string()));
                    }
                    match tokens.next()?.to_lowercase().as_str() {
                        "fov" => {
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                value = tokens.number()? as f32;
                            }
                        }
                        "startfov" => {
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                start = tokens.number()? as f32;
                            }
                        }
                        "endfov" => {
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                end = tokens.number()? as f32;
                            }
                        }
                        "time" => time = tokens.number()?,
                        field => return Err(camera_error(format!("Unknown camera FOV property {field}"))),
                    }
                }
                tokens.expect("}")?;
            }
            key => return Err(camera_error(format!("Unknown camera property {key}"))),
        }
    }
    tokens.expect("}")?;
    let Some(position) = camera else {
        return Err(camera_error("Invalid camera definition".to_string()));
    };
    if tokens.peek().is_some()
        || seconds <= 0.0
        || time < 0.0
        || [value, start, end].iter().any(|fov| !(*fov > 0.0 && *fov < 180.0))
    {
        return Err(camera_error("Invalid camera definition".to_string()));
    }
    for event in &events {
        if event.event_type == 4 && !targets.iter().any(|target| target.path().name == event.param) {
            return Err(camera_error(format!("Unknown camera target {}", event.param)));
        }
    }
    Ok(CameraDefinition {
        seconds,
        position,
        targets,
        events,
        fov: CameraFov {
            value,
            start,
            end,
            time,
        },
    })
}

fn quote(value: &str) -> Result<String, ClientError> {
    if value.chars().any(|character| matches!(character, '"' | '\r' | '\n')) {
        return Err(camera_error(
            "Camera names cannot contain quotes or newlines".to_string(),
        ));
    }
    Ok(format!("\"{value}\""))
}

fn vector_text(value: Vec3) -> String {
    format!("( {} {} {} )", value.x, value.y, value.z)
}

/// Serialize a camera definition (donor `serializeCamera`).
pub fn serialize_camera(camera: &CameraDefinition) -> Result<String, ClientError> {
    let write = |path: &CameraPosition, prefix: &str| -> Result<String, ClientError> {
        let fields = path.path();
        let mut text = format!(
            "{}_{} {{\nname {}\ntime {}\nbaseVelocity {}\n",
            prefix,
            match path {
                CameraPosition::Fixed { .. } => "fixed",
                CameraPosition::Interpolated { .. } => "interpolated",
                CameraPosition::Spline { .. } => "spline",
            },
            quote(&fields.name)?,
            fields.time,
            fields.base_velocity
        );
        for velocity in &fields.velocities {
            text.push_str(&format!(
                "velocity {} {} {}\n",
                velocity.start, velocity.duration, velocity.speed
            ));
        }
        match path {
            CameraPosition::Fixed { point, .. } => text.push_str(&format!("pos {}\n", vector_text(*point))),
            CameraPosition::Interpolated { start, end, .. } => {
                text.push_str(&format!(
                    "startPos {}\nendPos {}\n",
                    vector_text(*start),
                    vector_text(*end)
                ));
            }
            CameraPosition::Spline {
                granularity, points, ..
            } => {
                text.push_str(&format!(
                    "target {{\ngranularity {}\n{}\n}}\n",
                    granularity,
                    points
                        .iter()
                        .map(|point| vector_text(*point))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        text.push_str("}\n");
        Ok(text)
    };
    let mut text = format!("cameraPathDef {{\ntime {}\n", camera.seconds);
    text.push_str(&write(&camera.position, "camera")?);
    for target in &camera.targets {
        text.push_str(&write(target, "target")?);
    }
    for event in &camera.events {
        text.push_str(&format!(
            "event {{\ntype {}\nparam {}\ntime {}\n}}\n",
            event.event_type,
            quote(&event.param)?,
            event.time
        ));
    }
    text.push_str(&format!(
        "fov {{\nfov {}\nstartFOV {}\nendFOV {}\ntime {}\n}}\n}}\n",
        camera.fov.value, camera.fov.start, camera.fov.end, camera.fov.time
    ));
    Ok(text)
}

fn spline_points(points: &[Vec3], granularity: f32) -> Result<Vec<Vec3>, ClientError> {
    let mut result = Vec::new();
    for control in 3..points.len() {
        let mut t = 0.0f32;
        while f64::from(t) < 1.001 {
            let time = f64::from(t);
            #[allow(clippy::cast_possible_truncation)]
            let weights = [
                ((1.0 - time).powi(3) / 6.0) as f32,
                ((3.0 * time.powi(3) - 6.0 * time.powi(2) + 4.0) / 6.0) as f32,
                ((-3.0 * time.powi(3) + 3.0 * time.powi(2) + 3.0 * time + 1.0) / 6.0) as f32,
                ((time.powi(3)) / 6.0) as f32,
            ];
            let mut point = vec3(0.0, 0.0, 0.0);
            for (offset, weight) in weights.iter().enumerate() {
                let Some(source) = points.get(control - 3 + offset) else {
                    return Err(camera_error("Invalid spline control span".to_string()));
                };
                #[allow(clippy::cast_possible_truncation)]
                {
                    point.x = (f64::from(point.x) + f64::from(source.x) * f64::from(*weight)) as f32;
                    point.y = (f64::from(point.y) + f64::from(source.y) * f64::from(*weight)) as f32;
                    point.z = (f64::from(point.z) + f64::from(source.z) * f64::from(*weight)) as f32;
                }
            }
            result.push(point);
            t += granularity;
        }
    }
    Ok(result)
}

/// Playback cursor over one path.
struct PositionPlayback {
    path: CameraPosition,
    duration: f64,
    velocities: Vec<CameraVelocity>,
    points: Vec<Vec3>,
    distances: Vec<f64>,
    distance: f64,
    start_time: f64,
    last_time: f64,
    traveled: f64,
}

impl PositionPlayback {
    fn new(path: CameraPosition, duration: f64, velocities: Option<Vec<CameraVelocity>>) -> Result<Self, ClientError> {
        let velocities = velocities.unwrap_or_else(|| path.path().velocities.clone());
        let points = match &path {
            CameraPosition::Spline {
                points, granularity, ..
            } => spline_points(points, *granularity)?,
            CameraPosition::Fixed { point, .. } => vec![*point],
            CameraPosition::Interpolated { start, end, .. } => vec![*start, *end],
        };
        let mut distances = vec![0.0];
        let mut distance = 0.0;
        for index in 1..points.len() {
            distance += f64::from(length3(sub3(points[index], points[index - 1])));
            distances.push(distance);
        }
        Ok(Self {
            path,
            duration,
            velocities,
            points,
            distances,
            distance,
            start_time: 0.0,
            last_time: 0.0,
            traveled: 0.0,
        })
    }

    fn start(&mut self, time: f64, duration: Option<f64>) {
        self.start_time = time;
        self.last_time = time;
        self.traveled = 0.0;
        if let Some(duration) = duration {
            self.duration = duration;
        }
    }

    fn sample(&mut self, time: f64) -> Vec3 {
        match &self.path {
            CameraPosition::Fixed { point, .. } => *point,
            CameraPosition::Interpolated { start, end, .. } => {
                let elapsed = time - self.start_time;
                let velocity = self
                    .velocities
                    .iter()
                    .find(|velocity| elapsed >= velocity.start && elapsed <= velocity.start + velocity.duration)
                    .map_or_else(
                        || {
                            if self.duration == 0.0 {
                                0.0
                            } else {
                                self.distance / (self.duration / 1000.0)
                            }
                        },
                        |velocity| velocity.speed,
                    );
                self.traveled += 0.0f64.max(time - self.last_time) / 1000.0 * velocity;
                self.last_time = time;
                let fraction = if self.distance == 0.0 {
                    0.0
                } else {
                    (self.traveled / self.distance).clamp(0.0, 1.0)
                };
                #[allow(clippy::cast_possible_truncation)]
                let fraction = fraction as f32;
                add3(scale3(*start, 1.0 - fraction), scale3(*end, fraction))
            }
            CameraPosition::Spline { .. } => {
                let desired = if self.duration == 0.0 {
                    self.distance
                } else {
                    (time - self.start_time) / self.duration * self.distance
                };
                let mut index = self
                    .distances
                    .iter()
                    .position(|distance| *distance >= desired)
                    .unwrap_or(self.points.len().saturating_sub(1));
                if self.distances.iter().all(|distance| *distance < desired) {
                    index = self.points.len().saturating_sub(1);
                }
                let (Some(low), Some(high), Some(lo), Some(hi)) = (
                    self.points.get(index.saturating_sub(1)),
                    self.points.get(index + 1),
                    self.distances.get(index.saturating_sub(1)),
                    self.distances.get(index + 1),
                ) else {
                    return self.points.get(index).copied().unwrap_or(vec3(0.0, 0.0, 0.0));
                };
                if index == 0 || hi <= lo {
                    return self.points.get(index).copied().unwrap_or(vec3(0.0, 0.0, 0.0));
                }
                let fraction = (desired - lo) / (hi - lo);
                #[allow(clippy::cast_possible_truncation)]
                let fraction = fraction as f32;
                add3(scale3(*low, 1.0 - fraction), scale3(*high, fraction))
            }
        }
    }
}

/// One playback sample (donor `CameraSample`).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraSample {
    /// Camera origin.
    pub origin: Vec3,
    /// Normalized view direction.
    pub direction: Vec3,
    /// Horizontal FOV in degrees.
    pub fov: f32,
    /// Events firing at this sample.
    pub events: Vec<CameraEvent>,
}

/// Spline camera playback (donor `CameraPlayback`).
pub struct CameraPlayback {
    /// Played definition.
    pub definition: CameraDefinition,
    /// Start time in milliseconds.
    pub start_time: f64,
    camera: PositionPlayback,
    targets: Vec<PositionPlayback>,
    triggered: HashSet<usize>,
    active_target: usize,
    stopped: bool,
    total_ms: f64,
    last_time: f64,
}

impl CameraPlayback {
    /// Start playback at `start_time` milliseconds.
    pub fn new(definition: CameraDefinition, start_time: f64) -> Result<Self, ClientError> {
        let waits: Vec<&CameraEvent> = definition.events.iter().filter(|event| event.event_type == 1).collect();
        let total_ms = definition.seconds * 1000.0
            + waits
                .iter()
                .map(|event| event.param.parse::<f64>().unwrap_or(0.0) * 1000.0)
                .sum::<f64>();
        let mut velocities = definition.position.path().velocities.clone();
        velocities.extend(waits.iter().map(|event| CameraVelocity {
            start: event.time,
            duration: event.param.parse::<f64>().unwrap_or(0.0) * 1000.0,
            speed: 0.0,
        }));
        let mut camera = PositionPlayback::new(
            definition.position.clone(),
            definition.seconds * 1000.0,
            Some(velocities),
        )?;
        camera.start(start_time, None);
        let total = total_ms;
        let mut targets = Vec::new();
        for target in &definition.targets {
            let duration = if target.path().time == 0.0 {
                total
            } else {
                target.path().time
            };
            let mut playback = PositionPlayback::new(target.clone(), duration, None)?;
            playback.start(start_time, None);
            targets.push(playback);
        }
        let mut playback = Self {
            definition,
            start_time,
            camera,
            targets,
            triggered: HashSet::new(),
            active_target: 0,
            stopped: false,
            total_ms,
            last_time: start_time,
        };
        let changes: Vec<CameraEvent> = playback
            .definition
            .events
            .iter()
            .filter(|event| event.event_type == 4)
            .cloned()
            .collect();
        let mut time_so_far = 0.0;
        for (index, event) in changes.iter().enumerate() {
            let duration = changes
                .get(index + 1)
                .map_or(playback.total_ms - time_so_far, |next| next.time);
            let target_index = playback
                .definition
                .targets
                .iter()
                .position(|target| target.path().name == event.param);
            if let Some(target_index) = target_index {
                if let Some(target) = playback.targets.get_mut(target_index) {
                    target.start(start_time, Some(duration));
                }
                playback.active_target = target_index;
            }
            time_so_far += duration;
        }
        Ok(playback)
    }

    /// Sample playback at `time` milliseconds (monotonic clock).
    pub fn sample(&mut self, time: f64) -> Result<Option<CameraSample>, ClientError> {
        if !time.is_finite() || time < self.last_time {
            return Err(camera_error("Camera playback requires a monotonic clock".to_string()));
        }
        self.last_time = time;
        if self.stopped || ((time - self.start_time) / 1000.0).trunc() > self.total_ms / 1000.0 {
            return Ok(None);
        }
        let mut events = Vec::new();
        for (index, event) in self.definition.events.iter().enumerate() {
            if !self.triggered.contains(&index) && time >= self.start_time + event.time {
                self.triggered.insert(index);
                events.push(event.clone());
                if event.event_type == 9 {
                    self.stopped = true;
                    return Ok(None);
                }
                if event.event_type == 4 {
                    let target = self
                        .definition
                        .targets
                        .iter()
                        .position(|target| target.path().name == event.param);
                    if let Some(target_index) = target {
                        self.active_target = target_index;
                        if let Some(target) = self.targets.get_mut(target_index) {
                            target.start(self.start_time + event.time, None);
                        }
                    }
                }
            }
        }
        let origin = self.camera.sample(time);
        let target = match self.targets.get_mut(self.active_target) {
            Some(target) => target.sample(time),
            None => origin,
        };
        let fov = &self.definition.fov;
        let fraction = if fov.time == 0.0 {
            0.0
        } else {
            ((time - self.start_time) / fov.time).clamp(0.0, 1.0)
        };
        #[allow(clippy::cast_possible_truncation)]
        let sample_fov = if fov.time == 0.0 {
            fov.value
        } else {
            (f64::from(fov.start) + (f64::from(fov.end) - f64::from(fov.start)) * fraction) as f32
        };
        Ok(Some(CameraSample {
            origin,
            direction: normalize3(sub3(target, origin)),
            fov: sample_fov,
            events,
        }))
    }
}

/// Loaded spline camera with at most one active playback (donor
/// `ApplicationSplineCamera`, synchronous: file IO stays with the host).
#[derive(Default)]
pub struct SplineCamera {
    definition: Option<CameraDefinition>,
    playback: Option<CameraPlayback>,
}

impl SplineCamera {
    /// Open an empty camera.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a definition from `.camera` text, returning the donor
    /// `Loaded camera ...` line.
    pub fn load_text(&mut self, path: &str, text: &str) -> Result<String, ClientError> {
        let definition = parse_camera(text)?;
        self.stop();
        let line = format!(
            "Loaded camera {path}: {}s, {} events\n",
            definition.seconds,
            definition.events.len()
        );
        self.definition = Some(definition);
        Ok(line)
    }

    /// Start playback at `now_ms`.
    pub fn start(&mut self, now_ms: f64) -> Result<(), ClientError> {
        let Some(definition) = self.definition.clone() else {
            return Err(camera_error("No camera loaded".to_string()));
        };
        self.playback = Some(CameraPlayback::new(definition, now_ms)?);
        Ok(())
    }

    /// Stop playback, keeping the definition.
    pub fn stop(&mut self) {
        self.playback = None;
    }

    /// Drop the definition and stop playback.
    pub fn reset(&mut self) {
        self.stop();
        self.definition = None;
    }

    /// Whether playback is active.
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.playback.is_some()
    }

    /// Loaded definition, if any.
    #[must_use]
    pub fn definition(&self) -> Option<&CameraDefinition> {
        self.definition.as_ref()
    }

    /// Serialize the loaded definition (donor `savecamera`).
    pub fn save_text(&self) -> Result<String, ClientError> {
        let Some(definition) = &self.definition else {
            return Err(camera_error("No camera loaded".to_string()));
        };
        serialize_camera(definition)
    }

    /// Override a scene camera with the playback sample at `now_ms`.
    ///
    /// Portal cameras pass through; exhausted playback stops and returns
    /// the input camera.
    pub fn apply(&mut self, camera: &SceneCamera, now_ms: f64) -> Result<SceneCamera, ClientError> {
        let Some(playback) = self.playback.as_mut() else {
            return Ok(*camera);
        };
        if camera.clip != CameraClip::None {
            return Ok(*camera);
        }
        let sample = playback.sample(now_ms)?;
        let Some(sample) = sample else {
            self.stop();
            return Ok(*camera);
        };
        let projection = camera.projection;
        let near = projection[14] / (projection[10] - 1.0);
        let far = projection[14] / (projection[10] + 1.0);
        let aspect = projection[5] / projection[0];
        #[allow(clippy::cast_possible_truncation)]
        let fov_y = ((f64::from(sample.fov) * std::f64::consts::PI / 360.0).tan() / f64::from(aspect)).atan() * 360.0
            / std::f64::consts::PI;
        #[allow(clippy::cast_possible_truncation)]
        let fov_y = fov_y as f32;
        Ok(SceneCamera {
            origin: sample.origin,
            axis: if length3(sample.direction) == 0.0 {
                camera.axis
            } else {
                angles_to_axis(vector_to_angles(sample.direction))
            },
            projection: perspective_projection(sample.fov, fov_y, far, near)?,
            viewport: camera.viewport,
            clip: camera.clip,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Rect;

    const FIXED_CAMERA: &str = r#"
cameraPathDef {
  time 5
  camera_fixed {
    name "cam"
    time 0
    baseVelocity 0
    pos ( 0 0 64 )
  }
  target_fixed {
    name "look"
    time 0
    baseVelocity 0
    pos ( 100 0 64 )
  }
  fov {
    fov 90
    startFOV 90
    endFOV 90
    time 0
  }
}
"#;

    fn scene_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(1.0, 2.0, 3.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: perspective_projection(90.0, 90.0, 4096.0, 4.0).unwrap(),
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    #[test]
    fn parses_serializes_and_plays_fixed_cameras() {
        let definition = parse_camera(FIXED_CAMERA).unwrap();
        assert_eq!(definition.seconds, 5.0);
        assert_eq!(definition.targets.len(), 1);
        let text = serialize_camera(&definition).unwrap();
        let reparsed = parse_camera(&text).unwrap();
        assert_eq!(reparsed.seconds, 5.0);
        assert_eq!(reparsed.targets.len(), 1);
        let mut camera = SplineCamera::new();
        let line = camera.load_text("demo.camera", FIXED_CAMERA).unwrap();
        assert!(line.starts_with("Loaded camera demo.camera: 5s, 0 events"));
        camera.start(0.0).unwrap();
        let overridden = camera.apply(&scene_camera(), 1000.0).unwrap();
        assert_eq!(overridden.origin, vec3(0.0, 0.0, 64.0));
        assert!(camera.apply(&scene_camera(), 30_000.0).unwrap().origin == scene_camera().origin);
        assert!(!camera.is_playing());
    }

    #[test]
    fn rejects_bad_definitions_and_clocks() {
        assert!(parse_camera("cameraPathDef { time 0 camera_fixed { pos ( 0 0 0 ) } }").is_err());
        assert!(parse_camera("cameraPathDef { bogus 1 }").is_err());
        let definition = parse_camera(FIXED_CAMERA).unwrap();
        let mut playback = CameraPlayback::new(definition, 1000.0).unwrap();
        assert!(playback.sample(500.0).is_err());
        let mut camera = SplineCamera::new();
        assert!(camera.start(0.0).is_err());
        assert!(camera.save_text().is_err());
    }

    #[test]
    fn interpolates_and_fires_events() {
        let text = r#"
camera {
  time 2
  camera_interpolated {
    name "cam"
    time 0
    baseVelocity 0
    startPos ( 0 0 0 )
    endPos ( 100 0 0 )
  }
  event {
    type 9
    param ""
    time 1500
  }
  fov {
    fov 90
    startFOV 60
    endFOV 120
    time 1000
  }
}
"#;
        let definition = parse_camera(text).unwrap();
        let mut playback = CameraPlayback::new(definition, 0.0).unwrap();
        let first = playback.sample(500.0).unwrap().unwrap();
        assert!((first.fov - 90.0).abs() < f32::EPSILON);
        assert!(first.origin.x > 0.0 && first.origin.x < 100.0);
        assert!(playback.sample(1600.0).unwrap().is_none());
    }
}
