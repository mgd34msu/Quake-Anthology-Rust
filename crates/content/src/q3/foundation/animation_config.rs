//! Quake III foundation: animation config.
//!
//! Donor provenance: `src/content/q3/foundation/animation-config.ts`.

use crate::q3anim::{PlayerFootsteps, PlayerGender};
use qa_core::math::{vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;
use qa_world::movement::q3::constants::player_animation;

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// animation-config.ts: parse failure.
// ---------------------------------------------------------------------------

/// Animation config failure (donor `TextParseError` and `RangeError` throws).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnimationConfigError {
    /// Text parse failure with source position (`TextParseError`).
    Parse {
        /// Source name.
        source: String,
        /// 1-based line.
        line: i32,
        /// 1-based column.
        column: i32,
        /// Message.
        message: String,
    },
    /// Out-of-range value (donor `RangeError`).
    Range(String),
}

impl std::fmt::Display for AnimationConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse {
                source,
                line,
                column,
                message,
            } => write!(f, "{source}:{line}:{column}: {message}"),
            Self::Range(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AnimationConfigError {}

fn range(message: impl Into<String>) -> AnimationConfigError {
    AnimationConfigError::Range(message.into())
}

// ---------------------------------------------------------------------------
// animation-config.ts: CG_ParseAnimationFile.
// ---------------------------------------------------------------------------

pub(crate) const MAX_TEXT_BYTES: usize = 19_998;

pub(crate) const SOURCE_ANIMATION_COUNT: usize = 31;

pub(crate) const TOTAL_ANIMATION_COUNT: usize = 37;

pub(crate) const MAX_ANIMATIONS_SENTINEL: usize = 31;

/// One parsed animation row (`Animation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Animation {
    /// First frame.
    pub first_frame: i32,
    /// Frame count.
    pub num_frames: i32,
    /// Loop frames.
    pub loop_frames: i32,
    /// Milliseconds per frame.
    pub frame_lerp: i32,
    /// Initial lerp.
    pub initial_lerp: i32,
    /// Reversed playback.
    pub reversed: bool,
    /// Flip-flop playback.
    pub flipflop: bool,
}

/// Retained client cells a parse writes into (`PlayerAnimationTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationTarget {
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Head offset.
    pub head_offset: Vec3,
    /// Gender.
    pub gender: PlayerGender,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Animation cells.
    pub animations: [Animation; TOTAL_ANIMATION_COUNT],
}

impl Default for PlayerAnimationTarget {
    fn default() -> Self {
        Self {
            footsteps: PlayerFootsteps::Normal,
            head_offset: vec3(0.0, 0.0, 0.0),
            gender: PlayerGender::Male,
            fixed_legs: false,
            fixed_torso: false,
            animations: [Animation::default(); TOTAL_ANIMATION_COUNT],
        }
    }
}

/// Diagnostic warning with source position (`AnimationWarning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationWarning {
    /// 1-based line.
    pub line: i32,
    /// 1-based column.
    pub column: i32,
    /// Message.
    pub message: String,
}

/// Parsed player animation config (`PlayerAnimationConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationConfig {
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Head offset.
    pub head_offset: Vec3,
    /// Gender.
    pub gender: PlayerGender,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Indexed by player animation; the `MAX_ANIMATIONS` sentinel is `None`.
    pub animations: [Option<Animation>; TOTAL_ANIMATION_COUNT],
    /// Diagnostics.
    pub warnings: Vec<AnimationWarning>,
}

pub(crate) fn animation_cell(
    target: &mut PlayerAnimationTarget,
    index: usize,
) -> Result<&mut Animation, AnimationConfigError> {
    target
        .animations
        .get_mut(index)
        .ok_or_else(|| range(format!("Missing client animation cell {index}")))
}

