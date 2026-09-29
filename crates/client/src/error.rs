//! Typed client errors.
//!
//! Donor provenance: error sites in `src/network/q1/prediction.ts`,
//! `src/network/q2/prediction.ts`, `src/movement/q3/prediction.ts`,
//! `src/content/q3/presentation/{prediction,view,refdef}.ts`,
//! `src/input/{user-command,seat,mouse,impulse}.ts`,
//! `src/audio/{mixer,types}.ts`, `src/ui/hud/q2-native.ts`, and
//! `src/render/scene/{view,submissions,source-sort}.ts`, `src/materials/*`,
//! `src/text/*`, and `src/media/*`.

use thiserror::Error;

/// Errors for headless client presentation, prediction, and input.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClientError {
    /// A command-history read asked for a command newer than the newest.
    #[error("command {requested} is newer than the newest recorded command {current}")]
    FutureCommand {
        /// Requested command number.
        requested: i32,
        /// Newest recorded command number.
        current: i32,
    },
    /// A command-history ring slot is missing (violated backup window).
    #[error("prediction command source violated its backup window at {0}")]
    MissingCommand(i32),
    /// Prediction history cannot replay: too few recorded commands.
    #[error("prediction history exhausted")]
    HistoryExhausted,
    /// A prediction frame has no usable command to replay.
    #[error("no pending prediction command")]
    NoPendingCommand,
    /// The predicted actor was removed during replay.
    #[error("predicted actor removed")]
    ActorRemoved,
    /// A viewport has no positive area.
    #[error("camera view requires a positive viewport")]
    EmptyViewport,
    /// A projection needs finite fields of view and ordered clip distances.
    #[error("camera requires finite fields of view and ordered positive clipping distances")]
    BadProjection,
    /// A refdef render-text table is malformed.
    #[error("refdef render text is invalid: {0}")]
    BadRenderText(String),
    /// A refdef area mask has the wrong length.
    #[error("refdef requires 32 area-mask bytes")]
    BadAreaMask,
    /// A draw-sort field is out of range.
    #[error("draw-sort field {field} value {value} exceeds its range")]
    BadDrawSort {
        /// Field name.
        field: &'static str,
        /// Offending value.
        value: u32,
    },
    /// A mouse sample needs a positive frame duration.
    #[error("mouse sample needs a positive frame duration")]
    BadMouseFrame,
    /// View angles must be finite.
    #[error("view angles must be finite")]
    BadViewAngles,
    /// A command frame and builder dialect differ.
    #[error("user-command frame and input dialect differ")]
    DialectMismatch,
    /// An input impulse must fit in a byte.
    #[error("input impulse must fit a byte")]
    BadImpulse,
    /// A sound channel number is invalid for its family.
    #[error("invalid source sound channel {channel} for family {family}")]
    BadSoundChannel {
        /// Family name.
        family: &'static str,
        /// Channel number.
        channel: i32,
    },
    /// No free mixer channel is available.
    #[error("no free sound channel")]
    NoFreeChannel,
    /// A HUD stat index is outside its block.
    #[error("HUD stat {index} is outside its block")]
    BadHudStat {
        /// Stat index.
        index: i32,
    },
    /// A HUD layout references an image outside configstrings.
    #[error("HUD image {index} is outside configstrings")]
    BadHudImage {
        /// Image index.
        index: i32,
    },
    /// A HUD layout references a configstring outside its table.
    #[error("HUD configstring {index} is outside its table")]
    BadHudConfigstring {
        /// Configstring index.
        index: i32,
    },
    /// A HUD layout references a client outside clientinfo.
    #[error("HUD client {index} is outside clientinfo")]
    BadHudClient {
        /// Client index.
        index: i32,
    },
    /// A HUD layout argument count exceeds source limits.
    #[error("HUD argument count exceeds source limits")]
    BadHudCount,
    /// An audio listener or entity index is outside its table.
    #[error("audio entity {index} is outside its table")]
    BadAudioEntity {
        /// Entity index.
        index: usize,
    },
    /// A value must be finite float32.
    #[error("{0} must be finite float32")]
    NotFinite(&'static str),
    /// A color component is outside `[0, 1]`.
    #[error("mark colors must be in [0, 1]")]
    BadMarkColor,
    /// A renderer readback has invalid RGBA dimensions.
    #[error("Renderer returned invalid RGBA frame dimensions")]
    BadCaptureFrame,
    /// A capture path escapes the capture directory.
    #[error("Capture path escapes the capture directory: {0}")]
    BadCapturePath(String),
    /// A capture write failed.
    #[error("Capture write failed: {0}")]
    BadCaptureWrite(String),
    /// No free sequenced screenshot slot remains.
    #[error("No free screenshot filename between shot0000 and shot9999")]
    NoFreeScreenshot,
    /// A capture operation failed (carries the donor message).
    #[error("{0}")]
    BadCapture(String),
    /// A camera file or playback step failed (carries the donor message).
    #[error("{0}")]
    BadCamera(String),
    /// A shader script failed to parse (carries the donor message).
    #[error("{0}")]
    BadShader(String),
    /// A material failed to compile or evaluate (carries the donor message).
    #[error("{0}")]
    BadMaterial(String),
    /// A text layout, atlas, or localization step failed (carries the donor message).
    #[error("{0}")]
    BadText(String),
    /// A font failed to load or validate (carries the donor message).
    #[error("{0}")]
    BadFont(String),
    /// A cinematic container or playback step failed (carries the donor message).
    #[error("{0}")]
    BadMedia(String),
}