pub(crate) fn cg_print(
    format: &str,
    args: &[&str],
    print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<String, AnimationConfigError> {
    let mut position = 0;
    let mut message = String::new();
    let mut rest = format;
    while let Some(hit) = rest.find("%s") {
        message.push_str(&rest[..hit]);
        message.push_str(args.get(position).copied().unwrap_or(""));
        position += 1;
        rest = &rest[hit + 2..];
    }
    message.push_str(rest);
    if message.chars().count() >= 1024 {
        return Err(range("CG_Printf exceeds its 1024-byte source buffer"));
    }
    if let Some(print) = print {
        print(&message);
    }
    Ok(message)
}

/// Parse into retained client cells (`parsePlayerAnimationConfig` with context).
pub fn parse_player_animation_config_into(
    target: &mut PlayerAnimationTarget,
    parser: &mut CommonParseState,
    text: &str,
    source: &str,
    mut print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<PlayerAnimationConfig, AnimationConfigError> {
    let mut warnings: Vec<AnimationWarning> = Vec::new();
    if text.is_empty() {
        return Err(AnimationConfigError::Parse {
            source: source.to_string(),
            line: 1,
            column: 1,
            message: "empty animation file".to_string(),
        });
    }
    if text.len() > MAX_TEXT_BYTES {
        let message = cg_print("File %s too long\n", &[source], print.as_deref_mut())?;
        return Err(AnimationConfigError::Parse {
            source: source.to_string(),
            line: 1,
            column: 1,
            message,
        });
    }
    let mut cursor = CommonParseCursor::new(text)?;
    target.footsteps = PlayerFootsteps::Normal;
    target.head_offset = vec3(0.0, 0.0, 0.0);
    target.gender = PlayerGender::Male;
    target.fixed_legs = false;
    target.fixed_torso = false;

    loop {
        let previous = cursor.offset();
        let token = parser.parse(&mut cursor)?;
        let directive = token.to_lowercase();
        if directive == "footsteps" {
            let value = parser.parse(&mut cursor)?;
            match value.to_lowercase().as_str() {
                "default" | "normal" => target.footsteps = PlayerFootsteps::Normal,
                "boot" => target.footsteps = PlayerFootsteps::Boot,
                "flesh" => target.footsteps = PlayerFootsteps::Flesh,
                "mech" => target.footsteps = PlayerFootsteps::Mech,
                "energy" => target.footsteps = PlayerFootsteps::Energy,
                _ => {
                    let message = cg_print(
                        "Bad footsteps parm in %s: %s\n",
                        &[source, &value],
                        print.as_deref_mut(),
                    )?;
                    warnings.push(AnimationWarning {
                        line: parser.line().wrapping_add(1),
                        column: 1,
                        message,
                    });
                }
            }
        } else if directive == "headoffset" {
            let x = game_atof(&parser.parse(&mut cursor)?)?;
            let y = game_atof(&parser.parse(&mut cursor)?)?;
            let z = game_atof(&parser.parse(&mut cursor)?)?;
            target.head_offset = vec3(x, y, z);
        } else if directive == "sex" {
            let first = parser
                .parse(&mut cursor)?
                .chars()
                .next()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .unwrap_or('\0');
            target.gender = if first == 'f' {
                PlayerGender::Female
            } else if first == 'n' {
                PlayerGender::Neuter
            } else {
                PlayerGender::Male
            };
        } else if directive == "fixedlegs" {
            target.fixed_legs = true;
        } else if directive == "fixedtorso" {
            target.fixed_torso = true;
        } else {
            let first = token.chars().next();
            if matches!(first, Some('0'..='9')) {
                cursor.set_offset(previous)?;
                break;
            }
            let message = cg_print("unknown token '%s' is %s\n", &[&token, source], print.as_deref_mut())?;
            warnings.push(AnimationWarning {
                line: parser.line().wrapping_add(1),
                column: 1,
                message,
            });
            if previous == cursor.offset() {
                return Err(range("CG animation prelude reached the source nonprogress cycle"));
            }
        }
    }

    let mut skip = 0i32;
    let mut index = 0usize;
    while index < SOURCE_ANIMATION_COUNT {
        let first_token = parser.parse(&mut cursor)?;
        if first_token.is_empty() {
            if (player_animation::TORSO_GETFLAG as usize..=player_animation::TORSO_NEGATIVE as usize).contains(&index) {
                let gesture = target.animations[player_animation::TORSO_GESTURE as usize];
                let animation = animation_cell(target, index)?;
                animation.first_frame = gesture.first_frame;
                animation.frame_lerp = gesture.frame_lerp;
                animation.initial_lerp = gesture.initial_lerp;
                animation.loop_frames = gesture.loop_frames;
                animation.num_frames = gesture.num_frames;
                animation.reversed = false;
                animation.flipflop = false;
                index += 1;
                continue;
            }
            break;
        }
        let first_frame = game_atoi(&first_token)?;
        animation_cell(target, index)?.first_frame = first_frame;
        if index == player_animation::LEGS_WALKCR as usize {
            let gesture = target.animations[player_animation::TORSO_GESTURE as usize].first_frame;
            skip = target.animations[index].first_frame.wrapping_sub(gesture);
        }
        if (player_animation::LEGS_WALKCR as usize..player_animation::TORSO_GETFLAG as usize).contains(&index) {
            let animation = animation_cell(target, index)?;
            animation.first_frame = animation.first_frame.wrapping_sub(skip);
        }
        let count_token = parser.parse(&mut cursor)?;
        if count_token.is_empty() {
            break;
        }
        let num_frames = game_atoi(&count_token)?;
        let animation = animation_cell(target, index)?;
        animation.num_frames = num_frames;
        animation.reversed = false;
        animation.flipflop = false;
        if animation.num_frames < 0 {
            animation.num_frames = animation.num_frames.wrapping_neg();
            animation.reversed = true;
        }
        let loop_token = parser.parse(&mut cursor)?;
        if loop_token.is_empty() {
            break;
        }
        animation_cell(target, index)?.loop_frames = game_atoi(&loop_token)?;
        let fps_token = parser.parse(&mut cursor)?;
        if fps_token.is_empty() {
            break;
        }
        let mut fps = game_atof(&fps_token)?;
        if fps == 0.0 {
            fps = 1.0;
        }
        let lerp = qvm_float_to_int((1000.0f64 / f64::from(fps)) as f32);
        let animation = animation_cell(target, index)?;
        animation.frame_lerp = lerp;
        animation.initial_lerp = lerp;
        index += 1;
    }
    if index != SOURCE_ANIMATION_COUNT {
        let message = cg_print("Error parsing animation file: %s", &[source], print)?;
        return Err(AnimationConfigError::Parse {
            source: source.to_string(),
            line: parser.line().wrapping_add(1),
            column: 1,
            message,
        });
    }

    let walk_cr = target.animations[player_animation::LEGS_WALKCR as usize];
    let back_cr = animation_cell(target, player_animation::LEGS_BACKCR as usize)?;
    *back_cr = walk_cr;
    back_cr.reversed = true;
    let walk = target.animations[player_animation::LEGS_WALK as usize];
    let back_walk = animation_cell(target, player_animation::LEGS_BACKWALK as usize)?;
    *back_walk = walk;
    back_walk.reversed = true;
    let flag_run = animation_cell(target, player_animation::FLAG_RUN as usize)?;
    flag_run.first_frame = 0;
    flag_run.num_frames = 16;
    flag_run.loop_frames = 16;
    flag_run.frame_lerp = 66;
    flag_run.initial_lerp = 66;
    flag_run.reversed = false;
    let flag_stand = animation_cell(target, player_animation::FLAG_STAND as usize)?;
    flag_stand.first_frame = 16;
    flag_stand.num_frames = 5;
    flag_stand.loop_frames = 0;
    flag_stand.frame_lerp = 50;
    flag_stand.initial_lerp = 50;
    flag_stand.reversed = false;
    let flag_run2 = animation_cell(target, player_animation::FLAG_STAND2RUN as usize)?;
    flag_run2.first_frame = 16;
    flag_run2.num_frames = 5;
    flag_run2.loop_frames = 1;
    flag_run2.frame_lerp = 66;
    flag_run2.initial_lerp = 66;
    flag_run2.reversed = true;

    let mut animations: [Option<Animation>; TOTAL_ANIMATION_COUNT] = [None; TOTAL_ANIMATION_COUNT];
    for (slot, cell) in animations.iter_mut().enumerate() {
        if slot != MAX_ANIMATIONS_SENTINEL {
            *cell = Some(target.animations[slot]);
        }
    }
    Ok(PlayerAnimationConfig {
        footsteps: target.footsteps,
        head_offset: target.head_offset,
        gender: target.gender,
        fixed_legs: target.fixed_legs,
        fixed_torso: target.fixed_torso,
        animations,
        warnings,
    })
}

/// Parse an owned config snapshot (`parsePlayerAnimationConfig` standalone).
pub fn parse_player_animation_config(text: &str, source: &str) -> Result<PlayerAnimationConfig, AnimationConfigError> {
    let mut target = PlayerAnimationTarget::default();
    let mut parser = CommonParseState::new();
    parse_player_animation_config_into(&mut target, &mut parser, text, source, None)
}

// ---------------------------------------------------------------------------
// bg_lib number scans (`src/core/game-numeric.ts` scalar paths).
// ---------------------------------------------------------------------------

pub(crate) struct NumberInput<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> NumberInput<'a> {
    fn new(text: &'a str) -> Result<Self, AnimationConfigError> {
        if text.chars().any(|c| c as u32 > 255) {
            return Err(range("Game numbers require byte characters"));
        }
        Ok(Self {
            bytes: text.as_bytes(),
            offset: 0,
        })
    }

    fn byte(&self) -> Result<i32, AnimationConfigError> {
        if self.offset > self.bytes.len() {
            return Err(range("Game number scan reads beyond its backing string"));
        }
        if self.offset == self.bytes.len() {
            return Ok(0);
        }
        let byte = self.bytes[self.offset];
        Ok(if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        })
    }

    fn take(&mut self) -> Result<i32, AnimationConfigError> {
        let byte = self.byte()?;
        self.offset += 1;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) -> Result<(), AnimationConfigError> {
        while self.byte()? <= 32 && self.byte()? != 0 {
            self.offset += 1;
        }
        Ok(())
    }

    fn sign(&mut self) -> Result<i32, AnimationConfigError> {
        let byte = self.byte()?;
        if byte != 43 && byte != 45 {
            return Ok(1);
        }
        self.offset += 1;
        Ok(if byte == 45 { -1 } else { 1 })
    }
}

pub(crate) fn read_game_float(input: &mut NumberInput<'_>) -> Result<f32, AnimationConfigError> {
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0.0);
    }
    let sign = input.sign()?;
    let mut value = 0.0f32;
    let mut character = input.byte()?;
    if input.byte()? != 46 {
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + (character - 48) as f32;
        }
    } else {
        input.offset += 1;
    }
    if character == 46 {
        let mut fraction = 0.1f32;
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value += (character - 48) as f32 * fraction;
            fraction *= 0.1f32;
        }
    }
    Ok(value * sign as f32)
}

/// `bg_lib` atof: decimal prefix only, binary32 operations (`gameAtof`).
pub fn game_atof(text: &str) -> Result<f32, AnimationConfigError> {
    read_game_float(&mut NumberInput::new(text)?)
}

/// `bg_lib` atoi: wraps every integer digit operation (`gameAtoi`).
pub fn game_atoi(text: &str) -> Result<i32, AnimationConfigError> {
    let mut input = NumberInput::new(text)?;
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0);
    }
    let sign = input.sign()?;
    let mut value = 0i32;
    loop {
        let character = input.take()?;
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(character - 48);
    }
    Ok(value.wrapping_mul(sign))
}

// ---------------------------------------------------------------------------
// COM_Parse (`CommonParseCursor`/`CommonParseState` paths in
// `src/core/common-parse.ts` used by the animation config parser).
// ---------------------------------------------------------------------------

pub(crate) const COM_TOKEN_MAX: usize = 1024;

/// Byte cursor over a Latin-1 source string (`CommonParseCursor`).
#[derive(Debug, Clone)]
pub struct CommonParseCursor {
    bytes: Vec<u8>,
    terminator: usize,
    offset: Option<usize>,
}

impl CommonParseCursor {
    /// Build a cursor; non-Latin-1 sources are rejected.
    pub fn new(source: &str) -> Result<Self, AnimationConfigError> {
        if source.chars().any(|c| c as u32 > 255) {
            return Err(range("COM_Parse source is not a Latin-1 byte string"));
        }
        let bytes = source.as_bytes().to_vec();
        let terminator = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(Self {
            bytes,
            terminator,
            offset: Some(0),
        })
    }

    /// Current offset; `None` once exhausted.
    #[must_use]
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// Reposition the cursor within the C byte string.
    pub fn set_offset(&mut self, value: Option<usize>) -> Result<(), AnimationConfigError> {
        if let Some(offset) = value {
            if offset > self.terminator {
                return Err(range("COM_Parse cursor is outside its C byte string"));
            }
        }
        self.offset = value;
        Ok(())
    }

    fn signed_byte(&self, offset: usize) -> i32 {
        if offset >= self.bytes.len() {
            return 0;
        }
        let byte = self.bytes[offset];
        if byte >= 128 {
            i32::from(byte) - 256
        } else {
            i32::from(byte)
        }
    }

    fn char_at(&self, offset: usize) -> char {
        self.bytes.get(offset).map_or('\0', |b| *b as char)
    }
}

/// Tokenizer state with the shared token and line (`CommonParseState`).
#[derive(Debug, Clone, Default)]
pub struct CommonParseState {
    token: String,
    line: i32,
}

impl CommonParseState {
    /// Fresh tokenizer state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current 0-based line.
    #[must_use]
    pub fn line(&self) -> i32 {
        self.line
    }

    /// Current shared token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    fn push_char(&mut self, c: char) {
        if self.token.chars().count() < COM_TOKEN_MAX {
            self.token.push(c);
        }
    }

    /// Parse one token (`COM_Parse` with line breaks allowed).
    pub fn parse(&mut self, cursor: &mut CommonParseCursor) -> Result<String, AnimationConfigError> {
        let Some(mut data) = cursor.offset else {
            self.token.clear();
            return Ok(String::new());
        };
        self.token.clear();
        let mut has_new_lines = false;
        let mut c: i32;
        loop {
            loop {
                c = cursor.signed_byte(data);
                if c > 32 {
                    break;
                }
                if c == 0 {
                    cursor.offset = None;
                    return Ok(String::new());
                }
                if c == 10 {
                    self.line = self.line.wrapping_add(1);
                    has_new_lines = true;
                }
                data += 1;
            }
            let _ = has_new_lines;
            if c == 47 && cursor.signed_byte(data + 1) == 47 {
                data += 2;
                while {
                    c = cursor.signed_byte(data);
                    c != 0 && c != 10
                } {
                    data += 1;
                }
            } else if c == 47 && cursor.signed_byte(data + 1) == 42 {
                data += 2;
                while cursor.signed_byte(data) != 0
                    && (cursor.signed_byte(data) != 42 || cursor.signed_byte(data + 1) != 47)
                {
                    data += 1;
                }
                if cursor.signed_byte(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        if c == 34 {
            data += 1;
            loop {
                c = cursor.signed_byte(data);
                data += 1;
                if c == 34 || c == 0 {
                    if self.token.chars().count() == COM_TOKEN_MAX {
                        return Err(range("COM_Parse quoted token terminator exceeds 1024-byte storage"));
                    }
                    cursor.offset = if c == 0 { None } else { Some(data) };
                    return Ok(std::mem::take(&mut self.token));
                }
                self.push_char(cursor.char_at(data - 1));
            }
        }
        loop {
            self.push_char(cursor.char_at(data));
            data += 1;
            c = cursor.signed_byte(data);
            if c == 10 {
                self.line = self.line.wrapping_add(1);
            }
            if c <= 32 {
                break;
            }
        }
        if self.token.chars().count() == COM_TOKEN_MAX {
            self.token.clear();
        }
        cursor.offset = Some(data);
        Ok(std::mem::take(&mut self.token))
    }
}

#[cfg(test)]
mod tests {
    use crate::q3anim::{PlayerFootsteps, PlayerGender};
    use qa_core::math::vec3;

    use super::*;

    fn animation_fixture() -> String {
        let mut text = String::from("sex f\nfootsteps boot\nheadoffset 1 2 3\nfixedlegs\nfixedtorso\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 6 0 10\n"));
        }
        text
    }

    #[test]
    fn parses_animation_config_fixture() {
        let config = parse_player_animation_config(&animation_fixture(), "<test>").unwrap();
        assert_eq!(config.footsteps, PlayerFootsteps::Boot);
        assert_eq!(config.gender, PlayerGender::Female);
        assert_eq!(config.head_offset, vec3(1.0, 2.0, 3.0));
        assert!(config.fixed_legs);
        assert!(config.fixed_torso);
        assert!(config.warnings.is_empty());
        let death = config.animations[0].as_ref().unwrap();
        assert_eq!(death.first_frame, 0);
        assert_eq!(death.num_frames, 6);
        assert_eq!(death.frame_lerp, 100);
        assert_eq!(death.initial_lerp, 100);
        assert!(config.animations[31].is_none());
        let walk_cr = config.animations[13].as_ref().unwrap();
        assert_eq!(walk_cr.first_frame, 6);
        let back_cr = config.animations[32].as_ref().unwrap();
        assert_eq!(back_cr.first_frame, 6);
        assert!(back_cr.reversed);
        let back_walk = config.animations[33].as_ref().unwrap();
        assert_eq!(
            back_walk.first_frame,
            config.animations[14].as_ref().unwrap().first_frame
        );
        assert!(back_walk.reversed);
        assert_eq!(
            config.animations[34].as_ref().unwrap(),
            &Animation {
                first_frame: 0,
                num_frames: 16,
                loop_frames: 16,
                frame_lerp: 66,
                initial_lerp: 66,
                reversed: false,
                flipflop: false,
            }
        );
        assert_eq!(config.animations[35].as_ref().unwrap().first_frame, 16);
        assert!(config.animations[36].as_ref().unwrap().reversed);
    }

    #[test]
    fn animation_config_directives_warnings_and_errors() {
        let mut text = String::from("sex n\nfootsteps squeak\nmystery\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 6 0 10\n"));
        }
        let config = parse_player_animation_config(&text, "<test>").unwrap();
        assert_eq!(config.gender, PlayerGender::Neuter);
        assert_eq!(config.warnings.len(), 2);
        assert!(config.warnings[0].message.contains("Bad footsteps"));
        assert!(config.warnings[1].message.contains("unknown token"));

        let err = parse_player_animation_config("", "<s>").unwrap_err();
        assert!(matches!(err, AnimationConfigError::Parse { line: 1, .. }));

        let long = "x".repeat(19_999);
        assert!(parse_player_animation_config(&long, "<s>").is_err());

        let mut short = String::from("sex m\n");
        for frame in 0..20 {
            short.push_str(&format!("{frame} 6 0 10\n"));
        }
        let err = parse_player_animation_config(&short, "<s>").unwrap_err();
        assert!(matches!(err, AnimationConfigError::Parse { .. }));

        let err = parse_player_animation_config("foo", "<s>").unwrap_err();
        assert!(matches!(err, AnimationConfigError::Range(_)));
    }

    #[test]
    fn animation_config_row_edge_cases() {
        let mut text = String::new();
        for frame in 0..31 {
            if frame == 0 {
                text.push_str("5 -4 2 0\n");
            } else {
                text.push_str(&format!("{frame} 6 0 10\n"));
            }
        }
        let config = parse_player_animation_config(&text, "<t>").unwrap();
        let first = config.animations[0].as_ref().unwrap();
        assert_eq!(first.num_frames, 4);
        assert!(first.reversed);
        assert_eq!(first.frame_lerp, 1000);

        let mut partial = String::new();
        for frame in 0..25 {
            partial.push_str(&format!("{frame} 6 0 10\n"));
        }
        let config = parse_player_animation_config(&partial, "<t>").unwrap();
        let gesture = *config.animations[6].as_ref().unwrap();
        for index in 25..31 {
            let row = config.animations[index].as_ref().unwrap();
            assert_eq!(row.first_frame, gesture.first_frame);
            assert!(!row.reversed);
        }

        let mut target = PlayerAnimationTarget::default();
        let mut parser = CommonParseState::new();
        let mut printed = Vec::new();
        let mut sink = |message: &str| printed.push(message.to_string());
        let config = parse_player_animation_config_into(
            &mut target,
            &mut parser,
            "bogus\n0 6 0 10\n1 6 0 10\n2 6 0 10\n3 6 0 10\n4 6 0 10\n5 6 0 10\n6 6 0 10\n7 6 0 10\n8 6 0 10\n9 6 0 10\n10 6 0 10\n11 6 0 10\n12 6 0 10\n13 6 0 10\n14 6 0 10\n15 6 0 10\n16 6 0 10\n17 6 0 10\n18 6 0 10\n19 6 0 10\n20 6 0 10\n21 6 0 10\n22 6 0 10\n23 6 0 10\n24 6 0 10\n25 6 0 10\n26 6 0 10\n27 6 0 10\n28 6 0 10\n29 6 0 10\n30 6 0 10\n",
            "<t>",
            Some(&mut sink),
        )
        .unwrap();
        assert_eq!(printed.len(), 1);
        assert!(printed[0].contains("unknown token"));
        assert_eq!(config.warnings.len(), 1);
        assert_eq!(target.animations[0].first_frame, 0);
    }

    #[test]
    fn com_parse_tokens_comments_and_limits() {
        let mut parser = CommonParseState::new();
        let mut cursor = CommonParseCursor::new("hello // rest\n\"quoted token\" /* block */ word").unwrap();
        assert_eq!(parser.parse(&mut cursor).unwrap(), "hello");
        assert_eq!(parser.parse(&mut cursor).unwrap(), "quoted token");
        assert_eq!(parser.parse(&mut cursor).unwrap(), "word");
        assert_eq!(parser.parse(&mut cursor).unwrap(), "");
        assert_eq!(parser.line(), 1);

        let mut parser = CommonParseState::new();
        let mut cursor = CommonParseCursor::new(&"w".repeat(2000)).unwrap();
        assert_eq!(parser.parse(&mut cursor).unwrap(), "");

        let mut parser = CommonParseState::new();
        let mut cursor = CommonParseCursor::new(&format!("\"{}\"", "q".repeat(1024))).unwrap();
        assert!(parser.parse(&mut cursor).is_err());

        assert!(CommonParseCursor::new("héllo \u{0100}").is_err());
    }

    #[test]
    fn game_numbers_match_bg_lib() {
        assert_eq!(game_atof("3.5").unwrap(), 3.5);
        assert_eq!(game_atof("  -12x").unwrap(), -12.0);
        assert_eq!(game_atof("abc").unwrap(), 0.0);
        assert_eq!(game_atof(".5").unwrap(), 0.5);
        assert_eq!(game_atof("").unwrap(), 0.0);
        assert_eq!(game_atoi("  +42 ").unwrap(), 42);
        assert_eq!(game_atoi("-7up").unwrap(), -7);
        assert_eq!(game_atoi("2147483648").unwrap(), i32::MIN);
        assert_eq!(game_atoi("").unwrap(), 0);
        assert!(game_atof("ÿ\u{0100}").is_err());
    }
}
